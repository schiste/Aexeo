use std::{fs, path::Path, process::Command};

fn input() -> serde_json::Value {
    serde_json::json!({
        "schema": "content-gate.change/v1", "change_id": "example",
        "created_at": "2026-01-01T00:00:00Z", "origin": {"kind": "manual"},
        "base": {"kind": "opaque", "revision": "r1"},
        "entries": [{"entity_id": "article", "operation": "create", "media_type": "text/plain",
            "after": {"inline": "abc", "sha256": "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"}}]
    })
}

fn run(
    root: &Path,
    change: &serde_json::Value,
    preset: &str,
    extra: &[&str],
) -> std::process::Output {
    let change_file = root.join("change.json");
    let verdict_file = root.join("verdict.json");
    let preset_file = root.join("preset.toml");
    fs::write(change_file.clone(), serde_json::to_vec(change).unwrap()).unwrap();
    fs::write(&preset_file, preset).unwrap();
    Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .args([
            "content-change",
            "--preset",
            "test",
            "--preset-source",
            &format!("test={}", preset_file.display()),
        ])
        .args(extra)
        .env("CONTENT_GATE_CHANGE_FILE", change_file)
        .env("CONTENT_GATE_VERDICT_FILE", verdict_file)
        .output()
        .unwrap()
}

fn html_change(source: &str, digest: &str) -> serde_json::Value {
    let mut change = input();
    change["entries"][0]["media_type"] = "text/html".into();
    change["entries"][0]["after"] = serde_json::json!({"inline": source, "sha256": digest});
    change
}

#[test]
fn html_main_scope_excludes_navigation_and_revalidates_only_the_same_scope() {
    let dir = tempfile::tempdir().unwrap();
    let change = html_change(
        "<nav>word</nav><main><p>safe</p></main><footer>word</footer>",
        "1a092dfafca8e00d06f070ace94d81061c2749235da07c5aca309f5dc0d106b2",
    );
    let policy =
        "version=1\nname='test'\n[[gates]]\nid='terms'\nkind='forbidden_terms'\nterms=['word']\n";
    let document = run(dir.path(), &change, policy, &[]);
    assert_eq!(
        document.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&document.stderr)
    );
    let path = dir.path().join("verdict.json");
    let doc_copy = dir.path().join("document.json");
    fs::copy(&path, &doc_copy).unwrap();
    let main = run(
        dir.path(),
        &change,
        policy,
        &["--html-scope", "main", "--telemetry"],
    );
    assert_eq!(
        main.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&main.stderr)
    );
    let verdict: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(verdict["extensions"]["aexeo.html-content-scope.v1"], "main");
    assert!(verdict["findings"].as_array().unwrap().is_empty());
    let event: serde_json::Value = serde_json::from_slice(&main.stderr).unwrap();
    assert_eq!(
        event,
        serde_json::json!({"event":"content_change_html_projection", "version":1, "scope":"main", "eligible_entries":1})
    );
    let main_copy = dir.path().join("main.json");
    fs::copy(&path, &main_copy).unwrap();
    let main_path = main_copy.to_str().unwrap();
    let doc_path = doc_copy.to_str().unwrap();
    assert_eq!(
        run(
            dir.path(),
            &change,
            policy,
            &["--html-scope", "main", "--verify-verdict", main_path]
        )
        .status
        .code(),
        Some(0)
    );
    let before = fs::read(&path).unwrap();
    for options in [
        vec!["--html-scope", "main", "--verify-verdict", doc_path],
        vec!["--verify-verdict", main_path],
    ] {
        let result = run(dir.path(), &change, policy, &options);
        assert_eq!(result.status.code(), Some(2));
        assert!(
            String::from_utf8_lossy(&result.stderr).contains("previous verdict does not match")
        );
        assert_eq!(fs::read(&path).unwrap(), before);
    }
}

#[test]
fn html_main_scope_incomplete_is_not_checked() {
    let dir = tempfile::tempdir().unwrap();
    let policy =
        "version=1\nname='test'\n[[gates]]\nid='terms'\nkind='forbidden_terms'\nterms=['word']\n";
    for (source, digest) in [
        (
            "<p>safe</p>",
            "adc3b3e94ea3095bd354fb8397e7927993eb5d246add0eee4887780a0455ee19",
        ),
        (
            "<main>safe</main><main>safe</main>",
            "7cd94121c9a7db06b8caafb89dad731e41f0f681e9fa927de212b545935ff8a8",
        ),
    ] {
        let result = run(
            dir.path(),
            &html_change(source, digest),
            policy,
            &["--html-scope", "main"],
        );
        assert_eq!(result.status.code(), Some(2));
        let verdict: serde_json::Value =
            serde_json::from_slice(&fs::read(dir.path().join("verdict.json")).unwrap()).unwrap();
        assert_eq!(verdict["status"], "not_checked");
        assert!(verdict["findings"].as_array().unwrap().is_empty());
    }
}

#[test]
fn html_main_scope_link_revalidation_checks_scope_before_observation() {
    let dir = tempfile::tempdir().unwrap();
    let change = html_change(
        "<nav><a href='/nav'>nav</a></nav><main><a href='/body'>body</a></main>",
        "e0e2dde15c5da6b57d63ee84eba388e19d3eb1c0411f0a74f485b45142fbc4a6",
    );
    let policy = "version=1\nname='test'\n[[gates]]\nid='links'\nkind='link_reachability'\n";
    let options = [
        "--html-scope",
        "main",
        "--check-link-reachability",
        "--telemetry",
    ];
    let first = run(dir.path(), &change, policy, &options);
    assert_eq!(
        first.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let events: Vec<serde_json::Value> = String::from_utf8(first.stderr)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert!(
        events
            .iter()
            .any(|e| e["event"] == "content_link_reachability" && e["destinations"] == 1)
    );
    let previous = dir.path().join("main-links.json");
    fs::copy(dir.path().join("verdict.json"), &previous).unwrap();
    let path = previous.to_str().unwrap();
    let same = [
        "--html-scope",
        "main",
        "--check-link-reachability",
        "--verify-verdict",
        path,
    ];
    assert_eq!(
        run(dir.path(), &change, policy, &same).status.code(),
        Some(0)
    );
    let different = [
        "--check-link-reachability",
        "--verify-verdict",
        path,
        "--telemetry",
    ];
    let result = run(dir.path(), &change, policy, &different);
    assert_ne!(result.status.code(), Some(0));
    assert!(!String::from_utf8_lossy(&result.stderr).contains("content_link_reachability"));
}

#[test]
fn html_main_scope_receipts_bind_scope_and_revalidate() {
    let dir = tempfile::tempdir().unwrap();
    let change = html_change(
        "<nav>word</nav><main><p>safe</p></main><footer>word</footer>",
        "1a092dfafca8e00d06f070ace94d81061c2749235da07c5aca309f5dc0d106b2",
    );
    let policy =
        "version=1\nname='test'\n[[gates]]\nid='terms'\nkind='forbidden_terms'\nterms=['word']\n";
    let common = [
        "--offline-receipt",
        "--evaluator-identity",
        "trusted-build",
        "--html-scope",
        "main",
        "--telemetry",
    ];
    let first = run(dir.path(), &change, policy, &common);
    assert_eq!(
        first.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let event: serde_json::Value = serde_json::from_slice(&first.stderr).unwrap();
    assert_eq!(event["event"], "content_change_html_projection");
    assert_eq!(event["eligible_entries"], 1);
    let path = dir.path().join("verdict.json");
    let receipt: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(
        receipt["verdict"]["extensions"]["aexeo.html-content-scope.v1"],
        "main"
    );
    let main_options = receipt["decision_inputs"]["host_options"]["sha256"].clone();
    let saved = dir.path().join("main-receipt.json");
    fs::copy(&path, &saved).unwrap();
    let saved_path = saved.to_str().unwrap();
    let verify = [
        "--offline-receipt",
        "--evaluator-identity",
        "trusted-build",
        "--html-scope",
        "main",
        "--verify-offline-receipt",
        saved_path,
    ];
    assert_eq!(
        run(dir.path(), &change, policy, &verify).status.code(),
        Some(0)
    );
    let mismatched = [
        "--offline-receipt",
        "--evaluator-identity",
        "trusted-build",
        "--verify-offline-receipt",
        saved_path,
    ];
    let before = fs::read(&path).unwrap();
    let rejected = run(dir.path(), &change, policy, &mismatched);
    assert_eq!(rejected.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("previous receipt does not match"));
    assert_eq!(fs::read(&path).unwrap(), before);
    let document = ["--offline-receipt", "--evaluator-identity", "trusted-build"];
    assert_eq!(
        run(dir.path(), &change, policy, &document).status.code(),
        Some(1)
    );
    let doc: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_ne!(
        doc["decision_inputs"]["host_options"]["sha256"],
        main_options
    );
    let doc_copy = dir.path().join("document-receipt.json");
    fs::copy(&path, &doc_copy).unwrap();
    let verify_document_as_main = [
        "--offline-receipt",
        "--evaluator-identity",
        "trusted-build",
        "--html-scope",
        "main",
        "--verify-offline-receipt",
        doc_copy.to_str().unwrap(),
    ];
    let before = fs::read(&path).unwrap();
    let rejected = run(dir.path(), &change, policy, &verify_document_as_main);
    assert_eq!(rejected.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("previous receipt does not match"));
    assert_eq!(fs::read(&path).unwrap(), before);
}

#[test]
fn emits_bound_pass_fail_and_not_checked_verdicts() {
    let dir = tempfile::tempdir().unwrap();
    let preset = |min| {
        format!("version=1\nname='test'\n[[gates]]\nid='words'\nkind='word_count'\nmin={min}\n")
    };
    for (min, expected_code, expected_status) in [(1, 0, "pass"), (2, 1, "fail")] {
        let output = run(dir.path(), &input(), &preset(min), &[]);
        assert_eq!(
            output.status.code(),
            Some(expected_code),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.stdout.is_empty());
        let verdict: serde_json::Value =
            serde_json::from_slice(&fs::read(dir.path().join("verdict.json")).unwrap()).unwrap();
        assert_eq!(verdict["status"], expected_status);
        assert_eq!(verdict["subject"]["change_id"], "example");
        assert_eq!(
            verdict["subject"]["entries"][0]["sha256"],
            input()["entries"][0]["after"]["sha256"]
        );
        assert!(verdict["evaluated_at"].as_str().unwrap().ends_with('Z'));
    }
    let mut unsupported = input();
    unsupported["entries"][0]["media_type"] = "application/json".into();
    let output = run(dir.path(), &unsupported, &preset(1), &[]);
    assert_eq!(output.status.code(), Some(2));
    let verdict: serde_json::Value =
        serde_json::from_slice(&fs::read(dir.path().join("verdict.json")).unwrap()).unwrap();
    assert_eq!(verdict["status"], "not_checked");
}

#[test]
fn required_terms_gate_is_available_to_content_change() {
    let dir = tempfile::tempdir().unwrap();
    let preset = "version=1\nname='test'\n[[gates]]\nid='terms'\nkind='required_terms'\nterms=['abc','missing']\n";
    let output = run(dir.path(), &input(), preset, &[]);
    assert_eq!(
        output.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let verdict: serde_json::Value =
        serde_json::from_slice(&fs::read(dir.path().join("verdict.json")).unwrap()).unwrap();
    assert_eq!(verdict["status"], "fail");
    assert!(verdict["findings"][0]["location"].is_null());
}

#[test]
fn link_observer_requires_opt_in_and_does_not_fetch_relative_links() {
    let dir = tempfile::tempdir().unwrap();
    let mut change = input();
    change["entries"][0]["media_type"] = "text/markdown".into();
    change["entries"][0]["after"] = serde_json::json!({"inline":"[x](/x)", "sha256":"f0028e4ca25f0bd43662926fcd36d300576e7e7b3d49045cc6f89a409eaf971e"});
    let preset =
        "version=1\nname='test'\n[[gates]]\nid='links'\nkind='link_reachability'\nblocking=true\n";
    let offline = run(dir.path(), &change, preset, &[]);
    assert_eq!(offline.status.code(), Some(2));
    let online = run(
        dir.path(),
        &change,
        preset,
        &["--check-link-reachability", "--telemetry"],
    );
    assert_eq!(
        online.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&online.stderr)
    );
    let verdict: serde_json::Value =
        serde_json::from_slice(&fs::read(dir.path().join("verdict.json")).unwrap()).unwrap();
    assert_eq!(verdict["status"], "pass");
    let event: serde_json::Value = serde_json::from_slice(&online.stderr).unwrap();
    assert_eq!(event["network_checks"], 0);
    assert!(!String::from_utf8_lossy(&online.stderr).contains("/x"));
}

#[test]
fn link_budget_is_shared_between_entries() {
    let dir = tempfile::tempdir().unwrap();
    let mut change = input();
    change["entries"] = serde_json::json!([
        {"entity_id":"a", "operation":"create", "media_type":"text/markdown", "after":{"inline":"[x](http://127.0.0.1/a)", "sha256":"f8fee02e4780a4cf0c950e349143cf77ab1de060e26def310c8b24565032f6c3"}},
        {"entity_id":"b", "operation":"create", "media_type":"text/markdown", "after":{"inline":"[y](http://127.0.0.1/b)", "sha256":"11270c8f0f7adc7d95cf09d39bb6095efa80f852882286048c3ccb35a964b0a6"}}
    ]);
    let preset =
        "version=1\nname='test'\n[[gates]]\nid='links'\nkind='link_reachability'\nblocking=true\n";
    let output = run(
        dir.path(),
        &change,
        preset,
        &[
            "--check-link-reachability",
            "--link-max-checks",
            "1",
            "--telemetry",
        ],
    );
    assert_eq!(
        output.status.code(),
        Some(2),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8(output.stderr).unwrap();
    let events = stderr
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0]["network_checks"], 1);
    assert_eq!(events[1]["network_checks"], 0);
    assert!(!stderr.contains("127.0.0.1"));
}

#[test]
fn link_observer_reuses_same_destination_across_entries() {
    let dir = tempfile::tempdir().unwrap();
    let mut change = input();
    change["entries"] = serde_json::json!([
        {"entity_id":"a", "operation":"create", "media_type":"text/markdown", "after":{"inline":"[x](http://127.0.0.1/a)", "sha256":"f8fee02e4780a4cf0c950e349143cf77ab1de060e26def310c8b24565032f6c3"}},
        {"entity_id":"b", "operation":"create", "media_type":"text/markdown", "after":{"inline":"[x](http://127.0.0.1/a)", "sha256":"f8fee02e4780a4cf0c950e349143cf77ab1de060e26def310c8b24565032f6c3"}}
    ]);
    let preset =
        "version=1\nname='test'\n[[gates]]\nid='links'\nkind='link_reachability'\nblocking=true\n";
    let output = run(
        dir.path(),
        &change,
        preset,
        &[
            "--check-link-reachability",
            "--link-max-checks",
            "2",
            "--telemetry",
        ],
    );
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8(output.stderr).unwrap();
    let events = stderr
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0]["network_checks"], 1);
    assert_eq!(events[1]["network_checks"], 0);
    let verdict: serde_json::Value =
        serde_json::from_slice(&fs::read(dir.path().join("verdict.json")).unwrap()).unwrap();
    assert_eq!(verdict["status"], "not_checked");
    assert!(!stderr.contains("127.0.0.1"));
}

#[test]
fn metadata_length_gate_evaluates_change_metadata_without_prose_location() {
    let dir = tempfile::tempdir().unwrap();
    let mut change = input();
    change["entries"][0]["metadata"] = serde_json::json!({"title":"Long title"});
    let preset = "version=1\nname='test'\n[[gates]]\nid='title'\nkind='metadata_length'\nfield='title'\nmax=4\n";
    let output = run(dir.path(), &change, preset, &[]);
    assert_eq!(
        output.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let verdict: serde_json::Value =
        serde_json::from_slice(&fs::read(dir.path().join("verdict.json")).unwrap()).unwrap();
    assert_eq!(verdict["status"], "fail");
    assert!(verdict["findings"][0].get("location").is_none());
    change["entries"][0]["metadata"] = serde_json::json!({});
    assert_eq!(run(dir.path(), &change, preset, &[]).status.code(), Some(2));
}

#[test]
fn external_results_with_duplicate_nested_keys_do_not_replace_verdict() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("external.json");
    let verdict = dir.path().join("verdict.json");
    fs::write(&verdict, "previous verdict").unwrap();
    fs::write(&path, r#"[{"extension":"example.rule","parameters":{"mode":"a","mode":"b"},"input":{"id":"article"},"diagnostics":[]}]"#).unwrap();
    let preset = "version=1\nname='test'\n[[gates]]\nid='external'\nkind='custom'\nextension='example.rule'\n";
    let output = run(
        dir.path(),
        &input(),
        preset,
        &["--extension-results", path.to_str().unwrap()],
    );
    assert_ne!(output.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&output.stderr).contains("invalid external gate results JSON"));
    assert_eq!(fs::read(&verdict).unwrap(), b"previous verdict");
}

#[test]
fn invalid_external_results_are_user_errors_and_preserve_verdict() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("external.json");
    let verdict = dir.path().join("verdict.json");
    let preset = "version=1\nname='test'\n[[gates]]\nid='external'\nkind='custom'\nextension='example.rule'\n";
    let mut records = serde_json::json!([{
        "extension":"example.rule", "parameters":{},
        "input":{"id":"article", "text":"abc", "source_text":"abc", "headings":null, "metadata":null, "evidence_sources":null},
        "diagnostics":[{"message":"Review", "line":9, "column":1}]
    }]);
    for (line, expected) in [
        (9, "external diagnostic line is outside projected text"),
        (0, "external gate diagnostics are invalid or exceed limits"),
    ] {
        records[0]["diagnostics"][0]["line"] = line.into();
        fs::write(&path, serde_json::to_vec(&records).unwrap()).unwrap();
        fs::write(&verdict, "previous verdict").unwrap();
        let output = run(
            dir.path(),
            &input(),
            preset,
            &["--extension-results", path.to_str().unwrap()],
        );
        assert_eq!(
            output.status.code(),
            Some(2),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stderr).contains(expected));
        assert_eq!(fs::read(&verdict).unwrap(), b"previous verdict");
    }
}

#[test]
fn external_gates_bind_each_entry_and_withhold_stale_results() {
    let dir = tempfile::tempdir().unwrap();
    let mut change = input();
    change["entries"].as_array_mut().unwrap().push(serde_json::json!({
        "entity_id":"second", "operation":"create", "media_type":"text/plain",
        "after":{"inline":"def", "sha256":"cb8379ac2098aa165029e3938a51da0bcecfc008fd6795f401178647f96c5b34"}
    }));
    let preset = "version=1\nname='test'\n[[gates]]\nid='external'\nkind='custom'\nextension='example.rule'\nblocking=true\n";
    let path = dir.path().join("external.json");
    let mut records = serde_json::json!([
        {"extension":"example.rule", "parameters":{}, "input":{"id":"article", "text":"abc", "source_text":"abc", "headings":null, "metadata":null, "evidence_sources":null}, "diagnostics":[{"message":"Review", "line":1, "column":2}]},
        {"extension":"example.rule", "parameters":{}, "input":{"id":"second", "text":"def", "source_text":"def", "headings":null, "metadata":null, "evidence_sources":null}, "diagnostics":[]}
    ]);
    fs::write(&path, serde_json::to_vec(&records).unwrap()).unwrap();
    let flags = ["--extension-results", path.to_str().unwrap(), "--telemetry"];
    let output = run(dir.path(), &change, preset, &flags);
    assert_eq!(
        output.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let verdict: serde_json::Value =
        serde_json::from_slice(&fs::read(dir.path().join("verdict.json")).unwrap()).unwrap();
    assert_eq!(
        verdict["findings"][0]["location"],
        serde_json::json!({"line":1,"column":2})
    );
    assert_eq!(verdict["summary"]["passed"], 1);
    let event: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(event["event"], "content_change_external_gates");
    assert_eq!(event["configured_custom_gates"], 1);
    assert!(!String::from_utf8_lossy(&output.stderr).contains("abc"));

    records[0]["diagnostics"] = serde_json::json!([]);
    records[1]["input"]["text"] = "stale".into();
    fs::write(&path, serde_json::to_vec(&records).unwrap()).unwrap();
    let output = run(dir.path(), &change, preset, &flags);
    assert_eq!(output.status.code(), Some(2));
    let verdict: serde_json::Value =
        serde_json::from_slice(&fs::read(dir.path().join("verdict.json")).unwrap()).unwrap();
    assert_eq!(verdict["status"], "not_checked");
    assert_eq!(verdict["summary"]["passed"], 1);
    assert_eq!(verdict["summary"]["not_checked"], 1);

    let prior_verdict = fs::read(dir.path().join("verdict.json")).unwrap();
    records[1]["input"]["text"] = "def".into();
    records[1]["diagnostics"] = serde_json::json!([{"message":"bad", "line":9, "column":1}]);
    fs::write(&path, serde_json::to_vec(&records).unwrap()).unwrap();
    let output = run(dir.path(), &change, preset, &flags);
    assert_ne!(output.status.code(), Some(0));
    assert_eq!(
        fs::read(dir.path().join("verdict.json")).unwrap(),
        prior_verdict
    );
}

#[test]
fn revalidates_custom_results_alone_and_with_observed_links() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("draft.md");
    fs::write(&source, "[x](/x)").unwrap();
    let exported = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .args([
            "content",
            source.to_str().unwrap(),
            "--input-format",
            "markdown",
            "--export-adapter-input",
        ])
        .output()
        .unwrap();
    assert!(
        exported.status.success(),
        "{}",
        String::from_utf8_lossy(&exported.stderr)
    );
    let mut projected: serde_json::Value = serde_json::from_slice(&exported.stdout).unwrap();
    projected["id"] = "article".into();
    let path = dir.path().join("external.json");
    let mut records = serde_json::json!([{"extension":"example.rule", "parameters":{}, "input":projected, "diagnostics":[]}]);
    fs::write(&path, serde_json::to_vec(&records).unwrap()).unwrap();
    let ext = path.to_str().unwrap();
    let mut change = input();
    change["entries"][0]["media_type"] = "text/markdown".into();
    change["entries"][0]["after"] = serde_json::json!({"inline":"[x](/x)", "sha256":"f0028e4ca25f0bd43662926fcd36d300576e7e7b3d49045cc6f89a409eaf971e"});
    let preset = "version=1\nname='test'\n[[gates]]\nid='external'\nkind='custom'\nextension='example.rule'\n[[gates]]\nid='links'\nkind='link_reachability'\nblocking=false\n";
    let exported_links = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .args([
            "content",
            source.to_str().unwrap(),
            "--input-format",
            "markdown",
            "--export-link-observations",
        ])
        .output()
        .unwrap();
    assert!(exported_links.status.success());
    let mut entry: serde_json::Value = serde_json::from_slice(&exported_links.stdout).unwrap();
    entry["observations"][0] = "not_applicable".into();
    let snapshots = dir.path().join("links.json");
    fs::write(
        &snapshots,
        serde_json::to_vec(
            &serde_json::json!({"version":1,"change_id":"example","entries":{"article":entry}}),
        )
        .unwrap(),
    )
    .unwrap();
    for mode in 0..3 {
        let mut flags = vec!["--extension-results", ext];
        if mode == 1 {
            flags.push("--check-link-reachability");
        } else if mode == 2 {
            flags.extend(["--link-observations", snapshots.to_str().unwrap()]);
        }
        let first = run(dir.path(), &change, preset, &flags);
        assert_eq!(
            first.status.code(),
            Some(0),
            "{}",
            String::from_utf8_lossy(&first.stderr)
        );
        let verdict = dir.path().join("verdict.json");
        let previous = dir.path().join("previous.json");
        fs::copy(&verdict, &previous).unwrap();
        let original = fs::read(&previous).unwrap();
        let saved: serde_json::Value = serde_json::from_slice(&original).unwrap();
        let binding = &saved["extensions"]["aexeo.external-observations.v1"];
        assert!(binding["extensions_sha256"].as_str().is_some());
        assert_eq!(binding["links_sha256"].is_string(), mode != 0);
        assert!(!binding.to_string().contains("example.rule"));
        flags.extend([
            "--verify-verdict",
            previous.to_str().unwrap(),
            "--telemetry",
        ]);
        let matching = run(dir.path(), &change, preset, &flags);
        assert_eq!(
            matching.status.code(),
            Some(0),
            "{}",
            String::from_utf8_lossy(&matching.stderr)
        );
        assert!(String::from_utf8_lossy(&matching.stderr).contains("\"matched\":true"));
        let current = fs::read(&verdict).unwrap();
        // Even an unused result is part of the caller-supplied snapshot.
        let mut unused = records[0].clone();
        unused["extension"] = "other.rule".into();
        records.as_array_mut().unwrap().push(unused);
        fs::write(&path, serde_json::to_vec(&records).unwrap()).unwrap();
        let rejected = run(dir.path(), &change, preset, &flags);
        assert_eq!(rejected.status.code(), Some(2));
        let stderr = String::from_utf8_lossy(&rejected.stderr);
        assert!(stderr.contains("\"matched\":false"));
        assert!(!stderr.contains("content_link_reachability"));
        assert!(!stderr.contains("other.rule"));
        assert_eq!(fs::read(&verdict).unwrap(), current);
        records.as_array_mut().unwrap().pop();
        fs::write(&path, serde_json::to_vec(&records).unwrap()).unwrap();
        let mut tampered = saved;
        tampered["extensions"]["aexeo.external-observations.v1"]["extensions_sha256"] =
            "0".repeat(64).into();
        fs::write(&previous, serde_json::to_vec(&tampered).unwrap()).unwrap();
        let rejected = run(dir.path(), &change, preset, &flags);
        assert_eq!(rejected.status.code(), Some(2));
        assert!(String::from_utf8_lossy(&rejected.stderr).contains("\"matched\":false"));
        assert_eq!(fs::read(&verdict).unwrap(), current);
    }
}

#[test]
fn external_markdown_result_maps_projected_location_to_original_source() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("draft.md");
    fs::write(&source, "**abc**").unwrap();
    let exported = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .args([
            "content",
            source.to_str().unwrap(),
            "--input-format",
            "markdown",
            "--export-adapter-input",
        ])
        .output()
        .unwrap();
    assert!(
        exported.status.success(),
        "{}",
        String::from_utf8_lossy(&exported.stderr)
    );
    let mut projected: serde_json::Value = serde_json::from_slice(&exported.stdout).unwrap();
    projected["id"] = "article".into();
    let path = dir.path().join("external.json");
    fs::write(
        &path,
        serde_json::to_vec(&serde_json::json!([{
            "extension":"example.rule", "parameters":{}, "input":projected,
            "diagnostics":[{"message":"Review", "line":1, "column":2}]
        }]))
        .unwrap(),
    )
    .unwrap();
    let mut change = input();
    change["entries"][0]["media_type"] = "text/markdown".into();
    change["entries"][0]["after"] = serde_json::json!({"inline":"**abc**", "sha256":"eedd8ef9da3bf94812b49afc562b48f35046d62ed8f609403290ad69d9e60b8e"});
    let preset = "version=1\nname='test'\n[[gates]]\nid='external'\nkind='custom'\nextension='example.rule'\nblocking=true\n";
    let output = run(
        dir.path(),
        &change,
        preset,
        &["--extension-results", path.to_str().unwrap()],
    );
    assert_eq!(
        output.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let verdict: serde_json::Value =
        serde_json::from_slice(&fs::read(dir.path().join("verdict.json")).unwrap()).unwrap();
    assert_eq!(
        verdict["findings"][0]["location"],
        serde_json::json!({"line":1,"column":4})
    );
}

#[test]
fn paragraph_text_is_opt_in_and_preserves_original_locations() {
    let dir = tempfile::tempdir().unwrap();
    let mut change = input();
    change["entries"][0]["after"] = serde_json::json!({
        "inline": "First.\n\nÉlan and other words.",
        "sha256": "10a67226b86aa8c50102c329593fdd2f2d3333c4bbc3d5bc506873b362797fa4"
    });
    let preset = "version=1\nname='test'\n[[gates]]\nid='length'\nkind='paragraph_length'\nmax_words=2\nblocking=true\n";
    let offline = run(dir.path(), &change, preset, &[]);
    assert_eq!(offline.status.code(), Some(2));
    let enabled = run(
        dir.path(),
        &change,
        preset,
        &["--plain-text-paragraphs", "--telemetry"],
    );
    assert_eq!(
        enabled.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&enabled.stderr)
    );
    let telemetry: serde_json::Value = serde_json::from_slice(&enabled.stderr).unwrap();
    assert_eq!(telemetry["event"], "content_change_paragraph_adapter");
    assert_eq!(telemetry["eligible_entries"], 1);
    assert!(!String::from_utf8_lossy(&enabled.stderr).contains("Élan"));
    let verdict: serde_json::Value =
        serde_json::from_slice(&fs::read(dir.path().join("verdict.json")).unwrap()).unwrap();
    assert_eq!(verdict["status"], "fail");
    assert_eq!(
        verdict["findings"][0]["location"],
        serde_json::json!({"line":3,"column":1})
    );
}

#[test]
fn verdict_contains_plain_text_source_location() {
    let dir = tempfile::tempdir().unwrap();
    let preset =
        "version=1\nname='test'\n[[gates]]\nid='terms'\nkind='forbidden_terms'\nterms=['abc']\n";
    let output = run(dir.path(), &input(), preset, &[]);
    assert_eq!(
        output.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let verdict: serde_json::Value =
        serde_json::from_slice(&fs::read(dir.path().join("verdict.json")).unwrap()).unwrap();
    assert_eq!(
        verdict["findings"][0]["location"],
        serde_json::json!({"line": 1, "column": 1})
    );
}

#[test]
fn markdown_image_alt_finding_has_original_source_location() {
    let dir = tempfile::tempdir().unwrap();
    let preset =
        "version=1\nname='test'\n[[gates]]\nid='terms'\nkind='forbidden_terms'\nterms=['bad']\n";
    let mut change = input();
    change["entries"][0]["media_type"] = "text/markdown".into();
    change["entries"][0]["after"] = serde_json::json!({"inline":"![bad](x)", "sha256":"4109ea405f70db2e2f6936be0a07a2b3e67393dc75b6c80b3b974b6bd23c03f6"});
    let output = run(dir.path(), &change, preset, &[]);
    assert_eq!(
        output.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let verdict: serde_json::Value =
        serde_json::from_slice(&fs::read(dir.path().join("verdict.json")).unwrap()).unwrap();
    assert_eq!(
        verdict["findings"][0]["location"],
        serde_json::json!({"line":1,"column":3})
    );
}

#[test]
fn digest_mismatch_and_unapproved_files_cannot_succeed() {
    let dir = tempfile::tempdir().unwrap();
    let preset = "version=1\nname='test'\n[[gates]]\nid='words'\nkind='word_count'\nmin=1\n";
    let mut change = input();
    change["entries"][0]["after"]["inline"] = "different".into();
    let output = run(dir.path(), &change, preset, &[]);
    assert_eq!(output.status.code(), Some(2));
    assert!(!dir.path().join("verdict.json").exists());
    fs::write(dir.path().join("article.txt"), "abc").unwrap();
    change["content_root"] = dir.path().to_str().unwrap().into();
    change["entries"][0]["after"] = serde_json::json!({"file": "article.txt", "sha256": "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"});
    let output = run(dir.path(), &change, preset, &[]);
    assert_eq!(output.status.code(), Some(2));
    assert!(!dir.path().join("verdict.json").exists());
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        let output = run(
            dir.path(),
            &change,
            preset,
            &["--content-root", dir.path().to_str().unwrap()],
        );
        assert_eq!(
            output.status.code(),
            Some(0),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn rejects_verdict_aliases_of_configuration_inputs() {
    let dir = tempfile::tempdir().unwrap();
    let change = dir.path().join("change.json");
    let preset = dir.path().join("preset.toml");
    let snapshot = dir.path().join("snapshot.json");
    let external = dir.path().join("external.json");
    fs::write(&change, serde_json::to_vec(&input()).unwrap()).unwrap();
    fs::write(
        &preset,
        "version=1\nname='test'\n[[gates]]\nid='words'\nkind='word_count'\nmin=1\n",
    )
    .unwrap();
    fs::write(&snapshot, "snapshot must not be replaced").unwrap();
    fs::write(&external, "external input must not be replaced").unwrap();
    for (target, extra) in [
        (&preset, None),
        (&snapshot, Some("--collection-snapshot")),
        (&snapshot, Some("--link-observations")),
        (&external, Some("--extension-results")),
    ] {
        let original = fs::read(target).unwrap();
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"));
        cmd.args([
            "content-change",
            "--preset",
            "test",
            "--preset-source",
            &format!("test={}", preset.display()),
        ]);
        if let Some(flag) = extra {
            cmd.arg(flag).arg(target);
        }
        let output = cmd
            .env("CONTENT_GATE_CHANGE_FILE", &change)
            .env("CONTENT_GATE_VERDICT_FILE", target)
            .output()
            .unwrap();
        assert!(
            !output.status.success(),
            "alias should fail: {}",
            target.display()
        );
        assert_eq!(fs::read(target).unwrap(), original);
    }
}

#[cfg(unix)]
#[test]
fn rejects_symlink_to_preset_without_replacing_it() {
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    let change = dir.path().join("change.json");
    let preset = dir.path().join("preset.toml");
    let output_path = dir.path().join("verdict-link.json");
    let original = b"version=1\nname='test'\n[[gates]]\nid='words'\nkind='word_count'\nmin=1\n";
    fs::write(&change, serde_json::to_vec(&input()).unwrap()).unwrap();
    fs::write(&preset, original).unwrap();
    symlink(&preset, &output_path).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .args([
            "content-change",
            "--preset",
            "test",
            "--preset-source",
            &format!("test={}", preset.display()),
        ])
        .env("CONTENT_GATE_CHANGE_FILE", &change)
        .env("CONTENT_GATE_VERDICT_FILE", &output_path)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("preset source and verdict paths must be different")
    );
    assert!(
        fs::symlink_metadata(&output_path)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(fs::read(&preset).unwrap(), original);
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn rejects_verdict_aliases_of_confined_content() {
    let dir = tempfile::tempdir().unwrap();
    let change_path = dir.path().join("change.json");
    let article = dir.path().join("article.txt");
    let alias = dir.path().join("article-alias.txt");
    fs::write(&article, "abc").unwrap();
    fs::hard_link(&article, &alias).unwrap();
    let mut change = input();
    change["content_root"] = dir.path().to_str().unwrap().into();
    change["entries"][0]["after"] = serde_json::json!({"file":"article.txt", "sha256":"ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"});
    fs::write(&change_path, serde_json::to_vec(&change).unwrap()).unwrap();
    for output_path in [&article, &alias] {
        let output = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
            .args([
                "content-change",
                "--preset",
                "article",
                "--content-root",
                dir.path().to_str().unwrap(),
            ])
            .env("CONTENT_GATE_CHANGE_FILE", &change_path)
            .env("CONTENT_GATE_VERDICT_FILE", output_path)
            .output()
            .unwrap();
        assert!(!output.status.success(), "content alias should fail");
        assert_eq!(fs::read(&article).unwrap(), b"abc");
        assert_eq!(fs::read(&alias).unwrap(), b"abc");
    }
}

#[test]
fn rejects_same_change_and_verdict_path_without_destroying_input() {
    let dir = tempfile::tempdir().unwrap();
    let change = dir.path().join("change.json");
    let original = serde_json::to_vec(&input()).unwrap();
    fs::write(&change, &original).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .args(["content-change", "--preset", "article"])
        .env("CONTENT_GATE_CHANGE_FILE", &change)
        .env("CONTENT_GATE_VERDICT_FILE", &change)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(fs::read(change).unwrap(), original);
}

#[test]
fn failed_run_does_not_replace_prior_verdict() {
    let dir = tempfile::tempdir().unwrap();
    let preset = "version=1\nname='test'\n[[gates]]\nid='words'\nkind='word_count'\nmin=1\n";
    let verdict = dir.path().join("verdict.json");
    fs::write(&verdict, "prior verdict").unwrap();
    let mut change = input();
    change["entries"][0]["after"]["inline"] = "changed".into();
    let output = run(dir.path(), &change, preset, &[]);
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(fs::read_to_string(verdict).unwrap(), "prior verdict");
}

#[test]
fn rejects_invalid_protocol_fields_without_producing_a_verdict() {
    let dir = tempfile::tempdir().unwrap();
    let preset = "version=1\nname='test'\n[[gates]]\nid='words'\nkind='word_count'\nmin=1\n";
    let mut invalid = Vec::new();
    let mut value = input();
    value["created_at"] = "not-a-date".into();
    invalid.push(value);
    let mut value = input();
    value["origin"]["system"] = "".into();
    invalid.push(value);
    let mut value = input();
    value["site"] = serde_json::json!({});
    invalid.push(value);
    let mut value = input();
    value["site"] = serde_json::json!({"base_url": "not a URI"});
    invalid.push(value);
    let mut value = input();
    value["author"] = serde_json::json!({"id": ""});
    invalid.push(value);
    let mut value = input();
    value["entries"][0]["after"]["revision"] = "".into();
    invalid.push(value);
    let mut value = input();
    value["entries"][0]["operation"] = "update".into();
    value["entries"][0]["before"] = serde_json::json!({"uri": "not a URI", "sha256": "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"});
    invalid.push(value);
    let mut value = input();
    value["entries"][0]["before"] = serde_json::Value::Null;
    invalid.push(value);
    let mut value = input();
    value["entries"][0]["after"]["uri"] = serde_json::Value::Null;
    invalid.push(value);
    let mut value = input();
    value["entries"][0]["after"]["inline"] = serde_json::Value::Null;
    invalid.push(value);
    for value in invalid {
        let output = run(dir.path(), &value, preset, &[]);
        assert_eq!(
            output.status.code(),
            Some(2),
            "accepted {value}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!dir.path().join("verdict.json").exists());
    }
}

#[test]
fn allows_valid_extensions_and_rfc3339_offsets() {
    let dir = tempfile::tempdir().unwrap();
    let preset = "version=1\nname='test'\n[[gates]]\nid='words'\nkind='word_count'\nmin=1\n";
    let mut change = input();
    change["created_at"] = "2026-02-28T12:34:56.123+01:00".into();
    change["site"] = serde_json::json!({"base_url": "https://example.org/", "future_field": true});
    change["future_field"] = serde_json::json!(null);
    let output = run(dir.path(), &change, preset, &[]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn explicit_collection_snapshot_is_bound_and_conservative() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("collection.json");
    let mut change = input();
    change["entries"][0]["metadata"] = serde_json::json!({"slug":"private-slug"});
    let preset = "version=1\nname='test'\n[[gates]]\nid='unique'\nkind='unique_collection_metadata'\nfield='slug'\n";
    let mut snapshot = serde_json::json!({"change_id":"example", "complete":true,
        "items":[{"id":"article", "metadata":{"slug":"private-slug"}}]});
    for (complete, expected_code, expected_status) in [(true, 0, "pass"), (false, 2, "not_checked")]
    {
        snapshot["complete"] = complete.into();
        fs::write(&path, serde_json::to_vec(&snapshot).unwrap()).unwrap();
        let output = run(
            dir.path(),
            &change,
            preset,
            &[
                "--collection-snapshot",
                path.to_str().unwrap(),
                "--telemetry",
            ],
        );
        assert_eq!(
            output.status.code(),
            Some(expected_code),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let verdict: serde_json::Value =
            serde_json::from_slice(&fs::read(dir.path().join("verdict.json")).unwrap()).unwrap();
        assert_eq!(verdict["status"], expected_status);
        let telemetry: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
        assert_eq!(telemetry["items"], 1);
        assert_eq!(telemetry["complete"], complete);
        assert!(!String::from_utf8_lossy(&output.stderr).contains("private-slug"));
    }
    snapshot["change_id"] = "different".into();
    fs::write(&path, serde_json::to_vec(&snapshot).unwrap()).unwrap();
    fs::remove_file(dir.path().join("verdict.json")).unwrap();
    let output = run(
        dir.path(),
        &change,
        preset,
        &["--collection-snapshot", path.to_str().unwrap()],
    );
    assert!(!output.status.success());
    assert!(!dir.path().join("verdict.json").exists());
}

#[test]
fn revalidation_rejects_duplicate_nested_verdict_keys_without_replacing_output() {
    let dir = tempfile::tempdir().unwrap();
    let policy = "version=1\nname='test'\n[[gates]]\nid='words'\nkind='word_count'\nmin=1\n";
    assert_eq!(
        run(dir.path(), &input(), policy, &[]).status.code(),
        Some(0)
    );
    let output = dir.path().join("verdict.json");
    let original = fs::read(&output).unwrap();
    let parsed: serde_json::Value = serde_json::from_slice(&original).unwrap();
    let key = "aexeo.offline-input-binding.v1";
    let pair = format!("\"{}\": {}", key, parsed["extensions"][key]);
    let raw = String::from_utf8(original.clone()).unwrap();
    let ambiguous = raw.replacen(&pair, &format!("{pair}, {pair}"), 1);
    assert_ne!(ambiguous, raw);
    let previous = dir.path().join("previous.json");
    fs::write(&previous, ambiguous).unwrap();
    let result = run(
        dir.path(),
        &input(),
        policy,
        &["--verify-verdict", previous.to_str().unwrap()],
    );
    assert_eq!(result.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&result.stderr).contains("invalid previous verdict"));
    assert_eq!(fs::read(&output).unwrap(), original);
}

#[test]
fn revalidates_previous_verdict_against_current_inputs_and_result() {
    let dir = tempfile::tempdir().unwrap();
    let policy = "version=1\nname='test'\n[[gates]]\nid='words'\nkind='word_count'\nmin=1\n";
    assert_eq!(
        run(dir.path(), &input(), policy, &[]).status.code(),
        Some(0)
    );
    let previous = dir.path().join("previous.json");
    fs::copy(dir.path().join("verdict.json"), &previous).unwrap();
    let path = previous.to_str().unwrap();
    let args = ["--verify-verdict", path];
    let valid = run(
        dir.path(),
        &input(),
        policy,
        &["--verify-verdict", path, "--telemetry"],
    );
    let event: serde_json::Value = serde_json::from_slice(&valid.stderr).unwrap();
    assert_eq!(
        event,
        serde_json::json!({"event":"content_change_revalidation", "version":1, "matched":true})
    );
    assert_eq!(
        valid.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&valid.stderr)
    );

    let mut changed = input();
    changed["entries"][0]["metadata"] = serde_json::json!({"slug":"different"});
    let rejected = run(
        dir.path(),
        &changed,
        policy,
        &["--verify-verdict", path, "--telemetry"],
    );
    assert_eq!(rejected.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&rejected.stderr);
    let event: serde_json::Value = serde_json::from_str(stderr.lines().next().unwrap()).unwrap();
    assert_eq!(
        event,
        serde_json::json!({"event":"content_change_revalidation", "version":1, "matched":false})
    );
    assert!(!stderr.contains("different"));
    let other_policy = policy.replace("min=1", "min=2");
    assert_eq!(
        run(dir.path(), &input(), &other_policy, &args)
            .status
            .code(),
        Some(2)
    );

    let mut tampered: serde_json::Value =
        serde_json::from_slice(&fs::read(&previous).unwrap()).unwrap();
    tampered["checks"][0]["status"] = "failed".into();
    fs::write(&previous, serde_json::to_vec(&tampered).unwrap()).unwrap();
    let result = run(dir.path(), &input(), policy, &args);
    assert_eq!(result.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&result.stderr)
            .contains("differs from the current offline evaluation")
    );
}

#[test]
fn exports_change_link_templates_and_imports_them_without_network() {
    let dir = tempfile::tempdir().unwrap();
    let mut change = input();
    change["entries"][0]["media_type"] = "text/markdown".into();
    change["entries"][0]["after"] = serde_json::json!({"inline":"[first](https://example.org/a)", "sha256":"9b109ca77f8b3af5b0f54a9ae1aeb8d01f5ed6fbd1d350ffe84289bf42000e09"});
    change["entries"].as_array_mut().unwrap().push(serde_json::json!({"entity_id":"other", "operation":"create", "media_type":"text/html", "after":{"inline":"<main><p><a href=\"https://example.org/b\">second</a></p></main>", "sha256":"b65d7ee38bc936bda3e4e654a4f99e5d68833e81ac6eb9fc19563664b22dc5e4"}}));
    let change_path = dir.path().join("source.json");
    let snapshot = dir.path().join("links.json");
    fs::write(&change_path, serde_json::to_vec(&change).unwrap()).unwrap();
    let export = || {
        Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
            .args([
                "content-change",
                "--export-link-observations",
                "--html-scope",
                "main",
                "--telemetry",
            ])
            .env("CONTENT_GATE_CHANGE_FILE", &change_path)
            .env("CONTENT_GATE_VERDICT_FILE", &snapshot)
            .output()
            .unwrap()
    };
    let result = export();
    assert_eq!(
        result.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let event: serde_json::Value = serde_json::from_slice(&result.stderr).unwrap();
    assert_eq!(event["event"], "content_change_link_snapshot_export");
    assert_eq!(event["entries"], 2);
    assert!(!String::from_utf8_lossy(&result.stderr).contains("example.org"));
    let data: serde_json::Value = serde_json::from_slice(&fs::read(&snapshot).unwrap()).unwrap();
    assert_eq!(data["change_id"], "example");
    assert_eq!(
        data["entries"]["article"]["destinations"],
        serde_json::json!(["https://example.org/a"])
    );
    assert_eq!(
        data["entries"]["other"]["destinations"],
        serde_json::json!(["https://example.org/b"])
    );
    assert_eq!(
        data["entries"]["article"]["observations"],
        serde_json::json!(["not_checked"])
    );
    let preset = "version=1\nname='test'\n[[gates]]\nid='links'\nkind='link_reachability'\n";
    let checked = run(
        dir.path(),
        &change,
        preset,
        &[
            "--link-observations",
            snapshot.to_str().unwrap(),
            "--html-scope",
            "main",
        ],
    );
    assert_eq!(checked.status.code(), Some(0));
    let verdict: serde_json::Value =
        serde_json::from_slice(&fs::read(dir.path().join("verdict.json")).unwrap()).unwrap();
    assert!(
        verdict["checks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["status"] == "not_checked")
    );
    let before = fs::read(&snapshot).unwrap();
    change["entries"][0]["after"]["sha256"] = "0".repeat(64).into();
    fs::write(&change_path, serde_json::to_vec(&change).unwrap()).unwrap();
    let failure = export();
    assert_eq!(failure.status.code(), Some(2));
    assert_eq!(fs::read(&snapshot).unwrap(), before);
}

#[test]
fn change_link_export_rejects_unavailable_input_without_replacing_output() {
    let dir = tempfile::tempdir().unwrap();
    let change_path = dir.path().join("change.json");
    let output = dir.path().join("output.json");
    fs::write(&change_path, serde_json::to_vec(&input()).unwrap()).unwrap();
    fs::write(&output, b"existing").unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .args(["content-change", "--export-link-observations"])
        .env("CONTENT_GATE_CHANGE_FILE", &change_path)
        .env("CONTENT_GATE_VERDICT_FILE", &output)
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(2));
    assert_eq!(fs::read(&output).unwrap(), b"existing");
    let aliased = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .args(["content-change", "--export-link-observations"])
        .env("CONTENT_GATE_CHANGE_FILE", &change_path)
        .env("CONTENT_GATE_VERDICT_FILE", &change_path)
        .output()
        .unwrap();
    assert_eq!(aliased.status.code(), Some(2));
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&fs::read(&change_path).unwrap()).unwrap(),
        input()
    );
}

#[test]
fn host_link_snapshot_never_contacts_the_destination() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let source = format!("[local](http://{}/private)", listener.local_addr().unwrap());
    let article = dir.path().join("article.md");
    fs::write(&article, &source).unwrap();
    let exported = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .args([
            "content",
            article.to_str().unwrap(),
            "--input-format",
            "markdown",
            "--export-link-observations",
        ])
        .output()
        .unwrap();
    assert!(
        exported.status.success(),
        "{}",
        String::from_utf8_lossy(&exported.stderr)
    );
    let mut entry: serde_json::Value = serde_json::from_slice(&exported.stdout).unwrap();
    entry["observations"][0] = "reachable".into();
    let mut change = input();
    change["entries"][0]["media_type"] = "text/markdown".into();
    change["entries"][0]["after"] =
        serde_json::json!({"inline":source,"sha256":entry["source_sha256"]});
    let snapshot = dir.path().join("observations.json");
    fs::write(
        &snapshot,
        serde_json::to_vec(
            &serde_json::json!({"version":1,"change_id":"example","entries":{"article":entry}}),
        )
        .unwrap(),
    )
    .unwrap();
    let policy = "version=1\nname='test'\n[[gates]]\nid='links'\nkind='link_reachability'\n";
    let options = ["--link-observations", snapshot.to_str().unwrap()];
    let result = run(dir.path(), &change, policy, &options);
    assert_eq!(
        result.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
    let saved = fs::read(dir.path().join("verdict.json")).unwrap();
    let conflict = run(
        dir.path(),
        &change,
        policy,
        &[
            "--link-observations",
            snapshot.to_str().unwrap(),
            "--check-link-reachability",
        ],
    );
    assert_eq!(conflict.status.code(), Some(2));
    assert_eq!(fs::read(dir.path().join("verdict.json")).unwrap(), saved);
}

#[test]
fn host_link_snapshots_bind_multiple_entries_and_revalidate() {
    let dir = tempfile::tempdir().unwrap();
    let mut change = input();
    change["entries"][0]["media_type"] = "text/markdown".into();
    change["entries"][0]["after"] = serde_json::json!({"inline":"[first](https://example.org/a)", "sha256":"9b109ca77f8b3af5b0f54a9ae1aeb8d01f5ed6fbd1d350ffe84289bf42000e09"});
    change["entries"].as_array_mut().unwrap().push(serde_json::json!({"entity_id":"other", "operation":"create", "media_type":"text/html", "after":{"inline":"<main><p><a href=\"https://example.org/b\">second</a></p></main>", "sha256":"b65d7ee38bc936bda3e4e654a4f99e5d68833e81ac6eb9fc19563664b22dc5e4"}}));
    let policy = "version=1\nname='test'\n[[gates]]\nid='links'\nkind='link_reachability'\n";
    let snapshot = dir.path().join("link-snapshot.json");
    let mut body = serde_json::json!({"version":1,"change_id":"example","entries":{
        "article":{"version":1,"source_sha256":"9b109ca77f8b3af5b0f54a9ae1aeb8d01f5ed6fbd1d350ffe84289bf42000e09", "destinations":["https://example.org/a"],"observations":["not_found"]},
        "other":{"version":1,"source_sha256":"b65d7ee38bc936bda3e4e654a4f99e5d68833e81ac6eb9fc19563664b22dc5e4", "destinations":["https://example.org/b"],"observations":["not_checked"]}
    }});
    fs::write(&snapshot, serde_json::to_vec(&body).unwrap()).unwrap();
    let path = snapshot.to_str().unwrap();
    let first = run(
        dir.path(),
        &change,
        policy,
        &["--link-observations", path, "--telemetry"],
    );
    let events: Vec<serde_json::Value> = String::from_utf8_lossy(&first.stderr)
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(events[0]["event"], "content_change_link_snapshots");
    assert_eq!(events[0]["unchecked_observations"], 1);
    assert_eq!(events[0]["observed_entries_without_snapshot"], 0);
    assert!(!String::from_utf8_lossy(&first.stderr).contains("example.org"));
    assert!(
        first.status.code().is_some(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let output = dir.path().join("verdict.json");
    let first_verdict: serde_json::Value =
        serde_json::from_slice(&fs::read(&output).unwrap()).unwrap();
    assert!(
        first_verdict["findings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["entity_id"] == "article")
    );
    assert!(
        first_verdict["checks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["status"] == "not_checked")
    );
    let previous = dir.path().join("previous.json");
    fs::copy(&output, &previous).unwrap();
    let options = [
        "--link-observations",
        path,
        "--verify-verdict",
        previous.to_str().unwrap(),
    ];
    let again = run(dir.path(), &change, policy, &options);
    assert_eq!(
        again.status.code(),
        first.status.code(),
        "{}",
        String::from_utf8_lossy(&again.stderr)
    );
    let saved = fs::read(&output).unwrap();
    for altered in [
        {
            let mut v = body.clone();
            v["entries"]["article"]["source_sha256"] = "0".repeat(64).into();
            v
        },
        {
            let mut v = body.clone();
            v["entries"]["article"]["destinations"][0] = "https://wrong.invalid".into();
            v
        },
        {
            let mut v = body.clone();
            v["entries"]["article"]["observations"] = serde_json::json!([]);
            v
        },
        {
            let mut v = body.clone();
            v["change_id"] = "other-change".into();
            v
        },
        {
            let mut v = body.clone();
            v["entries"]["unknown"] = v["entries"]["article"].clone();
            v
        },
    ] {
        fs::write(&snapshot, serde_json::to_vec(&altered).unwrap()).unwrap();
        let rejected = run(dir.path(), &change, policy, &["--link-observations", path]);
        assert_eq!(
            rejected.status.code(),
            Some(2),
            "{}",
            String::from_utf8_lossy(&rejected.stderr)
        );
        assert_eq!(fs::read(&output).unwrap(), saved);
    }
    let raw = serde_json::to_string(&body).unwrap();
    let duplicate = raw.replacen("\"version\":1", "\"version\":1,\"version\":1", 1);
    assert_ne!(duplicate, raw);
    fs::write(&snapshot, duplicate).unwrap();
    let rejected = run(dir.path(), &change, policy, &["--link-observations", path]);
    assert_eq!(rejected.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("invalid link observations"));
    assert_eq!(fs::read(&output).unwrap(), saved);
    let mut partial = body.clone();
    partial["entries"].as_object_mut().unwrap().remove("other");
    fs::write(&snapshot, serde_json::to_vec(&partial).unwrap()).unwrap();
    let partial_result = run(
        dir.path(),
        &change,
        policy,
        &["--link-observations", path, "--telemetry"],
    );
    let partial_event: serde_json::Value = serde_json::from_str(
        String::from_utf8_lossy(&partial_result.stderr)
            .lines()
            .next()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(partial_event["observed_entries_without_snapshot"], 1);
    assert_eq!(partial_event["unchecked_observations"], 1);
    assert_eq!(partial_result.status.code(), first.status.code());
    fs::write(&output, &saved).unwrap();
    body["entries"]["article"]["observations"][0] = "reachable".into();
    fs::write(&snapshot, serde_json::to_vec(&body).unwrap()).unwrap();
    let rejected = run(dir.path(), &change, policy, &options);
    assert_eq!(rejected.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("previous verdict"));
    assert_eq!(fs::read(&output).unwrap(), saved);
}

#[test]
fn revalidates_observed_links_and_rejects_stale_inputs_before_observing() {
    let dir = tempfile::tempdir().unwrap();
    let mut change = input();
    change["entries"][0]["media_type"] = "text/markdown".into();
    change["entries"][0]["after"] = serde_json::json!({"inline":"[x](/x)", "sha256":"f0028e4ca25f0bd43662926fcd36d300576e7e7b3d49045cc6f89a409eaf971e"});
    let policy = "version=1\nname='test'\n[[gates]]\nid='links'\nkind='link_reachability'\n";
    assert_eq!(
        run(dir.path(), &change, policy, &["--check-link-reachability"])
            .status
            .code(),
        Some(0)
    );
    let verdict = dir.path().join("verdict.json");
    let previous = dir.path().join("previous.json");
    fs::copy(&verdict, &previous).unwrap();
    let path = previous.to_str().unwrap();
    let first: serde_json::Value = serde_json::from_slice(&fs::read(&previous).unwrap()).unwrap();
    assert_eq!(
        first["extensions"]["aexeo.cli-link-options.v1"],
        serde_json::json!({"max_checks":20,"timeout_seconds":5})
    );
    let ok = run(
        dir.path(),
        &change,
        policy,
        &[
            "--check-link-reachability",
            "--verify-verdict",
            path,
            "--telemetry",
        ],
    );
    assert_eq!(
        ok.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&ok.stderr)
    );
    let events: Vec<serde_json::Value> = String::from_utf8_lossy(&ok.stderr)
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0]["event"], "content_link_reachability");
    assert_eq!(events[0]["network_checks"], 0);
    assert_eq!(
        events[1],
        serde_json::json!({"event":"content_change_revalidation", "version":1, "matched":true})
    );
    let current = fs::read(&verdict).unwrap();
    let rejected = run(
        dir.path(),
        &change,
        policy,
        &[
            "--check-link-reachability",
            "--verify-verdict",
            path,
            "--link-max-checks",
            "1",
            "--telemetry",
        ],
    );
    assert_eq!(rejected.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&rejected.stderr);
    assert!(!stderr.contains("content_link_reachability"));
    assert!(stderr.contains("\"matched\":false"));
    assert_eq!(fs::read(&verdict).unwrap(), current);
    let mut changed = change.clone();
    changed["entries"][0]["metadata"] = serde_json::json!({"private":"secret"});
    let rejected = run(
        dir.path(),
        &changed,
        policy,
        &[
            "--check-link-reachability",
            "--verify-verdict",
            path,
            "--telemetry",
        ],
    );
    assert_eq!(rejected.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&rejected.stderr);
    assert!(!stderr.contains("content_link_reachability"));
    assert!(!stderr.contains("secret"));
    assert_eq!(fs::read(&verdict).unwrap(), current);
    let mut altered = first;
    altered["extensions"]["aexeo.observed-link-binding.v1"]["observations_sha256"] =
        "0".repeat(64).into();
    fs::write(&previous, serde_json::to_vec(&altered).unwrap()).unwrap();
    let rejected = run(
        dir.path(),
        &change,
        policy,
        &[
            "--check-link-reachability",
            "--verify-verdict",
            path,
            "--telemetry",
        ],
    );
    assert_eq!(rejected.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&rejected.stderr);
    assert!(stderr.contains("content_link_reachability"));
    assert!(stderr.contains("\"matched\":false"));
    assert_eq!(fs::read(&verdict).unwrap(), current);
}

#[test]
fn previous_verdict_verification_rejects_opaque_inputs_and_output_alias() {
    let dir = tempfile::tempdir().unwrap();
    let policy = "version=1\nname='test'\n[[gates]]\nid='words'\nkind='word_count'\nmin=1\n";
    assert_eq!(
        run(dir.path(), &input(), policy, &[]).status.code(),
        Some(0)
    );
    let verdict = dir.path().join("verdict.json");
    let path = verdict.to_str().unwrap();
    assert_eq!(
        run(dir.path(), &input(), policy, &["--verify-verdict", path])
            .status
            .code(),
        Some(2)
    );
    assert_eq!(
        run(
            dir.path(),
            &input(),
            policy,
            &["--verify-verdict", path, "--check-link-reachability"]
        )
        .status
        .code(),
        Some(2)
    );
}

#[test]
fn revalidation_detects_changed_collection_without_overwriting_output() {
    let dir = tempfile::tempdir().unwrap();
    let policy = "version=1\nname='test'\n[[gates]]\nid='words'\nkind='word_count'\nmin=1\n";
    let snapshot = dir.path().join("collection.json");
    fs::write(
        &snapshot,
        r#"{"change_id":"example","complete":true,"items":[{"id":"article","text":null}]}"#,
    )
    .unwrap();
    let snapshot_path = snapshot.to_str().unwrap();
    assert_eq!(
        run(
            dir.path(),
            &input(),
            policy,
            &["--collection-snapshot", snapshot_path]
        )
        .status
        .code(),
        Some(0)
    );
    let previous = dir.path().join("previous.json");
    fs::copy(dir.path().join("verdict.json"), &previous).unwrap();
    let old_output = fs::read(dir.path().join("verdict.json")).unwrap();
    fs::write(
        &snapshot,
        r#"{"change_id":"example","complete":false,"items":[{"id":"article","text":null}]}"#,
    )
    .unwrap();
    let output = run(
        dir.path(),
        &input(),
        policy,
        &[
            "--collection-snapshot",
            snapshot_path,
            "--verify-verdict",
            previous.to_str().unwrap(),
        ],
    );
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(
        fs::read(dir.path().join("verdict.json")).unwrap(),
        old_output
    );
}

#[test]
fn link_snapshot_receipt_bundle_revalidates_and_rejects_tampering() {
    use aexeo_contracts::content_gate_decision_inputs::{DecisionInputBytes, DecisionInputs};
    let dir = tempfile::tempdir().unwrap();
    let mut change = input();
    change["entries"][0]["media_type"] = "text/markdown".into();
    change["entries"][0]["after"] = serde_json::json!({"inline":"[first](https://example.org/a)", "sha256":"9b109ca77f8b3af5b0f54a9ae1aeb8d01f5ed6fbd1d350ffe84289bf42000e09"});
    let policy = "version=1\nname='test'\n[[gates]]\nid='links'\nkind='link_reachability'\n";
    let snapshot = dir.path().join("snapshot.json");
    let mut data = serde_json::json!({"version":1,"change_id":"example","entries":{"article":{"version":1,"source_sha256":"9b109ca77f8b3af5b0f54a9ae1aeb8d01f5ed6fbd1d350ffe84289bf42000e09","destinations":["https://example.org/a"],"observations":["reachable"]}}});
    fs::write(&snapshot, serde_json::to_vec(&data).unwrap()).unwrap();
    let path = snapshot.to_str().unwrap();
    let args = [
        "--offline-receipt",
        "--offline-receipt-bundle",
        "--evaluator-identity",
        "trusted",
        "--link-observations",
        path,
        "--telemetry",
    ];
    let created = run(dir.path(), &change, policy, &args);
    assert_eq!(
        created.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&created.stderr)
    );
    let telemetry: serde_json::Value = serde_json::from_slice(&created.stderr).unwrap();
    assert_eq!(telemetry["artifact_count"], 5);
    assert!(!String::from_utf8_lossy(&created.stderr).contains("example.org"));
    let output = dir.path().join("verdict.json");
    let mut bundle: serde_json::Value =
        serde_json::from_slice(&fs::read(&output).unwrap()).unwrap();
    assert_eq!(bundle["schema"], "aexeo.link-snapshot-receipt-bundle/v1");
    assert_eq!(
        bundle["receipt"]["schema"],
        "aexeo.link-snapshot-decision-receipt/v1"
    );
    let manifest: DecisionInputs =
        serde_json::from_value(bundle["receipt"]["decision_inputs"].clone()).unwrap();
    let artifact = |name: &str| bundle["artifacts"][name].as_str().unwrap().as_bytes();
    manifest
        .verify(&DecisionInputBytes {
            change: artifact("change"),
            resolved_policy: artifact("resolved_policy"),
            host_options: artifact("host_options"),
            evaluator: artifact("evaluator"),
            collection: None,
            external_evidence: Some(artifact("external_evidence")),
        })
        .unwrap();
    let previous = dir.path().join("previous.json");
    fs::copy(&output, &previous).unwrap();
    let verify = [
        "--offline-receipt",
        "--offline-receipt-bundle",
        "--evaluator-identity",
        "trusted",
        "--link-observations",
        path,
        "--verify-offline-receipt",
        previous.to_str().unwrap(),
    ];
    let verified = run(dir.path(), &change, policy, &verify);
    assert_eq!(
        verified.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&verified.stderr)
    );
    let saved = fs::read(&output).unwrap();
    data["entries"]["article"]["observations"][0] = "not_found".into();
    fs::write(&snapshot, serde_json::to_vec(&data).unwrap()).unwrap();
    let rejected = run(dir.path(), &change, policy, &verify);
    assert_eq!(rejected.status.code(), Some(2));
    assert_eq!(fs::read(&output).unwrap(), saved);
    let offline_only = [
        "--offline-receipt",
        "--evaluator-identity",
        "trusted",
        "--verify-offline-receipt",
        previous.to_str().unwrap(),
    ];
    assert_eq!(
        run(dir.path(), &change, policy, &offline_only)
            .status
            .code(),
        Some(2)
    );
    assert_eq!(fs::read(&output).unwrap(), saved);
    data["entries"]["article"]["observations"][0] = "reachable".into();
    fs::write(&snapshot, serde_json::to_vec(&data).unwrap()).unwrap();
    bundle["artifacts"]["external_evidence"] = "tampered".into();
    fs::write(&previous, serde_json::to_vec(&bundle).unwrap()).unwrap();
    let rejected_bundle = run(dir.path(), &change, policy, &verify);
    assert_eq!(rejected_bundle.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&rejected_bundle.stderr)
            .contains("invalid previous receipt bundle artifacts")
    );
    assert_eq!(fs::read(&output).unwrap(), saved);
}

#[test]
fn offline_receipt_bundle_exports_exact_inputs_and_rejects_tampering() {
    use aexeo_contracts::content_gate_decision_inputs::{DecisionInputBytes, DecisionInputs};
    let dir = tempfile::tempdir().unwrap();
    let policy = "version=1\nname='test'\n[[gates]]\nid='words'\nkind='word_count'\nmin=1\n";
    let opts = [
        "--offline-receipt",
        "--offline-receipt-bundle",
        "--evaluator-identity",
        "trusted-bât",
        "--telemetry",
    ];
    let created = run(dir.path(), &input(), policy, &opts);
    assert_eq!(
        created.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&created.stderr)
    );
    let telemetry: serde_json::Value = serde_json::from_slice(&created.stderr).unwrap();
    assert_eq!(telemetry["event"], "content_change_offline_bundle");
    assert_eq!(telemetry["artifact_count"], 4);
    assert!(!String::from_utf8_lossy(&created.stderr).contains("trusted-bât"));
    let output = dir.path().join("verdict.json");
    assert_eq!(
        telemetry["bundle_bytes"],
        fs::metadata(&output).unwrap().len()
    );
    let mut bundle: serde_json::Value =
        serde_json::from_slice(&fs::read(&output).unwrap()).unwrap();
    assert_eq!(bundle["schema"], "aexeo.offline-receipt-bundle/v1");
    assert!(bundle["artifacts"]["collection"].is_null());
    assert!(bundle["artifacts"]["external_evidence"].is_null());
    let manifest: DecisionInputs =
        serde_json::from_value(bundle["receipt"]["decision_inputs"].clone()).unwrap();
    let artifact = |name: &str| bundle["artifacts"][name].as_str().unwrap().as_bytes();
    assert!(
        manifest
            .verify(&DecisionInputBytes {
                change: artifact("change"),
                resolved_policy: artifact("resolved_policy"),
                host_options: artifact("host_options"),
                evaluator: artifact("evaluator"),
                collection: None,
                external_evidence: None,
            })
            .is_ok()
    );
    let previous = dir.path().join("previous.json");
    fs::copy(&output, &previous).unwrap();
    let verify = [
        "--offline-receipt",
        "--evaluator-identity",
        "trusted-bât",
        "--verify-offline-receipt",
        previous.to_str().unwrap(),
    ];
    let verified = run(dir.path(), &input(), policy, &verify);
    assert_eq!(
        verified.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&verified.stderr)
    );
    let original_output = fs::read(&output).unwrap();
    bundle["artifacts"]["change"] = "modified".into();
    fs::write(&previous, serde_json::to_vec(&bundle).unwrap()).unwrap();
    let rejected = run(dir.path(), &input(), policy, &verify);
    assert_eq!(rejected.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&rejected.stderr)
            .contains("invalid previous receipt bundle artifacts")
    );
    assert_eq!(fs::read(&output).unwrap(), original_output);
    // Each malformed form is tested against the original, independently of
    // the modified change. None may overwrite the existing output.
    // Recover the exact original artifact from the still-trusted local inputs
    // by creating another bundle, then retain its bytes for the cases below.
    let recreated = run(dir.path(), &input(), policy, &opts);
    assert_eq!(recreated.status.code(), Some(0));
    let valid: serde_json::Value = serde_json::from_slice(&fs::read(&output).unwrap()).unwrap();
    let safe_output = fs::read(&output).unwrap();
    let mut cases = Vec::new();
    let mut absent_to_empty = valid.clone();
    absent_to_empty["artifacts"]["collection"] = "".into();
    cases.push(absent_to_empty);
    let mut evidence = valid.clone();
    evidence["artifacts"]["external_evidence"] = "unexpected".into();
    cases.push(evidence);
    let mut unknown = valid.clone();
    unknown["artifacts"]["extra"] = "x".into();
    cases.push(unknown);
    let mut missing = valid.clone();
    missing["artifacts"]
        .as_object_mut()
        .unwrap()
        .remove("collection");
    cases.push(missing);
    let mut version = valid;
    version["schema"] = "aexeo.offline-receipt-bundle/v2".into();
    cases.push(version);
    for case in cases {
        fs::write(&previous, serde_json::to_vec(&case).unwrap()).unwrap();
        assert_eq!(
            run(dir.path(), &input(), policy, &verify).status.code(),
            Some(2)
        );
        assert_eq!(fs::read(&output).unwrap(), safe_output);
    }
}

#[test]
fn offline_receipt_bundle_binds_collection_completeness() {
    let dir = tempfile::tempdir().unwrap();
    let policy = "version=1\nname='test'\n[[gates]]\nid='words'\nkind='word_count'\nmin=1\n";
    let collection = dir.path().join("collection.json");
    fs::write(
        &collection,
        br#"{"change_id":"example","complete":true,"items":[{"id":"article","text":null}]}"#,
    )
    .unwrap();
    let path = collection.to_str().unwrap();
    let opts = [
        "--offline-receipt",
        "--offline-receipt-bundle",
        "--evaluator-identity",
        "build",
        "--collection-snapshot",
        path,
    ];
    assert_eq!(
        run(dir.path(), &input(), policy, &opts).status.code(),
        Some(0)
    );
    let output = dir.path().join("verdict.json");
    let bundle: serde_json::Value = serde_json::from_slice(&fs::read(&output).unwrap()).unwrap();
    let artifact: serde_json::Value =
        serde_json::from_str(bundle["artifacts"]["collection"].as_str().unwrap()).unwrap();
    assert_eq!(artifact["complete"], true);
    let previous = dir.path().join("previous.json");
    fs::copy(&output, &previous).unwrap();
    let old_output = fs::read(&output).unwrap();
    fs::write(
        &collection,
        br#"{"change_id":"example","complete":false,"items":[{"id":"article","text":null}]}"#,
    )
    .unwrap();
    let verify = [
        "--offline-receipt",
        "--evaluator-identity",
        "build",
        "--collection-snapshot",
        path,
        "--verify-offline-receipt",
        previous.to_str().unwrap(),
    ];
    let rejected = run(dir.path(), &input(), policy, &verify);
    assert_eq!(rejected.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("previous receipt does not match"));
    assert_eq!(fs::read(&output).unwrap(), old_output);
}

#[test]
fn candidate_bound_verdict_rebuilds_and_revalidates_inputs() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("base.json");
    let after = dir.path().join("after.json");
    let prior = dir.path().join("previous.json");
    fs::write(&base, serde_json::to_vec(&serde_json::json!({"scope":"site", "site_id":"site", "site_base_url":null, "base":{"kind":"opaque","revision":"r1"}, "complete":true, "items":[]})).unwrap()).unwrap();
    fs::write(&after, serde_json::to_vec(&serde_json::json!([{"entity_id":"article", "public_path":"/article", "media_type":"text/plain", "revision":null, "sha256":"ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad", "source":"abc", "links":null}])).unwrap()).unwrap();
    let policy = "version=1\nname='test'\n[[gates]]\nid='words'\nkind='word_count'\nmin=1\n";
    let opts = [
        "--offline-receipt",
        "--experimental-bound-verdict",
        "--evaluator-identity",
        "build",
        "--candidate-base",
        base.to_str().unwrap(),
        "--candidate-after",
        after.to_str().unwrap(),
    ];
    let first = run(dir.path(), &input(), policy, &opts);
    assert_eq!(
        first.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let output = dir.path().join("verdict.json");
    let verdict: serde_json::Value = serde_json::from_slice(&fs::read(&output).unwrap()).unwrap();
    assert_eq!(
        verdict["schema"],
        "aexeo.experimental-candidate-bound-verdict/v1"
    );
    fs::copy(&output, &prior).unwrap();
    let mut verify = opts.to_vec();
    verify.extend(["--verify-offline-receipt", prior.to_str().unwrap()]);
    let result = run(dir.path(), &input(), policy, &verify);
    assert_eq!(
        result.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let saved = fs::read(&output).unwrap();
    let mut snapshot: serde_json::Value =
        serde_json::from_slice(&fs::read(&base).unwrap()).unwrap();
    snapshot["scope"] = "changed".into();
    fs::write(&base, serde_json::to_vec(&snapshot).unwrap()).unwrap();
    assert_eq!(
        run(dir.path(), &input(), policy, &verify).status.code(),
        Some(2)
    );
    assert_eq!(fs::read(&output).unwrap(), saved);
    let mut conflict = opts.to_vec();
    conflict.push("--offline-receipt-bundle");
    assert_eq!(
        run(dir.path(), &input(), policy, &conflict).status.code(),
        Some(2)
    );
}

#[test]
fn experimental_bound_verdict_cli_revalidates_and_preserves_output_on_error() {
    let dir = tempfile::tempdir().unwrap();
    let policy = "version=1\nname='test'\n[[gates]]\nid='words'\nkind='word_count'\nmin=1\n";
    let options = [
        "--offline-receipt",
        "--experimental-bound-verdict",
        "--evaluator-identity",
        "trusted-build",
    ];
    let first = run(dir.path(), &input(), policy, &options);
    assert_eq!(
        first.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let output = dir.path().join("verdict.json");
    let produced: serde_json::Value = serde_json::from_slice(&fs::read(&output).unwrap()).unwrap();
    assert_eq!(produced["schema"], "aexeo.experimental-bound-verdict/v1");
    assert_eq!(produced["receipt"]["verdict"]["status"], "pass");
    let prior = dir.path().join("prior.json");
    fs::copy(&output, &prior).unwrap();
    let prior_path = prior.to_str().unwrap();
    let verify = [
        "--offline-receipt",
        "--experimental-bound-verdict",
        "--evaluator-identity",
        "trusted-build",
        "--verify-offline-receipt",
        prior_path,
        "--telemetry",
    ];
    let result = run(dir.path(), &input(), policy, &verify);
    assert_eq!(
        result.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let event: serde_json::Value = serde_json::from_slice(&result.stderr).unwrap();
    assert_eq!(
        event,
        serde_json::json!({"event":"content_change_bound_verdict_revalidation", "version":1, "matched":true})
    );
    let before = fs::read(&output).unwrap();
    let mut changed = input();
    changed["entries"][0]["metadata"] = serde_json::json!({"title":"changed"});
    let result = run(dir.path(), &changed, policy, &verify);
    assert_eq!(result.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&result.stderr).contains("previous bound verdict does not match")
    );
    assert_eq!(fs::read(&output).unwrap(), before);
    assert!(String::from_utf8_lossy(&result.stderr).contains("\"matched\":false"));
    let mut tampered = produced.clone();
    tampered["binding"] = "00".repeat(32).into();
    fs::write(&prior, serde_json::to_vec(&tampered).unwrap()).unwrap();
    let result = run(dir.path(), &input(), policy, &verify);
    assert_eq!(result.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&result.stderr).contains("previous bound verdict does not match")
    );
    assert_eq!(fs::read(&output).unwrap(), before);
    fs::write(&prior, serde_json::to_vec(&produced["receipt"]).unwrap()).unwrap();
    let result = run(dir.path(), &input(), policy, &verify);
    assert_eq!(result.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&result.stderr).contains("invalid previous bound verdict"));
    assert_eq!(fs::read(&output).unwrap(), before);
    fs::write(&prior, serde_json::to_vec(&produced).unwrap()).unwrap();
    let duplicate = String::from_utf8(fs::read(&prior).unwrap())
        .unwrap()
        .replacen("{", "{\"schema\":\"duplicate\",", 1);
    fs::write(&prior, duplicate).unwrap();
    let result = run(dir.path(), &input(), policy, &verify);
    assert_eq!(result.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&result.stderr).contains("invalid previous bound verdict"));
    assert_eq!(fs::read(&output).unwrap(), before);
}

#[test]
fn bound_bundle_preserves_and_rechecks_link_observations() {
    let dir = tempfile::tempdir().unwrap();
    let mut change = input();
    change["entries"][0]["media_type"] = "text/markdown".into();
    change["entries"][0]["after"] = serde_json::json!({"inline":"[first](https://example.org/a)", "sha256":"9b109ca77f8b3af5b0f54a9ae1aeb8d01f5ed6fbd1d350ffe84289bf42000e09"});
    let policy = "version=1\nname='test'\n[[gates]]\nid='links'\nkind='link_reachability'\n";
    let snapshot = dir.path().join("links.json");
    let mut data = serde_json::json!({"version":1,"change_id":"example","entries":{"article":{"version":1,"source_sha256":"9b109ca77f8b3af5b0f54a9ae1aeb8d01f5ed6fbd1d350ffe84289bf42000e09","destinations":["https://example.org/a"],"observations":["reachable"]}}});
    fs::write(&snapshot, serde_json::to_vec(&data).unwrap()).unwrap();
    let args = [
        "--offline-receipt",
        "--experimental-bound-verdict",
        "--offline-receipt-bundle",
        "--evaluator-identity",
        "trusted",
        "--link-observations",
        snapshot.to_str().unwrap(),
    ];
    let first = run(dir.path(), &change, policy, &args);
    assert_eq!(
        first.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let output = dir.path().join("verdict.json");
    let bundle: serde_json::Value = serde_json::from_slice(&fs::read(&output).unwrap()).unwrap();
    assert!(
        bundle["artifacts"]["external_evidence"]
            .as_str()
            .unwrap()
            .contains("example.org")
    );
    let previous = dir.path().join("previous.json");
    fs::copy(&output, &previous).unwrap();
    let mut verify = args.to_vec();
    verify.extend(["--verify-offline-receipt", previous.to_str().unwrap()]);
    assert_eq!(
        run(dir.path(), &change, policy, &verify).status.code(),
        Some(0)
    );
    let before = fs::read(&output).unwrap();
    data["entries"]["article"]["observations"][0] = "not_found".into();
    fs::write(&snapshot, serde_json::to_vec(&data).unwrap()).unwrap();
    assert_eq!(
        run(dir.path(), &change, policy, &verify).status.code(),
        Some(2)
    );
    assert_eq!(fs::read(&output).unwrap(), before);
}

#[test]
fn experimental_bound_bundle_exports_and_revalidates_independent_inputs() {
    let dir = tempfile::tempdir().unwrap();
    let policy = "version=1\nname='test'\n[[gates]]\nid='words'\nkind='word_count'\nmin=1\n";
    let args = [
        "--offline-receipt",
        "--experimental-bound-verdict",
        "--offline-receipt-bundle",
        "--evaluator-identity",
        "trusted-build",
        "--telemetry",
    ];
    let first = run(dir.path(), &input(), policy, &args);
    assert_eq!(
        first.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let telemetry: serde_json::Value = serde_json::from_slice(&first.stderr).unwrap();
    assert_eq!(telemetry["event"], "content_change_bound_bundle");
    assert_eq!(telemetry["artifact_count"], 4);
    assert!(!String::from_utf8_lossy(&first.stderr).contains("trusted-build"));
    let output = dir.path().join("verdict.json");
    let bundle: serde_json::Value = serde_json::from_slice(&fs::read(&output).unwrap()).unwrap();
    assert_eq!(
        bundle["schema"],
        "aexeo.experimental-bound-verdict-bundle/v1"
    );
    assert_eq!(bundle["verdict"]["receipt"]["verdict"]["status"], "pass");
    assert!(
        bundle["artifacts"]["change"]
            .as_str()
            .unwrap()
            .contains("example")
    );
    let prior = dir.path().join("bound-bundle.json");
    fs::copy(&output, &prior).unwrap();
    let verify = [
        "--offline-receipt",
        "--experimental-bound-verdict",
        "--offline-receipt-bundle",
        "--evaluator-identity",
        "trusted-build",
        "--verify-offline-receipt",
        prior.to_str().unwrap(),
    ];
    assert_eq!(
        run(dir.path(), &input(), policy, &verify).status.code(),
        Some(0)
    );
    let before = fs::read(&output).unwrap();
    let mut tampered = bundle.clone();
    tampered["artifacts"]["resolved_policy"] = "changed".into();
    fs::write(&prior, serde_json::to_vec(&tampered).unwrap()).unwrap();
    assert_eq!(
        run(dir.path(), &input(), policy, &verify).status.code(),
        Some(2)
    );
    assert_eq!(fs::read(&output).unwrap(), before);
    fs::write(&prior, serde_json::to_vec(&bundle).unwrap()).unwrap();
    let changed_policy = policy.replace("min=1", "min=2");
    assert_eq!(
        run(dir.path(), &input(), &changed_policy, &verify)
            .status
            .code(),
        Some(2)
    );
    assert_eq!(fs::read(&output).unwrap(), before);
    let duplicate = String::from_utf8(fs::read(&prior).unwrap())
        .unwrap()
        .replacen("{", "{\"schema\":\"duplicate\",", 1);
    fs::write(&prior, duplicate).unwrap();
    assert_eq!(
        run(dir.path(), &input(), policy, &verify).status.code(),
        Some(2)
    );
    assert_eq!(fs::read(&output).unwrap(), before);
}

#[test]
fn offline_receipt_is_explicit_bound_and_revalidated() {
    let dir = tempfile::tempdir().unwrap();
    let policy = "version=1\nname='test'\n[[gates]]\nid='words'\nkind='word_count'\nmin=1\n";
    let identity = [
        "--offline-receipt",
        "--evaluator-identity",
        "trusted-build-1",
    ];
    let first = run(dir.path(), &input(), policy, &identity);
    assert_eq!(
        first.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let path = dir.path().join("verdict.json");
    let receipt: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(receipt["schema"], "aexeo.offline-decision-receipt/v1");
    assert_eq!(receipt["verdict"]["status"], "pass");
    assert!(
        serde_json::from_value::<aexeo_contracts::content_gate_protocol::Verdict>(receipt).is_err()
    );
    let previous = dir.path().join("previous.json");
    fs::copy(&path, &previous).unwrap();
    let previous = previous.to_str().unwrap();
    let verify = [
        "--offline-receipt",
        "--evaluator-identity",
        "trusted-build-1",
        "--verify-offline-receipt",
        previous,
        "--telemetry",
    ];
    let result = run(dir.path(), &input(), policy, &verify);
    assert_eq!(
        result.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let telemetry: serde_json::Value = serde_json::from_slice(&result.stderr).unwrap();
    assert_eq!(
        telemetry,
        serde_json::json!({"event":"content_change_receipt_revalidation","version":1,"matched":true})
    );
    let before = fs::read(&path).unwrap();
    let changed = [
        "--offline-receipt",
        "--evaluator-identity",
        "other-build",
        "--verify-offline-receipt",
        previous,
        "--telemetry",
    ];
    let result = run(dir.path(), &input(), policy, &changed);
    assert_ne!(result.status.code(), Some(0));
    assert_eq!(fs::read(&path).unwrap(), before);
    assert!(String::from_utf8_lossy(&result.stderr).contains("\"matched\":false"));
    let mut modified = input();
    modified["entries"][0]["metadata"] = serde_json::json!({"title":"changed"});
    assert_ne!(
        run(dir.path(), &modified, policy, &verify).status.code(),
        Some(0)
    );
    assert_eq!(fs::read(&path).unwrap(), before);
    assert_ne!(
        run(dir.path(), &input(), policy, &["--offline-receipt"])
            .status
            .code(),
        Some(0)
    );
    assert_ne!(
        run(
            dir.path(),
            &input(),
            policy,
            &[
                "--offline-receipt",
                "--evaluator-identity",
                "trusted-build-1",
                "--check-link-reachability"
            ]
        )
        .status
        .code(),
        Some(0)
    );
    assert_ne!(
        run(
            dir.path(),
            &input(),
            policy,
            &[
                "--offline-receipt",
                "--evaluator-identity",
                "trusted-build-1",
                "--verify-offline-receipt",
                path.to_str().unwrap()
            ]
        )
        .status
        .code(),
        Some(0)
    );
    assert_eq!(fs::read(&path).unwrap(), before);
}

#[test]
fn offline_receipt_rejects_unrepresented_and_duplicate_inputs() {
    let dir = tempfile::tempdir().unwrap();
    let policy = "version=1\nname='test'\n[[gates]]\nid='words'\nkind='word_count'\nmin=1\n";
    let options = ["--offline-receipt", "--evaluator-identity", "build"];
    assert_eq!(
        run(dir.path(), &input(), policy, &options).status.code(),
        Some(0)
    );
    let output_path = dir.path().join("verdict.json");
    let original = fs::read(&output_path).unwrap();
    let mut changed = input();
    changed["entries"][0]["after"]["future_semantics"] = serde_json::json!("unknown");
    assert_ne!(
        run(dir.path(), &changed, policy, &options).status.code(),
        Some(0)
    );
    assert_eq!(fs::read(&output_path).unwrap(), original);
    // Duplicate keys in the original change cannot be represented by serde_json::Value.
    let raw_change = serde_json::to_string(&input()).unwrap().replacen(
        "\"change_id\":\"example\"",
        "\"change_id\":\"example\",\"change_id\":\"example\"",
        1,
    );
    assert_eq!(raw_change.matches("\"change_id\"").count(), 2);
    let change_file = dir.path().join("change.json");
    fs::write(&change_file, raw_change).unwrap();
    let preset_file = dir.path().join("preset.toml");
    let duplicate = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .args([
            "content-change",
            "--preset",
            "test",
            "--preset-source",
            &format!("test={}", preset_file.display()),
        ])
        .args(options)
        .env("CONTENT_GATE_CHANGE_FILE", &change_file)
        .env("CONTENT_GATE_VERDICT_FILE", &output_path)
        .output()
        .unwrap();
    assert_ne!(duplicate.status.code(), Some(0));
    assert_eq!(fs::read(&output_path).unwrap(), original);
    // Ordinary v1 evaluation retains its documented permissive behavior.
    let other = tempfile::tempdir().unwrap();
    assert_eq!(
        run(other.path(), &changed, policy, &[]).status.code(),
        Some(0)
    );

    let previous = dir.path().join("previous.json");
    let mut forged: serde_json::Value = serde_json::from_slice(&original).unwrap();
    forged["verdict"]["future_semantics"] = serde_json::json!(true);
    fs::write(&previous, serde_json::to_vec(&forged).unwrap()).unwrap();
    let verify = [
        "--offline-receipt",
        "--evaluator-identity",
        "build",
        "--verify-offline-receipt",
        previous.to_str().unwrap(),
    ];
    assert_ne!(
        run(dir.path(), &input(), policy, &verify).status.code(),
        Some(0)
    );
    assert_eq!(fs::read(&output_path).unwrap(), original);
    let duplicate = String::from_utf8(original.clone()).unwrap().replacen(
        "\"status\": \"pass\"",
        "\"status\": \"pass\", \"status\": \"pass\"",
        1,
    );
    assert_ne!(duplicate.as_bytes(), original);
    fs::write(&previous, duplicate).unwrap();
    assert_ne!(
        run(dir.path(), &input(), policy, &verify).status.code(),
        Some(0)
    );
    assert_eq!(fs::read(&output_path).unwrap(), original);
}

#[test]
fn offline_receipt_rejects_duplicate_collection_fields() {
    let dir = tempfile::tempdir().unwrap();
    let policy = "version=1\nname='test'\n[[gates]]\nid='words'\nkind='word_count'\nmin=1\n";
    let mut change = input();
    change["entries"][0]["metadata"] = serde_json::json!({"title":"x"});
    let snapshot = dir.path().join("collection.json");
    let valid = r#"{"change_id":"example","complete":true,"items":[{"id":"article","text":"abc","metadata":{"title":"x"}}]}"#;
    fs::write(&snapshot, valid).unwrap();
    let options = [
        "--offline-receipt",
        "--evaluator-identity",
        "build",
        "--collection-snapshot",
        snapshot.to_str().unwrap(),
    ];
    let output = run(dir.path(), &change, policy, &options);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let before = fs::read(dir.path().join("verdict.json")).unwrap();
    for duplicate in [
        valid.replace("\"complete\":true", "\"complete\":true,\"complete\":true"),
        valid.replace("\"title\":\"x\"", "\"title\":\"x\",\"title\":\"x\""),
    ] {
        assert_ne!(duplicate, valid);
        fs::write(&snapshot, duplicate).unwrap();
        assert_ne!(
            run(dir.path(), &change, policy, &options).status.code(),
            Some(0)
        );
        assert_eq!(fs::read(dir.path().join("verdict.json")).unwrap(), before);
    }
}

#[test]
fn baseline_verdict_compares_renamed_entities_without_changing_gate_outcome() {
    let dir = tempfile::tempdir().unwrap();
    let policy =
        "version=1\nname='test'\n[[gates]]\nid='terms'\nkind='forbidden_terms'\nterms=['abc']\n";
    let mut change = input();
    let first = run(dir.path(), &change, policy, &[]);
    assert_eq!(
        first.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let baseline = dir.path().join("baseline.json");
    fs::copy(dir.path().join("verdict.json"), &baseline).unwrap();
    change["entries"][0]["entity_id"] = "renamed".into();
    change["entries"][0]["operation"] = "rename".into();
    change["entries"][0]["previous_entity_id"] = "article".into();
    let without_baseline = run(dir.path(), &change, policy, &[]);
    let next = run(
        dir.path(),
        &change,
        policy,
        &["--baseline-verdict", baseline.to_str().unwrap()],
    );
    assert_eq!(
        next.status.code(),
        without_baseline.status.code(),
        "{}",
        String::from_utf8_lossy(&next.stderr)
    );
    let value: serde_json::Value =
        serde_json::from_slice(&fs::read(dir.path().join("verdict.json")).unwrap()).unwrap();
    let annotation = &value["extensions"]["aexeo.advisory-entity-baseline.v1"];
    assert_eq!(annotation["status"], "advisory");
    assert_eq!(
        annotation["comparison"]["unchanged_findings"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(
        annotation["comparison"]["new_findings"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        annotation["limitations"]
            .as_str()
            .unwrap()
            .contains("Not a publication decision")
    );
}

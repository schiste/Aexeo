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
    let valid = run(dir.path(), &input(), policy, &args);
    assert_eq!(
        valid.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&valid.stderr)
    );

    let mut changed = input();
    changed["entries"][0]["metadata"] = serde_json::json!({"slug":"different"});
    assert_eq!(
        run(dir.path(), &changed, policy, &args).status.code(),
        Some(2)
    );
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

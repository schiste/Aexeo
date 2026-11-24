use serde_json::{Value, json};
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
    time::{Duration, Instant},
};

struct Session {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
}
impl Session {
    fn read(&mut self) -> Value {
        let (sender, receiver) = std::sync::mpsc::channel();
        let line = std::thread::scope(|scope| {
            let output = &mut self.output;
            scope.spawn(move || {
                let mut line = String::new();
                let result = output.read_line(&mut line).map(|n| (n, line));
                let _ = sender.send(result);
            });
            match receiver.recv_timeout(Duration::from_secs(30)) {
                Ok(result) => Some(result.unwrap()),
                Err(_) => {
                    let _ = self.child.kill();
                    None
                }
            }
        })
        .expect("timed out waiting for revision event");
        assert!(line.0 > 0, "missing event");
        serde_json::from_str(&line.1).unwrap()
    }
    fn send(&mut self, value: Value) {
        writeln!(self.input, "{value}").unwrap();
        self.input.flush().unwrap();
    }
    fn finish(mut self) -> i32 {
        drop(self.input);
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                return status.code().unwrap();
            }
            if Instant::now() >= deadline {
                let _ = self.child.kill();
                let _ = self.child.wait();
                panic!("timed out waiting for revision command to exit");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
fn start(draft: &str, preset: &str, format: &str, extra: &[&str]) -> Session {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"));
    cmd.args([
        "content-revise",
        "--input",
        draft,
        "--preset",
        "test",
        "--preset-source",
        preset,
        "--input-format",
        format,
    ]);
    cmd.args(extra);
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let input = child.stdin.take().unwrap();
    let output = BufReader::new(child.stdout.take().unwrap());
    Session {
        child,
        input,
        output,
    }
}
fn preset(min: usize) -> String {
    format!(
        "version=1\nname='test'\n[[gates]]\nid='words'\nkind='word_count'\nmin={min}\nblocking=true\n"
    )
}
#[test]
fn multi_format_revisions_evaluate_real_candidates_without_writing_sources() {
    let dir = tempfile::tempdir().unwrap();
    let policy = dir.path().join("policy.toml");
    fs::write(&policy, preset(3)).unwrap();
    let source = format!("test={}", policy.display());
    for (format, initial, revised, extra) in [
        ("text", "one", "one two three", vec![]),
        ("paragraph-text", "one", "one two three", vec![]),
        ("markdown", "# One", "# One\n\nTwo three", vec![]),
        (
            "html",
            "<main><p>One</p></main>",
            "<main><p>One two three</p></main>",
            vec!["--html-scope", "main"],
        ),
    ] {
        let draft = dir.path().join(format!("{format}.txt"));
        fs::write(&draft, initial).unwrap();
        let mut session = start(draft.to_str().unwrap(), &source, format, &extra);
        let request = session.read();
        assert_eq!(
            request["event"], "proposal_requested",
            "{format}: {request}"
        );
        assert_eq!(request["report"]["gates"][0]["status"], "failed");
        session.send(json!({"action":"submit", "candidate":revised}));
        let done = session.read();
        assert_eq!(done["event"], "complete");
        assert_eq!(done["stop"], "satisfied", "{format}: {done}");
        assert_eq!(done["evaluations"], 2);
        assert_eq!(done["proposals"], 1);
        assert_eq!(done["report"]["gates"][0]["status"], "passed");
        assert_eq!(session.finish(), 0);
        assert_eq!(fs::read_to_string(draft).unwrap(), initial);
    }
}
#[test]
fn source_bound_patches_preserve_unedited_unicode_and_reject_stale_or_invalid_ranges() {
    let dir = tempfile::tempdir().unwrap();
    let draft = dir.path().join("draft");
    let policy = dir.path().join("policy");
    let initial = "Élan";
    fs::write(&draft, initial).unwrap();
    fs::write(&policy, preset(3)).unwrap();
    let source = format!("test={}", policy.display());

    let mut session = start(draft.to_str().unwrap(), &source, "text", &[]);
    let request = session.read();
    session.send(
        json!({"action":"patch", "base_sha256":request["candidate_sha256"],
        "start_byte":initial.len(), "end_byte":initial.len(), "replacement":" deux mots"}),
    );
    let done = session.read();
    assert_eq!(done["stop"], "satisfied");
    assert_eq!(done["report"]["gates"][0]["status"], "passed");
    assert_eq!(session.finish(), 0);
    assert_eq!(fs::read_to_string(&draft).unwrap(), initial);

    for (start_byte, end_byte, stale) in
        [(0, 0, true), (1, 2, false), (0, 99, false), (3, 2, false)]
    {
        let mut session = start(draft.to_str().unwrap(), &source, "text", &[]);
        let request = session.read();
        let digest = if stale {
            json!("stale")
        } else {
            request["candidate_sha256"].clone()
        };
        session.send(json!({"action":"patch", "base_sha256":digest,
            "start_byte":start_byte, "end_byte":end_byte, "replacement":"ignored"}));
        assert_ne!(session.finish(), 0);
        assert_eq!(fs::read_to_string(&draft).unwrap(), initial);
    }
}

#[test]
fn loop_stops_on_cycle_exhaustion_and_decline() {
    let dir = tempfile::tempdir().unwrap();
    let draft = dir.path().join("draft");
    let policy = dir.path().join("policy");
    fs::write(&draft, "one").unwrap();
    fs::write(&policy, preset(3)).unwrap();
    let source = format!("test={}", policy.display());
    let mut cycle = start(draft.to_str().unwrap(), &source, "text", &[]);
    assert_eq!(cycle.read()["event"], "proposal_requested");
    cycle.send(json!({"action":"submit", "candidate":"one"}));
    assert_eq!(cycle.read()["stop"], "cycle");
    assert_eq!(cycle.finish(), 1);
    let mut exhausted = start(
        draft.to_str().unwrap(),
        &source,
        "text",
        &["--max-revisions", "0"],
    );
    assert_eq!(exhausted.read()["stop"], "budget_exhausted");
    assert_eq!(exhausted.finish(), 1);
    let mut declined = start(draft.to_str().unwrap(), &source, "text", &[]);
    declined.read();
    declined.send(json!({"action":"decline"}));
    assert_eq!(declined.read()["stop"], "declined");
    assert_eq!(declined.finish(), 1);
}
#[test]
fn interactive_agent_cannot_exceed_nonzero_budget_across_formats() {
    let dir = tempfile::tempdir().unwrap();
    let policy = dir.path().join("policy.toml");
    fs::write(&policy, preset(10)).unwrap();
    let source = format!("test={}", policy.display());
    for (format, drafts, extra) in [
        ("text", ["one", "two", "three", "four"], vec![]),
        ("paragraph-text", ["one", "two", "three", "four"], vec![]),
        ("markdown", ["# one", "# two", "# three", "# four"], vec![]),
        (
            "html",
            [
                "<main><p>one</p></main>",
                "<main><p>two</p></main>",
                "<main><p>three</p></main>",
                "<main><p>four</p></main>",
            ],
            vec!["--html-scope", "main"],
        ),
    ] {
        let draft = dir.path().join(format!("budget-{format}"));
        fs::write(&draft, drafts[0]).unwrap();
        let mut args = vec!["--max-revisions", "3"];
        args.extend(extra);
        let mut session = start(draft.to_str().unwrap(), &source, format, &args);
        for (index, candidate) in drafts.iter().enumerate().skip(1) {
            let request = session.read();
            assert_eq!(
                request["event"], "proposal_requested",
                "{format}: {request}"
            );
            assert_eq!(request["proposal"], index, "{format}: {request}");
            assert_eq!(request["report"]["gates"][0]["status"], "failed");
            session.send(json!({"action":"submit", "candidate":candidate}));
        }
        let done = session.read();
        assert_eq!(done["event"], "complete", "{format}: {done}");
        assert_eq!(done["stop"], "budget_exhausted", "{format}: {done}");
        assert_eq!(done["evaluations"], 4);
        assert_eq!(done["proposals"], 3);
        assert_eq!(done["report"]["gates"][0]["status"], "failed");
        assert_eq!(session.finish(), 1);
        assert_eq!(fs::read_to_string(draft).unwrap(), drafts[0]);
    }
}

#[test]
fn unavailable_gate_stops_without_proposals() {
    let dir = tempfile::tempdir().unwrap();
    let draft = dir.path().join("draft");
    let policy = dir.path().join("policy");
    fs::write(&draft, "one").unwrap();
    fs::write(&policy, "version=1\nname='test'\n[[gates]]\nid='meta'\nkind='required_metadata'\nfields=['description']\nblocking=true\n").unwrap();
    let mut session = start(
        draft.to_str().unwrap(),
        &format!("test={}", policy.display()),
        "text",
        &[],
    );
    let result = session.read();
    assert_eq!(result["stop"], "incomplete");
    assert_eq!(result["proposals"], 0);
    assert_eq!(session.finish(), 2);
}
#[test]
fn rejects_changed_policy_even_if_gate_identity_is_the_same() {
    let dir = tempfile::tempdir().unwrap();
    let draft = dir.path().join("draft");
    let policy = dir.path().join("policy");
    fs::write(&draft, "one").unwrap();
    fs::write(&policy, preset(3)).unwrap();
    let mut session = start(
        draft.to_str().unwrap(),
        &format!("test={}", policy.display()),
        "text",
        &[],
    );
    assert_eq!(session.read()["event"], "proposal_requested");
    fs::write(&policy, preset(2)).unwrap();
    session.send(json!({"action":"submit", "candidate":"one two"}));
    assert_eq!(session.finish(), 2);
    assert_eq!(fs::read_to_string(draft).unwrap(), "one");
}

#[test]
fn rejects_changed_metadata_between_evaluations() {
    let dir = tempfile::tempdir().unwrap();
    let draft = dir.path().join("draft");
    let policy = dir.path().join("policy");
    let metadata = dir.path().join("metadata.json");
    fs::write(&draft, "one").unwrap();
    fs::write(&policy, preset(3)).unwrap();
    fs::write(&metadata, r#"{"description":"first"}"#).unwrap();
    let mut session = start(
        draft.to_str().unwrap(),
        &format!("test={}", policy.display()),
        "text",
        &["--metadata", metadata.to_str().unwrap()],
    );
    assert_eq!(session.read()["event"], "proposal_requested");
    fs::write(&metadata, r#"{"description":"changed"}"#).unwrap();
    session.send(json!({"action":"submit", "candidate":"one two three"}));
    assert_eq!(session.finish(), 2);
    assert_eq!(fs::read_to_string(draft).unwrap(), "one");
}

#[test]
fn telemetry_is_aggregate_and_does_not_include_draft() {
    let dir = tempfile::tempdir().unwrap();
    let draft = dir.path().join("draft");
    let policy = dir.path().join("policy");
    let private_text = "private-handover-sentinel";
    fs::write(&draft, private_text).unwrap();
    fs::write(&policy, preset(3)).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .args([
            "content-revise",
            "--input",
            draft.to_str().unwrap(),
            "--preset",
            "test",
            "--preset-source",
            &format!("test={}", policy.display()),
            "--input-format",
            "text",
            "--max-revisions",
            "0",
            "--telemetry",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let telemetry = String::from_utf8(output.stderr).unwrap();
    assert!(!telemetry.contains(private_text));
    let event: Value = serde_json::from_str(&telemetry).unwrap();
    assert_eq!(event["event"], "content_revision_loop");
    assert_eq!(event["evaluations"], 1);
    assert_eq!(event["patch_proposals"], 0);
    assert!(event["evaluation_ms"].as_f64().is_some());
    assert!(event["elapsed_ms"].as_f64().is_some());
}

#[test]
fn proposal_request_includes_original_markdown_finding_location() {
    let dir = tempfile::tempdir().unwrap();
    let draft = dir.path().join("draft.md");
    let policy = dir.path().join("policy.toml");
    fs::write(&draft, "# Header\n\nA generic phrase appears.\n").unwrap();
    fs::write(&policy, "version=1\nname='test'\n[[gates]]\nid='phrase'\nkind='forbidden_terms'\nterms=['generic phrase']\nblocking=false\n").unwrap();
    let mut session = start(
        draft.to_str().unwrap(),
        &format!("test={}", policy.display()),
        "markdown",
        &[],
    );
    let request = session.read();
    let location = &request["report"]["source_locations"][0];
    assert_eq!(location["gate_id"], "phrase");
    assert_eq!(location["line"], 3);
    session.send(
        json!({"action":"submit", "candidate":"# Header\n\nA useful description appears.\n"}),
    );
    assert_eq!(session.read()["stop"], "satisfied");
    assert_eq!(session.finish(), 0);
}

#[test]
fn adapter_projection_revisions_bind_exact_bytes_and_withhold_missing_capabilities() {
    let dir = tempfile::tempdir().unwrap();
    let draft = dir.path().join("projection.json");
    let policy = dir.path().join("policy.toml");
    fs::write(
        &policy,
        "version=1\nname='test'\n[[gates]]\nid='words'\nkind='word_count'\nmin=3\nblocking=true\n[[gates]]\nid='paragraphs'\nkind='paragraph_count'\nmin=1\nblocking=true\n",
    )
    .unwrap();
    let initial =
        r#"{"schema":"aexeo.content-adapter/v1","text":"one","paragraphs":[{"start":0,"end":3}]}"#;
    let revised = r#"{"schema":"aexeo.content-adapter/v1","text":"one two three","paragraphs":[{"start":0,"end":13}]}"#;
    fs::write(&draft, initial).unwrap();
    let source = format!("test={}", policy.display());
    let mut session = start(draft.to_str().unwrap(), &source, "adapter-json", &[]);
    let request = session.read();
    assert_eq!(request["event"], "proposal_requested");
    session.send(json!({"action":"submit", "candidate":revised}));
    let result = session.read();
    assert_eq!(result["stop"], "satisfied");
    assert_eq!(result["report"]["input_format"], "adapter-json");
    assert_eq!(
        result["candidate_sha256"],
        result["report"]["input_fingerprint"]["text_sha256"]
    );
    assert_eq!(session.finish(), 0);
    assert_eq!(fs::read_to_string(&draft).unwrap(), initial);

    let mut session = start(draft.to_str().unwrap(), &source, "adapter-json", &[]);
    assert_eq!(session.read()["event"], "proposal_requested");
    session.send(json!({"action":"submit", "candidate":r#"{"schema":"aexeo.content-adapter/v1","text":"one two three"}"#}));
    let result = session.read();
    assert_eq!(result["stop"], "incomplete");
    assert_eq!(result["report"]["gates"][1]["status"], "not_checked");
    assert_eq!(session.finish(), 2);

    let mut session = start(draft.to_str().unwrap(), &source, "adapter-json", &[]);
    assert_eq!(session.read()["event"], "proposal_requested");
    session.send(json!({"action":"submit", "candidate":r#"{"schema":"aexeo.content-adapter/v1","text":"abc","text":"different"}"#}));
    assert_eq!(session.finish(), 2);
    assert_eq!(fs::read_to_string(&draft).unwrap(), initial);
}

#[test]
fn adapter_revision_rejects_changed_metadata_coverage() {
    let dir = tempfile::tempdir().unwrap();
    let draft = dir.path().join("projection.json");
    let policy = dir.path().join("policy.toml");
    fs::write(&policy, preset(3)).unwrap();
    fs::write(&draft, r#"{"schema":"aexeo.content-adapter/v1","text":"one","metadata":{},"observed_metadata_fields":[]}"#).unwrap();
    let source = format!("test={}", policy.display());
    let mut session = start(draft.to_str().unwrap(), &source, "adapter-json", &[]);
    assert_eq!(session.read()["event"], "proposal_requested");
    session.send(json!({"action":"submit", "candidate":r#"{"schema":"aexeo.content-adapter/v1","text":"one two three","metadata":{},"observed_metadata_fields":["description"]}"#}));
    assert_eq!(session.finish(), 2);
}

#[test]
fn adapter_revisions_recompute_native_paths_for_each_exact_candidate() {
    let dir = tempfile::tempdir().unwrap();
    let draft = dir.path().join("projection.json");
    let policy = dir.path().join("policy.toml");
    fs::write(&policy, "version=1\nname='test'\n[[gates]]\nid='terms'\nkind='forbidden_terms'\nterms=['bad']\nblocking=true\n").unwrap();
    let candidate = |text: &str, block: usize| {
        json!({"schema":"aexeo.content-adapter/v1", "text":text,
            "native_source_map":{"spans":[{"start":0,"end":text.len(),"path":["data","content",block,"children",0,"text"]}]}}).to_string()
    };
    fs::write(&draft, candidate("🙂 bad", 0)).unwrap();
    let source = format!("test={}", policy.display());
    let mut session = start(draft.to_str().unwrap(), &source, "adapter-json", &[]);
    let request = session.read();
    assert_eq!(
        request["report"]["native_source_locations"][0]["utf16_offset"],
        3
    );
    assert_eq!(
        request["report"]["native_source_locations"][0]["path"][2],
        0
    );
    session.send(json!({"action":"submit", "candidate":candidate("New 🙂 bad", 1)}));
    let request = session.read();
    assert_eq!(request["event"], "proposal_requested");
    assert_eq!(
        request["report"]["native_source_locations"][0]["utf16_offset"],
        7
    );
    assert_eq!(
        request["report"]["native_source_locations"][0]["path"][2],
        1
    );
    session.send(json!({"action":"submit", "candidate":candidate("New 🙂 fine", 1)}));
    let result = session.read();
    assert_eq!(result["stop"], "satisfied");
    assert_eq!(
        result["candidate_sha256"],
        result["report"]["input_fingerprint"]["text_sha256"]
    );
    assert_eq!(session.finish(), 0);
}

#[test]
fn adapter_revision_loop_recomputes_native_structural_paths() {
    let dir = tempfile::tempdir().unwrap();
    let draft = dir.path().join("projection.json");
    let policy = dir.path().join("policy.toml");
    fs::write(&policy, "version=1\nname='test'\n[[gates]]\nid='heading'\nkind='heading_length'\nmax_chars=3\nblocking=true\n").unwrap();
    let candidate = |heading: &str, block: usize| {
        json!({"schema":"aexeo.content-adapter/v1", "text":"Body", "headings":[heading],
            "native_source_map":{"spans":[], "heading_paths":[["data","content",block,"children",0,"text"]]}}).to_string()
    };
    fs::write(&draft, candidate("Long", 0)).unwrap();
    let source = format!("test={}", policy.display());
    let mut session = start(draft.to_str().unwrap(), &source, "adapter-json", &[]);
    let request = session.read();
    assert_eq!(
        request["report"]["native_source_locations"][0]["path"][2],
        0
    );
    session.send(json!({"action":"submit", "candidate":candidate("Still long", 3)}));
    let request = session.read();
    assert_eq!(request["event"], "proposal_requested");
    assert_eq!(
        request["report"]["native_source_locations"][0]["path"][2],
        3
    );
    let final_candidate = candidate("OK", 3);
    session.send(json!({"action":"submit", "candidate":final_candidate}));
    let result = session.read();
    assert_eq!(result["stop"], "satisfied");
    assert_eq!(
        result["candidate_sha256"],
        result["report"]["input_fingerprint"]["text_sha256"]
    );
    assert_eq!(session.finish(), 0);
}

#[test]
fn revisions_use_only_observations_bound_to_each_candidate() {
    let dir = tempfile::tempdir().unwrap();
    let draft = dir.path().join("draft.md");
    let next = dir.path().join("next.md");
    let policy = dir.path().join("policy.toml");
    let first_snapshot = dir.path().join("first.json");
    let next_snapshot = dir.path().join("next.json");
    let initial = "[broken](https://example.org/broken)";
    let candidate = "[working](https://example.org/working)";
    fs::write(&draft, initial).unwrap();
    fs::write(&next, candidate).unwrap();
    fs::write(
        &policy,
        "version=1\nname='test'\n[[gates]]\nid='links'\nkind='link_reachability'\nblocking=true\n",
    )
    .unwrap();
    for (article, path, observation) in [
        (&draft, &first_snapshot, "not_found"),
        (&next, &next_snapshot, "reachable"),
    ] {
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
        assert!(exported.status.success());
        let mut value: Value = serde_json::from_slice(&exported.stdout).unwrap();
        value["observations"] = json!([observation]);
        fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
    }
    let source = format!("test={}", policy.display());
    let extra = ["--link-observations", first_snapshot.to_str().unwrap()];
    let mut session = start(draft.to_str().unwrap(), &source, "markdown", &extra);
    let request = session.read();
    assert_eq!(request["report"]["gates"][0]["status"], "failed");
    let snapshot_hash = |path: &std::path::Path| -> String {
        ring::digest::digest(&ring::digest::SHA256, &fs::read(path).unwrap())
            .as_ref()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    };
    assert_eq!(
        request["report"]["input_fingerprint"]["link_observations_sha256"],
        snapshot_hash(&first_snapshot)
    );
    session
        .send(json!({"action":"submit", "candidate":candidate, "link_observations":next_snapshot}));
    let result = session.read();
    assert_eq!(result["stop"], "satisfied");
    assert_eq!(result["report"]["gates"][0]["status"], "passed");
    assert_eq!(
        result["report"]["input_fingerprint"]["link_observations_sha256"],
        snapshot_hash(&next_snapshot)
    );
    assert_eq!(session.finish(), 0);

    let mut missing = start(draft.to_str().unwrap(), &source, "markdown", &extra);
    assert_eq!(missing.read()["event"], "proposal_requested");
    missing.send(json!({"action":"submit", "candidate":candidate}));
    let result = missing.read();
    assert_eq!(result["stop"], "incomplete");
    assert_eq!(result["report"]["gates"][0]["status"], "not_checked");
    assert!(
        result["report"]["input_fingerprint"]
            .get("link_observations_sha256")
            .is_none()
    );
    assert_eq!(missing.finish(), 2);

    let mut stale = start(draft.to_str().unwrap(), &source, "markdown", &extra);
    assert_eq!(stale.read()["event"], "proposal_requested");
    stale.send(
        json!({"action":"submit", "candidate":candidate, "link_observations":first_snapshot}),
    );
    assert_eq!(stale.finish(), 2);

    let mut measured = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .args([
            "content-revise",
            "--input",
            draft.to_str().unwrap(),
            "--preset",
            "test",
            "--preset-source",
            &source,
            "--link-observations",
            first_snapshot.to_str().unwrap(),
            "--telemetry",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    writeln!(
        measured.stdin.take().unwrap(),
        "{}",
        json!({"action":"submit", "candidate":candidate, "link_observations":next_snapshot})
    )
    .unwrap();
    let output = measured.wait_with_output().unwrap();
    assert!(output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(!stderr.contains("example.org"));
    assert!(!stderr.contains(first_snapshot.to_str().unwrap()));
    let event: Value = serde_json::from_str(&stderr).unwrap();
    assert_eq!(event["initial_link_observations"], true);
    assert_eq!(event["proposals_with_link_observations"], 1);
    assert_eq!(fs::read_to_string(draft).unwrap(), initial);
}

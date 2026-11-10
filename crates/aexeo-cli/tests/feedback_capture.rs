use std::{fs, process::Command};

#[test]
fn captures_metadata_hash_without_storing_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let text = dir.path().join("text");
    let report = dir.path().join("report");
    let metadata = dir.path().join("metadata");
    let note = dir.path().join("note");
    fs::write(&text, "text").unwrap();
    fs::write(&report, r#"{"gates":[{"gate_id":"g","status":"passed"}]}"#).unwrap();
    fs::write(&metadata, r#"{"titre":"Été","lang":"fr"}"#).unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .args(["feedback-capture", "--text"])
        .arg(&text)
        .arg("--report")
        .arg(&report)
        .arg("--metadata")
        .arg(&metadata)
        .args(["--gate", "g", "--example", "e", "--output"])
        .arg(&note)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let note = fs::read_to_string(note).unwrap();
    assert!(!note.contains("Été"));
    let note: serde_json::Value = serde_json::from_str(&note).unwrap();
    assert_eq!(
        note["metadata_sha256"],
        "c2dd6ca92cc1c5c1a1208812e55ddfac14cbdbcb54ef0f9441d0deb31e3d541e"
    );
}

#[test]
fn captures_private_note_and_refuses_overwrite() {
    let dir = tempfile::tempdir().unwrap();
    let text = dir.path().join("text");
    let report = dir.path().join("report");
    let note = dir.path().join("note");
    fs::write(&text, b"private text\r\n").unwrap();
    fs::write(
        &report,
        br#"{"gates":[{"gate_id":"gate-private","status":"failed"}]}"#,
    )
    .unwrap();
    let run = || {
        Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
            .args(["feedback-capture", "--text"])
            .arg(&text)
            .arg("--report")
            .arg(&report)
            .args([
                "--gate",
                "gate-private",
                "--example",
                "example-private",
                "--human-status",
                "passed",
                "--reviewer",
                "person-private",
                "--method",
                "manual",
                "--telemetry",
                "--output",
            ])
            .arg(&note)
            .output()
            .unwrap()
    };
    let result = run();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let event: serde_json::Value = serde_json::from_slice(&result.stderr).unwrap();
    assert_eq!(event["operation"], "capture");
    let telemetry = String::from_utf8(result.stderr).unwrap();
    assert!(!telemetry.contains("private"));
    let bytes = fs::read(&note).unwrap();
    let parsed: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(parsed["human_status"], "passed");
    assert!(
        !String::from_utf8(bytes.clone())
            .unwrap()
            .contains("private text")
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(fs::metadata(&note).unwrap().permissions().mode() & 0o077, 0);
    }
    assert_eq!(run().status.code(), Some(2));
    assert_eq!(fs::read(note).unwrap(), bytes);
}

#[test]
fn malformed_report_is_an_input_error() {
    let dir = tempfile::tempdir().unwrap();
    let text = dir.path().join("text");
    let report = dir.path().join("report");
    fs::write(&text, b"text").unwrap();
    fs::write(&report, b"not json").unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .args(["feedback-capture", "--text"])
        .arg(text)
        .arg("--report")
        .arg(report)
        .args(["--gate", "g", "--example", "e", "--output"])
        .arg(dir.path().join("note"))
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(2));
}

#[test]
fn export_matches_existing_python_validator_on_representative_inputs() {
    use serde_json::{Value, json};
    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../scripts/content-review-note.py");
    let corpus = json!({"version":1,"gate_id":"g","examples":[{"id":"e","split":"development","text":"abc\r\n","expected":"passed"}]});
    let note = json!({"version":1,"kind":"reported_human_feedback","gate_id":"g","example_id":"e","text_sha256":"552bab6864c7a7b69a502ed1854b9245c0e1a30f008aaa0b281da62585fdb025","model_status":"failed","human_status":"passed","reviewer":"human","method":"manual","reason":"private reason"});
    let mut cases: Vec<(Value, Value)> = vec![(corpus.clone(), note.clone())];
    let mut policy_note = note.clone();
    policy_note["resolved_policy_fingerprint"] = json!({"version": 1, "sha256": "a".repeat(64)});
    cases.push((corpus.clone(), policy_note.clone()));
    policy_note["resolved_policy_fingerprint"]["sha256"] = json!("bad");
    cases.push((corpus.clone(), policy_note));
    let mut metadata_corpus = corpus.clone();
    metadata_corpus["examples"][0]["metadata"] = json!({"titre":"Été", "lang":"fr"});
    let mut metadata_note = note.clone();
    metadata_note["metadata_sha256"] =
        json!("c2dd6ca92cc1c5c1a1208812e55ddfac14cbdbcb54ef0f9441d0deb31e3d541e");
    cases.push((metadata_corpus.clone(), metadata_note.clone()));
    cases.push((metadata_corpus.clone(), note.clone()));
    metadata_corpus["examples"][0]["metadata"]["lang"] = json!("en");
    cases.push((metadata_corpus, metadata_note));
    for (field, value) in [
        ("human_status", json!("uncertain")),
        ("gate_id", json!("other")),
        ("text_sha256", json!("stale")),
        ("model_status", json!("unknown")),
        ("reviewer", json!("  ")),
    ] {
        let mut changed = note.clone();
        changed[field] = value;
        cases.push((corpus.clone(), changed));
    }
    let mut duplicate = corpus.clone();
    duplicate["examples"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id":"e2","split":"holdout","text":"abc\r\n","expected":"passed"}));
    cases.push((duplicate, note.clone()));
    let mut duplicate_id = corpus.clone();
    duplicate_id["examples"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id":"e","split":"development","text":"different","expected":"passed"}));
    cases.push((duplicate_id, note.clone()));
    let mut synthetic = corpus.clone();
    synthetic["fixture_kind"] = json!("synthetic_regression");
    synthetic["examples"][0]["review_status"] = json!("reviewed");
    cases.push((synthetic, note));
    for (corpus, note) in cases {
        let dir = tempfile::tempdir().unwrap();
        let cp = dir.path().join("corpus");
        let np = dir.path().join("note");
        fs::write(&cp, serde_json::to_vec(&corpus).unwrap()).unwrap();
        fs::write(&np, serde_json::to_vec(&note).unwrap()).unwrap();
        let native_path = dir.path().join("native");
        let python_path = dir.path().join("python");
        let native = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
            .args(["feedback-export", "--corpus"])
            .arg(&cp)
            .arg("--note")
            .arg(&np)
            .arg("--output")
            .arg(&native_path)
            .output()
            .unwrap();
        let python = Command::new("python3")
            .arg(&script)
            .args(["export-review", "--corpus"])
            .arg(&cp)
            .arg("--note")
            .arg(&np)
            .arg("--output")
            .arg(&python_path)
            .output()
            .unwrap();
        assert_eq!(
            native.status.success(),
            python.status.success(),
            "native: {}; python: {}",
            String::from_utf8_lossy(&native.stderr),
            String::from_utf8_lossy(&python.stderr)
        );
        if native.status.success() {
            let a: Value = serde_json::from_slice(&fs::read(native_path).unwrap()).unwrap();
            let b: Value = serde_json::from_slice(&fs::read(python_path).unwrap()).unwrap();
            assert_eq!(a, b);
            assert!(!a.to_string().contains("private reason"));
        }
    }
}

#[test]
fn export_telemetry_is_private_and_existing_output_is_preserved() {
    let dir = tempfile::tempdir().unwrap();
    let corpus = dir.path().join("corpus");
    let note = dir.path().join("note");
    let output = dir.path().join("output");
    fs::write(&corpus, br#"{"version":1,"gate_id":"private-gate","examples":[{"id":"private-example","split":"development","text":"abc\r\n","expected":"passed"}]}"#).unwrap();
    fs::write(&note, br#"{"version":1,"kind":"reported_human_feedback","gate_id":"private-gate","example_id":"private-example","text_sha256":"552bab6864c7a7b69a502ed1854b9245c0e1a30f008aaa0b281da62585fdb025","model_status":"failed","human_status":"passed","reviewer":"private-human","method":"manual"}"#).unwrap();
    let run = || {
        Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
            .args(["feedback-export", "--corpus"])
            .arg(&corpus)
            .arg("--note")
            .arg(&note)
            .arg("--telemetry")
            .arg("--output")
            .arg(&output)
            .output()
            .unwrap()
    };
    let first = run();
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let event: serde_json::Value = serde_json::from_slice(&first.stderr).unwrap();
    assert_eq!(event["operation"], "export-review");
    assert!(!String::from_utf8(first.stderr).unwrap().contains("private"));
    let bytes = fs::read(&output).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&output).unwrap().permissions().mode() & 0o077,
            0
        );
    }
    assert_eq!(run().status.code(), Some(2));
    assert_eq!(fs::read(output).unwrap(), bytes);
}

#[test]
fn capture_checks_report_against_original_markdown_and_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let text = dir.path().join("article.md");
    let metadata = dir.path().join("metadata.json");
    let report = dir.path().join("report.json");
    fs::write(&text, "# Titre\r\n\r\nTexte privé.\r\n").unwrap();
    fs::write(&metadata, r#"{"lang":"fr"}"#).unwrap();
    let evaluated = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .arg("content")
        .arg(&text)
        .args([
            "--input-format",
            "markdown",
            "--preset",
            "article",
            "--format",
            "json",
            "--metadata",
        ])
        .arg(&metadata)
        .output()
        .unwrap();
    assert!(
        evaluated.status.success(),
        "{}",
        String::from_utf8_lossy(&evaluated.stderr)
    );
    fs::write(&report, &evaluated.stdout).unwrap();
    let report_json: serde_json::Value = serde_json::from_slice(&evaluated.stdout).unwrap();
    assert_eq!(report_json["input_fingerprint"]["version"], 1);
    assert!(!String::from_utf8_lossy(&evaluated.stdout).contains("Texte privé"));
    let capture = |output: &std::path::Path| {
        Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
            .args(["feedback-capture", "--text"])
            .arg(&text)
            .arg("--report")
            .arg(&report)
            .arg("--metadata")
            .arg(&metadata)
            .args([
                "--gate",
                "content.minimum_words",
                "--example",
                "e",
                "--output",
            ])
            .arg(output)
            .output()
            .unwrap()
    };
    let note = dir.path().join("note.json");
    let result = capture(&note);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let note_json: serde_json::Value = serde_json::from_slice(&fs::read(note).unwrap()).unwrap();
    assert_eq!(note_json["report_binding"], "verified");
    assert_eq!(
        note_json["resolved_policy_fingerprint"],
        report_json["resolved_policy_fingerprint"]
    );
    fs::write(&text, "# Autre texte\n").unwrap();
    assert_eq!(
        capture(&dir.path().join("stale-text")).status.code(),
        Some(2)
    );
    fs::write(&text, "# Titre\r\n\r\nTexte privé.\r\n").unwrap();
    fs::write(&metadata, r#"{"lang":"en"}"#).unwrap();
    assert_eq!(
        capture(&dir.path().join("stale-metadata")).status.code(),
        Some(2)
    );
}

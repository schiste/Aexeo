use std::{fs, process::Command};

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

use std::{fs, process::Command};

#[test]
fn gate_timings_are_opt_in_and_do_not_change_or_expose_content() {
    let dir = tempfile::tempdir().unwrap();
    let preset = dir.path().join("preset.toml");
    let article = dir.path().join("article.txt");
    fs::write(&preset, "version=1\nname='private-preset'\n[[gates]]\nid='private-gate'\nkind='word_count'\nmin=1\n[[gates]]\nid='missing'\nkind='required_headings'\nheadings=['secret heading']\nblocking=false\n").unwrap();
    fs::write(&article, "private body").unwrap();
    let run = |telemetry: bool| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"));
        command.args([
            "content",
            article.to_str().unwrap(),
            "--preset",
            "custom",
            "--preset-source",
            &format!("custom={}", preset.display()),
            "--format",
            "json",
        ]);
        if telemetry {
            command.arg("--telemetry");
        }
        command.output().unwrap()
    };
    let plain = run(false);
    let measured = run(true);
    assert_eq!(plain.status.code(), Some(0));
    assert_eq!(measured.status.code(), Some(0));
    assert_eq!(plain.stdout, measured.stdout);
    assert!(plain.stderr.is_empty());
    let event: serde_json::Value = serde_json::from_slice(&measured.stderr).unwrap();
    let timings = event["gate_timings_ms"].as_array().unwrap();
    assert_eq!(timings.len(), 2);
    assert!(
        timings
            .iter()
            .all(|value| value.as_f64().is_some_and(|v| v >= 0.0))
    );
    let stderr = String::from_utf8_lossy(&measured.stderr);
    for secret in [
        "private body",
        "private-preset",
        "private-gate",
        "secret heading",
    ] {
        assert!(!stderr.contains(secret));
    }
}

#[test]
fn length_counts_appear_in_reports_without_leaking_into_telemetry() {
    let dir = tempfile::tempdir().unwrap();
    let preset = dir.path().join("preset.toml");
    let article = dir.path().join("article.md");
    fs::write(&preset, "version=1\nname='lengths'\n[[gates]]\nid='p'\nkind='paragraph_length'\nmax_words=2\nblocking=false\n[[gates]]\nid='s'\nkind='sentence_length'\nmax_words=2\nblocking=false\n").unwrap();
    fs::write(&article, "secret three words.\n\nShort.\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .args([
            "content",
            article.to_str().unwrap(),
            "--input-format",
            "markdown",
            "--preset",
            "custom",
            "--preset-source",
            &format!("custom={}", preset.display()),
            "--format",
            "json",
            "--telemetry",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        report["gates"][0]["measurements"]["paragraphs_exceeding_limit"],
        1
    );
    assert_eq!(
        report["gates"][1]["measurements"]["sentences_exceeding_limit"],
        1
    );
    let event: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(event["event"], "content_evaluation");
    let telemetry = String::from_utf8_lossy(&output.stderr);
    assert!(!telemetry.contains("secret"));
    assert_eq!(event["length_checks"]["measured_gates"], 2);
    assert_eq!(event["length_checks"]["exceedances"], 2);
    assert!(!telemetry.contains("exceeding_limit"));
}

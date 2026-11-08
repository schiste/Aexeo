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

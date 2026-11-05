use std::{fs, process::Command};

#[test]
fn markdown_destinations_are_checked_without_leaking_them_in_telemetry() {
    let dir = tempfile::tempdir().unwrap();
    let preset = dir.path().join("preset.toml");
    let article = dir.path().join("article.md");
    fs::write(&preset, "version=1\nname='links'\n[[gates]]\nid='urls'\nkind='link_destinations'\nallowed_schemes=['https']\n").unwrap();
    fs::write(
        &article,
        "[bad](javascript:private) [good][ref]\n\n[ref]: https://example.org",
    )
    .unwrap();
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
    assert_eq!(output.status.code(), Some(1));
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["gates"][0]["status"], "failed");
    assert_eq!(
        report["gates"][0]["measurements"]["checked_destinations"],
        2
    );
    assert_eq!(
        report["gates"][0]["measurements"]["invalid_destinations"],
        1
    );
    assert!(!String::from_utf8_lossy(&output.stderr).contains("private"));
}

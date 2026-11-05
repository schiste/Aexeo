use std::{fs, process::Command};

#[test]
fn markdown_destinations_are_checked_without_leaking_them_in_telemetry() {
    let dir = tempfile::tempdir().unwrap();
    let preset = dir.path().join("preset.toml");
    let article = dir.path().join("article.md");
    fs::write(&preset, "version=1\nname='links'\n[[gates]]\nid='urls'\nkind='link_destinations'\nallowed_schemes=['https']\n").unwrap();
    fs::write(
        &article,
        "é [bad](javascript:private)\n[badref][ref] [good](https://example.org)\n\n[ref]: javascript:private",
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
        3
    );
    assert_eq!(
        report["gates"][0]["measurements"]["invalid_destinations"],
        2
    );
    assert_eq!(report["source_locations"][0]["finding_index"], 0);
    assert_eq!(report["source_locations"][0]["line"], 1);
    assert_eq!(report["source_locations"][0]["column"], 3);
    assert_eq!(report["source_locations"][1]["finding_index"], 1);
    assert_eq!(report["source_locations"][1]["line"], 2);
    assert_eq!(report["source_locations"][1]["column"], 1);
    let telemetry: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(telemetry["source_mapping"]["mapped"], 2);
    assert_eq!(telemetry["source_mapping"]["unmapped"], 0);
    assert!(!String::from_utf8_lossy(&output.stderr).contains("private"));
}

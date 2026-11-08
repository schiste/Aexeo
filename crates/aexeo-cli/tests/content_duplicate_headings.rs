use std::{fs, process::Command};

#[test]
fn markdown_duplicate_headings_use_custom_policy_and_aggregate_telemetry() {
    let dir = tempfile::tempdir().unwrap();
    let preset = dir.path().join("preset.toml");
    let article = dir.path().join("article.md");
    fs::write(&preset, "version=1\nname='headings'\n[[gates]]\nid='unique'\nkind='duplicate_headings'\ncase_sensitive=false\nblocking=false\n").unwrap();
    fs::write(&article, "# Private heading\n\n## PRIVATE HEADING\n").unwrap();
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
    assert_eq!(report["gates"][0]["status"], "failed");
    assert_eq!(report["gates"][0]["measurements"]["heading_count"], 2);
    assert_eq!(report["gates"][0]["measurements"]["duplicate_headings"], 1);
    assert_eq!(report["source_locations"][0]["line"], 3);
    assert_eq!(report["source_locations"][0]["column"], 1);
    assert!(!String::from_utf8_lossy(&output.stderr).contains("Private heading"));
}

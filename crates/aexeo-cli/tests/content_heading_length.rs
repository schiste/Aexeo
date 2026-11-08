use std::{fs, process::Command};

#[test]
fn markdown_heading_length_uses_portable_policy_and_private_telemetry() {
    let dir = tempfile::tempdir().unwrap();
    let preset = dir.path().join("preset.toml");
    let article = dir.path().join("article.md");
    fs::write(&preset, "version=1\nname='headings'\n[[gates]]\nid='length'\nkind='heading_length'\nmax_chars=3\nblocking=false\n").unwrap();
    fs::write(&article, "# ok\n\n## é😀abcd\n").unwrap();
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
    assert_eq!(
        report["gates"][0]["measurements"]["longest_heading_chars"],
        6
    );
    assert_eq!(
        report["gates"][0]["measurements"]["headings_exceeding_limit"],
        1
    );
    assert_eq!(report["source_locations"][0]["line"], 3);
    assert_eq!(report["source_locations"][0]["column"], 1);
    let telemetry: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(telemetry["source_mapping"]["mapped"], 1);
    assert!(!String::from_utf8_lossy(&output.stderr).contains("é😀abcd"));
}

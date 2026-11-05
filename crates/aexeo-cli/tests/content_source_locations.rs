use std::{fs, process::Command};

#[test]
fn markdown_report_adds_only_exact_source_positions() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("article.md");
    let preset = dir.path().join("preset.toml");
    fs::write(
        &file,
        "---\ntitle: hidden\n---\n# Héllo *world*\n\nEscaped \\*star\\*.\n",
    )
    .unwrap();
    fs::write(&preset, "version=1\nname='test'\n[[gates]]\nid='terms'\nkind='forbidden_terms'\nterms=['world','star']\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .args([
            "content",
            file.to_str().unwrap(),
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
    let telemetry: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(telemetry["source_mapping"]["mapped"], 1);
    assert_eq!(telemetry["source_mapping"]["unmapped"], 1);
    assert!(!String::from_utf8_lossy(&output.stderr).contains("world"));
    assert_eq!(report["gates"][0]["findings"][0]["line"], 1);
    assert_eq!(
        report["source_locations"],
        serde_json::json!([{ "gate_id":"terms", "finding_index":0, "line":4, "column":10 }])
    );
}

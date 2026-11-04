use std::{
    io::Write,
    process::{Command, Stdio},
};

#[test]
fn stdin_content_returns_structured_findings() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .args(["content", "-", "--preset", "glossary", "--format", "json"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(b"").unwrap();
    let output = child.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(1));
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["gates"][0]["status"], "failed");
}

#[test]
fn missing_adapter_capability_does_not_return_success() {
    let dir = tempfile::tempdir().unwrap();
    let preset = dir.path().join("preset.toml");
    std::fs::write(&preset, "version=1\nname='Custom'\n[[gates]]\nid='metadata'\nkind='required_metadata'\nfields=['title']\n").unwrap();
    let source = format!("custom={}", preset.display());
    let mut child = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .args([
            "content",
            "-",
            "--preset",
            "custom",
            "--preset-source",
            &source,
            "--format",
            "json",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(b"Hello").unwrap();
    let output = child.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(2));
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["gates"][0]["status"], "not_checked");
}

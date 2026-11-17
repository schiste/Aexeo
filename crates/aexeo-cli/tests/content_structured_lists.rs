use std::{fs, process::Command};

#[test]
fn structured_list_gate_checks_markdown_and_withholds_unknown_structure() {
    let dir = tempfile::tempdir().unwrap();
    let preset = dir.path().join("preset.toml");
    fs::write(&preset, "version=1\nname='lists'\n[[gates]]\nid='review'\nkind='list_item_count'\nflagged_counts=[3]\n").unwrap();
    for (format, content, expected) in [
        ("markdown", "- one\n- two\n- three\n", "failed"),
        ("markdown", "- one\n- two\n", "passed"),
        ("markdown", "<ul><li>unknown</li></ul>", "not_checked"),
        ("text", "one, two and three", "not_checked"),
    ] {
        let article = dir.path().join("article");
        fs::write(&article, content).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
            .args([
                "content",
                article.to_str().unwrap(),
                "--input-format",
                format,
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
        assert_eq!(
            output.status.code(),
            Some(0),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(
            report["gates"][0]["status"], expected,
            "{format}: {content}"
        );
        assert!(!String::from_utf8_lossy(&output.stderr).contains(content));
    }
}

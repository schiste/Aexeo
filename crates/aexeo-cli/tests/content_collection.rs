use std::{fs, process::Command};

#[test]
fn collection_cli_reports_duplicates_missing_data_and_private_telemetry() {
    let dir = tempfile::tempdir().unwrap();
    let preset = dir.path().join("preset.toml");
    let input = dir.path().join("private-collection.json");
    fs::write(&preset, "version=1\nname='collection'\n[[gates]]\nid='unique'\nkind='unique_collection_metadata'\nfield='slug'\n").unwrap();
    let source = format!("custom={}", preset.display());
    let run = |json: &str| {
        fs::write(&input, json).unwrap();
        Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
            .args([
                "content",
                "--no-text",
                "--collection",
                input.to_str().unwrap(),
                "--preset",
                "custom",
                "--preset-source",
                &source,
                "--format",
                "json",
                "--telemetry",
            ])
            .output()
            .unwrap()
    };
    let duplicate = run(
        r#"[{"id":"/a","metadata":{"slug":"secret"}},{"id":"/b","metadata":{"slug":"secret"}}]"#,
    );
    assert_eq!(duplicate.status.code(), Some(1));
    let report: serde_json::Value = serde_json::from_slice(&duplicate.stdout).unwrap();
    assert_eq!(report["gates"][0]["status"], "failed");
    assert_eq!(report["gates"][0]["scope"], "collection");
    assert_eq!(report["gates"][0]["findings"][0]["path"], "/b");
    let telemetry = String::from_utf8(duplicate.stderr).unwrap();
    assert!(!telemetry.contains("secret"));
    assert!(!telemetry.contains("private-collection"));
    let missing = run(r#"[{"id":"/a","metadata":{"slug":"a"}},{"id":"/b"}]"#);
    assert_eq!(missing.status.code(), Some(2));
    let report: serde_json::Value = serde_json::from_slice(&missing.stdout).unwrap();
    assert_eq!(report["gates"][0]["status"], "not_checked");
    let ambiguous = run(r#"[{"id":"/a"},{"id":"/a"}]"#);
    assert!(!ambiguous.status.success());
    let failure: serde_json::Value = serde_json::from_slice(&ambiguous.stdout).unwrap();
    assert!(
        failure["error"]
            .as_str()
            .unwrap()
            .contains("IDs must be nonempty and unique")
    );
}

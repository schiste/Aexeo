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

#[test]
fn incomplete_collection_retains_observed_duplicates_without_passing() {
    let dir = tempfile::tempdir().unwrap();
    let preset = dir.path().join("preset.toml");
    let input = dir.path().join("collection.json");
    fs::write(&preset, "version=1\nname='collection'\n[[gates]]\nid='unique'\nkind='unique_collection_metadata'\nfield='slug'\n").unwrap();
    fs::write(
        &input,
        r#"[{"id":"/a","metadata":{"slug":"same"}},{"id":"/b","metadata":{"slug":"same"}}]"#,
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .args([
            "content",
            "--no-text",
            "--collection",
            input.to_str().unwrap(),
            "--collection-incomplete",
            "--preset",
            "custom",
            "--preset-source",
            &format!("custom={}", preset.display()),
            "--format",
            "json",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["gates"][0]["status"], "not_checked");
    assert_eq!(report["gates"][0]["findings"][0]["path"], "/b");
    assert!(
        report["gates"][0]["reason"]
            .as_str()
            .unwrap()
            .contains("collection is incomplete")
    );
}

#[test]
fn collection_grouping_reports_only_known_intra_group_duplicates() {
    let dir = tempfile::tempdir().unwrap();
    let preset = dir.path().join("preset.toml");
    let input = dir.path().join("collection.json");
    fs::write(&preset, "version=1\nname='collection'\n[[gates]]\nid='unique'\nkind='unique_collection_metadata'\nfield='slug'\ngroup_by='lang'\n").unwrap();
    fs::write(&input, r#"[{"id":"/en","metadata":{"slug":"same","lang":"en"}},{"id":"/fr","metadata":{"slug":"same","lang":"fr"}},{"id":"/en2","metadata":{"slug":"same","lang":"en"}},{"id":"/missing","metadata":{"slug":"same"}}]"#).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .args([
            "content",
            "--no-text",
            "--collection",
            input.to_str().unwrap(),
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
    assert_eq!(output.status.code(), Some(2));
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let gate = &report["gates"][0];
    assert_eq!(gate["status"], "not_checked");
    assert_eq!(gate["measurements"]["duplicate_items"], 1);
    assert_eq!(gate["measurements"]["items_missing_group"], 1);
    assert_eq!(gate["findings"].as_array().unwrap().len(), 1);
    assert_eq!(gate["findings"][0]["path"], "/en2");
    assert!(!String::from_utf8_lossy(&output.stderr).contains("same"));
}

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

#[test]
fn malformed_percent_link_is_reported_with_markdown_location() {
    let dir = tempfile::tempdir().unwrap();
    let preset = dir.path().join("preset.toml");
    let article = dir.path().join("article.md");
    fs::write(&preset, "version=1\nname='links'\n[[gates]]\nid='urls'\nkind='link_destinations'\nallowed_schemes=['https']\n").unwrap();
    fs::write(&article, "[bad](/bad%zz) [good](/good%20path)").unwrap();
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
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        report["gates"][0]["measurements"]["invalid_destinations"],
        1
    );
    assert!(
        report["gates"][0]["findings"][0]["message"]
            .as_str()
            .unwrap()
            .contains("percent")
    );
    assert_eq!(report["source_locations"][0]["line"], 1);
    assert_eq!(report["source_locations"][0]["column"], 1);
}

#[test]
fn shared_destination_fixture_matches_markdown_and_html() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/cross-adapter-destinations.json")).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let preset = dir.path().join("links.toml");
    let draft = dir.path().join("article");
    fs::write(&preset, fixture["preset"].as_str().unwrap()).unwrap();
    for format in ["markdown", "html"] {
        fs::write(&draft, fixture[format].as_str().unwrap()).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
            .args([
                "content",
                draft.to_str().unwrap(),
                "--input-format",
                format,
                "--preset",
                "links",
                "--preset-source",
                &format!("links={}", preset.display()),
                "--format",
                "json",
                "--telemetry",
            ])
            .output()
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(1),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        let gate = &report["gates"][0];
        assert_eq!(gate["status"], "failed");
        assert_eq!(
            gate["measurements"]["checked_destinations"],
            fixture["expected"]["checked_destinations"]
        );
        assert_eq!(
            gate["measurements"]["invalid_destinations"],
            fixture["expected"]["invalid_destinations"]
        );
        assert_eq!(report["source_locations"].as_array().unwrap().len(), 1);
        assert_eq!(report["source_locations"][0]["line"], 1);
        assert_eq!(
            report["source_locations"][0]["column"],
            if format == "html" { 4 } else { 1 }
        );
        assert!(!String::from_utf8_lossy(&output.stderr).contains("bad%zz"));
    }
}

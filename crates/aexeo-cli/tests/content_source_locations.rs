use std::{fs, process::Command};

#[test]
fn markdown_report_maps_exact_and_reconstructed_source_positions() {
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
    assert_eq!(
        report["source_mapping"],
        serde_json::json!({"eligible": 2, "mapped": 2, "unmapped": 0})
    );
    assert_eq!(telemetry["source_mapping"]["eligible"], 2);
    assert_eq!(telemetry["source_mapping"]["mapped"], 2);
    assert_eq!(telemetry["source_mapping"]["unmapped"], 0);
    assert!(!String::from_utf8_lossy(&output.stderr).contains("world"));
    assert_eq!(report["gates"][0]["findings"][0]["line"], 1);
    assert_eq!(
        report["source_locations"],
        serde_json::json!([{ "gate_id":"terms", "finding_index":0, "line":4, "column":10 }, { "gate_id":"terms", "finding_index":1, "line":6, "column":11 }])
    );
}

#[test]
fn image_alt_findings_map_in_json_and_text_reports() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("article.md");
    let preset = dir.path().join("preset.toml");
    fs::write(&file, "![bad](x)\n").unwrap();
    fs::write(
        &preset,
        "version=1\nname='test'\n[[gates]]\nid='terms'\nkind='forbidden_terms'\nterms=['bad']\n",
    )
    .unwrap();
    let args = [
        "content",
        file.to_str().unwrap(),
        "--input-format",
        "markdown",
        "--preset",
        "custom",
    ];
    let preset_arg = format!("custom={}", preset.display());
    let output = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .args(args)
        .args([
            "--preset-source",
            &preset_arg,
            "--format",
            "json",
            "--telemetry",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let telemetry: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(
        report["source_locations"],
        serde_json::json!([{ "gate_id":"terms", "finding_index":0, "line":1, "column":3 }])
    );
    assert_eq!(telemetry["source_mapping"]["mapped"], 1);
    assert!(!String::from_utf8_lossy(&output.stderr).contains("bad"));
    let text = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .args(args)
        .args(["--preset-source", &preset_arg])
        .output()
        .unwrap();
    assert_eq!(text.status.code(), Some(1));
    assert!(
        String::from_utf8(text.stdout)
            .unwrap()
            .contains(":1:3: Forbidden term: bad")
    );
}

#[test]
fn unchanged_text_after_an_entity_has_an_actionable_source_position() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("article.md");
    let preset = dir.path().join("preset.toml");
    fs::write(&file, "Avant &amp; après élan.\n").unwrap();
    fs::write(
        &preset,
        "version=1\nname='test'\n[[gates]]\nid='terms'\nkind='forbidden_terms'\nterms=['après']\n",
    )
    .unwrap();
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
    assert_eq!(
        report["source_locations"],
        serde_json::json!([{ "gate_id":"terms", "finding_index":0, "line":1, "column":13 }])
    );
    assert_eq!(telemetry["source_mapping"]["mapped"], 1);
    assert_eq!(telemetry["source_mapping"]["unmapped"], 0);
}

#[test]
fn wrapped_quote_with_entity_reports_the_middle_line_and_mapping_telemetry() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("article.md");
    let preset = dir.path().join("preset.toml");
    fs::write(&file, "> Avant\n> phrase &amp; interdite\n> Après\n").unwrap();
    fs::write(&preset, "version=1\nname='test'\n[[gates]]\nid='terms'\nkind='forbidden_terms'\nterms=['interdite']\n").unwrap();
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
    assert_eq!(
        report["source_locations"],
        serde_json::json!([
            { "gate_id":"terms", "finding_index":0, "line":2, "column":16 }
        ])
    );
    assert_eq!(telemetry["source_mapping"]["mapped"], 1);
    assert_eq!(telemetry["source_mapping"]["unmapped"], 0);
    assert!(!String::from_utf8_lossy(&output.stderr).contains("interdite"));
}

#[test]
fn plain_text_formats_report_exact_unicode_source_positions() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("article.txt");
    let preset = dir.path().join("preset.toml");
    fs::write(&file, "Avant\r\n\r\nÉlan interdit ici.\r\n").unwrap();
    fs::write(&preset, "version=1\nname='test'\n[[gates]]\nid='terms'\nkind='forbidden_terms'\nterms=['interdit']\n").unwrap();
    for format in ["text", "paragraph-text"] {
        let output = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
            .args([
                "content",
                file.to_str().unwrap(),
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
        assert_eq!(output.status.code(), Some(1));
        let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        let telemetry: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
        assert_eq!(
            report["source_locations"],
            serde_json::json!([
                {"gate_id":"terms", "finding_index":0, "line":3, "column":6}
            ])
        );
        assert_eq!(telemetry["source_mapping"]["mapped"], 1);
        assert_eq!(telemetry["source_mapping"]["unmapped"], 0);
        assert!(!String::from_utf8_lossy(&output.stderr).contains("interdit"));
    }
}

#[test]
fn text_report_keeps_sparse_source_locations_across_gates() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("article.md");
    let preset = dir.path().join("preset.toml");
    fs::write(
        &file,
        "---\ntitle: hidden\n---\n# Héllo *world*\n\nEscaped \\*star\\* &unknown;.\n",
    )
    .unwrap();
    fs::write(&preset, "version=1\nname='test'\n[[gates]]\nid='one'\nkind='forbidden_terms'\nterms=['star','world']\n[[gates]]\nid='two'\nkind='forbidden_terms'\nterms=['world','star']\n").unwrap();
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
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(
        text.contains("Locations mapped: 2 of 4 emitted positional findings mapped; 2 unmapped")
    );
    assert_eq!(text.matches(":4:10: Forbidden term: world").count(), 2);
    assert_eq!(
        text.matches("Forbidden term: star (projected text ")
            .count(),
        2
    );
}

#[test]
fn html_main_maps_to_article_body_in_a_complete_document() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("article.html");
    let preset = dir.path().join("preset.toml");
    fs::write(&file, "<!doctype html>\n<html lang=\"en\"><head><title>shared</title><meta charset=\"utf-8\"><!-- shared --><link rel=\"stylesheet\" href=\"/site.css\"><script>let x = 'shared';</script><style>p { color: red }</style></head>\n<body><nav><a href=\"/same\">shared</a></nav>\n<main><h1>Heading</h1><p><a href=\"/same\">shared</a></p></main>\n</body></html>").unwrap();
    fs::write(
        &preset,
        "version=1\nname='test'\n[[gates]]\nid='terms'\nkind='forbidden_terms'\nterms=['shared']\n",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .args([
            "content",
            file.to_str().unwrap(),
            "--input-format",
            "html",
            "--html-scope",
            "main",
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
    assert_eq!(
        report["source_locations"],
        serde_json::json!([{ "gate_id":"terms", "finding_index":0, "line":4, "column":42 }])
    );
    assert_eq!(telemetry["source_mapping"]["mapped"], 1);
    assert_eq!(telemetry["source_mapping"]["unmapped"], 0);
    assert!(!String::from_utf8_lossy(&output.stderr).contains("shared"));
}

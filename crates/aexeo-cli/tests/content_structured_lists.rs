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
        (
            "html",
            "<ul><li>one</li><li>two</li><li>three</li></ul>",
            "failed",
        ),
        ("html", "<ul><li>one</li><li>two</li></ul>", "passed"),
        ("html", "<ul hidden><li>one</li></ul>", "not_checked"),
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

#[test]
fn list_findings_identify_each_offending_markdown_list() {
    let dir = tempfile::tempdir().unwrap();
    let preset = dir.path().join("preset.toml");
    fs::write(&preset, "version=1\nname='lists'\n[[gates]]\nid='review'\nkind='list_item_count'\nflagged_counts=[3]\n").unwrap();
    let article = dir.path().join("article.md");
    fs::write(
        &article,
        "Intro.\n\n- one\n- two\n- three\n\nBetween.\n\n- another\n- list\n- here\n",
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
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["gates"][0]["status"], "failed");
    assert_eq!(report["source_locations"][0]["line"], 3);
    assert_eq!(report["source_locations"][1]["line"], 9);
    let telemetry: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(telemetry["source_mapping"]["eligible"], 2);
    assert_eq!(telemetry["source_mapping"]["mapped"], 2);
    assert_eq!(telemetry["source_mapping"]["unmapped"], 0);
    assert!(!String::from_utf8_lossy(&output.stderr).contains("another"));
}

#[test]
fn prose_variance_observes_markdown_lists_and_withholds_unknown_structure() {
    let dir = tempfile::tempdir().unwrap();
    let preset = dir.path().join("preset.toml");
    fs::write(&preset, "version=1\nname='prose'\n[[gates]]\nid='variance'\nkind='prose_sentence_length_variance'\nwindow_size=3\nmin_stddev_words=2\n").unwrap();
    for (format, content, status) in [
        (
            "markdown",
            "One two. Three four. Five six.\n\n- List one. List two. List three.\n\nSeven eight. Nine ten. Eleven twelve.\n",
            "failed",
        ),
        (
            "markdown",
            "One two. Three four. Five six.\n\n- \n\nSeven eight. Nine ten. Eleven twelve.\n",
            "not_checked",
        ),
        ("text", "One two. Three four. Five six.", "not_checked"),
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
            ])
            .output()
            .unwrap();
        let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(report["gates"][0]["status"], status, "{format}: {content}");
        if status == "failed" {
            assert_eq!(
                report["gates"][0]["measurements"]["list_paragraphs_excluded"],
                1
            );
            assert_eq!(report["gates"][0]["measurements"]["windows_checked"], 2);
            assert_eq!(report["source_locations"][0]["line"], 1);
            assert_eq!(report["source_locations"][1]["line"], 5);
        } else {
            assert!(
                report["gates"][0]["findings"]
                    .as_array()
                    .unwrap()
                    .is_empty()
            );
        }
    }
}

#[test]
fn prose_variance_starter_is_available_to_cli() {
    let dir = tempfile::tempdir().unwrap();
    let article = dir.path().join("article.md");
    fs::write(&article, "One two. Three four. Five six. Seven eight. Nine ten. Eleven twelve.\n\n- One. Two. Three.\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .args([
            "content",
            article.to_str().unwrap(),
            "--input-format",
            "markdown",
            "--preset",
            "prose-variance-review",
            "--format",
            "json",
        ])
        .output()
        .unwrap();
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["gates"][0]["gate_id"], "editorial.sentence_variance");
    assert_eq!(report["gates"][0]["status"], "failed");
    assert_eq!(
        report["gates"][0]["measurements"]["list_paragraphs_excluded"],
        1
    );
}

#[test]
fn shared_list_cases_match_markdown_html_and_wasm() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/cross-adapter-lists.json")).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let preset = dir.path().join("preset.toml");
    let article = dir.path().join("article");
    fs::write(&preset, fixture["preset"].as_str().unwrap()).unwrap();
    for case in fixture["cases"].as_array().unwrap() {
        for format in ["markdown", "html"] {
            fs::write(&article, case[format].as_str().unwrap()).unwrap();
            let output = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
                .args([
                    "content",
                    article.to_str().unwrap(),
                    "--input-format",
                    format,
                    "--preset",
                    "lists",
                    "--preset-source",
                    &format!("lists={}", preset.display()),
                    "--format",
                    "json",
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
                report["gates"][0]["status"], case["expected"],
                "{}: {format}",
                case["name"]
            );
        }
    }
}

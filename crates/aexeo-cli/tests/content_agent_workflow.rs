use std::{fs, process::Command};

fn evaluate(path: &std::path::Path, source: &str) -> serde_json::Value {
    let output = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .args([
            "content",
            path.to_str().unwrap(),
            "--input-format",
            "markdown",
            "--preset",
            "review",
            "--preset-source",
            source,
            "--format",
            "json",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    serde_json::from_slice(&output.stdout).unwrap()
}

fn status<'a>(report: &'a serde_json::Value, gate_id: &str) -> &'a str {
    report["gates"]
        .as_array()
        .unwrap()
        .iter()
        .find(|gate| gate["gate_id"] == gate_id)
        .unwrap()["status"]
        .as_str()
        .unwrap()
}

#[test]
fn advisory_diagnostic_can_drive_a_local_revision_and_recheck() {
    let dir = tempfile::tempdir().unwrap();
    let preset = dir.path().join("review.toml");
    let draft = dir.path().join("draft.md");
    fs::write(
        &preset,
        "version=1\nname='Review'\nextends=['article-quality','natural-style-en']\n",
    )
    .unwrap();
    let source = format!("review={}", preset.display());
    fs::write(
        &draft,
        "# Data exports\n\nIn today's fast-paced world, teams need reliable exports.\n",
    )
    .unwrap();
    let before = evaluate(&draft, &source);
    assert_eq!(status(&before, "editorial.generic_phrases"), "failed");
    assert_eq!(status(&before, "editorial.metadata"), "not_checked");
    assert!(
        before["source_locations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|location| location["gate_id"] == "editorial.generic_phrases")
    );

    fs::write(&draft, "# Data exports\n\nTeams need reliable exports.\n").unwrap();
    let after = evaluate(&draft, &source);
    assert_eq!(status(&after, "editorial.generic_phrases"), "passed");
    assert_eq!(status(&after, "editorial.metadata"), "not_checked");
}

#[test]
fn repeated_paragraph_openings_are_advisory_and_located() {
    let dir = tempfile::tempdir().unwrap();
    let preset = dir.path().join("review.toml");
    let draft = dir.path().join("draft.md");
    fs::write(
        &preset,
        "version=1\nname='Review'\n[[gates]]\nid='openings'\nkind='repeated_paragraph_openings'\nopening_words=2\nmax_occurrences=2\nblocking=false\n",
    )
    .unwrap();
    let source = format!("review={}", preset.display());
    fs::write(
        &draft,
        "The team reviews logs.\n\nTHE TEAM checks access.\n\nThe team restores backups.\n",
    )
    .unwrap();
    let report = evaluate(&draft, &source);
    assert_eq!(status(&report, "openings"), "failed");
    let location = report["source_locations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|location| location["gate_id"] == "openings")
        .unwrap();
    assert_eq!(location["line"], 5);
    assert_eq!(
        report["gates"][0]["measurements"]["openings_exceeding_limit"],
        1
    );
}

#[test]
fn unicode_opening_findings_can_drive_an_agent_revision() {
    let dir = tempfile::tempdir().unwrap();
    let preset = dir.path().join("review.toml");
    let draft = dir.path().join("draft.md");
    fs::write(&preset, "version=1\nname='Review'\n[[gates]]\nid='openings'\nkind='repeated_paragraph_openings'\nopening_words=1\nmax_occurrences=1\nblocking=false\n").unwrap();
    let source = format!("review={}", preset.display());
    fs::write(&draft, "Café arrive.\n\ncafe\u{301} repart.\n").unwrap();
    let before = evaluate(&draft, &source);
    assert_eq!(status(&before, "openings"), "failed");
    assert_eq!(before["source_locations"][0]["line"], 3);
    fs::write(&draft, "Café arrive.\n\nLe café repart.\n").unwrap();
    assert_eq!(status(&evaluate(&draft, &source), "openings"), "passed");
}

#[test]
fn duplicate_paragraphs_locate_the_second_markdown_paragraph() {
    let dir = tempfile::tempdir().unwrap();
    let preset = dir.path().join("review.toml");
    let draft = dir.path().join("draft.md");
    fs::write(&preset, "version=1\nname='Review'\n[[gates]]\nid='duplicates'\nkind='duplicate_paragraphs'\nmax_occurrences=1\n").unwrap();
    fs::write(&draft, "A repeated paragraph.\n\nA repeated paragraph.\n").unwrap();
    let report = evaluate(&draft, &format!("review={}", preset.display()));
    assert_eq!(status(&report, "duplicates"), "failed");
    assert_eq!(report["source_locations"][0]["line"], 3);
}

#[test]
fn builtin_duplicate_review_composes_overrides_and_guides_revision() {
    let dir = tempfile::tempdir().unwrap();
    let preset = dir.path().join("review.toml");
    let draft = dir.path().join("draft.md");
    fs::write(
        &preset,
        "version=1\nname='Review'\nextends=['article-editorial','duplicate-paragraph-review']\n",
    )
    .unwrap();
    let source = format!("review={}", preset.display());
    let paragraph = "The team checks backups before every planned deployment.";
    fs::write(
        &draft,
        format!(
            "{paragraph}\n\n{}\n\nShort.\n\nShort.\n",
            paragraph.to_uppercase()
        ),
    )
    .unwrap();
    let report = evaluate(&draft, &source);
    assert_eq!(status(&report, "editorial.duplicate_paragraphs"), "failed");
    assert!(report["source_locations"].as_array().unwrap().iter().any(
        |location| location["gate_id"] == "editorial.duplicate_paragraphs" && location["line"] == 3
    ));
    let gate = report["gates"]
        .as_array()
        .unwrap()
        .iter()
        .find(|gate| gate["gate_id"] == "editorial.duplicate_paragraphs")
        .unwrap();
    assert_eq!(gate["blocking"], false);
    assert_eq!(gate["measurements"]["paragraphs_skipped"], 2);
    fs::write(
        &draft,
        format!("{paragraph}\n\nA different paragraph explains who owns recovery.\n"),
    )
    .unwrap();
    assert_eq!(
        status(&evaluate(&draft, &source), "editorial.duplicate_paragraphs"),
        "passed"
    );
    fs::write(&draft, format!("{paragraph}\n\n{paragraph}\n")).unwrap();
    fs::write(&preset, "version=1\nname='Review'\nextends=['duplicate-paragraph-review']\n[[gates]]\nid='editorial.duplicate_paragraphs'\nkind='duplicate_paragraphs'\nmax_occurrences=2\nmin_chars=40\ncase_insensitive=true\nblocking=false\n").unwrap();
    assert_eq!(
        status(&evaluate(&draft, &source), "editorial.duplicate_paragraphs"),
        "passed"
    );
    let list = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .args(["content", "--list-presets"])
        .output()
        .unwrap();
    assert!(list.status.success());
    assert!(
        String::from_utf8(list.stdout)
            .unwrap()
            .contains("duplicate-paragraph-review")
    );
    let telemetry = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .args([
            "content",
            draft.to_str().unwrap(),
            "--input-format",
            "markdown",
            "--preset",
            "duplicate-paragraph-review",
            "--format",
            "json",
            "--telemetry",
        ])
        .output()
        .unwrap();
    assert!(telemetry.status.success());
    let event: serde_json::Value = serde_json::from_slice(&telemetry.stderr).unwrap();
    assert_eq!(event["summary"]["failed"], 1);
    assert!(!String::from_utf8_lossy(&telemetry.stderr).contains(paragraph));
}

#[test]
fn repeated_openings_with_unassessable_paragraphs_preserve_findings() {
    let dir = tempfile::tempdir().unwrap();
    let preset = dir.path().join("review.toml");
    let draft = dir.path().join("draft.md");
    fs::write(&preset, "version=1\nname='Review'\n[[gates]]\nid='openings'\nkind='repeated_paragraph_openings'\nopening_words=2\nmax_occurrences=1\nblocking=false\n").unwrap();
    fs::write(
        &draft,
        "The team reviews.\n\nThe team responds.\n\n中文文章沒有空格。\n",
    )
    .unwrap();
    let report = evaluate(&draft, &format!("review={}", preset.display()));
    assert_eq!(status(&report, "openings"), "not_checked");
    assert_eq!(report["gates"][0]["measurements"]["openings_skipped"], 1);
    assert_eq!(report["gates"][0]["findings"].as_array().unwrap().len(), 1);
    assert!(
        report["source_locations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|location| location["gate_id"] == "openings" && location["line"] == 3)
    );
}

#[test]
fn external_extension_results_are_bound_to_the_projected_input_and_parameters() {
    let dir = tempfile::tempdir().unwrap();
    let preset = dir.path().join("preset.toml");
    let draft = dir.path().join("draft.md");
    let results = dir.path().join("results.json");
    fs::write(&draft, "# Guide\n\nReview this text.\n").unwrap();
    fs::write(&preset, "version=1\nname='External'\n[[gates]]\nid='external'\nkind='custom'\nextension='example.rule'\nblocking=false\n[gates.parameters]\nmode='strict'\n").unwrap();
    let source = format!("review={}", preset.display());
    let base = [
        "content",
        draft.to_str().unwrap(),
        "--input-format",
        "markdown",
        "--preset",
        "review",
        "--preset-source",
        &source,
    ];
    let export = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .args(base)
        .arg("--export-adapter-input")
        .output()
        .unwrap();
    assert!(export.status.success());
    let input: serde_json::Value = serde_json::from_slice(&export.stdout).unwrap();
    let external = serde_json::json!([{"extension":"example.rule", "parameters":{"mode":"strict"}, "input":input, "diagnostics":[{"message":"Review opening", "line":2, "column":1}]}]);
    fs::write(&results, serde_json::to_vec(&external).unwrap()).unwrap();
    let run = || {
        Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
            .args(base)
            .args([
                "--extension-results",
                results.to_str().unwrap(),
                "--format",
                "json",
            ])
            .output()
            .unwrap()
    };
    let output = run();
    assert!(output.status.success());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(status(&report, "external"), "failed");
    fs::write(&draft, "# Guide\n\nChanged text.\n").unwrap();
    let stale: serde_json::Value = serde_json::from_slice(&run().stdout).unwrap();
    assert_eq!(status(&stale, "external"), "not_checked");
}

#[test]
fn metadata_length_gate_uses_supplied_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let draft = dir.path().join("draft.md");
    let preset = dir.path().join("preset.toml");
    let metadata = dir.path().join("metadata.json");
    fs::write(&draft, "# A draft\n").unwrap();
    fs::write(&preset, "version=1\nname='test'\n[[gates]]\nid='title'\nkind='metadata_length'\nfield='title'\nmax=3\nblocking=false\n").unwrap();
    fs::write(&metadata, r#"{"title":"é🙂ab"}"#).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .args([
            "content",
            draft.to_str().unwrap(),
            "--input-format",
            "markdown",
            "--preset",
            "review",
            "--preset-source",
            &format!("review={}", preset.display()),
            "--metadata",
            metadata.to_str().unwrap(),
            "--format",
            "json",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(status(&report, "title"), "failed");
    assert_eq!(report["gates"][0]["measurements"]["character_count"], 4);
    assert!(report["source_locations"].as_array().unwrap().is_empty());
}

#[test]
fn external_results_reject_duplicate_nested_keys() {
    let dir = tempfile::tempdir().unwrap();
    let draft = dir.path().join("draft.txt");
    let results = dir.path().join("results.json");
    fs::write(&draft, "abc").unwrap();
    fs::write(&results, r#"[{"extension":"example.rule","parameters":{"mode":"a","mode":"b"},"input":{"id":"draft"},"diagnostics":[]}]"#).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .args([
            "content",
            draft.to_str().unwrap(),
            "--preset",
            "article",
            "--extension-results",
            results.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert_ne!(output.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&output.stderr).contains("invalid external gate results JSON"));
}

#[test]
fn sentence_openings_builtin_supports_revision_and_private_telemetry() {
    let dir = tempfile::tempdir().unwrap();
    let draft = dir.path().join("draft.md");
    fs::write(&draft, "The team plans. The team acts. THE TEAM checks.\n").unwrap();
    let run = || {
        Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
            .args([
                "content",
                draft.to_str().unwrap(),
                "--input-format",
                "markdown",
                "--preset",
                "sentence-openings-review",
                "--format",
                "json",
                "--telemetry",
            ])
            .output()
            .unwrap()
    };
    let output = run();
    assert!(output.status.success());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(status(&report, "editorial.sentence_openings"), "failed");
    assert_eq!(report["source_locations"][0]["line"], 1);
    assert_eq!(report["source_locations"][0]["column"], 32);
    let event: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
    assert!(event["gate_timings_ms"].is_array());
    assert!(!String::from_utf8_lossy(&output.stderr).contains("The team"));
    fs::write(
        &draft,
        "The team plans. Été. The team acts. Then everyone checks.\n",
    )
    .unwrap();
    let output = run();
    let revised: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(status(&revised, "editorial.sentence_openings"), "passed");
}

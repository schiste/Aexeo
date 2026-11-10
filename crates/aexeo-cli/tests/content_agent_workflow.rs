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

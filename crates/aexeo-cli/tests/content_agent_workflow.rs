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

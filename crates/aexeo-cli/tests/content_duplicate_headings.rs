use std::{fs, process::Command};

#[test]
fn markdown_duplicate_headings_use_custom_policy_and_aggregate_telemetry() {
    let dir = tempfile::tempdir().unwrap();
    let preset = dir.path().join("preset.toml");
    let article = dir.path().join("article.md");
    fs::write(&preset, "version=1\nname='headings'\n[[gates]]\nid='unique'\nkind='duplicate_headings'\ncase_sensitive=false\nblocking=false\n").unwrap();
    fs::write(&article, "# Private heading\n\n## PRIVATE HEADING\n").unwrap();
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
    assert_eq!(output.status.code(), Some(0));
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["gates"][0]["status"], "failed");
    assert_eq!(report["gates"][0]["measurements"]["heading_count"], 2);
    assert_eq!(report["gates"][0]["measurements"]["duplicate_headings"], 1);
    assert_eq!(report["source_locations"][0]["line"], 3);
    assert_eq!(report["source_locations"][0]["column"], 1);
    assert!(!String::from_utf8_lossy(&output.stderr).contains("Private heading"));
}

#[test]
fn heading_hierarchy_uses_observed_levels_and_withholds_mixed_markdown() {
    let dir = tempfile::tempdir().unwrap();
    let preset = dir.path().join("preset.toml");
    fs::write(&preset, "version=1\nname='headings'\n[[gates]]\nid='hierarchy'\nkind='heading_hierarchy'\nblocking=false\n").unwrap();
    for (format, content, expected) in [
        ("markdown", "## First\n\n#### Fourth\n", "failed"),
        ("markdown", "## First\n\n### Third\n", "passed"),
        (
            "markdown",
            "## First\n\n<div><h3>Hidden heading</h3></div>\n\n#### Fourth\n",
            "not_checked",
        ),
        (
            "html",
            "<main><h2>First</h2><h4>Fourth</h4></main>",
            "failed",
        ),
        (
            "html",
            "<main><h2>First</h2><h3>Third</h3></main>",
            "passed",
        ),
    ] {
        let article = dir.path().join(format!("article.{format}"));
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
        let measurements = &report["gates"][0]["measurements"];
        if expected == "not_checked" {
            assert!(measurements.get("first_heading_level").is_none());
            assert!(measurements.get("deepest_heading_level").is_none());
        } else {
            assert_eq!(measurements["first_heading_level"], 2);
            assert_eq!(
                measurements["deepest_heading_level"],
                if expected == "failed" { 4 } else { 3 }
            );
        }
        if expected == "failed" {
            assert_eq!(report["gates"][0]["measurements"]["skipped_levels"], 1);
            let position = &report["source_locations"][0];
            if format == "markdown" {
                assert_eq!(position["line"], 3);
                assert_eq!(position["column"], 1);
            } else {
                assert_eq!(position["line"], 1);
                assert_eq!(position["column"], content.find("<h4>").unwrap() + 1);
            }
        }
        assert!(!String::from_utf8_lossy(&output.stderr).contains("First"));
    }
}

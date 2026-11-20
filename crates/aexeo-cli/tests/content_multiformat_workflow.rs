//! Cross-format agent workflow. Synthetic cases exercise mechanics, not quality.
use serde_json::Value;
use std::{fs, path::Path, process::Command};

fn evaluate(draft: &Path, preset: &Path, format: &str) -> (Value, Value) {
    let output = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .args([
            "content",
            draft.to_str().unwrap(),
            "--input-format",
            format,
            "--preset",
            "workflow",
            "--preset-source",
            &format!("workflow={}", preset.display()),
            "--format",
            "json",
            "--telemetry",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report = serde_json::from_slice(&output.stdout).unwrap();
    let event = String::from_utf8_lossy(&output.stderr)
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .find(|event| event["event"] == "content_evaluation")
        .unwrap();
    (report, event)
}

fn capture(draft: &Path, report: &Path, note: &Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .args([
            "feedback-capture",
            "--text",
            draft.to_str().unwrap(),
            "--report",
            report.to_str().unwrap(),
            "--gate",
            "style.phrase",
            "--example",
            "synthetic",
            "--output",
            note.to_str().unwrap(),
            "--telemetry",
        ])
        .output()
        .unwrap()
}

#[test]
fn localized_revision_and_feedback_are_bound_across_text_markdown_and_html() {
    let dir = tempfile::tempdir().unwrap();
    let preset = dir.path().join("workflow.toml");
    fs::write(&preset, "version=1\nname='workflow'\n[[gates]]\nid='style.phrase'\nkind='forbidden_terms'\nterms=['generic phrase']\nblocking=false\n[[gates]]\nid='metadata.required'\nkind='required_metadata'\nfields=['title']\nblocking=false\n").unwrap();
    for (format, original) in [
        (
            "paragraph-text",
            "Élan.\n\nThis generic phrase needs review.\n",
        ),
        (
            "markdown",
            "# Élan\n\nThis **generic phrase** needs review.\n",
        ),
        (
            "html",
            "<h1>Élan</h1>\n<p>&copy; This generic phrase needs review.</p>",
        ),
    ] {
        let draft = dir.path().join(format!("{format}.txt"));
        let report_path = dir.path().join(format!("{format}.json"));
        let note = dir.path().join(format!("{format}-note.json"));
        fs::write(&draft, original).unwrap();
        let (before, telemetry) = evaluate(&draft, &preset, format);
        assert_eq!(before["gates"][0]["status"], "failed", "{format}");
        assert_eq!(before["gates"][1]["status"], "not_checked", "{format}");
        assert!(!telemetry.to_string().contains("generic phrase"));
        assert_eq!(telemetry["source_mapping"]["mapped"], 1, "{format}");
        let location = &before["source_locations"][0];
        let line = location["line"].as_u64().unwrap() as usize;
        let column = location["column"].as_u64().unwrap() as usize;
        let byte = original
            .lines()
            .take(line - 1)
            .map(|line| line.len() + 1)
            .sum::<usize>()
            + original
                .lines()
                .nth(line - 1)
                .unwrap()
                .char_indices()
                .nth(column - 1)
                .unwrap()
                .0;
        assert!(original[byte..].starts_with("generic phrase"), "{format}");
        fs::write(&report_path, serde_json::to_vec(&before).unwrap()).unwrap();
        assert!(capture(&draft, &report_path, &note).status.success());
        let captured: Value = serde_json::from_slice(&fs::read(&note).unwrap()).unwrap();
        assert_eq!(captured["report_binding"], "verified");
        assert!(captured["human_status"].is_null());
        assert!(captured["reviewer"].is_null());
        let mut revised = original.to_owned();
        revised.replace_range(byte..byte + "generic phrase".len(), "specific note");
        fs::write(&draft, &revised).unwrap();
        let stale = capture(
            &draft,
            &report_path,
            &dir.path().join(format!("{format}-stale.json")),
        );
        assert!(
            !stale.status.success(),
            "stale report was accepted for {format}"
        );
        let (after, _) = evaluate(&draft, &preset, format);
        assert_eq!(after["gates"][0]["status"], "passed", "{format}");
        assert_eq!(after["gates"][1]["status"], "not_checked", "{format}");
        assert_eq!(
            revised.replacen("specific note", "generic phrase", 1),
            original
        );
    }
}

#[test]
fn scoped_html_corpus_replays_against_the_real_cli() {
    let dir = tempfile::tempdir().unwrap();
    let corpus = dir.path().join("corpus.json");
    fs::write(&corpus, serde_json::to_vec(&serde_json::json!({
        "version": 1, "preset": "custom", "gate_id": "words", "input_format": "html",
        "content_projection": {"format": "html", "scope": "main"},
        "preset_toml": "version=1\nname='custom'\n[[gates]]\nid='words'\nkind='forbidden_terms'\nterms=['navigation']\n",
        "examples": [{"id":"a", "split":"holdout", "text":"<nav>navigation</nav><main><p>Article</p></main>", "expected":"passed"}]
    })).unwrap()).unwrap();
    let script =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/evaluate-content-gates.py");
    let output = Command::new("python3")
        .arg(script)
        .arg(corpus)
        .args([
            "--binary",
            env!("CARGO_BIN_EXE_aexeo-cli"),
            "--assert-regression",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["counts"]["true_negative"], 1);
    assert_eq!(report["content_projection"]["scope"], "main");
}

#[test]
fn scoped_html_reports_body_coordinates_after_head_script_comparison() {
    let dir = tempfile::tempdir().unwrap();
    let draft = dir.path().join("draft.html");
    let preset = dir.path().join("preset.toml");
    let html = "<!doctype html>\n<html><head><script>if (x < 3) x++;</script></head>\n<body><nav>generic phrase</nav>\n<main><p>generic phrase</p></main></body></html>";
    fs::write(&draft, html).unwrap();
    fs::write(&preset, "version=1\nname='workflow'\n[[gates]]\nid='style.phrase'\nkind='forbidden_terms'\nterms=['generic phrase']\nblocking=false\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .args([
            "content",
            draft.to_str().unwrap(),
            "--input-format",
            "html",
            "--html-scope",
            "main",
            "--preset",
            "workflow",
            "--preset-source",
            &format!("workflow={}", preset.display()),
            "--format",
            "json",
            "--telemetry",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["source_locations"][0]["line"], 4, "{report:#}");
    assert_eq!(report["source_locations"][0]["column"], 10, "{report:#}");
    let telemetry = String::from_utf8_lossy(&output.stderr)
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .find(|event| event["event"] == "content_evaluation")
        .unwrap();
    assert_eq!(telemetry["source_mapping"]["mapped"], 1);
    assert!(!telemetry.to_string().contains("generic phrase"));
}

#[test]
fn shared_editorial_fixture_matches_markdown_and_html_cli_contracts() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/cross-adapter-editorial.json")).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let draft = dir.path().join("example");
    let preset = dir.path().join("cross.toml");
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
                "cross",
                "--preset-source",
                &format!("cross={}", preset.display()),
                "--format",
                "json",
                "--telemetry",
            ])
            .output()
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(2),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let report: Value = serde_json::from_slice(&output.stdout).unwrap();
        let telemetry: Value = serde_json::from_slice(&output.stderr).unwrap();
        for field in ["eligible", "mapped", "unmapped"] {
            assert_eq!(
                telemetry["source_mapping"][field],
                fixture["expected"]["source_mapping"][field]
            );
        }
        assert!(
            telemetry["source_mapping"]["duration_ms"]
                .as_f64()
                .is_some()
        );
        let serialized = String::from_utf8_lossy(&output.stderr);
        assert!(!serialized.contains("generic phrase"));
        assert!(!serialized.contains("example.org"));
        let statuses: Vec<_> = report["gates"]
            .as_array()
            .unwrap()
            .iter()
            .map(|g| g["status"].clone())
            .collect();
        assert_eq!(
            statuses,
            fixture["expected"]["statuses"].as_array().unwrap().clone()
        );
        assert_eq!(
            report["gates"][1]["measurements"]["paragraph_count"],
            fixture["expected"]["paragraph_count"]
        );
        assert_eq!(
            report["gates"][2]["measurements"]["link_count"],
            fixture["expected"]["link_count"]
        );
        assert_eq!(report["gates"][5]["measurements"]["skipped_levels"], 1);
        assert_eq!(
            report["source_mapping"],
            fixture["expected"]["source_mapping"]
        );
        assert_eq!(
            report["source_locations"],
            serde_json::json!([
                fixture["expected"][format!("{format}_location")].clone(),
                fixture["expected"][format!("{format}_heading_location")].clone(),
                fixture["expected"][format!("{format}_hierarchy_location")].clone()
            ])
        );
        fs::write(&draft, fixture[format!("empty_{format}")].as_str().unwrap()).unwrap();
        let empty = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
            .args([
                "content",
                draft.to_str().unwrap(),
                "--input-format",
                format,
                "--preset",
                "cross",
                "--preset-source",
                &format!("cross={}", preset.display()),
                "--format",
                "json",
            ])
            .output()
            .unwrap();
        assert_eq!(
            empty.status.code(),
            Some(2),
            "{}",
            String::from_utf8_lossy(&empty.stderr)
        );
        let empty: Value = serde_json::from_slice(&empty.stdout).unwrap();
        let statuses: Vec<_> = empty["gates"]
            .as_array()
            .unwrap()
            .iter()
            .map(|g| g["status"].clone())
            .collect();
        assert_eq!(
            statuses,
            fixture["empty_statuses"].as_array().unwrap().clone()
        );
        assert_eq!(empty["gates"][1]["measurements"]["paragraph_count"], 0);
        assert_eq!(empty["gates"][2]["measurements"]["link_count"], 0);
    }
}

#[test]
fn unicode_openings_match_across_markdown_and_html() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/cross-adapter-openings.json")).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let draft = dir.path().join("draft");
    let preset = dir.path().join("workflow.toml");
    fs::write(&preset, fixture["preset"].as_str().unwrap()).unwrap();
    for case in fixture["cases"].as_array().unwrap() {
        for format in ["markdown", "html"] {
            fs::write(&draft, case[format].as_str().unwrap()).unwrap();
            let (report, telemetry) = evaluate(&draft, &preset, format);
            let statuses: Vec<_> = report["gates"]
                .as_array()
                .unwrap()
                .iter()
                .map(|gate| gate["status"].clone())
                .collect();
            assert_eq!(
                statuses,
                *case["statuses"].as_array().unwrap(),
                "{} {format}",
                case["name"]
            );
            assert!(!telemetry.to_string().contains("Café"));
            if case["name"] == "equivalent" {
                let locations = report["source_locations"].as_array().unwrap();
                assert_eq!(locations.len(), 2);
                let paragraph = locations
                    .iter()
                    .find(|item| item["gate_id"] == "paragraph")
                    .unwrap();
                let sentence = locations
                    .iter()
                    .find(|item| item["gate_id"] == "sentence")
                    .unwrap();
                assert_eq!(
                    (
                        paragraph["line"].as_u64().unwrap(),
                        paragraph["column"].as_u64().unwrap()
                    ),
                    if format == "html" { (2, 4) } else { (3, 1) }
                );
                assert_eq!(
                    (
                        sentence["line"].as_u64().unwrap(),
                        sentence["column"].as_u64().unwrap()
                    ),
                    if format == "html" { (1, 17) } else { (1, 14) }
                );
                assert_eq!(telemetry["source_mapping"]["mapped"], 2);
            }
        }
    }
}

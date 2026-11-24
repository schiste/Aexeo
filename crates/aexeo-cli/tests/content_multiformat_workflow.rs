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
            report["gates"][6]["measurements"]["word_count"],
            fixture["expected"]["word_count"]
        );
        assert_eq!(
            report["gates"][7]["measurements"]["character_count"],
            fixture["expected"]["character_count_without_whitespace"]
        );
        assert_eq!(
            report["gates"][8]["measurements"]["character_count"],
            fixture["expected"]["character_count_with_whitespace"][format]
        );
        assert_eq!(
            report["gates"][9]["measurements"]["present_terms"],
            fixture["expected"]["required_present_terms"]
        );
        assert_eq!(
            report["gates"][9]["measurements"]["missing_terms"],
            fixture["expected"]["required_missing_terms"]
        );
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
        assert_eq!(empty["gates"][6]["measurements"]["word_count"], 0);
        assert_eq!(empty["gates"][7]["measurements"]["character_count"], 0);
        assert_eq!(empty["gates"][8]["measurements"]["character_count"], 0);
        assert_eq!(empty["gates"][9]["measurements"]["present_terms"], 0);
        assert_eq!(empty["gates"][9]["measurements"]["missing_terms"], 2);
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

#[test]
fn url_sentence_boundaries_match_shared_cross_adapter_cases() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/cross-adapter-urls.json")).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let draft = dir.path().join("example");
    let preset = dir.path().join("workflow.toml");
    fs::write(&preset, fixture["preset"].as_str().unwrap()).unwrap();
    for case in fixture["cases"].as_array().unwrap() {
        for format in ["html", "markdown"] {
            let source = case[format].as_str().unwrap();
            fs::write(&draft, source).unwrap();
            let (report, telemetry) = evaluate(&draft, &preset, format);
            assert_eq!(
                report["gates"][0]["measurements"]["sentence_count"], case["expected_sentences"],
                "{format}: {}",
                case["name"]
            );
            assert_eq!(
                report["gates"][0]["findings"].as_array().unwrap().len(),
                case["expected_findings"].as_u64().unwrap() as usize,
                "{format}: {}",
                case["name"]
            );
            assert_eq!(
                telemetry["source_mapping"]["mapped"],
                case["expected_findings"]
            );
            assert!(!telemetry.to_string().contains("example.test"));
        }
    }
}

#[test]
fn numeric_citations_do_not_merge_sentences_across_markdown_and_html() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/cross-adapter-citations.json")).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let draft = dir.path().join("example");
    let preset = dir.path().join("workflow.toml");
    fs::write(&preset, fixture["preset"].as_str().unwrap()).unwrap();
    for format in ["html", "markdown"] {
        fs::write(&draft, fixture[format].as_str().unwrap()).unwrap();
        let (report, _) = evaluate(&draft, &preset, format);
        assert_eq!(
            report["gates"][0]["measurements"]["sentence_count"], fixture["expected_sentences"],
            "{format}"
        );
    }
}

#[test]
fn paragraph_scoped_citations_keep_sentence_boundaries_across_formats() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/cross-adapter-citations.json")).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let draft = dir.path().join("example");
    let preset = dir.path().join("workflow.toml");
    let scoped_preset = format!(
        "{}paragraphs_only=true\n",
        fixture["preset"].as_str().unwrap()
    );
    fs::write(&preset, scoped_preset).unwrap();
    for format in ["html", "markdown"] {
        fs::write(&draft, fixture[format].as_str().unwrap()).unwrap();
        let (report, _) = evaluate(&draft, &preset, format);
        assert_eq!(
            report["gates"][0]["measurements"]["sentence_count"], fixture["expected_sentences"],
            "{format}"
        );
        assert_eq!(report["gates"][0]["status"], "failed", "{format}");
        assert_eq!(report["source_mapping"]["mapped"], 2, "{format}");
    }
}

#[test]
fn markdown_footnote_markers_keep_separate_words_for_editorial_checks() {
    let dir = tempfile::tempdir().unwrap();
    let draft = dir.path().join("example.md");
    let preset = dir.path().join("workflow.toml");
    fs::write(&preset, "version=1\nname='t'\n[[gates]]\nid='length'\nkind='sentence_length'\nmax_words=1\nblocking=false\n").unwrap();
    for source in [
        "Alpha[^note]beta. Next sentence.\n\n[^note]: A.",
        "Alpha[^note]**beta**. Next sentence.\n\n[^note]: A.",
    ] {
        fs::write(&draft, source).unwrap();
        let (report, _) = evaluate(&draft, &preset, "markdown");
        assert_eq!(report["gates"][0]["measurements"]["sentence_count"], 3);
        assert_eq!(
            report["gates"][0]["findings"].as_array().unwrap().len(),
            2,
            "{report}"
        );
        assert_eq!(report["source_locations"][0]["column"], 1);
        assert_eq!(
            report["source_locations"][1]["column"],
            source.find("Next").unwrap() + 1
        );
    }
}

#[test]
fn literal_terms_can_see_reference_labels_without_affecting_rhythm() {
    let dir = tempfile::tempdir().unwrap();
    let draft = dir.path().join("example");
    let preset = dir.path().join("workflow.toml");
    fs::write(&preset, "version=1\nname='t'\n[[gates]]\nid='required'\nkind='required_terms'\nterms=['[17]']\nblocking=false\n[[gates]]\nid='forbidden'\nkind='forbidden_terms'\nterms=['[17]']\nblocking=false\n[[gates]]\nid='repeated'\nkind='repeated_terms'\nterms=['[17]']\nmax_occurrences=1\nblocking=false\n[[gates]]\nid='length'\nkind='sentence_length'\nmax_words=20\nblocking=false\n").unwrap();
    for (format, source) in [
        (
            "html",
            "<p>Élan.<a href='#reference-source'>[17]</a> Next sentence.<a href='#reference-source'>[17]</a></p>",
        ),
        (
            "markdown",
            "Élan.[[17]](#reference-source) Next sentence.[[17]](#reference-source)",
        ),
    ] {
        fs::write(&draft, source).unwrap();
        let (report, _) = evaluate(&draft, &preset, format);
        assert_eq!(report["gates"][0]["status"], "passed", "{format}");
        assert_eq!(report["gates"][1]["status"], "failed", "{format}");
        assert_eq!(report["gates"][2]["status"], "failed", "{format}");
        assert_eq!(
            report["gates"][3]["measurements"]["sentence_count"], 2,
            "{format}"
        );
    }
}

#[test]
fn arabic_decimal_sentences_keep_source_positions_across_formats() {
    let dir = tempfile::tempdir().unwrap();
    let draft = dir.path().join("example");
    let preset = dir.path().join("workflow.toml");
    fs::write(&preset, "version=1\nname='t'\n[[gates]]\nid='length'\nkind='sentence_character_length'\nmax_chars=5\nblocking=false\n").unwrap();
    for (format, source) in [
        ("html", "<p>Value ٣.١٤ units. Next sentence.</p>"),
        ("markdown", "Value ۳.۱۴ units. Next sentence."),
    ] {
        fs::write(&draft, source).unwrap();
        let (report, _) = evaluate(&draft, &preset, format);
        assert_eq!(
            report["gates"][0]["measurements"]["sentence_count"], 2,
            "{format}"
        );
        assert_eq!(
            report["source_locations"].as_array().unwrap().len(),
            2,
            "{format}"
        );
        let column = source[..source.find("Next").unwrap()].chars().count() + 1;
        assert_eq!(report["source_locations"][1]["column"], column, "{format}");
    }
}

#[test]
fn multi_period_abbreviations_work_across_markdown_and_html() {
    let dir = tempfile::tempdir().unwrap();
    let draft = dir.path().join("example");
    let preset = dir.path().join("workflow.toml");
    fs::write(&preset, "version=1\nname='t'\n[[gates]]\nid='length'\nkind='sentence_length'\nmax_words=10\nabbreviations=['e.g.']\n").unwrap();
    for (format, source) in [
        ("html", "<p>First e.g. example. Next sentence.</p>"),
        ("markdown", "First e.g. example. Next sentence."),
    ] {
        fs::write(&draft, source).unwrap();
        let (report, _) = evaluate(&draft, &preset, format);
        assert_eq!(
            report["gates"][0]["measurements"]["sentence_count"], 2,
            "{format}"
        );
    }
}

#[test]
fn formatted_citations_preserve_sentence_count_in_both_formats() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/cross-adapter-citations.json")).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let draft = dir.path().join("example");
    let preset = dir.path().join("workflow.toml");
    fs::write(&preset, fixture["preset"].as_str().unwrap()).unwrap();
    for (format, text) in [
        (
            "html",
            "<p>In 2025.<a href='#reference-source'><sup>1</sup></a> Next sentence.</p>",
        ),
        (
            "markdown",
            "In 2025.[*1*](#reference-source) Next sentence.",
        ),
        (
            "markdown",
            "In 2025.[**1**][ref] Next sentence.\n\n[ref]: #reference-source\n",
        ),
    ] {
        fs::write(&draft, text).unwrap();
        let (report, _) = evaluate(&draft, &preset, format);
        assert_eq!(
            report["gates"][0]["measurements"]["sentence_count"], 2,
            "{text}"
        );
    }
}

#[test]
fn embedded_html_links_are_not_misreported_as_absent_in_markdown() {
    let dir = tempfile::tempdir().unwrap();
    let preset = dir.path().join("workflow.toml");
    let draft = dir.path().join("article.md");
    fs::write(&preset, "version=1\nname='workflow'\n[[gates]]\nid='links'\nkind='link_count'\nmin=1\nblocking=false\n[[gates]]\nid='destinations'\nkind='link_destinations'\nallowed_schemes=['https']\nblocking=false\n").unwrap();
    fs::write(
        &draft,
        "Read <a href=\"https://example.org\">the source</a>.\n",
    )
    .unwrap();
    let (report, telemetry) = evaluate(&draft, &preset, "markdown");
    assert_eq!(report["gates"][0]["status"], "not_checked");
    assert_eq!(report["gates"][1]["status"], "not_checked");
    assert_eq!(telemetry["summary"]["not_checked"], 2);
    assert!(!telemetry.to_string().contains("example.org"));
}

#[test]
fn markdown_with_html_does_not_treat_unobserved_prose_as_empty() {
    let dir = tempfile::tempdir().unwrap();
    let preset = dir.path().join("workflow.toml");
    let draft = dir.path().join("article.md");
    fs::write(&preset, "version=1\nname='workflow'\n[[gates]]\nid='words'\nkind='word_count'\nmin=3\nblocking=false\n[[gates]]\nid='paragraphs'\nkind='paragraph_count'\nmin=1\nblocking=false\n[[gates]]\nid='heading'\nkind='heading_length'\nmax_chars=3\nblocking=false\n[[gates]]\nid='newline'\nkind='final_newline'\nblocking=false\n").unwrap();
    for source in [
        "<div>These words are present in HTML.</div>\n",
        "Visible Markdown.\n\n<div>More text.</div>\n",
        "A <em>marked</em> word.\n",
    ] {
        fs::write(&draft, source).unwrap();
        let (report, telemetry) = evaluate(&draft, &preset, "markdown");
        for gate in &report["gates"].as_array().unwrap()[..3] {
            assert_eq!(gate["status"], "not_checked", "{source}");
            assert_eq!(
                gate["reason"],
                "embedded HTML is not observed by the Markdown adapter"
            );
            assert!(gate["findings"].as_array().unwrap().is_empty());
        }
        assert_eq!(report["gates"][3]["status"], "passed");
        assert_eq!(telemetry["summary"]["not_checked"], 3);
        assert!(!telemetry.to_string().contains("present in HTML"));
    }
}

#[test]
fn markdown_html_preserves_independent_composition_unavailability_reason() {
    let dir = tempfile::tempdir().unwrap();
    let preset = dir.path().join("workflow.toml");
    let draft = dir.path().join("article.md");
    fs::write(&preset, "version=1\nname='workflow'\n[[gates]]\nid='pacing'\nkind='editorial_pacing'\nsource='composition'\nblocking=false\nprofile={version=1,name='short',fields=[{name='hook',max_characters=3}]}\n").unwrap();
    fs::write(&draft, "<div>Text in HTML.</div>\n").unwrap();
    let (report, telemetry) = evaluate(&draft, &preset, "markdown");
    assert_eq!(report["gates"][0]["status"], "not_checked");
    assert_eq!(
        report["gates"][0]["reason"],
        "document or structured composition is unavailable"
    );
    assert_eq!(telemetry["summary"]["not_checked"], 1);
    assert!(!telemetry.to_string().contains("Text in HTML"));
}

use std::{fs, process::Command};

#[test]
fn html_content_uses_composable_gates_and_does_not_guess_source_positions() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("article.html");
    let preset = dir.path().join("preset.toml");
    fs::write(
        &input,
        "<h1>Title</h1><p>Intro <a href='http://example.org'>word</a></p><p hidden>secret</p>",
    )
    .unwrap();
    fs::write(&preset, "version=1\nname='test'\n[[gates]]\nid='words'\nkind='forbidden_terms'\nterms=['word','secret']\n[[gates]]\nid='links'\nkind='link_destinations'\nallowed_schemes=['https']\n[[gates]]\nid='online'\nkind='link_reachability'\n[[gates]]\nid='wordcount'\nkind='word_count'\nmin=100\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .args([
            "content",
            input.to_str().unwrap(),
            "--input-format",
            "html",
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
    assert_eq!(report["gates"][0]["findings"].as_array().unwrap().len(), 1);
    assert_eq!(report["gates"][1]["status"], "failed");
    assert_eq!(report["gates"][2]["status"], "not_checked");
    assert_eq!(report["gates"][3]["status"], "failed");
    assert!(report.get("source_locations").is_none());
    assert_eq!(
        report["source_mapping"],
        serde_json::json!({"eligible": 2, "mapped": 0, "unmapped": 2})
    );
    assert_eq!(
        report["source_location_capability"]["status"],
        "unavailable"
    );
    assert_eq!(
        report["source_location_capability"]["finding_coordinates"],
        "projected_text"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.contains("example.org"));
    assert!(!stderr.contains("secret"));
    let event: serde_json::Value = stderr
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .find(|event| event["event"] == "content_evaluation")
        .unwrap();
    assert_eq!(event["source_mapping"]["eligible"], 2);
    assert_eq!(event["source_mapping"]["mapped"], 0);
    assert_eq!(event["source_mapping"]["unmapped"], 2);
    assert_eq!(
        event["html_projection"],
        serde_json::json!({"status":"complete", "headings":1, "paragraphs":1, "links":1, "source_location_capability":"unavailable"})
    );
}

#[test]
fn html_plain_fragment_maps_only_identical_projection() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("article.html");
    let preset = dir.path().join("preset.toml");
    fs::write(&input, "word").unwrap();
    fs::write(
        &preset,
        "version=1\nname='test'\n[[gates]]\nid='words'\nkind='forbidden_terms'\nterms=['word']\n",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .args([
            "content",
            input.to_str().unwrap(),
            "--input-format",
            "html",
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
        report["source_location_capability"]["status"],
        "identity_projection"
    );
    assert_eq!(
        report["source_locations"],
        serde_json::json!([{ "gate_id":"words", "finding_index":0, "line":1, "column":1 }])
    );
    assert!(output.stderr.is_empty(), "telemetry must be opt-in");
}

#[test]
fn html_literal_fragment_maps_prose_and_reports_capability() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("article.html");
    let preset = dir.path().join("preset.toml");
    fs::write(&input, "<p>Élan <a href='/x'>world</a></p>").unwrap();
    fs::write(
        &preset,
        "version=1\nname='test'\n[[gates]]\nid='words'\nkind='forbidden_terms'\nterms=['world']\n",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .args([
            "content",
            input.to_str().unwrap(),
            "--input-format",
            "html",
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
    assert_eq!(
        report["source_location_capability"]["status"],
        "literal_html"
    );
    assert_eq!(
        report["source_locations"],
        serde_json::json!([{ "gate_id":"words", "finding_index":0, "line":1, "column":22 }])
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.contains("world"));
    let event: serde_json::Value = stderr
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .find(|event| event["event"] == "content_evaluation")
        .unwrap();
    assert_eq!(
        event["html_projection"]["source_location_capability"],
        "literal_html"
    );
    assert_eq!(event["source_mapping"]["mapped"], 1);
}

#[test]
fn html_image_alt_findings_and_telemetry_use_source_positions() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("article.html");
    let preset = dir.path().join("preset.toml");
    let html = "<p>Before <img alt='bad'> after bad</p>";
    fs::write(&input, html).unwrap();
    fs::write(
        &preset,
        "version=1\nname='test'\n[[gates]]\nid='words'\nkind='forbidden_terms'\nterms=['bad']\n",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .args([
            "content",
            input.to_str().unwrap(),
            "--input-format",
            "html",
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
    assert_eq!(
        report["source_locations"],
        serde_json::json!([
            {"gate_id":"words", "finding_index":0, "line":1, "column":html.find("bad").unwrap()+1},
            {"gate_id":"words", "finding_index":1, "line":1, "column":html.rfind("bad").unwrap()+1}
        ])
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    let event: serde_json::Value = stderr
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .find(|event: &serde_json::Value| event["event"] == "content_evaluation")
        .unwrap();
    assert_eq!(event["source_mapping"]["mapped"], 2);
    assert_eq!(event["source_mapping"]["unmapped"], 0);
    assert!(!stderr.contains("bad"));
}

#[test]
fn html_encoded_image_alt_findings_map_to_entity_source() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("article.html");
    let preset = dir.path().join("preset.toml");
    let html = "<p>Before <img alt='&#98;ad'> after bad</p>";
    fs::write(&input, html).unwrap();
    fs::write(
        &preset,
        "version=1\nname='test'\n[[gates]]\nid='words'\nkind='forbidden_terms'\nterms=['bad']\n",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .args([
            "content",
            input.to_str().unwrap(),
            "--input-format",
            "html",
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
    assert_eq!(
        report["source_locations"],
        serde_json::json!([
            {"gate_id":"words", "finding_index":0, "line":1, "column":html.find("&#98;").unwrap()+1},
            {"gate_id":"words", "finding_index":1, "line":1, "column":html.rfind("bad").unwrap()+1}
        ])
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    let event: serde_json::Value = stderr
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .find(|event: &serde_json::Value| event["event"] == "content_evaluation")
        .unwrap();
    assert_eq!(event["source_mapping"]["mapped"], 2);
    assert_eq!(event["source_mapping"]["unmapped"], 0);
    assert!(!stderr.contains("&#98;"));
}

#[test]
fn html_entities_map_findings_to_their_source_spelling() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("article.html");
    let preset = dir.path().join("preset.toml");
    fs::write(&input, "<p>É &#x62;ad &amp; &copy; bad</p>").unwrap();
    fs::write(
        &preset,
        "version=1\nname='test'\n[[gates]]\nid='words'\nkind='forbidden_terms'\nterms=['bad']\n",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .args([
            "content",
            input.to_str().unwrap(),
            "--input-format",
            "html",
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
    assert_eq!(
        report["source_locations"],
        serde_json::json!([
            {"gate_id":"words", "finding_index":0, "line":1, "column":6},
            {"gate_id":"words", "finding_index":1, "line":1, "column":28}
        ])
    );
    let event = String::from_utf8_lossy(&output.stderr)
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .find(|event| event["event"] == "content_evaluation")
        .unwrap();
    assert_eq!(event["source_mapping"]["mapped"], 2);
    assert_eq!(event["source_mapping"]["unmapped"], 0);
}

#[test]
fn html_headings_and_link_occurrences_map_to_source_tags() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("article.html");
    let preset = dir.path().join("preset.toml");
    let html = "<h1>Long title</h1>\n<h2>Long title</h2>\n<p><a href='http://bad'>x</a> <a href='https://ok'>y</a><a href='http://bad'>z</a></p>";
    fs::write(&input, html).unwrap();
    fs::write(&preset, "version=1\nname='test'\n[[gates]]\nid='length'\nkind='heading_length'\nmax_chars=3\n[[gates]]\nid='dup'\nkind='duplicate_headings'\n[[gates]]\nid='links'\nkind='link_destinations'\nallowed_schemes=['https']\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .args([
            "content",
            input.to_str().unwrap(),
            "--input-format",
            "html",
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
    assert_eq!(
        report["source_locations"],
        serde_json::json!([
            {"gate_id":"length", "finding_index":0, "line":1, "column":1},
            {"gate_id":"length", "finding_index":1, "line":2, "column":1},
            {"gate_id":"dup", "finding_index":0, "line":2, "column":1},
            {"gate_id":"links", "finding_index":0, "line":3, "column":4},
            {"gate_id":"links", "finding_index":1, "line":3, "column":57}
        ])
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    let event: serde_json::Value = stderr
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .find(|event| event["event"] == "content_evaluation")
        .unwrap();
    assert_eq!(event["source_mapping"]["mapped"], 5);
    assert!(!stderr.contains("http://bad"));
}

#[test]
fn html_limit_telemetry_reports_unavailable_without_fabricated_counts() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("large.html");
    fs::write(&input, "x".repeat(16 * 1024 * 1024 + 1)).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .args([
            "content",
            input.to_str().unwrap(),
            "--input-format",
            "html",
            "--preset",
            "article",
            "--format",
            "json",
            "--telemetry",
        ])
        .output()
        .unwrap();
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["gates"][0]["status"], "not_checked");
    let stderr = String::from_utf8_lossy(&output.stderr);
    let event: serde_json::Value = stderr
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .find(|event| event["event"] == "content_evaluation")
        .unwrap();
    assert_eq!(
        event["html_projection"],
        serde_json::json!({"status":"unavailable", "source_location_capability":"unavailable"})
    );
    assert!(!stderr.contains("large.html"));
}

#[test]
fn html_main_scope_excludes_navigation_and_reports_incomplete_scope() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("article.html");
    let preset = dir.path().join("preset.toml");
    fs::write(&preset, "version=1\nname='test'\n[[gates]]\nid='words'\nkind='forbidden_terms'\nterms=['navigation']\n").unwrap();
    let run = |source: &str| {
        fs::write(&input, source).unwrap();
        Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
            .args([
                "content",
                input.to_str().unwrap(),
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
            .unwrap()
    };
    let output = run("<nav>navigation</nav><main><p>Article</p></main><footer>navigation</footer>");
    assert_eq!(output.status.code(), Some(0));
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["gates"][0]["status"], "passed");
    assert_eq!(report["content_projection"]["scope"], "main");
    assert_eq!(
        report["source_location_capability"]["status"],
        "literal_html"
    );
    let telemetry = String::from_utf8_lossy(&output.stderr);
    assert!(!telemetry.contains("navigation"));
    let event: serde_json::Value = telemetry
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .find(|event| event["event"] == "content_evaluation")
        .unwrap();
    assert_eq!(event["html_projection"]["scope"], "main");
    for source in ["<p>Article</p>", "<main>One</main><main>Two</main>"] {
        let output = run(source);
        let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(report["gates"][0]["status"], "not_checked");
        assert!(
            report["gates"][0]["findings"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }
}

#[test]
fn html_scope_requires_html_input() {
    let output = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .args(["content", "--html-scope", "main", "--input-format", "text"])
        .output()
        .unwrap();
    assert!(!output.status.success());
}

#[test]
fn main_scope_maps_the_selected_occurrence_not_navigation_or_footer() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("article.html");
    let preset = dir.path().join("preset.toml");
    fs::write(
        &input,
        "<nav><p>word</p></nav>\n<main>\n<p>word</p>\n</main>\n<footer><p>word</p></footer>",
    )
    .unwrap();
    fs::write(
        &preset,
        "version=1\nname='test'\n[[gates]]\nid='words'\nkind='forbidden_terms'\nterms=['word']\n",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .args([
            "content",
            input.to_str().unwrap(),
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
        ])
        .output()
        .unwrap();
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        report["source_location_capability"]["status"],
        "literal_html"
    );
    assert_eq!(
        report["source_locations"],
        serde_json::json!([{"gate_id":"words", "finding_index":0, "line":3, "column":4}])
    );
}

#[test]
fn prose_variance_uses_html_list_boundaries_and_withholds_incomplete_structure() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("article.html");
    let preset = dir.path().join("preset.toml");
    fs::write(&preset, "version=1\nname='test'\n[[gates]]\nid='variance'\nkind='prose_sentence_length_variance'\nwindow_size=3\nmin_stddev_words=2\n").unwrap();
    for (source, status) in [
        (
            "<p>One two. Three four. Five six.</p><ul><li><p>List one. List two. List three.</p></li></ul><p>Seven eight. Nine ten. Eleven twelve.</p>",
            "failed",
        ),
        (
            "<p>One two. Three four. Five six.</p><ul><li>unrepresented</li></ul><p>Seven eight. Nine ten. Eleven twelve.</p>",
            "not_checked",
        ),
    ] {
        fs::write(&input, source).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
            .args([
                "content",
                input.to_str().unwrap(),
                "--input-format",
                "html",
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
        assert_eq!(
            report["gates"][0]["status"],
            status,
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        if status == "failed" {
            assert_eq!(
                report["gates"][0]["measurements"]["list_paragraphs_excluded"],
                1
            );
            assert_eq!(report["gates"][0]["measurements"]["windows_checked"], 2);
            assert_eq!(report["gates"][0]["findings"].as_array().unwrap().len(), 2);
            assert_eq!(report["source_locations"][0]["column"], 4);
            assert_eq!(report["source_locations"][1]["column"], 97);
            let telemetry = String::from_utf8_lossy(&output.stderr);
            let event: serde_json::Value = telemetry
                .lines()
                .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
                .find(|event| event["event"] == "content_evaluation")
                .unwrap();
            assert_eq!(
                event["prose_variance"],
                serde_json::json!({
                    "measured_gates": 1, "list_paragraphs_excluded": 1, "windows_checked": 2
                })
            );
            assert!(!telemetry.contains(source));
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

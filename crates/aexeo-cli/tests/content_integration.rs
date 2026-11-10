use std::{
    io::Write,
    process::{Command, Stdio},
};

#[test]
fn stdin_content_returns_structured_findings() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .args(["content", "-", "--preset", "glossary", "--format", "json"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(b"").unwrap();
    let output = child.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(1));
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["gates"][0]["status"], "failed");
}

#[test]
fn missing_adapter_capability_does_not_return_success() {
    let dir = tempfile::tempdir().unwrap();
    let preset = dir.path().join("preset.toml");
    std::fs::write(&preset, "version=1\nname='Custom'\n[[gates]]\nid='metadata'\nkind='required_metadata'\nfields=['title']\n").unwrap();
    let source = format!("custom={}", preset.display());
    let mut child = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .args([
            "content",
            "-",
            "--preset",
            "custom",
            "--preset-source",
            &source,
            "--format",
            "json",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(b"Hello").unwrap();
    let output = child.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(2));
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["gates"][0]["status"], "not_checked");
}

#[test]
fn advisory_failures_do_not_block_the_command() {
    let dir = tempfile::tempdir().unwrap();
    let preset = dir.path().join("preset.toml");
    std::fs::write(&preset, "version=1\nname='Advisory'\n[[gates]]\nid='terms'\nkind='forbidden_terms'\nterms=['hype']\nblocking=false\nseverity='warning'\n").unwrap();
    let source = format!("custom={}", preset.display());
    let mut child = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .args([
            "content",
            "-",
            "--preset",
            "custom",
            "--preset-source",
            &source,
            "--format",
            "json",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(b"hype").unwrap();
    let output = child.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(0));
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["gates"][0]["status"], "failed");
    assert_eq!(report["gates"][0]["findings"][0]["severity"], "warning");
    assert_eq!(report["gates"][0]["required_capability"], "text");
}

#[test]
fn website_preset_preserves_site_audit_findings() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("index.html"),
        "<html><head><title>Example</title></head><body>Welcome</body></html>",
    )
    .unwrap();
    let config = aexeo_core::Config::default();
    let root = dir.path().canonicalize().unwrap();
    let site = aexeo_core::load_site(&root).unwrap();
    let expected = aexeo_core::run_checks_for_site(&site, &config);
    let output = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .args([
            "content",
            "--no-text",
            "--site",
            dir.path().to_str().unwrap(),
            "--preset",
            "seo-website",
            "--format",
            "json",
        ])
        .output()
        .unwrap();
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let findings: Vec<aexeo_contracts::Finding> =
        serde_json::from_value(report["gates"][0]["findings"].clone()).unwrap();
    assert_eq!(findings, expected);
    assert_eq!(report["gates"][0]["required_capability"], "site");
}

#[test]
fn markdown_mode_excludes_code_and_checks_headings() {
    let dir = tempfile::tempdir().unwrap();
    let preset = dir.path().join("preset.toml");
    std::fs::write(&preset, "version=1\nname='Structure'\n[[gates]]\nid='headings'\nkind='required_headings'\nheadings=['Summary']\n[[gates]]\nid='terms'\nkind='forbidden_terms'\nterms=['hype']\n").unwrap();
    let source = format!("custom={}", preset.display());
    let mut child = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .args([
            "content",
            "-",
            "--input-format",
            "markdown",
            "--preset",
            "custom",
            "--preset-source",
            &source,
            "--format",
            "json",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"# Summary\n\nText\n\n```\nhype\n```\n")
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(0));
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["gates"][0]["status"], "passed");
    assert_eq!(report["gates"][1]["status"], "passed");
}

#[test]
fn explicit_fix_adds_only_the_configured_final_newline_and_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let preset = dir.path().join("preset.toml");
    let article = dir.path().join("article.md");
    std::fs::write(
        &preset,
        "version=1\nname='Format'\n[[gates]]\nid='newline'\nkind='final_newline'\n",
    )
    .unwrap();
    std::fs::write(&article, "First line\r\nLast line").unwrap();
    let source = format!("custom={}", preset.display());
    let invoke = || {
        Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
            .args([
                "content",
                article.to_str().unwrap(),
                "--preset",
                "custom",
                "--preset-source",
                &source,
                "--input-format",
                "markdown",
                "--format",
                "json",
                "--fix",
            ])
            .output()
            .unwrap()
    };
    let output = invoke();
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        std::fs::read(&article).unwrap(),
        b"First line\r\nLast line\r\n"
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["applied_fixes"], serde_json::json!(["newline"]));
    let output = invoke();
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["applied_fixes"], serde_json::json!([]));
    assert_eq!(
        std::fs::read(&article).unwrap(),
        b"First line\r\nLast line\r\n"
    );
}

#[cfg(unix)]
#[test]
fn fix_rejects_symlinks() {
    let dir = tempfile::tempdir().unwrap();
    let preset = dir.path().join("preset.toml");
    let article = dir.path().join("article.md");
    let link = dir.path().join("link.md");
    std::fs::write(
        &preset,
        "version=1\nname='Format'\n[[gates]]\nid='newline'\nkind='final_newline'\n",
    )
    .unwrap();
    std::fs::write(&article, "unchanged").unwrap();
    std::os::unix::fs::symlink(&article, &link).unwrap();
    let source = format!("custom={}", preset.display());
    let output = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .args([
            "content",
            link.to_str().unwrap(),
            "--preset",
            "custom",
            "--preset-source",
            &source,
            "--fix",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert_eq!(std::fs::read(&article).unwrap(), b"unchanged");
}

#[cfg(unix)]
#[test]
fn fix_rejects_files_with_multiple_hard_links() {
    let dir = tempfile::tempdir().unwrap();
    let preset = dir.path().join("preset.toml");
    let article = dir.path().join("article.md");
    let link = dir.path().join("other.md");
    std::fs::write(
        &preset,
        "version=1\nname='Format'\n[[gates]]\nid='newline'\nkind='final_newline'\n",
    )
    .unwrap();
    std::fs::write(&article, "unchanged").unwrap();
    std::fs::hard_link(&article, &link).unwrap();
    let source = format!("custom={}", preset.display());
    let output = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .args([
            "content",
            article.to_str().unwrap(),
            "--preset",
            "custom",
            "--preset-source",
            &source,
            "--fix",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert_eq!(std::fs::read(&link).unwrap(), b"unchanged");
}

#[test]
fn explain_emits_reusable_resolved_toml() {
    let output = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .args(["content", "--preset", "glossary-editorial", "--explain"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let source = String::from_utf8(output.stdout).unwrap();
    let preset = aexeo_core::content_gate::ContentPreset::from_toml(&source).unwrap();
    assert_eq!(preset.name, "Glossary editorial starter");
    assert_eq!(preset.gates.len(), 3);
}

#[test]
fn explain_respects_json_format() {
    let output = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .args([
            "content",
            "--preset",
            "article-editorial",
            "--explain",
            "--format",
            "json",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let preset: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(preset["name"], "Article editorial starter");
    assert_eq!(preset["gates"].as_array().unwrap().len(), 3);
}

#[test]
fn literal_evidence_checks_are_explicit_about_missing_and_mismatched_sources() {
    let dir = tempfile::tempdir().unwrap();
    let preset = dir.path().join("evidence.toml");
    let content = dir.path().join("article.txt");
    let source = dir.path().join("snapshot.txt");
    std::fs::write(&preset, "version=1\nname='anchors'\n[[gates]]\nid='anchors'\nkind='evidence_anchors'\nsource='study'\nclaim='The count is 14.'\nquote='14 participants'\n").unwrap();
    std::fs::write(&content, "The count is 14.").unwrap();
    std::fs::write(&source, "Study observed 14 participants.").unwrap();
    let preset_arg = format!("custom={}", preset.display());
    let source_arg = format!("study={}", source.display());
    let invoke = |source_args: &[&str]| {
        let output = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
            .arg("content")
            .arg(&content)
            .args([
                "--preset",
                "custom",
                "--preset-source",
                &preset_arg,
                "--format",
                "json",
            ])
            .args(source_args)
            .output()
            .unwrap();
        let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        (output.status.code().unwrap(), report)
    };
    let (code, missing) = invoke(&[]);
    assert_eq!(code, 2);
    assert_eq!(missing["gates"][0]["status"], "not_checked");
    let (code, matched) = invoke(&["--evidence-source", &source_arg]);
    assert_eq!(code, 0);
    assert_eq!(matched["gates"][0]["status"], "passed");
    std::fs::write(&source, "Study observed 12 participants.").unwrap();
    let (code, mismatch) = invoke(&["--evidence-source", &source_arg]);
    assert_eq!(code, 1);
    assert_eq!(mismatch["gates"][0]["status"], "failed");
}

#[test]
fn structured_facts_compare_values_and_report_missing_evidence() {
    let dir = tempfile::tempdir().unwrap();
    let preset = dir.path().join("facts.toml");
    let content = dir.path().join("article.txt");
    let source = dir.path().join("snapshot.json");
    std::fs::write(&preset, "version=1\nname='facts'\n[[gates]]\nid='count'\nkind='structured_fact'\nsource='study'\nclaim='The study had 14 participants.'\npointer='/sample/count'\nexpected=14\n").unwrap();
    std::fs::write(&content, "The study had 14 participants.").unwrap();
    let preset_arg = format!("custom={}", preset.display());
    let source_arg = format!("study={}", source.display());
    let invoke = || {
        let output = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
            .arg("content")
            .arg(&content)
            .args([
                "--preset",
                "custom",
                "--preset-source",
                &preset_arg,
                "--format",
                "json",
                "--evidence-source",
                &source_arg,
            ])
            .output()
            .unwrap();
        let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        (
            output.status.code().unwrap(),
            report["gates"][0]["status"].as_str().unwrap().to_owned(),
        )
    };
    std::fs::write(&source, r#"{"sample":{"count":14}}"#).unwrap();
    assert_eq!(invoke(), (0, "passed".into()));
    std::fs::write(&source, r#"{"sample":{"count":12}}"#).unwrap();
    assert_eq!(invoke(), (1, "failed".into()));
    std::fs::write(&source, r#"{"sample":{}}"#).unwrap();
    assert_eq!(invoke(), (2, "not_checked".into()));
    std::fs::write(&source, "not json").unwrap();
    assert_eq!(invoke(), (2, "not_checked".into()));
}

#[test]
fn report_explains_measurements_and_unchecked_capabilities() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .args([
            "content",
            "-",
            "--preset",
            "article-editorial",
            "--format",
            "json",
            "--telemetry",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"private words here")
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["summary"]["passed"], 1);
    assert_eq!(report["summary"]["failed"], 1);
    assert_eq!(report["summary"]["not_checked"], 1);
    assert_eq!(report["gates"][1]["measurements"]["word_count"], 3);
    assert_eq!(report["gates"][1]["measurements"]["min_words"], 50);
    assert_eq!(report["gates"][2]["measurements"]["required_fields"], 2);
    assert!(
        report["gates"][2]["measurements"]
            .get("present_fields")
            .is_none()
    );
    let telemetry: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(telemetry["event"], "content_evaluation");
    assert_eq!(telemetry["summary"], report["summary"]);
    assert!(telemetry["evaluation_ms"].as_f64().unwrap() >= 0.0);
    assert!(
        !String::from_utf8(output.stderr)
            .unwrap()
            .contains("private words")
    );
}

#[test]
fn telemetry_is_opt_in_and_reports_metadata_presence() {
    let dir = tempfile::tempdir().unwrap();
    let metadata = dir.path().join("metadata.json");
    std::fs::write(&metadata, r#"{"title":"Example","description":"Summary"}"#).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .args([
            "content",
            "-",
            "--preset",
            "article-editorial",
            "--metadata",
            metadata.to_str().unwrap(),
            "--format",
            "json",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"some words")
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.stderr.is_empty());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["gates"][2]["measurements"]["present_fields"], 2);
}

#[test]
fn heading_and_term_measurements_distinguish_absent_inputs() {
    let dir = tempfile::tempdir().unwrap();
    let preset = dir.path().join("preset.toml");
    std::fs::write(&preset, "version=1\nname='Measures'\n[[gates]]\nid='headings'\nkind='required_headings'\nheadings=['Intro','Summary']\nblocking=false\n[[gates]]\nid='terms'\nkind='forbidden_terms'\nterms=['hype','buzz']\nblocking=false\n").unwrap();
    let source = format!("custom={}", preset.display());
    let mut child = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .args([
            "content",
            "-",
            "--input-format",
            "markdown",
            "--preset",
            "custom",
            "--preset-source",
            &source,
            "--format",
            "json",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"# Intro\n\nhype hype\n")
        .unwrap();
    let output = child.wait_with_output().unwrap();
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["gates"][0]["measurements"]["required_headings"], 2);
    assert_eq!(report["gates"][0]["measurements"]["present_headings"], 1);
    assert_eq!(report["gates"][1]["measurements"]["configured_terms"], 2);
    assert_eq!(report["gates"][1]["measurements"]["matches"], 2);

    let output = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .args([
            "content",
            "--no-text",
            "--preset",
            "custom",
            "--preset-source",
            &source,
            "--format",
            "json",
        ])
        .output()
        .unwrap();
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["summary"]["not_checked"], 2);
    assert!(
        report["gates"][0]["measurements"]
            .get("present_headings")
            .is_none()
    );
    assert!(report["gates"][1]["measurements"].get("matches").is_none());
}

#[test]
fn sentence_character_gate_runs_from_stdin_with_a_portable_preset() {
    let dir = tempfile::tempdir().unwrap();
    let preset = dir.path().join("preset.toml");
    std::fs::write(&preset, "version=1\nname='Sentences'\n[[gates]]\nid='sentence'\nkind='sentence_character_length'\nmax_chars=4\ndelimiters='。'\nblocking=false\n").unwrap();
    let source = format!("custom={}", preset.display());
    let mut child = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .args([
            "content",
            "-",
            "--preset",
            "custom",
            "--preset-source",
            &source,
            "--format",
            "json",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all("中文文章測試。".as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["gates"][0]["status"], "failed");
    assert_eq!(
        report["gates"][0]["measurements"]["longest_sentence_chars"],
        6
    );
}

#[test]
fn sentence_length_gate_uses_configured_delimiters_and_reports_position() {
    let dir = tempfile::tempdir().unwrap();
    let preset = dir.path().join("preset.toml");
    std::fs::write(&preset, "version=1\nname='Sentences'\n[[gates]]\nid='sentence'\nkind='sentence_length'\nmax_words=3\ndelimiters='。!?'\nblocking=false\n").unwrap();
    let source = format!("custom={}", preset.display());
    let mut child = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .args([
            "content",
            "-",
            "--preset",
            "custom",
            "--preset-source",
            &source,
            "--format",
            "json",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all("Court。\n  Un deux trois quatre!\nFin".as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let gate = &report["gates"][0];
    assert_eq!(gate["status"], "failed");
    assert_eq!(gate["measurements"]["sentence_count"], 3);
    assert_eq!(gate["measurements"]["longest_sentence_words"], 4);
    assert_eq!(gate["findings"][0]["line"], 2);
    assert_eq!(gate["findings"][0]["column"], 3);
    let missing = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .args([
            "content",
            "--no-text",
            "--preset",
            "custom",
            "--preset-source",
            &source,
            "--format",
            "json",
        ])
        .output()
        .unwrap();
    let report: serde_json::Value = serde_json::from_slice(&missing.stdout).unwrap();
    assert_eq!(report["gates"][0]["status"], "not_checked");
    assert!(
        report["gates"][0]["measurements"]
            .get("sentence_count")
            .is_none()
    );
}

#[test]
fn readable_article_preset_composes_advisory_sentence_length() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .args([
            "content",
            "-",
            "--preset",
            "article-readable",
            "--format",
            "json",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(format!("{}.", "word ".repeat(31)).as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let gate = report["gates"]
        .as_array()
        .unwrap()
        .iter()
        .find(|gate| gate["gate_id"] == "editorial.sentence_length")
        .unwrap();
    assert_eq!(gate["status"], "failed");
    assert_eq!(gate["blocking"], false);
    assert_eq!(gate["measurements"]["longest_sentence_words"], 31);
}

#[test]
fn sentence_length_rejects_zero_limit_and_empty_delimiters() {
    for settings in ["max_words=0", "max_words=10\ndelimiters='' "] {
        let source = format!(
            "version=1\nname='Invalid'\n[[gates]]\nid='sentence'\nkind='sentence_length'\n{settings}\n"
        );
        assert!(aexeo_core::content_gate::ContentPreset::from_toml(&source).is_err());
    }
}

#[test]
fn sentence_gate_keeps_decimal_numbers_inside_sentences() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .args([
            "content",
            "-",
            "--preset",
            "article-readable",
            "--format",
            "json",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"Rates fell from 45.1 to 34.5 percent. Next sentence.")
        .unwrap();
    let output = child.wait_with_output().unwrap();
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let gate = report["gates"]
        .as_array()
        .unwrap()
        .iter()
        .find(|gate| gate["gate_id"] == "editorial.sentence_length")
        .unwrap();
    assert_eq!(gate["measurements"]["sentence_count"], 2);
    assert_eq!(gate["measurements"]["longest_sentence_words"], 7);
}

#[test]
fn paragraph_gate_uses_adapter_structure_and_does_not_count_headings() {
    let dir = tempfile::tempdir().unwrap();
    let preset = dir.path().join("preset.toml");
    std::fs::write(&preset, "version=1\nname='Paragraphs'\n[[gates]]\nid='paragraphs'\nkind='paragraph_length'\nmax_words=3\nblocking=false\n").unwrap();
    let source = format!("custom={}", preset.display());
    let mut child = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .args([
            "content",
            "-",
            "--input-format",
            "markdown",
            "--preset",
            "custom",
            "--preset-source",
            &source,
            "--format",
            "json",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"# A very long heading ignored\n\nOne two.\n\nOne two three four.\n")
        .unwrap();
    let output = child.wait_with_output().unwrap();
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let gate = &report["gates"][0];
    assert_eq!(gate["status"], "failed");
    assert_eq!(gate["required_capability"], "paragraphs");
    assert_eq!(gate["measurements"]["paragraph_count"], 2);
    assert_eq!(gate["measurements"]["longest_paragraph_words"], 4);
    assert_eq!(gate["findings"][0]["line"], 3);
    let mut raw = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .args([
            "content",
            "-",
            "--preset",
            "custom",
            "--preset-source",
            &source,
            "--format",
            "json",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    raw.stdin
        .take()
        .unwrap()
        .write_all(b"One two three four.")
        .unwrap();
    let report: serde_json::Value =
        serde_json::from_slice(&raw.wait_with_output().unwrap().stdout).unwrap();
    assert_eq!(report["gates"][0]["status"], "not_checked");
}

#[test]
fn paragraph_count_checks_bounds_and_unavailable_input() {
    let dir = tempfile::tempdir().unwrap();
    let preset = dir.path().join("preset.toml");
    std::fs::write(&preset, "version=1\nname='Paragraph count'\n[[gates]]\nid='count'\nkind='paragraph_count'\nmin=2\nmax=2\n").unwrap();
    let source = format!("custom={}", preset.display());
    for (format, input, code, status, count) in [
        (
            "markdown",
            "# Heading\n\nFirst.\n\nSecond.",
            0,
            "passed",
            Some(2),
        ),
        ("markdown", "", 1, "failed", Some(0)),
        (
            "markdown",
            "First.\n\nSecond.\n\nThird.",
            1,
            "failed",
            Some(3),
        ),
        ("text", "First.\n\nSecond.", 2, "not_checked", None),
    ] {
        let mut child = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
            .args([
                "content",
                "-",
                "--input-format",
                format,
                "--preset",
                "custom",
                "--preset-source",
                &source,
                "--format",
                "json",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert_eq!(output.status.code(), Some(code));
        let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        let gate = &report["gates"][0];
        assert_eq!(gate["status"], status);
        assert_eq!(gate["required_capability"], "paragraphs");
        assert_eq!(gate["measurements"]["paragraph_count"].as_u64(), count);
        assert_eq!(gate["measurements"]["min_paragraphs"], 2);
        assert_eq!(gate["measurements"]["max_paragraphs"], 2);
    }
    for bounds in ["", "min=3\nmax=2\n"] {
        let invalid = format!(
            "version=1\nname='Invalid'\n[[gates]]\nid='count'\nkind='paragraph_count'\n{bounds}"
        );
        assert!(aexeo_core::content_gate::ContentPreset::from_toml(&invalid).is_err());
    }
}

#[test]
fn paragraph_gate_rejects_zero_word_limit() {
    let source = "version=1\nname='Invalid'\n[[gates]]\nid='paragraphs'\nkind='paragraph_length'\nmax_words=0\n";
    assert!(aexeo_core::content_gate::ContentPreset::from_toml(source).is_err());
}

#[test]
fn markdown_link_gate_reports_measurements_and_opt_in_telemetry() {
    let dir = tempfile::tempdir().unwrap();
    let preset = dir.path().join("preset.toml");
    std::fs::write(&preset, "version=1\nname='links'\n[[gates]]\nid='links'\nkind='link_count'\nmin=2\nblocking=false\n").unwrap();
    let source = format!("custom={}", preset.display());
    let mut child = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .args([
            "content",
            "-",
            "--input-format",
            "markdown",
            "--preset",
            "custom",
            "--preset-source",
            &source,
            "--format",
            "json",
            "--telemetry",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"[one](https://example.org)")
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["gates"][0]["status"], "failed");
    assert_eq!(report["gates"][0]["measurements"]["link_count"], 1);
    let telemetry: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(telemetry["summary"], report["summary"]);
    assert!(
        !String::from_utf8(output.stderr)
            .unwrap()
            .contains("example.org")
    );
}

#[test]
fn repeated_term_gate_emits_aggregate_telemetry_without_draft_text() {
    let dir = tempfile::tempdir().unwrap();
    let preset = dir.path().join("preset.toml");
    std::fs::write(&preset, "version=1\nname='repeat'\n[[gates]]\nid='repeat'\nkind='repeated_terms'\nterms=['unique-term']\nmax_occurrences=1\nblocking=false\n").unwrap();
    let source = format!("custom={}", preset.display());
    let mut child = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .args([
            "content",
            "-",
            "--preset",
            "custom",
            "--preset-source",
            &source,
            "--format",
            "json",
            "--telemetry",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"unique-term unique-term")
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["gates"][0]["status"], "failed");
    assert_eq!(
        report["gates"][0]["measurements"]["matching_occurrences"],
        2
    );
    let telemetry: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(telemetry["summary"], report["summary"]);
    assert!(
        !String::from_utf8(output.stderr)
            .unwrap()
            .contains("unique-term")
    );
}

fn run_markdown(preset_toml: &str, content: &[u8]) -> (i32, serde_json::Value) {
    let dir = tempfile::tempdir().unwrap();
    let preset = dir.path().join("preset.toml");
    std::fs::write(&preset, preset_toml).unwrap();
    let path = dir.path().join("input.md");
    std::fs::write(&path, content).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .args([
            "content",
            path.to_str().unwrap(),
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
    (
        output.status.code().unwrap(),
        serde_json::from_slice(&output.stdout).unwrap_or_default(),
    )
}

const SENTENCE_PRESET: &str =
    "version=1\nname='r'\n[[gates]]\nid='s'\nkind='sentence_length'\nmax_words=30\n";

#[test]
fn unpunctuated_bullets_are_separate_sentences() {
    let bullets = "- one two three four five six\n".repeat(8);
    let (code, report) = run_markdown(SENTENCE_PRESET, bullets.as_bytes());
    assert_eq!(code, 0, "{report}");
    assert_eq!(report["gates"][0]["status"], "passed");
    assert_eq!(report["gates"][0]["measurements"]["sentence_count"], 8);
}

#[test]
fn deeply_nested_markdown_is_reported_as_not_checked() {
    let deep = format!("{} text\n", ">".repeat(200_000));
    let (code, report) = run_markdown(SENTENCE_PRESET, deep.as_bytes());
    assert_eq!(code, 2);
    assert_eq!(report["gates"][0]["status"], "not_checked");
    assert!(
        report["gates"][0]["reason"]
            .as_str()
            .unwrap()
            .contains("nesting exceeds")
    );
}

#[test]
fn oversized_input_is_reported_as_not_checked_without_being_evaluated() {
    let size = aexeo_core::content_gate::MAX_CONTENT_INPUT_BYTES + 1;
    let (code, report) = run_markdown(SENTENCE_PRESET, &vec![b'a'; size]);
    assert_eq!(code, 2);
    assert_eq!(report["gates"][0]["status"], "not_checked");
    assert!(
        report["gates"][0]["reason"]
            .as_str()
            .unwrap()
            .contains("byte limit")
    );
}

#[test]
fn truncated_heading_findings_keep_their_source_locations() {
    let headings = "# Same\n\n".repeat(1002);
    let (code, report) = run_markdown(
        "version=1\nname='r'\n[[gates]]\nid='d'\nkind='duplicate_headings'\n",
        headings.as_bytes(),
    );
    assert_eq!(code, 1);
    let gate = &report["gates"][0];
    assert_eq!(gate["findings"].as_array().unwrap().len(), 1000);
    assert_eq!(gate["measurements"]["findings_omitted"], 1);
    assert_eq!(gate["measurements"]["duplicate_headings"], 1001);
    assert_eq!(report["source_locations"].as_array().unwrap().len(), 1000);
    assert_eq!(report["source_locations"][0]["line"], 3);
}

#[test]
fn a_preset_that_checks_nothing_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let preset = dir.path().join("preset.toml");
    std::fs::write(&preset, "version=1\nname=\"x\"\n").unwrap();
    let path = dir.path().join("input.md");
    std::fs::write(&path, "text\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .args([
            "content",
            path.to_str().unwrap(),
            "--preset",
            "custom",
            "--preset-source",
            &format!("custom={}", preset.display()),
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert_ne!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("no gates"));
}

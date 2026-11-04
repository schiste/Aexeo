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

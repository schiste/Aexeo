mod support;

use std::fs;
use std::path::Path;
use std::process::Command;
use support::{bin, parse_json, write};

fn write_minimal_site(root: &Path) {
    write(
        &root.join("index.html"),
        "<html><head><title>Home</title><meta name=\"description\" content=\"Home page\"><link rel=\"canonical\" href=\"https://example.com/\"></head><body><h1>Home</h1><a href=\"/about\">About</a></body></html>",
    );
    write(
        &root.join("about/index.html"),
        "<html><head><title>About</title><meta name=\"description\" content=\"About page\"><link rel=\"canonical\" href=\"https://example.com/about/\"></head><body><h1>About</h1></body></html>",
    );
    write(
        &root.join("aexeo.toml"),
        "version = 1\n\n[site]\nurl = \"https://example.com\"\nsource_dir = \".\"\nadapter = \"auto\"\n",
    );
}

#[test]
fn check_command_reports_findings_for_temp_site() {
    let temp_dir = tempfile::tempdir().unwrap();
    write(
        &temp_dir.path().join("index.html"),
        "<html><head><title>Home</title></head><body><h1>Home</h1><a href=\"/missing\">Learn more</a></body></html>",
    );
    let output = Command::new(bin())
        .arg("check")
        .arg(temp_dir.path())
        .arg("--format")
        .arg("text")
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("SEO002"));
    assert!(stdout.contains("LNK001"));
}

#[test]
fn check_sarif_emits_config_warnings_on_stderr() {
    let temp_dir = tempfile::tempdir().unwrap();
    write(
        &temp_dir.path().join("aexeo.toml"),
        "site_url = \"https://example.com\"\n",
    );
    write(
        &temp_dir.path().join("index.html"),
        "<html><head><title>Home</title></head><body><h1>Home</h1></body></html>",
    );
    let output = Command::new(bin())
        .arg("check")
        .arg(temp_dir.path())
        .arg("--format")
        .arg("sarif")
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("CFGDEP001"));
}

#[test]
fn check_json_contract_reports_summary_and_exit_code() {
    let temp_dir = tempfile::tempdir().unwrap();
    write(
        &temp_dir.path().join("index.html"),
        "<html><head><title>Home</title></head><body><h1>Home</h1><a href=\"/missing\">Learn more</a></body></html>",
    );
    let output = Command::new(bin())
        .arg("check")
        .arg(temp_dir.path())
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();
    assert!(!output.status.success());
    let payload = parse_json(&output.stdout);
    assert_eq!(payload["command"], "check");
    assert_eq!(payload["success"], false);
    assert!(
        payload["audit_path"]
            .as_str()
            .unwrap()
            .ends_with("check-latest.json")
    );
    assert!(payload["summary"]["total"].as_u64().unwrap() >= 1);
    assert!(payload["findings"].is_array());
}

#[test]
fn generate_json_contract_reports_kind_and_output() {
    let temp_dir = tempfile::tempdir().unwrap();
    write_minimal_site(temp_dir.path());
    let output = Command::new(bin())
        .arg("generate")
        .arg("llms")
        .arg(temp_dir.path())
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();
    assert!(output.status.success());
    let payload = parse_json(&output.stdout);
    assert_eq!(payload["command"], "generate");
    assert_eq!(payload["success"], true);
    assert_eq!(payload["kind"], "llms");
    assert!(payload["output"].as_str().unwrap().contains("#"));
}

#[test]
fn fix_json_contract_reports_changed_paths() {
    let temp_dir = tempfile::tempdir().unwrap();
    write_minimal_site(temp_dir.path());
    let output = Command::new(bin())
        .arg("fix")
        .arg(temp_dir.path())
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();
    assert!(output.status.success());
    let payload = parse_json(&output.stdout);
    assert_eq!(payload["command"], "fix");
    assert_eq!(payload["success"], true);
    assert_eq!(payload["action"], "apply");
    assert!(payload["paths"].is_array());
}

#[test]
fn check_reads_title_and_h1_like_a_browser_and_survives_non_utf8_pages() {
    let fixture =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/html-parser");
    let temp_dir = tempfile::tempdir().unwrap();
    for name in ["index.html", "commented.html", "latin1.html"] {
        std::fs::copy(fixture.join(name), temp_dir.path().join(name)).unwrap();
    }

    let output = Command::new(bin())
        .arg("check")
        .arg(temp_dir.path())
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();

    // 70 is the internal-error exit a single non-UTF-8 page used to cause.
    assert_ne!(
        output.status.code(),
        Some(70),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let payload = parse_json(&output.stdout);
    let findings = payload["findings"].as_array().unwrap();
    assert!(
        findings
            .iter()
            .any(|finding| finding["path"].as_str().unwrap().ends_with("latin1.html")),
        "the non-UTF-8 page must still be audited: {payload}"
    );
    for finding in findings {
        let rule_id = finding["rule_id"].as_str().unwrap();
        assert!(
            !matches!(rule_id, "SEO001" | "SEO005" | "SEO006"),
            "false title/h1 finding: {finding}"
        );
    }
}

#[test]
fn fix_keeps_a_hand_written_llms_txt_and_reports_it_as_skipped() {
    let temp_dir = tempfile::tempdir().unwrap();
    write_minimal_site(temp_dir.path());
    let llms_path = temp_dir.path().join("llms.txt");
    let hand_written = "# Acme\n\n> Hand-written summary.\n\n## Pages\n- [Home](/): start here\n";
    std::fs::write(&llms_path, hand_written).unwrap();

    let output = Command::new(bin())
        .arg("fix")
        .arg(temp_dir.path())
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(std::fs::read_to_string(&llms_path).unwrap(), hand_written);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("warning: skipped"), "{stderr}");
    assert!(stderr.contains("llms.txt"), "{stderr}");
}

#[test]
fn fix_dry_run_reports_planned_changes_without_writing() {
    let temp_dir = tempfile::tempdir().unwrap();
    write_minimal_site(temp_dir.path());
    let index_path = temp_dir.path().join("index.html");
    let llms_path = temp_dir.path().join("llms.txt");
    let index_before = std::fs::read(&index_path).unwrap();
    let hand_written = "# Acme\n";
    std::fs::write(&llms_path, hand_written).unwrap();

    let output = Command::new(bin())
        .arg("fix")
        .arg(temp_dir.path())
        .arg("--dry-run")
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();

    assert!(output.status.success());
    let payload = parse_json(&output.stdout);
    assert_eq!(payload["action"], "dry-run");
    assert!(
        payload["paths"]
            .as_array()
            .unwrap()
            .iter()
            .any(|path| path.as_str().unwrap().ends_with("index.html")),
        "{payload}"
    );
    assert!(
        payload["skipped"][0]["path"]
            .as_str()
            .unwrap()
            .ends_with("llms.txt"),
        "{payload}"
    );
    assert!(payload["skipped"][0]["reason"].is_string());
    assert_eq!(std::fs::read(&index_path).unwrap(), index_before);
    assert_eq!(std::fs::read_to_string(&llms_path).unwrap(), hand_written);
}

#[test]
fn page_title_cannot_break_out_of_an_injected_schema_block() {
    let temp_dir = tempfile::tempdir().unwrap();
    let root = temp_dir.path();
    let home = root.join("index.html");
    write(
        &home,
        "<html lang=\"en\"><head><title>&lt;/script&gt;&lt;script&gt;alert(1)&lt;/script&gt;</title></head><body><h1>Home</h1></body></html>",
    );

    let bundle = Command::new(bin())
        .args(["generate", "public-bundle"])
        .arg(root)
        .args(["--site-url", "https://example.com", "--write-dir"])
        .arg(root)
        .output()
        .unwrap();
    assert!(bundle.status.success());
    let fix = Command::new(bin())
        .arg("fix")
        .arg(root)
        .arg("--inject-schema")
        .output()
        .unwrap();
    assert!(fix.status.success());

    let html = std::fs::read_to_string(&home).unwrap();
    assert!(html.contains("application/ld+json"), "{html}");
    assert!(!html.contains("<script>alert"), "{html}");
    assert_eq!(html.matches("</script>").count(), 1, "{html}");
}

#[test]
fn baseline_json_contract_reports_output_path() {
    let temp_dir = tempfile::tempdir().unwrap();
    write_minimal_site(temp_dir.path());
    let output = Command::new(bin())
        .arg("baseline")
        .arg(temp_dir.path())
        .arg("--output")
        .arg(temp_dir.path().join("custom-baseline.json"))
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();
    assert!(output.status.success());
    let payload = parse_json(&output.stdout);
    assert_eq!(payload["command"], "baseline");
    assert_eq!(payload["success"], true);
    assert!(
        payload["path"]
            .as_str()
            .unwrap()
            .ends_with("custom-baseline.json")
    );
}

#[test]
fn diff_json_contract_reports_summary() {
    let temp_dir = tempfile::tempdir().unwrap();
    write(
        &temp_dir.path().join("index.html"),
        "<html><head><title>Home</title></head><body><h1>Home</h1></body></html>",
    );
    let first = Command::new(bin())
        .arg("check")
        .arg(temp_dir.path())
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();
    assert!(!first.status.success());
    let first_payload = parse_json(&first.stdout);
    let baseline_path = temp_dir.path().join("baseline-audit.json");
    fs::copy(
        first_payload["audit_path"].as_str().unwrap(),
        &baseline_path,
    )
    .unwrap();

    write(
        &temp_dir.path().join("index.html"),
        "<html><head><title>Home</title><meta name=\"description\" content=\"Home page\"></head><body><h1>Home</h1><a href=\"/missing\">Learn more</a></body></html>",
    );
    let second = Command::new(bin())
        .arg("check")
        .arg(temp_dir.path())
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();
    assert!(!second.status.success());
    let second_payload = parse_json(&second.stdout);
    let current_path = second_payload["audit_path"].as_str().unwrap().to_string();

    let diff_output = Command::new(bin())
        .arg("diff")
        .arg(&baseline_path)
        .arg(&current_path)
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();
    assert!(!diff_output.status.success());
    let payload = parse_json(&diff_output.stdout);
    assert_eq!(payload["command"], "diff");
    assert_eq!(payload["success"], false);
    assert!(payload["summary"]["new"].as_u64().unwrap() >= 1);
    assert!(payload["diff"]["new_findings"].is_array());
}

/// Regression: a command that legitimately reports findings and an internal
/// error both used to exit 1, so a CI gate could not tell "your site has
/// problems" from "aexeo failed to run". Internal errors now use 70
/// (`EX_SOFTWARE`), keeping 0/1/2 exactly as `SPEC.md` documents them.
#[test]
fn internal_errors_exit_distinctly_from_findings() {
    let temp_dir = tempfile::tempdir().unwrap();
    write_minimal_site(temp_dir.path());

    // A real run that finds problems: a result, exit 1.
    write(
        &temp_dir.path().join("index.html"),
        "<html><head><title>Home</title><meta name=\"description\" content=\"Home page\"></head><body><h1>Home</h1><a href=\"/nowhere\">Learn more</a></body></html>",
    );
    let with_findings = Command::new(bin())
        .arg("check")
        .arg(temp_dir.path())
        .arg("--format")
        .arg("text")
        .output()
        .unwrap();
    assert_eq!(
        with_findings.status.code(),
        Some(1),
        "findings are a result and must stay exit 1"
    );

    // An unexpected error: aexeo itself failed, exit 70.
    let internal_error = Command::new(bin())
        .arg("check")
        .arg(temp_dir.path().join("does-not-exist"))
        .arg("--format")
        .arg("text")
        .output()
        .unwrap();
    assert_eq!(
        internal_error.status.code(),
        Some(70),
        "an internal error must not be reported as findings; stderr: {}",
        String::from_utf8_lossy(&internal_error.stderr)
    );
}

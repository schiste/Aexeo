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
        &root.join("seogeo.toml"),
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
        &temp_dir.path().join("seogeo.toml"),
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

mod runtime_support;
mod support;

use runtime_support::spawn_server;
use std::process::Command;
use support::{bin, parse_json, write};

#[test]
fn crawl_and_verify_commands_work_end_to_end() {
    let (base_url, handle) = spawn_server(12);
    let temp_dir = tempfile::tempdir().unwrap();
    let crawl_output = Command::new(bin())
        .current_dir(temp_dir.path())
        .arg("crawl")
        .arg(&base_url)
        .arg("--engine")
        .arg("http")
        .arg("--max-pages")
        .arg("10")
        .arg("--format")
        .arg("text")
        .output()
        .unwrap();
    assert!(!crawl_output.status.success());
    let crawl_stdout = String::from_utf8_lossy(&crawl_output.stdout);
    assert!(crawl_stdout.contains("Audit results:"));

    let verify_output = Command::new(bin())
        .current_dir(temp_dir.path())
        .arg("verify")
        .arg(&base_url)
        .arg("--engine")
        .arg("http")
        .arg("--max-pages")
        .arg("10")
        .arg("--baseline")
        .arg(".seogeo-reports/crawl-latest.json")
        .arg("--format")
        .arg("text")
        .output()
        .unwrap();
    assert!(verify_output.status.success());
    let verify_stdout = String::from_utf8_lossy(&verify_output.stdout);
    assert!(verify_stdout.contains("New findings: 0"));
    handle.join().unwrap();
}

#[test]
fn crawl_uses_configured_runtime_engine_when_flag_is_omitted() {
    let (base_url, handle) = spawn_server(6);
    let temp_dir = tempfile::tempdir().unwrap();
    write(
        &temp_dir.path().join("seogeo.toml"),
        "browser_engine = \"http\"\n",
    );
    let output = Command::new(bin())
        .current_dir(temp_dir.path())
        .arg("crawl")
        .arg(&base_url)
        .arg("--max-pages")
        .arg("10")
        .arg("--format")
        .arg("text")
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Audit results:"));
    handle.join().unwrap();
}

#[test]
fn crawl_rejects_reserved_playwright_engine() {
    let (base_url, handle) = spawn_server(0);
    let output = Command::new(bin())
        .arg("crawl")
        .arg(&base_url)
        .arg("--engine")
        .arg("playwright")
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("not implemented"));
    handle.join().unwrap();
}

#[test]
fn crawl_sarif_emits_config_warnings_on_stderr() {
    let (base_url, handle) = spawn_server(6);
    let temp_dir = tempfile::tempdir().unwrap();
    write(
        &temp_dir.path().join("seogeo.toml"),
        "browser_engine = \"http\"\n",
    );
    let output = Command::new(bin())
        .current_dir(temp_dir.path())
        .arg("crawl")
        .arg(&base_url)
        .arg("--max-pages")
        .arg("10")
        .arg("--format")
        .arg("sarif")
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("CFGDEP001"));
    handle.join().unwrap();
}

#[test]
fn crawl_json_contract_reports_summary_and_exit_code() {
    let (base_url, handle) = spawn_server(6);
    let temp_dir = tempfile::tempdir().unwrap();
    let output = Command::new(bin())
        .current_dir(temp_dir.path())
        .arg("crawl")
        .arg(&base_url)
        .arg("--engine")
        .arg("http")
        .arg("--max-pages")
        .arg("10")
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();
    assert!(!output.status.success());
    let payload = parse_json(&output.stdout);
    assert_eq!(payload["command"], "crawl");
    assert_eq!(payload["success"], false);
    assert!(
        payload["audit_path"]
            .as_str()
            .unwrap()
            .ends_with("crawl-latest.json")
    );
    assert!(payload["findings"].is_array());
    handle.join().unwrap();
}

#[test]
fn verify_json_contract_reports_diff_summary() {
    let (base_url, handle) = spawn_server(12);
    let temp_dir = tempfile::tempdir().unwrap();
    let crawl_output = Command::new(bin())
        .current_dir(temp_dir.path())
        .arg("crawl")
        .arg(&base_url)
        .arg("--engine")
        .arg("http")
        .arg("--max-pages")
        .arg("10")
        .arg("--format")
        .arg("text")
        .output()
        .unwrap();
    assert!(!crawl_output.status.success());

    let verify_output = Command::new(bin())
        .current_dir(temp_dir.path())
        .arg("verify")
        .arg(&base_url)
        .arg("--engine")
        .arg("http")
        .arg("--max-pages")
        .arg("10")
        .arg("--baseline")
        .arg(".seogeo-reports/crawl-latest.json")
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();
    assert!(verify_output.status.success());
    let payload = parse_json(&verify_output.stdout);
    assert_eq!(payload["command"], "verify");
    assert_eq!(payload["success"], true);
    assert_eq!(payload["summary"]["new"], 0);
    assert!(payload["diff"]["new_findings"].is_array());
    handle.join().unwrap();
}

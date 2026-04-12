mod runtime_support;
mod support;

use runtime_support::spawn_server;
use std::process::Command;
use support::{bin, parse_json, write};

#[test]
fn crawl_and_verify_commands_work_end_to_end() {
    let (base_url, handle) = spawn_server(16);
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
    let (base_url, handle) = spawn_server(8);
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
fn crawl_rejects_playwright_when_runner_override_is_missing() {
    let output = Command::new(bin())
        .env(
            "SEOGEO_PLAYWRIGHT_EXECUTABLE",
            "/definitely/missing/seogeo-playwright-runner",
        )
        .arg("crawl")
        .arg("http://127.0.0.1:1")
        .arg("--engine")
        .arg("playwright")
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.trim().is_empty(), "unexpected stdout: {stdout}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("requires a local Playwright runtime"),
        "unexpected stderr: {stderr}"
    );
}

#[test]
fn crawl_accepts_playwright_with_custom_runner() {
    let (base_url, handle) = spawn_server(3);
    let temp_dir = tempfile::tempdir().unwrap();
    let runner = temp_dir.path().join("playwright-runner.sh");
    write(
        &runner,
        &format!(
            "#!/bin/sh\nprintf '%s' '{{\"statusCode\":200,\"contentType\":\"text/html\",\"body\":\"<html><head><title>Home</title><meta name='\\''description'\\'' content='\\''Root'\\''><link rel='\\''canonical'\\'' href='\\''{base_url}/'\\''></head><body><h1>Home</h1><a href='/about'>About</a></body></html>\",\"headers\":{{\"content-type\":\"text/html\"}},\"effectiveUrl\":\"{base_url}/\"}}'\n"
        ),
    );
    let mut permissions = std::fs::metadata(&runner).unwrap().permissions();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        permissions.set_mode(0o755);
    }
    std::fs::set_permissions(&runner, permissions).unwrap();

    let output = Command::new(bin())
        .current_dir(temp_dir.path())
        .env("SEOGEO_PLAYWRIGHT_EXECUTABLE", &runner)
        .arg("crawl")
        .arg(&base_url)
        .arg("--engine")
        .arg("playwright")
        .arg("--max-pages")
        .arg("1")
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    let payload = parse_json(&output.stdout);
    assert_eq!(payload["command"], "crawl");
    assert_eq!(payload["success"], true);
    handle.join().unwrap();
}

#[test]
fn crawl_sarif_emits_config_warnings_on_stderr() {
    let (base_url, handle) = spawn_server(8);
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
    let (base_url, handle) = spawn_server(8);
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
    let (base_url, handle) = spawn_server(16);
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

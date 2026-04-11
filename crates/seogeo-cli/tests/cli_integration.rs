use serde_json::Value;
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::Command;
use std::thread;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_seogeo-cli")
}

fn write(path: &Path, text: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, text).unwrap();
}

fn respond(mut stream: TcpStream, status: &str, content_type: &str, body: &str) {
    let response = format!(
        "HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        status,
        content_type,
        body.len(),
        body
    );
    stream.write_all(response.as_bytes()).unwrap();
    stream.flush().unwrap();
}

fn spawn_server(expected_requests: usize) -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let handle = thread::spawn(move || {
        for _ in 0..expected_requests {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buffer = [0_u8; 4096];
            let size = stream.read(&mut buffer).unwrap();
            let request = String::from_utf8_lossy(&buffer[..size]);
            let path = request
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .unwrap_or("/");
            match path {
                "/" => respond(
                    stream,
                    "200 OK",
                    "text/html",
                    "<html><head><title>Home</title><meta name=\"description\" content=\"Root\"><link rel=\"canonical\" href=\"http://example.test/\"></head><body><h1>Home</h1><a href=\"/about\">About</a></body></html>",
                ),
                "/about" => respond(
                    stream,
                    "200 OK",
                    "text/html",
                    "<html><head><meta name=\"description\" content=\"About page\"></head><body><h1>About</h1></body></html>",
                ),
                "/robots.txt" => {
                    respond(stream, "200 OK", "text/plain", "User-agent: *\nAllow: /\n")
                }
                "/llms.txt" => respond(
                    stream,
                    "200 OK",
                    "text/plain",
                    "# Site\n\n## Pages\n- [Home](/)\n",
                ),
                "/sitemap.xml" => respond(
                    stream,
                    "200 OK",
                    "application/xml",
                    "<urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\"><url><loc>http://example.test/</loc></url><url><loc>http://example.test/about</loc></url></urlset>",
                ),
                _ => respond(stream, "404 Not Found", "text/plain", "missing"),
            }
        }
    });
    (format!("http://{}", address), handle)
}

#[test]
fn rules_command_lists_builtin_groups() {
    let output = Command::new(bin()).arg("rules").output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("html"));
    assert!(stdout.contains("structure"));
}

fn parse_json(output: &[u8]) -> Value {
    serde_json::from_slice(output).unwrap()
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
fn plugin_check_validates_python_manifest_literal() {
    let temp_dir = tempfile::tempdir().unwrap();
    write(
        &temp_dir.path().join("example/plugin.py"),
        "SEOGEO_PLUGIN_API_VERSION = 1\nSEOGEO_PLUGIN_MANIFEST = {'name':'Example Plugin','namespace':'example.plugin','version':'1.0.0','capabilities':['rules','adapters']}\ndef seogeo_register(registry):\n    return None\n",
    );
    let output = Command::new(bin())
        .env("PYTHONPATH", temp_dir.path())
        .arg("plugin-check")
        .arg("example.plugin")
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Example Plugin"));
    assert!(stdout.contains("example.plugin"));
}

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
fn config_print_renders_resolved_canonical_config() {
    let temp_dir = tempfile::tempdir().unwrap();
    write(
        &temp_dir.path().join("base.toml"),
        r#"
version = 1
profile = "chau7"

[site]
url = "https://example.com"

[rules.links]
enabled = false
min_inbound_links = 2
"#,
    );
    write(
        &temp_dir.path().join("seogeo.toml"),
        r#"
extends = ["base.toml"]
version = 1

[site]
source_dir = "dist"
adapter = "astro-dist"
"#,
    );
    let output = Command::new(bin())
        .arg("config")
        .arg("print")
        .arg(temp_dir.path())
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();
    assert!(output.status.success());
    let payload = parse_json(&output.stdout);
    assert_eq!(payload["command"], "config print");
    assert_eq!(payload["success"], true);
    assert_eq!(payload["config"]["version"], 1);
    assert_eq!(payload["config"]["profile"], "chau7");
    assert_eq!(payload["config"]["site"]["adapter"], "astro-dist");
    assert_eq!(payload["config"]["rules"]["links"]["enabled"], false);
    assert_eq!(payload["config"]["rules"]["links"]["min_inbound_links"], 2);
}

#[test]
fn config_print_json_contract_reports_deprecation_warnings() {
    let temp_dir = tempfile::tempdir().unwrap();
    write(
        &temp_dir.path().join("seogeo.toml"),
        r#"
site_url = "https://example.com"
browser_engine = "http"
"#,
    );
    let output = Command::new(bin())
        .arg("config")
        .arg("print")
        .arg(temp_dir.path())
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();
    assert!(output.status.success());
    let payload = parse_json(&output.stdout);
    assert!(payload["warnings"].is_array());
    assert!(
        payload["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|warning| warning["code"] == "CFGDEP001"
                && warning["message"].as_str().unwrap().contains("site_url"))
    );
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
fn quality_json_contract_reports_summary_and_exit_code() {
    let temp_dir = tempfile::tempdir().unwrap();
    let output = Command::new(bin())
        .arg("quality")
        .arg(temp_dir.path())
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();
    assert!(!output.status.success());
    let payload = parse_json(&output.stdout);
    assert_eq!(payload["command"], "quality");
    assert_eq!(payload["success"], false);
    assert!(payload["summary"]["errors"].as_u64().unwrap() >= 1);
    assert!(payload["findings"].is_array());
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

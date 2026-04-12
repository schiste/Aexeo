mod support;

use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::Command;
use std::thread;

use support::{bin, parse_json, write};

#[test]
fn snippet_inspect_json_contract_reports_live_controls() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut buffer = [0_u8; 2048];
        let _ = stream.read(&mut buffer).unwrap();
        let body = "<html><head><title>Home</title><meta name=\"description\" content=\"desc\"><meta name=\"robots\" content=\"nosnippet\"></head><body><h1>Home</h1></body></html>";
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        stream.write_all(response.as_bytes()).unwrap();
        stream.flush().unwrap();
    });
    let output = Command::new(bin())
        .arg("snippet")
        .arg("inspect")
        .arg("--url")
        .arg(format!("http://{}", address))
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();
    assert!(output.status.success());
    let payload = parse_json(&output.stdout);
    assert_eq!(payload["command"], "snippet inspect");
    assert_eq!(payload["success"], true);
    assert!(payload["result"]["route"].is_string());
    assert_eq!(payload["result"]["snippet_blocked"], true);
    handle.join().unwrap();
}

#[test]
fn snippet_inspect_supports_static_site_paths() {
    let temp_dir = tempfile::tempdir().unwrap();
    write(
        &temp_dir.path().join("index.html"),
        "<html><head><title>Home</title><meta name=\"description\" content=\"desc\"><meta name=\"robots\" content=\"nosnippet\"></head><body><h1>Home</h1><div data-nosnippet=\"true\">x</div></body></html>",
    );
    let output = Command::new(bin())
        .arg("snippet")
        .arg("inspect")
        .arg("--path")
        .arg(temp_dir.path())
        .arg("--route")
        .arg("")
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();
    assert!(output.status.success());
    let payload = parse_json(&output.stdout);
    assert_eq!(payload["result"]["snippet_blocked"], true);
    assert_eq!(payload["result"]["data_nosnippet_blocks"], 1);
}

#[test]
fn indexnow_validate_json_contract_reports_key_mismatches() {
    let temp_dir = tempfile::tempdir().unwrap();
    write(&temp_dir.path().join("abc123.txt"), "mismatch");
    let output = Command::new(bin())
        .arg("indexnow")
        .arg("validate")
        .arg("https://example.com")
        .arg("abc123")
        .arg("--path")
        .arg(temp_dir.path())
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();
    assert!(!output.status.success());
    let payload = parse_json(&output.stdout);
    assert_eq!(payload["command"], "indexnow validate");
    assert_eq!(payload["success"], false);
    assert!(
        payload["result"]["errors"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item.as_str().unwrap().contains("do not match"))
    );
}

#[test]
fn indexnow_submit_json_contract_posts_urls() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut buffer = [0_u8; 4096];
        let size = stream.read(&mut buffer).unwrap();
        let request = String::from_utf8_lossy(&buffer[..size]);
        assert!(request.contains("\"urlList\":[\"https://example.com/a\"]"));
        let response = "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok";
        stream.write_all(response.as_bytes()).unwrap();
        stream.flush().unwrap();
    });

    let output = Command::new(bin())
        .arg("indexnow")
        .arg("submit")
        .arg(format!("http://{}", address))
        .arg("https://example.com")
        .arg("abc123")
        .arg("https://example.com/a")
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();
    assert!(output.status.success());
    let payload = parse_json(&output.stdout);
    assert_eq!(payload["command"], "indexnow submit");
    assert_eq!(payload["result"]["success"], true);
    handle.join().unwrap();
}

#[test]
fn bing_ai_import_json_contract_reports_citation_rollups() {
    let temp_dir = tempfile::tempdir().unwrap();
    write(
        &temp_dir.path().join("bing-ai.csv"),
        "url,query,citations\nhttps://example.com/docs,What is docs,3\nhttps://example.com/docs,How docs work,2\n",
    );
    let output = Command::new(bin())
        .arg("bing-ai")
        .arg("import")
        .arg(temp_dir.path().join("bing-ai.csv"))
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();
    assert!(output.status.success());
    let payload = parse_json(&output.stdout);
    assert_eq!(payload["command"], "bing-ai import");
    assert_eq!(payload["result"]["rows_read"], 2);
    assert_eq!(payload["result"]["cited_urls"][0]["citations"], 5);
}

#[test]
fn search_console_export_supports_csv_output() {
    let temp_dir = tempfile::tempdir().unwrap();
    write(
        &temp_dir.path().join("audit.json"),
        r#"{"version":2,"command":"check","status":"complete","generated_at":0,"summary":{"total":2,"errors":1,"warnings":1,"actionable":2,"heuristic":0},"findings":[{"rule_id":"SEO001","message":"missing <title>","path":"crawl/about/index.html","line":1,"column":1,"severity":"error","scope":"page"},{"rule_id":"SEO002","message":"missing meta description","path":"crawl/about/index.html","line":1,"column":1,"severity":"warning","scope":"page"}]}"#,
    );
    let output = Command::new(bin())
        .arg("search-console")
        .arg("export")
        .arg(temp_dir.path().join("audit.json"))
        .arg("--site-url")
        .arg("https://example.com")
        .arg("--format")
        .arg("csv")
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("route,url,findings,errors,warnings"));
    assert!(stdout.contains("about,https://example.com/about,2,1,1"));
}

#[test]
fn publish_hook_run_json_contract_reports_changed_routes() {
    let temp_dir = tempfile::tempdir().unwrap();
    write(
        &temp_dir.path().join("seogeo.toml"),
        "version = 1\n[site]\nurl = \"https://example.com\"\nsource_dir = \".\"\n",
    );
    write(
        &temp_dir.path().join("index.html"),
        "<html><head><meta name=\"description\" content=\"x\"></head><body><h1>x</h1></body></html>",
    );
    write(&temp_dir.path().join("abc123.txt"), "abc123");
    let output = Command::new(bin())
        .arg("publish-hook")
        .arg("run")
        .arg(temp_dir.path())
        .arg("--changed-url")
        .arg("https://example.com/")
        .arg("--indexnow-key")
        .arg("abc123")
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();
    assert!(output.status.success());
    let payload = parse_json(&output.stdout);
    assert_eq!(payload["command"], "publish-hook run");
    assert_eq!(payload["result"]["changed_routes"][0], "");
    assert!(payload["warnings"].is_array());
}

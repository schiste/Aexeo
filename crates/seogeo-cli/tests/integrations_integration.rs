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
fn indexnow_validate_json_contract_supports_remote_live_checks() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut buffer = [0_u8; 2048];
        let _ = stream.read(&mut buffer).unwrap();
        let body = "abc123";
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        stream.write_all(response.as_bytes()).unwrap();
        stream.flush().unwrap();
    });
    let output = Command::new(bin())
        .arg("indexnow")
        .arg("validate")
        .arg(format!("http://{}", address))
        .arg("abc123")
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();
    assert!(output.status.success());
    let payload = parse_json(&output.stdout);
    assert_eq!(payload["command"], "indexnow validate");
    assert_eq!(payload["success"], true);
    assert_eq!(payload["result"]["validation_mode"], "remote");
    assert_eq!(payload["result"]["key_file_present"], true);
    assert_eq!(payload["result"]["key_file_matches"], true);
    assert_eq!(payload["result"]["remote_status_code"], 200);
    handle.join().unwrap();
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
fn bing_ai_opportunities_json_contract_reports_scores() {
    let temp_dir = tempfile::tempdir().unwrap();
    write(
        &temp_dir.path().join("bing-ai.csv"),
        "url,query,citations\nhttps://example.com/docs,What is docs,5\n",
    );
    write(
        &temp_dir.path().join("audit.json"),
        r#"{"version":2,"command":"check","status":"complete","generated_at":0,"summary":{"total":2,"errors":1,"warnings":1,"actionable":2,"heuristic":0},"findings":[{"rule_id":"SEO001","message":"missing <title>","path":"crawl/docs/index.html","line":1,"column":1,"severity":"error","scope":"page"},{"rule_id":"SEO002","message":"missing meta description","path":"crawl/docs/index.html","line":1,"column":1,"severity":"warning","scope":"page"}]}"#,
    );
    let output = Command::new(bin())
        .arg("bing-ai")
        .arg("opportunities")
        .arg(temp_dir.path().join("bing-ai.csv"))
        .arg("--audit")
        .arg(temp_dir.path().join("audit.json"))
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();
    assert!(output.status.success());
    let payload = parse_json(&output.stdout);
    assert_eq!(payload["command"], "bing-ai opportunities");
    assert!(
        payload["result"]["opportunities"][0]["score"]
            .as_u64()
            .unwrap()
            > 0
    );
}

#[test]
fn bing_ai_trend_import_and_show_work_end_to_end() {
    let temp_dir = tempfile::tempdir().unwrap();
    write(
        &temp_dir.path().join("audit.json"),
        r#"{"version":2,"command":"check","status":"complete","generated_at":0,"summary":{"total":1,"errors":0,"warnings":1,"actionable":1,"heuristic":0},"findings":[{"rule_id":"SEO002","message":"missing meta description","path":"crawl/docs/index.html","line":1,"column":1,"severity":"warning","scope":"page"}]}"#,
    );
    write(
        &temp_dir.path().join("bing-one.csv"),
        "url,query,citations\nhttps://example.com/docs,What is docs,2\n",
    );
    write(
        &temp_dir.path().join("bing-two.csv"),
        "url,query,citations\nhttps://example.com/docs,What is docs,5\n",
    );

    let first = Command::new(bin())
        .arg("bing-ai")
        .arg("trend")
        .arg("import")
        .arg(temp_dir.path().join("bing-one.csv"))
        .arg("--root")
        .arg(temp_dir.path())
        .arg("--audit")
        .arg(temp_dir.path().join("audit.json"))
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();
    assert!(first.status.success());

    let second = Command::new(bin())
        .arg("bing-ai")
        .arg("trend")
        .arg("import")
        .arg(temp_dir.path().join("bing-two.csv"))
        .arg("--root")
        .arg(temp_dir.path())
        .arg("--audit")
        .arg(temp_dir.path().join("audit.json"))
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();
    assert!(second.status.success());

    let show = Command::new(bin())
        .arg("bing-ai")
        .arg("trend")
        .arg("show")
        .arg(temp_dir.path())
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();
    assert!(show.status.success());
    let payload = parse_json(&show.stdout);
    assert_eq!(payload["command"], "bing-ai trend show");
    assert_eq!(payload["result"]["snapshots"], 2);
    assert_eq!(payload["result"]["increased"][0]["route"], "docs");
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
    assert!(
        payload["result"]["search_console_export_path"]
            .as_str()
            .unwrap()
            .ends_with("publish-hook-search-console.csv")
    );
    assert!(payload["warnings"].is_array());
}

#[test]
fn indexnow_ledger_and_retry_json_contracts_work() {
    let temp_dir = tempfile::tempdir().unwrap();
    write(&temp_dir.path().join("abc123.txt"), "abc123");
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let handle = thread::spawn(move || {
        for status in ["429 Too Many Requests", "200 OK"] {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buffer = [0_u8; 4096];
            let _ = stream.read(&mut buffer).unwrap();
            let response =
                format!("HTTP/1.1 {status}\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok");
            stream.write_all(response.as_bytes()).unwrap();
            stream.flush().unwrap();
        }
    });

    let submit = Command::new(bin())
        .arg("indexnow")
        .arg("submit")
        .arg(format!("http://{}", address))
        .arg("https://example.com")
        .arg("abc123")
        .arg("--path")
        .arg(temp_dir.path())
        .arg("https://example.com/a")
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();
    assert!(!submit.status.success());

    let ledger = Command::new(bin())
        .arg("indexnow")
        .arg("ledger")
        .arg(temp_dir.path())
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();
    assert!(ledger.status.success());
    let ledger_payload = parse_json(&ledger.stdout);
    assert_eq!(ledger_payload["result"]["entries"][0]["retryable"], true);

    let retry = Command::new(bin())
        .arg("indexnow")
        .arg("retry")
        .arg("--path")
        .arg(temp_dir.path())
        .arg("abc123")
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();
    assert!(retry.status.success());
    let retry_payload = parse_json(&retry.stdout);
    assert_eq!(retry_payload["result"]["succeeded"], 1);
    handle.join().unwrap();
}

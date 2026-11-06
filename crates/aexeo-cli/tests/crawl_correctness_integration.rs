//! Binary-level crawl behaviour against a local site that records every request.
mod support;

use serde_json::Value;
use std::collections::BTreeMap;
use std::io::{ErrorKind, Read, Write};
use std::net::TcpListener;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use support::{bin, parse_json};

/// Status, content type and body of one path.
type Route = (&'static str, &'static str, String);

fn html(body: &str) -> Route {
    (
        "200 OK",
        "text/html",
        format!("<html><body>{body}</body></html>"),
    )
}

struct Site {
    base_url: String,
    /// `(path, user agent)` of every request, in arrival order.
    requests: Arc<Mutex<Vec<(String, String)>>>,
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl Site {
    fn start(routes: Vec<(&'static str, Route)>) -> Self {
        let routes = routes.into_iter().collect::<BTreeMap<_, _>>();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (log, stopping) = (requests.clone(), stop.clone());
        let handle = thread::spawn(move || {
            while !stopping.load(Ordering::Relaxed) {
                let mut stream = match listener.accept() {
                    Ok((stream, _)) => stream,
                    Err(error) if error.kind() == ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                    Err(error) => panic!("accept failed: {error}"),
                };
                stream.set_nonblocking(false).unwrap();
                let mut request = Vec::new();
                let mut buffer = [0_u8; 4096];
                while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                    let size = stream.read(&mut buffer).unwrap_or(0);
                    if size == 0 {
                        break;
                    }
                    request.extend_from_slice(&buffer[..size]);
                }
                let request = String::from_utf8_lossy(&request).to_string();
                let path = request.split_whitespace().nth(1).unwrap_or("/").to_string();
                let agent = request
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("user-agent")
                            .then(|| value.trim().to_string())
                    })
                    .unwrap_or_default();
                let not_found = ("404 Not Found", "text/plain", "missing".to_string());
                let (status, content_type, body) = routes.get(path.as_str()).unwrap_or(&not_found);
                log.lock().unwrap().push((path, agent));
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes());
            }
        });
        Self {
            base_url,
            requests,
            stop,
            handle: Some(handle),
        }
    }

    fn requested(&self, path: &str) -> bool {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .any(|(seen, _)| seen == path)
    }
}

impl Drop for Site {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn crawl(site: &Site, extra: &[&str]) -> (Output, Value) {
    let temp = tempfile::tempdir().unwrap();
    let output = Command::new(bin())
        .current_dir(temp.path())
        .args([
            "crawl",
            &site.base_url,
            "--engine",
            "http",
            "--progress",
            "off",
        ])
        .args(["--format", "json"])
        .args(extra)
        .output()
        .unwrap();
    let payload = parse_json(&output.stdout);
    (output, payload)
}

fn rule_messages(payload: &Value, rule_id: &str) -> Vec<String> {
    payload["artifact"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|finding| finding["rule_id"] == rule_id)
        .map(|finding| finding["message"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn crawl_follows_links_the_way_a_browser_resolves_them() {
    let site = Site::start(vec![
        (
            "/",
            html(
                r#"<a href="docs">docs</a> <a href=/unquoted>x</a> <a data-href="/wrong" href="/right">y</a> <a href="/list?page=2">list</a>"#,
            ),
        ),
        (
            "/docs",
            html(r#"<a href="sub/page">sub</a> <a href="../up">up</a>"#),
        ),
        ("/sub/page", html("")),
        ("/up", html("")),
        ("/unquoted", html("")),
        ("/right", html("")),
        ("/list", html("")),
    ]);
    let (_, payload) = crawl(&site, &[]);
    for path in ["/docs", "/sub/page", "/unquoted", "/right", "/list"] {
        assert!(site.requested(path), "{path} was never requested");
    }
    assert!(!site.requested("/wrong"), "data-href must not be followed");
    assert!(
        !site
            .requests
            .lock()
            .unwrap()
            .iter()
            .any(|(path, _)| path.contains('?')),
        "query variants are documented as collapsing onto their path"
    );
    assert!(rule_messages(&payload, "LNK002").is_empty(), "{payload:#}");
}

#[test]
fn crawl_honours_base_href() {
    let site = Site::start(vec![
        ("/", html(r#"<base href="/en/"><a href="intro">Intro</a>"#)),
        ("/en/intro", html("")),
    ]);
    crawl(&site, &[]);
    assert!(site.requested("/en/intro"));
    assert!(!site.requested("/intro"));
}

#[test]
fn relative_links_count_for_broken_link_and_inbound_rules() {
    let site = Site::start(vec![
        ("/", html(r#"<a href="reached">a</a> <a href="gone">b</a>"#)),
        ("/reached", html("")),
    ]);
    let (_, payload) = crawl(&site, &[]);
    assert_eq!(
        rule_messages(&payload, "LNK001"),
        ["broken internal link: /gone"],
        "{payload:#}"
    );
    assert!(rule_messages(&payload, "LNK002").is_empty(), "{payload:#}");
}

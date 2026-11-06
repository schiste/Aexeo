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
use support::{bin, parse_json, write};

/// Status, content type and body of one path.
type Route = (&'static str, &'static str, String);

fn html(body: &str) -> Route {
    (
        "200 OK",
        "text/html",
        format!("<html><body>{body}</body></html>"),
    )
}

/// Status of a route whose connection is closed without any response.
const CLOSE: &str = "close";

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
                if routes
                    .get(path.as_str())
                    .is_some_and(|route| route.0 == CLOSE)
                {
                    log.lock().unwrap().push((path, agent));
                    continue;
                }
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

#[test]
fn non_2xx_responses_are_recorded_but_are_not_pages() {
    for (status, code) in [("404 Not Found", 404), ("500 Internal Server Error", 500)] {
        let site = Site::start(vec![
            ("/", html(r#"<a href="/bad">bad</a>"#)),
            (
                "/bad",
                (status, "text/html", "<title>Error</title>".to_string()),
            ),
        ]);
        let temp = tempfile::tempdir().unwrap();
        let graph = temp.path().join("graph.json");
        let (_, payload) = crawl(&site, &["--link-graph-output", graph.to_str().unwrap()]);
        let artifact = &payload["artifact"];

        let routes = artifact["site"]["pages"].as_array().unwrap();
        assert!(
            routes.iter().all(|page| page["route"] != "bad"),
            "{status}: {payload:#}"
        );
        let failed = artifact["site"]["crawl_meta"]["failed_responses"]
            .as_array()
            .unwrap();
        assert_eq!(failed.len(), 1, "{status}");
        assert_eq!(failed[0]["status_code"], code);
        assert_eq!(
            rule_messages(&payload, "LNK001"),
            ["broken internal link: /bad"]
        );
        assert!(rule_messages(&payload, "CRW001")[0].contains(&format!("HTTP {code}")));
        assert!(
            rule_messages(&payload, "SEO001")
                .iter()
                .all(|m| !m.contains("/bad"))
        );

        assert_eq!(artifact["status"], "partial", "{status}");
        assert_eq!(artifact["crawl"]["fetch_failures"], 1);
        assert!(
            artifact["truncation_reason"]
                .as_str()
                .unwrap()
                .contains("non-2xx")
        );

        let report: Value = serde_json::from_slice(&std::fs::read(&graph).unwrap()).unwrap();
        assert!(
            report["issues"]
                .to_string()
                .contains("non_success_response"),
            "{report:#}"
        );
    }
}

#[test]
fn a_crawl_with_only_successful_fetches_stays_complete() {
    let site = Site::start(vec![
        ("/", html(r#"<a href="/ok">ok</a>"#)),
        ("/ok", html("")),
    ]);
    let (_, payload) = crawl(&site, &[]);
    assert_eq!(payload["artifact"]["status"], "complete", "{payload:#}");
    assert!(payload["artifact"]["truncation_reason"].is_null());
}

#[test]
fn a_failed_home_page_yields_a_partial_audit_rather_than_an_error() {
    let site = Site::start(vec![(
        "/",
        (
            "503 Service Unavailable",
            "text/html",
            "<p>down</p>".to_string(),
        ),
    )]);
    let (output, payload) = crawl(&site, &[]);
    assert_eq!(payload["artifact"]["status"], "partial", "{payload:#}");
    assert_eq!(
        payload["artifact"]["site"]["pages"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
    assert_eq!(
        output.status.code(),
        Some(1),
        "CRW001 is a blocking finding"
    );
}

#[test]
fn failed_fetches_make_content_mode_exit_2_like_any_partial_crawl() {
    let site = Site::start(vec![("/", html(r#"<a href="/bad">bad</a>"#))]);
    let temp = tempfile::tempdir().unwrap();
    let output = Command::new(bin())
        .current_dir(temp.path())
        .args([
            "content",
            "--crawl-url",
            &site.base_url,
            "--preset",
            "seo-website",
        ])
        .args(["--crawl-engine", "http", "--format", "json"])
        .output()
        .unwrap();
    let report = parse_json(&output.stdout);
    assert_eq!(report["crawl_status"], "partial", "{report:#}");
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn an_unreachable_optional_artifact_degrades_to_a_warning() {
    let site = Site::start(vec![
        ("/", html("")),
        ("/llms.txt", (CLOSE, "", String::new())),
    ]);
    let (output, payload) = crawl(&site, &[]);
    assert!(site.requested("/llms.txt"));
    assert_ne!(
        output.status.code(),
        Some(70),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(payload["artifact"]["status"], "complete", "{payload:#}");
    let warnings = rule_messages(&payload, "CRW002");
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("llms.txt"));
    assert!(
        !payload["artifact"]["site"]["pages"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

fn robots(rules: &str) -> Route {
    ("200 OK", "text/plain", rules.to_string())
}

#[test]
fn robots_txt_is_fetched_first_and_disallowed_urls_are_recorded_not_requested() {
    let site = Site::start(vec![
        (
            "/",
            html(r#"<a href="/allowed">a</a> <a href="/private/page">b</a>"#),
        ),
        ("/robots.txt", robots("User-agent: *\nDisallow: /private\n")),
        ("/allowed", html("")),
        ("/private/page", html("secret")),
        (
            "/sitemap.xml",
            (
                "200 OK",
                "application/xml",
                "<urlset><url><loc>http://example.test/private/from-sitemap</loc></url></urlset>"
                    .to_string(),
            ),
        ),
        ("/private/from-sitemap", html("secret")),
    ]);
    let (_, payload) = crawl(&site, &["--seed", "/private/seeded"]);
    assert_eq!(site.requests.lock().unwrap()[0].0, "/robots.txt");
    assert!(site.requested("/allowed"));
    for path in ["/private/page", "/private/from-sitemap", "/private/seeded"] {
        assert!(!site.requested(path), "{path} is disallowed");
    }
    let artifact = &payload["artifact"];
    let skipped = artifact["site"]["crawl_meta"]["robots_skipped_urls"].to_string();
    for path in ["/private/page", "/private/from-sitemap", "/private/seeded"] {
        assert!(skipped.contains(path), "{path} not recorded in {skipped}");
    }
    assert_eq!(artifact["crawl"]["skipped_by_robots"], 3);
    assert_eq!(artifact["status"], "complete", "a skip is not a failure");
    assert!(
        rule_messages(&payload, "LNK001").is_empty(),
        "a route we chose not to request is not broken: {payload:#}"
    );
    assert!(
        artifact["site"]["robots_text"]
            .as_str()
            .unwrap()
            .contains("Disallow")
    );
}

#[test]
fn ignore_robots_requests_disallowed_urls_and_still_audits_robots_txt() {
    let site = Site::start(vec![
        ("/", html(r#"<a href="/private">b</a>"#)),
        ("/robots.txt", robots("User-agent: *\nDisallow: /\n")),
        ("/private", html("")),
    ]);
    let (_, payload) = crawl(&site, &["--ignore-robots"]);
    assert!(site.requested("/private"));
    assert_eq!(payload["artifact"]["crawl"]["skipped_by_robots"], 0);
    assert!(payload["artifact"]["site"]["robots_text"].is_string());
}

#[test]
fn ignore_robots_can_be_set_in_config() {
    let site = Site::start(vec![
        ("/", html("")),
        ("/robots.txt", robots("User-agent: *\nDisallow: /\n")),
    ]);
    let temp = tempfile::tempdir().unwrap();
    let config = temp.path().join("aexeo.toml");
    write(&config, "version = 1\n\n[runtime]\nignore_robots = true\n");
    let (_, payload) = crawl(&site, &["--config", config.to_str().unwrap()]);
    assert!(site.requested("/"), "{payload:#}");
}

#[test]
fn a_site_disallowing_everything_is_only_asked_for_robots_txt() {
    let site = Site::start(vec![
        ("/", html("")),
        ("/robots.txt", robots("User-agent: *\nDisallow: /\n")),
        ("/llms.txt", robots("# site\n")),
    ]);
    let (_, payload) = crawl(&site, &[]);
    let requests = site.requests.lock().unwrap().clone();
    assert_eq!(
        requests
            .iter()
            .map(|(path, _)| path.as_str())
            .collect::<Vec<_>>(),
        ["/robots.txt"],
        "{payload:#}"
    );
    assert_eq!(payload["artifact"]["status"], "partial");
    assert!(
        payload["artifact"]["truncation_reason"]
            .as_str()
            .unwrap()
            .contains("robots.txt")
    );
}

#[test]
fn an_unreachable_robots_txt_means_the_site_is_not_crawled() {
    let site = Site::start(vec![
        ("/", html("")),
        (
            "/robots.txt",
            ("503 Service Unavailable", "text/plain", String::new()),
        ),
    ]);
    let (_, payload) = crawl(&site, &[]);
    assert!(!site.requested("/"));
    assert_eq!(payload["artifact"]["status"], "partial");
    assert!(rule_messages(&payload, "CRW002")[0].contains("HTTP 503"));
}

#[test]
fn a_missing_robots_txt_allows_everything() {
    let site = Site::start(vec![("/", html(""))]);
    let (_, payload) = crawl(&site, &[]);
    assert!(site.requested("/"));
    assert_eq!(payload["artifact"]["status"], "complete", "{payload:#}");
}

#[test]
fn rules_for_the_configured_user_agent_apply_and_every_request_identifies_itself() {
    let rules = "User-agent: *\nDisallow: /star\n\nUser-agent: CustomBot\nDisallow: /custom\n";
    let routes = || {
        vec![
            (
                "/",
                html(r#"<a href="/star">a</a> <a href="/custom">b</a>"#),
            ),
            ("/robots.txt", robots(rules)),
            ("/star", html("")),
            ("/custom", html("")),
        ]
    };

    let default = Site::start(routes());
    crawl(&default, &[]);
    assert!(!default.requested("/star"));
    assert!(default.requested("/custom"));
    let expected = format!("aexeo/{}", env!("CARGO_PKG_VERSION"));
    let requests = default.requests.lock().unwrap().clone();
    assert!(
        requests.iter().all(|(_, agent)| agent == &expected),
        "{requests:?}"
    );

    let custom = Site::start(routes());
    let temp = tempfile::tempdir().unwrap();
    let config = temp.path().join("aexeo.toml");
    write(
        &config,
        "version = 1\n\n[runtime]\nheaders = { user-agent = \"CustomBot/2.0\" }\n",
    );
    crawl(&custom, &["--config", config.to_str().unwrap()]);
    assert!(custom.requested("/star"));
    assert!(!custom.requested("/custom"));
    let requests = custom.requests.lock().unwrap().clone();
    assert!(
        requests.iter().all(|(_, agent)| agent == "CustomBot/2.0"),
        "{requests:?}"
    );
}

#[test]
fn crawl_delay_spaces_out_batches() {
    let site = Site::start(vec![
        ("/", html(r#"<a href="/a">a</a> <a href="/b">b</a>"#)),
        ("/a", html("")),
        ("/b", html("")),
    ]);
    let temp = tempfile::tempdir().unwrap();
    let config = temp.path().join("aexeo.toml");
    write(
        &config,
        "version = 1\nmax_workers = 1\n\n[runtime]\ncrawl_delay_ms = 300\n",
    );
    let started_at = std::time::Instant::now();
    crawl(&site, &["--config", config.to_str().unwrap()]);
    assert!(started_at.elapsed() >= Duration::from_millis(600));
}

/// Every byte the crawl writes: stdout, stderr and the artifacts it leaves behind.
fn all_crawl_output(url: &str) -> String {
    let temp = tempfile::tempdir().unwrap();
    let output = Command::new(bin())
        .current_dir(temp.path())
        .args(["crawl", url, "--engine", "http", "--format", "json"])
        .args(["--progress", "json"])
        .output()
        .unwrap();
    let mut text = String::from_utf8_lossy(&output.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    let mut dirs = vec![temp.path().to_path_buf()];
    while let Some(dir) = dirs.pop() {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                dirs.push(path);
            } else {
                text.push_str(&String::from_utf8_lossy(&std::fs::read(path).unwrap()));
            }
        }
    }
    text
}

#[test]
fn url_credentials_never_reach_findings_or_artifacts() {
    // Exercises every recorded URL: a page, a redirect-free non-2xx response, a
    // transport failure (CRW001), an optional artifact failure (CRW002) and a
    // robots.txt skip.
    let routes = || {
        vec![
            (
                "/",
                html(
                    r#"<a href="/ok">a</a> <a href="/bad">b</a> <a href="/dropped">c</a> <a href="/private">d</a>"#,
                ),
            ),
            ("/ok", html("")),
            (
                "/bad",
                ("500 Internal Server Error", "text/html", String::new()),
            ),
            ("/dropped", (CLOSE, "", String::new())),
            ("/llms.txt", (CLOSE, "", String::new())),
            ("/robots.txt", robots("User-agent: *\nDisallow: /private\n")),
        ]
    };
    let site = Site::start(routes());
    let authority = site.base_url.trim_start_matches("http://");
    let text = all_crawl_output(&format!("http://user:s3cret-password@{authority}/"));
    assert!(
        text.contains("artifact"),
        "the crawl produced no output: {text}"
    );
    assert!(site.requested("/dropped") && site.requested("/llms.txt"));
    assert!(!text.contains("s3cret-password"), "password leaked: {text}");

    let site = Site::start(routes());
    let text = all_crawl_output(&format!("{}/?token=s3cret-token", site.base_url));
    assert!(!text.contains("s3cret-token"), "query value leaked: {text}");

    // A connection that fails outright, including the robots.txt warning.
    let closed = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap();
    let text = all_crawl_output(&format!("http://user:s3cret-password@{closed}/"));
    assert!(!text.contains("s3cret-password"), "password leaked: {text}");
}

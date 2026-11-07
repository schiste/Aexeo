//! Binary-level crawl behaviour against a local site that records every request.
mod support;

use serde_json::Value;
use std::collections::BTreeMap;
use std::io::{ErrorKind, Read, Write};
use std::net::TcpListener;
use std::path::Path;
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
    /// Raw header lines, each ending in CRLF, added to every response.
    headers: Arc<Mutex<String>>,
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
        let headers = Arc::new(Mutex::new(String::new()));
        let (log, stopping, extra) = (requests.clone(), stop.clone(), headers.clone());
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
                    "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\n{}Connection: close\r\n\r\n{body}",
                    body.len(),
                    extra.lock().unwrap()
                );
                let _ = stream.write_all(response.as_bytes());
            }
        });
        Self {
            base_url,
            requests,
            headers,
            stop,
            handle: Some(handle),
        }
    }

    /// Answer every later request with `name: value` as well.
    fn serve_header(&self, name: &str, value: &str) {
        *self.headers.lock().unwrap() = format!("{name}: {value}\r\n");
    }

    fn request_count(&self, path: &str) -> usize {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|(seen, _)| seen == path)
            .count()
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

fn site_with_linked_response(status: &'static str) -> Site {
    Site::start(vec![
        ("/", html(r#"<a href="/bad">bad</a>"#)),
        (
            "/bad",
            (status, "text/html", "<title>Error</title>".to_string()),
        ),
    ])
}

/// A config with only the link rules on, so the exit code answers for the crawl
/// alone rather than for the metadata the bare fixture pages lack.
fn link_rules_only(dir: &Path) -> std::path::PathBuf {
    let path = dir.join("aexeo.toml");
    let mut text = String::from("version = 1\n");
    for group in [
        "html",
        "sitemap",
        "robots",
        "social",
        "schema",
        "llm",
        "surfaces",
        "content",
        "editorial",
        "structure",
        "accessibility",
        "agent_discovery",
        "well_known",
        "headers",
    ] {
        text.push_str(&format!("[rules.{group}]\nenabled = false\n"));
    }
    write(&path, &text);
    path
}

fn assert_recorded_but_not_a_page(payload: &Value, code: u16) {
    let artifact = &payload["artifact"];
    let pages = artifact["site"]["pages"].as_array().unwrap();
    assert!(
        pages.iter().all(|page| page["route"] != "bad"),
        "{payload:#}"
    );
    let failed = artifact["site"]["crawl_meta"]["failed_responses"]
        .as_array()
        .unwrap();
    assert_eq!(failed.len(), 1, "{payload:#}");
    assert_eq!(failed[0]["status_code"], code);
    assert!(rule_messages(payload, "CRW001")[0].contains(&format!("HTTP {code}")));
}

#[test]
fn a_broken_link_is_a_finding_in_a_complete_audit() {
    for (status, code) in [("404 Not Found", 404), ("410 Gone", 410)] {
        let site = site_with_linked_response(status);
        let temp = tempfile::tempdir().unwrap();
        let graph = temp.path().join("graph.json");
        let config = link_rules_only(temp.path());
        let (output, payload) = crawl(
            &site,
            &[
                "--config",
                config.to_str().unwrap(),
                "--link-graph-output",
                graph.to_str().unwrap(),
            ],
        );

        assert_recorded_but_not_a_page(&payload, code);
        assert_eq!(payload["artifact"]["status"], "complete", "{payload:#}");
        assert!(payload["artifact"]["truncation_reason"].is_null());
        assert_eq!(output.status.code(), Some(1));
        assert_eq!(
            rule_messages(&payload, "LNK001"),
            ["broken internal link: /bad"]
        );
        assert_eq!(payload["artifact"]["crawl"]["fetch_failures"], 0);
        assert_eq!(payload["artifact"]["crawl"]["observed_error_responses"], 1);
        assert_eq!(
            site.request_count("/bad"),
            1,
            "a definite answer is not retried"
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
fn a_followed_redirect_chain_reaches_the_artifact_and_the_link_report() {
    // The status line carries the Location header: the fixture server has no
    // other way to add one.
    let site = Site::start(vec![
        ("/", html(r#"<a href="/old">old</a>"#)),
        (
            "/old",
            (
                "301 Moved Permanently\r\nLocation: /middle",
                "text/html",
                String::new(),
            ),
        ),
        (
            "/middle",
            ("302 Found\r\nLocation: /new", "text/html", String::new()),
        ),
        ("/new", html("new")),
    ]);
    let temp = tempfile::tempdir().unwrap();
    let graph = temp.path().join("graph.json");
    let config = link_rules_only(temp.path());
    let (_, payload) = crawl(
        &site,
        &[
            "--config",
            config.to_str().unwrap(),
            "--link-graph-output",
            graph.to_str().unwrap(),
        ],
    );
    let base = &site.base_url;
    let expected_chain = serde_json::json!([
        {"url": format!("{base}/old"), "status_code": 301},
        {"url": format!("{base}/middle"), "status_code": 302},
    ]);

    let pages = payload["artifact"]["site"]["pages"].as_array().unwrap();
    let redirected = pages
        .iter()
        .find(|page| page["crawl_response"]["requested_url"] == format!("{base}/old"))
        .unwrap_or_else(|| panic!("{payload:#}"));
    assert_eq!(
        redirected["crawl_response"]["redirect_chain"], expected_chain,
        "{payload:#}"
    );
    let home = pages
        .iter()
        .find(|page| page["crawl_response"]["requested_url"] == format!("{base}/"))
        .unwrap();
    assert!(home["crawl_response"].get("redirect_chain").is_none());

    let report: Value = serde_json::from_slice(&std::fs::read(&graph).unwrap()).unwrap();
    let page = report["pages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|page| page["url"] == format!("{base}/new"))
        .unwrap_or_else(|| panic!("{report:#}"));
    assert_eq!(page["redirect_chain"], expected_chain, "{report:#}");
    assert_eq!(page["distinct_source_pages"], 0);
    assert_eq!(page["redirected_source_pages"], 1);
    assert!(
        !report["issues"]
            .to_string()
            .contains("audit_redirect_chain_unavailable"),
        "{report:#}"
    );
}

#[test]
fn a_server_failure_leaves_the_audit_partial_after_retries() {
    for (status, code) in [
        ("503 Service Unavailable", 503),
        ("500 Internal Server Error", 500),
        ("429 Too Many Requests", 429),
    ] {
        let site = site_with_linked_response(status);
        let temp = tempfile::tempdir().unwrap();
        let config = link_rules_only(temp.path());
        let (output, payload) = crawl(
            &site,
            &["--retry-budget", "1", "--config", config.to_str().unwrap()],
        );

        assert_recorded_but_not_a_page(&payload, code);
        assert_eq!(payload["artifact"]["status"], "partial", "{status}");
        assert_eq!(payload["artifact"]["crawl"]["fetch_failures"], 1);
        assert!(
            payload["artifact"]["truncation_reason"]
                .as_str()
                .unwrap()
                .contains("not audited")
        );
        assert_eq!(output.status.code(), Some(1));
        assert_eq!(site.request_count("/bad"), 2, "{status} is retried");
    }
}

#[test]
fn denied_access_limits_coverage_without_claiming_the_link_is_broken() {
    for (status, code) in [("401 Unauthorized", 401), ("403 Forbidden", 403)] {
        let site = site_with_linked_response(status);
        let temp = tempfile::tempdir().unwrap();
        let config = link_rules_only(temp.path());
        let (output, payload) = crawl(&site, &["--config", config.to_str().unwrap()]);

        assert_recorded_but_not_a_page(&payload, code);
        assert_eq!(payload["artifact"]["status"], "partial", "{status}");
        assert_eq!(payload["artifact"]["crawl"]["access_denied"], 1);
        assert_eq!(payload["artifact"]["crawl"]["fetch_failures"], 0);
        assert!(rule_messages(&payload, "LNK001").is_empty(), "{payload:#}");
        assert!(
            payload["artifact"]["truncation_reason"]
                .as_str()
                .unwrap()
                .contains("denied access")
        );
        assert_eq!(output.status.code(), Some(0), "{payload:#}");
    }
}

#[test]
fn a_missing_home_page_means_nothing_was_audited() {
    let site = Site::start(vec![]);
    let (output, payload) = crawl(&site, &[]);
    assert_eq!(payload["artifact"]["status"], "partial", "{payload:#}");
    assert!(
        payload["artifact"]["truncation_reason"]
            .as_str()
            .unwrap()
            .contains("nothing was audited")
    );
    assert_eq!(output.status.code(), Some(1));
}

#[test]
fn the_link_only_config_is_clean_without_a_bad_response() {
    let site = Site::start(vec![
        ("/", html(r#"<a href="/ok">ok</a>"#)),
        ("/ok", html("")),
    ]);
    let temp = tempfile::tempdir().unwrap();
    let config = link_rules_only(temp.path());
    let (output, payload) = crawl(&site, &["--config", config.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(0), "{payload:#}");
    assert_eq!(payload["artifact"]["status"], "complete");
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
fn content_mode_exits_2_only_for_an_unfinished_audit() {
    for (status, expected, code) in [
        ("404 Not Found", "complete", 1),
        ("503 Service Unavailable", "partial", 2),
    ] {
        let site = site_with_linked_response(status);
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
        assert_eq!(report["crawl_status"], expected, "{report:#}");
        assert_eq!(output.status.code(), Some(code), "{status}");
    }
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
            .contains("--ignore-robots")
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
    let warning = &rule_messages(&payload, "CRW002")[0];
    assert!(
        warning.contains("HTTP 503") && warning.contains("--ignore-robots"),
        "{warning}"
    );
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

/// Crawl with `--max-pages 1`, leaving a checkpoint with `/next` still queued.
fn interrupted_crawl(site: &Site, checkpoint: &Path) {
    let (_, payload) = crawl(
        site,
        &[
            "--max-pages",
            "1",
            "--checkpoint",
            checkpoint.to_str().unwrap(),
            "--checkpoint-every",
            "1",
        ],
    );
    assert_eq!(payload["artifact"]["status"], "partial");
    assert!(checkpoint.exists());
}

fn resumable_site() -> Site {
    Site::start(vec![
        ("/", html(r#"<a href="/next">next</a>"#)),
        ("/next", html("")),
    ])
}

fn resume(url: &str, checkpoint: &Path) -> Output {
    Command::new(bin())
        .args(["crawl", url, "--engine", "http", "--progress", "off"])
        .args(["--format", "json", "--resume", checkpoint.to_str().unwrap()])
        .current_dir(checkpoint.parent().unwrap())
        .output()
        .unwrap()
}

#[test]
fn resume_refuses_a_checkpoint_for_another_site_without_contacting_it() {
    let (original, other) = (resumable_site(), resumable_site());
    let temp = tempfile::tempdir().unwrap();
    let checkpoint = temp.path().join("checkpoint.json");
    interrupted_crawl(&original, &checkpoint);
    let requests_before = original.requests.lock().unwrap().len();

    let output = resume(&other.base_url, &checkpoint);
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_ne!(output.status.code(), Some(0), "{text}");
    assert!(text.contains("checkpoint belongs to"), "{text}");
    assert_eq!(original.requests.lock().unwrap().len(), requests_before);
    assert!(other.requests.lock().unwrap().is_empty());
}

#[test]
fn resume_refuses_a_foreign_planner_host_version_or_engine() {
    let site = resumable_site();
    let temp = tempfile::tempdir().unwrap();
    let checkpoint = temp.path().join("checkpoint.json");
    interrupted_crawl(&site, &checkpoint);
    let original: Value = serde_json::from_slice(&std::fs::read(&checkpoint).unwrap()).unwrap();
    for (pointer, value, message) in [
        ("/version", serde_json::json!(99), "version 99"),
        (
            "/engine",
            serde_json::json!("playwright"),
            "engine 'playwright'",
        ),
        (
            "/planner/base_host",
            serde_json::json!("evil.test"),
            "checkpoint belongs to",
        ),
    ] {
        let mut tampered = original.clone();
        *tampered.pointer_mut(pointer).unwrap() = value;
        std::fs::write(&checkpoint, tampered.to_string()).unwrap();
        let output = resume(&site.base_url, &checkpoint);
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert_ne!(output.status.code(), Some(0), "{pointer}: {text}");
        assert!(text.contains(message), "{pointer}: {text}");
    }
}

#[test]
fn resume_crawls_the_route_that_did_not_fit_before() {
    let site = resumable_site();
    let temp = tempfile::tempdir().unwrap();
    let checkpoint = temp.path().join("checkpoint.json");
    interrupted_crawl(&site, &checkpoint);
    assert!(!site.requested("/next"));

    let output = resume(&site.base_url, &checkpoint);
    assert!(site.requested("/next"));
    assert_eq!(parse_json(&output.stdout)["artifact"]["status"], "complete");
}

#[test]
fn checkpoints_follow_a_page_threshold_whatever_the_batch_size() {
    // Four workers visit 1, 5, 9, 13, 17, 21, 25 pages. Multiples of 5 would hit
    // only 5 and 25; a threshold of 5 pages since the last checkpoint hits 5, 13, 21.
    let paths = (0..24)
        .map(|index| &*Box::leak(format!("/p{index}").into_boxed_str()))
        .collect::<Vec<&'static str>>();
    let links = paths
        .iter()
        .map(|path| format!(r#"<a href="{path}">x</a>"#))
        .collect::<String>();
    let mut routes = vec![("/", html(&links))];
    routes.extend(paths.iter().map(|path| (*path, html(""))));
    let site = Site::start(routes);
    let temp = tempfile::tempdir().unwrap();
    let checkpoint = temp.path().join("checkpoint.json");
    let (_, payload) = crawl(
        &site,
        &[
            "--checkpoint",
            checkpoint.to_str().unwrap(),
            "--checkpoint-every",
            "5",
        ],
    );
    assert_eq!(payload["artifact"]["crawl"]["visited_pages"], 25);
    assert_eq!(payload["artifact"]["crawl"]["checkpoints_written"], 3);
    let leftovers = std::fs::read_dir(temp.path()).unwrap().count();
    assert_eq!(
        leftovers, 0,
        "a finished crawl leaves no checkpoint or temp file"
    );
}

#[test]
fn resume_never_sends_requests_to_a_tampered_normalized_base() {
    let site = resumable_site();
    // Same host as the site, on another port: same-site, so only ignoring the
    // persisted value keeps the crawl away from it.
    let attacker = resumable_site();
    let temp = tempfile::tempdir().unwrap();
    let checkpoint = temp.path().join("checkpoint.json");
    interrupted_crawl(&site, &checkpoint);
    let original: Value = serde_json::from_slice(&std::fs::read(&checkpoint).unwrap()).unwrap();

    let mut tampered = original.clone();
    tampered["planner"]["normalized_base"] = format!("{}/", attacker.base_url).into();
    std::fs::write(&checkpoint, tampered.to_string()).unwrap();
    let output = resume(&site.base_url, &checkpoint);
    assert!(
        attacker.requests.lock().unwrap().is_empty(),
        "credentials would have gone to another origin"
    );
    assert!(
        site.requested("/next"),
        "the crawl continues on the given URL"
    );
    assert_eq!(parse_json(&output.stdout)["artifact"]["status"], "complete");

    // Another host is refused outright, with or without userinfo.
    for foreign in ["http://evil.test/", "http://user:pw@evil.test/"] {
        let mut tampered = original.clone();
        tampered["planner"]["normalized_base"] = foreign.into();
        std::fs::write(&checkpoint, tampered.to_string()).unwrap();
        let output = resume(&site.base_url, &checkpoint);
        let text = String::from_utf8_lossy(&output.stdout).to_string();
        assert_ne!(output.status.code(), Some(0), "{text}");
        assert!(text.contains("checkpoint belongs to"), "{text}");
        assert!(!text.contains("pw@"), "{text}");
    }
}

#[test]
fn checkpoints_do_not_store_url_credentials() {
    let site = resumable_site();
    let authority = site.base_url.trim_start_matches("http://").to_string();
    let temp = tempfile::tempdir().unwrap();
    let checkpoint = temp.path().join("checkpoint.json");
    let output = Command::new(bin())
        .current_dir(temp.path())
        .args(["crawl", &format!("http://user:s3cret@{authority}/")])
        .args(["--engine", "http", "--progress", "off", "--format", "json"])
        .args([
            "--max-pages",
            "1",
            "--checkpoint-every",
            "1",
            "--checkpoint",
        ])
        .arg(&checkpoint)
        .output()
        .unwrap();
    assert_eq!(parse_json(&output.stdout)["artifact"]["status"], "partial");
    let saved = std::fs::read_to_string(&checkpoint).unwrap();
    assert!(!saved.contains("s3cret"), "{saved}");

    let resumed = resume(&format!("http://user:s3cret@{authority}/"), &checkpoint);
    assert_eq!(
        parse_json(&resumed.stdout)["artifact"]["status"],
        "complete"
    );
}

/// A link inventory context for `site`, usable both as `--seed-file` and as
/// `link-graph --context`: `paths` are published pages on the site, `extra`
/// are absolute URLs added verbatim.
fn inventory_context(site: &Site, paths: &[&str], extra: &[&str]) -> Value {
    let identity = serde_json::json!({"origin": site.base_url, "release": "r1", "surface": "public", "snapshot": "s1"});
    let pages = paths
        .iter()
        .map(|path| format!("{}{path}", site.base_url))
        .chain(extra.iter().map(|url| url.to_string()))
        .map(|url| serde_json::json!({"url": url, "disposition": "published", "target": null, "locale": null}))
        .collect::<Vec<_>>();
    serde_json::json!({"identity": identity, "snapshot_date": "2026-11-07",
        "inventory": {"identity": identity, "complete": true, "pages": pages}})
}

/// Crawl `site` from `dir` seeded with `context`, returning the command output
/// and the saved audit artifact path.
fn seeded_crawl(site: &Site, dir: &Path, context: &Value, extra: &[&str]) -> (Value, String) {
    let seed = dir.join("context.json");
    write(&seed, &context.to_string());
    let output = Command::new(bin())
        .current_dir(dir)
        .args([
            "crawl",
            &site.base_url,
            "--engine",
            "http",
            "--progress",
            "off",
        ])
        .args(["--format", "json", "--no-sitemap-seed", "--seed-file"])
        .arg(&seed)
        .args(extra)
        .output()
        .unwrap();
    let payload = parse_json(&output.stdout);
    let audit = payload["audit_path"]
        .as_str()
        .unwrap_or_else(|| panic!("{payload:#}"))
        .to_string();
    (payload, audit)
}

fn audit_link_report(dir: &Path, audit: &str) -> Value {
    let output = Command::new(bin())
        .current_dir(dir)
        .args(["link-graph", audit, "--audit", "--context", "context.json"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    parse_json(&output.stdout)
}

fn orphan_site() -> Site {
    Site::start(vec![
        ("/", html(r#"<main><a href="/a">A</a></main>"#)),
        ("/a", html("<main>A</main>")),
        ("/orphan", html("<main>Nothing links here</main>")),
    ])
}

#[test]
fn an_inventory_seeded_crawl_reaches_an_unlinked_page_and_attests_completeness() {
    let site = orphan_site();
    let temp = tempfile::tempdir().unwrap();
    let context = inventory_context(&site, &["/", "/a", "/orphan"], &[]);
    let (payload, audit) = seeded_crawl(&site, temp.path(), &context, &[]);
    assert!(
        site.requested("/orphan"),
        "the seed reaches the unlinked page"
    );
    let attestation = &payload["artifact"]["crawl_attestation"];
    let mut observed = attestation.clone();
    let snapshot = observed
        .as_object_mut()
        .unwrap()
        .remove("snapshot")
        .unwrap();
    assert_eq!(
        observed,
        serde_json::json!({"seeded_from": {"inventory_identity": context["identity"], "url_count": 3},
            "engine": "http", "out_of_origin_urls": [], "unfetched_urls": [], "robots_skipped_urls": []}),
        "{payload:#}"
    );
    assert!(snapshot["started_at"].as_str().unwrap() <= snapshot["finished_at"].as_str().unwrap());
    assert!(snapshot.get("resumed").is_none());
    let saved: Value = serde_json::from_slice(&std::fs::read(&audit).unwrap()).unwrap();
    assert_eq!(saved["crawl_attestation"], *attestation);

    let report = audit_link_report(temp.path(), &audit);
    assert_eq!(report["crawl_complete"], true, "{report:#}");
    assert!(
        !report["issues"].to_string().contains("audit_"),
        "{report:#}"
    );
    let reconciliation = &report["reconciliation"];
    assert_eq!(reconciliation["comparable"], true, "{report:#}");
    let orphan = format!("{}/orphan", site.base_url);
    assert!(
        reconciliation["eligible_pages"]
            .as_array()
            .unwrap()
            .contains(&Value::String(orphan.clone())),
        "{report:#}"
    );
    let page = report["pages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|page| page["url"] == orphan)
        .unwrap();
    assert_eq!(page["distinct_source_pages"], 0);
}

#[test]
fn a_crawl_without_a_seed_file_records_no_attestation_and_is_never_complete() {
    let site = orphan_site();
    let temp = tempfile::tempdir().unwrap();
    let (_, payload) = crawl(&site, &["--no-sitemap-seed"]);
    assert!(payload["artifact"].get("crawl_attestation").is_none());
    assert!(!site.requested("/orphan"));

    // The same crawl with a matching context still cannot claim coverage.
    let context = inventory_context(&site, &["/", "/a", "/orphan"], &[]);
    write(&temp.path().join("context.json"), &context.to_string());
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
        .args(["--format", "json", "--no-sitemap-seed"])
        .output()
        .unwrap();
    let audit = parse_json(&output.stdout)["audit_path"]
        .as_str()
        .unwrap()
        .to_string();
    let report = audit_link_report(temp.path(), &audit);
    assert_eq!(report["crawl_complete"], false);
    assert!(
        report["issues"]
            .to_string()
            .contains("audit_not_inventory_seeded")
    );
}

#[test]
fn a_seed_entry_outside_the_crawl_origin_is_reported_and_never_requested() {
    let site = orphan_site();
    let temp = tempfile::tempdir().unwrap();
    // The crawler maps a seed to a path on the target, so a foreign URL with
    // a path the site serves must still never be requested.
    let context = inventory_context(
        &site,
        &["/", "/a"],
        &["http://other.example/orphan?token=secret"],
    );
    let (payload, _) = seeded_crawl(&site, temp.path(), &context, &[]);
    assert!(!site.requested("/orphan"), "{payload:#}");
    let attestation = &payload["artifact"]["crawl_attestation"];
    assert_eq!(
        attestation["out_of_origin_urls"],
        serde_json::json!(["http://other.example/orphan"])
    );
    assert_eq!(attestation["seeded_from"]["url_count"], 2);
    assert_eq!(attestation["unfetched_urls"], serde_json::json!([]));
}

#[test]
fn seeded_urls_skipped_by_robots_are_attested_as_unfetched() {
    let site = Site::start(vec![
        ("/", html("<main>home</main>")),
        ("/robots.txt", robots("User-agent: *\nDisallow: /private\n")),
        ("/private", html("secret")),
    ]);
    let temp = tempfile::tempdir().unwrap();
    let context = inventory_context(&site, &["/", "/private"], &[]);
    let (payload, audit) = seeded_crawl(&site, temp.path(), &context, &[]);
    assert!(!site.requested("/private"));
    let private = format!("{}/private", site.base_url);
    let attestation = &payload["artifact"]["crawl_attestation"];
    assert_eq!(attestation["unfetched_urls"], serde_json::json!([private]));
    assert_eq!(
        attestation["robots_skipped_urls"],
        serde_json::json!([private])
    );
    let report = audit_link_report(temp.path(), &audit);
    assert_eq!(report["crawl_complete"], false);
    for issue in ["audit_robots_skipped", "audit_seed_unfetched"] {
        assert!(report["issues"].to_string().contains(issue), "{report:#}");
    }
}

#[test]
fn inventory_seeds_survive_a_checkpoint_and_resume() {
    let site = orphan_site();
    let temp = tempfile::tempdir().unwrap();
    let checkpoint = temp.path().join("checkpoint.json");
    let context = inventory_context(&site, &["/", "/a", "/orphan"], &[]);
    let checkpoint_args = [
        "--checkpoint",
        checkpoint.to_str().unwrap(),
        "--checkpoint-every",
        "1",
    ];
    let mut interrupted = vec!["--max-pages", "1"];
    interrupted.extend(checkpoint_args);
    let (payload, _) = seeded_crawl(&site, temp.path(), &context, &interrupted);
    assert_eq!(payload["artifact"]["status"], "partial");
    assert!(!site.requested("/orphan"));
    assert_eq!(
        payload["artifact"]["crawl_attestation"]["unfetched_urls"]
            .as_array()
            .unwrap()
            .len(),
        2
    );

    let (payload, audit) = seeded_crawl(
        &site,
        temp.path(),
        &context,
        &["--resume", checkpoint.to_str().unwrap()],
    );
    assert!(
        site.requested("/orphan"),
        "the seed was kept in the checkpoint"
    );
    assert_eq!(payload["artifact"]["status"], "complete");
    assert_eq!(
        payload["artifact"]["crawl_attestation"]["unfetched_urls"],
        serde_json::json!([])
    );
    assert_eq!(
        audit_link_report(temp.path(), &audit)["crawl_complete"],
        true
    );
}

/// `orphan_site` plus `/draft`, listed as a draft and answered with `status`.
fn crawl_with_draft(status: Option<&'static str>) -> (Site, Value, Value) {
    let mut routes = vec![
        ("/", html(r#"<main><a href="/a">A</a></main>"#)),
        ("/a", html("<main>A</main>")),
        ("/orphan", html("<main>Nothing links here</main>")),
    ];
    if let Some(status) = status {
        routes.push(("/draft", (status, "text/html", "error".to_string())));
    }
    let site = Site::start(routes);
    let temp = tempfile::tempdir().unwrap();
    let mut context = inventory_context(&site, &["/", "/a", "/orphan", "/draft"], &[]);
    context["inventory"]["pages"][3]["disposition"] = "draft".into();
    let (payload, audit) = seeded_crawl(&site, temp.path(), &context, &[]);
    assert!(site.requested("/draft"), "drafts are seeded too");
    let report = audit_link_report(temp.path(), &audit);
    (site, payload, report)
}

#[test]
fn a_seeded_draft_answering_404_keeps_the_crawl_complete() {
    // The fixture answers unknown paths with 404.
    let (site, payload, report) = crawl_with_draft(None);
    assert_eq!(
        payload["artifact"]["crawl_attestation"]["unfetched_urls"],
        serde_json::json!([])
    );
    assert_eq!(report["crawl_complete"], true, "{report:#}");
    assert!(
        !report["issues"].to_string().contains("audit_"),
        "{report:#}"
    );
    assert_eq!(report["reconciliation"]["comparable"], true, "{report:#}");
    let orphan = Value::String(format!("{}/orphan", site.base_url));
    assert!(
        report["reconciliation"]["eligible_pages"]
            .as_array()
            .unwrap()
            .contains(&orphan)
    );
}

#[test]
fn a_seeded_draft_answering_500_leaves_the_crawl_incomplete() {
    let (_, _, report) = crawl_with_draft(Some("500 Internal Server Error"));
    assert_eq!(report["crawl_complete"], false);
    assert!(
        report["issues"]
            .to_string()
            .contains("audit_failed_responses"),
        "{report:#}"
    );
}

fn release_attestation(payload: &Value) -> &Value {
    &payload["artifact"]["crawl_attestation"]["release"]
}

fn page_markers(payload: &Value) -> Vec<Value> {
    payload["artifact"]["site"]["pages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|page| page["crawl_response"]["release_marker"].clone())
        .collect()
}

#[test]
fn a_release_header_is_recorded_per_page_and_becomes_the_report_release() {
    let site = orphan_site();
    site.serve_header("X-Release", " r1 ");
    let temp = tempfile::tempdir().unwrap();
    let context = inventory_context(&site, &["/", "/a", "/orphan"], &[]);
    let (payload, audit) = seeded_crawl(
        &site,
        temp.path(),
        &context,
        &["--release-header", "x-release"],
    );
    assert_eq!(page_markers(&payload), vec![Value::from("r1"); 3]);
    assert_eq!(
        *release_attestation(&payload),
        serde_json::json!({"source": "header", "name": "x-release", "value": "r1",
            "pages_with_marker": 3, "pages_without_marker": 0, "distinct_values": ["r1"]})
    );
    let report = audit_link_report(temp.path(), &audit);
    assert_eq!(report["identity"]["release"], "r1");
    assert_eq!(report["crawl_complete"], true, "{report:#}");
    assert!(
        !report["issues"].to_string().contains("audit_"),
        "{report:#}"
    );
    assert_eq!(report["reconciliation"]["comparable"], true, "{report:#}");

    // The site serves another release than the caller and the inventory say.
    site.serve_header("X-Release", "r2");
    let temp = tempfile::tempdir().unwrap();
    let (_, audit) = seeded_crawl(
        &site,
        temp.path(),
        &context,
        &["--release-header", "X-Release"],
    );
    let report = audit_link_report(temp.path(), &audit);
    assert_eq!(report["identity"]["release"], "r2");
    let issues = report["issues"].to_string();
    assert!(issues.contains("audit_release_mismatch"), "{report:#}");
    let reconciliation = &report["reconciliation"];
    assert_eq!(reconciliation["comparable"], false);
    assert!(
        reconciliation["issues"]
            .to_string()
            .contains("surface_identity_mismatch")
    );
    assert_eq!(reconciliation["eligible_pages"], serde_json::json!([]));
}

#[test]
fn release_meta_markers_that_disagree_or_are_missing_are_not_attested() {
    let meta = |release: &str| format!(r#"<meta name="release" content="{release}">"#);
    let site = Site::start(vec![
        (
            "/",
            html(&format!(
                r#"{}<a href="/a">A</a><a href="/b">B</a>"#,
                meta("r1")
            )),
        ),
        ("/a", html(&meta("r2"))),
        (
            "/b",
            html("<!-- <meta name=\"release\" content=\"r1\"> -->"),
        ),
    ]);
    let temp = tempfile::tempdir().unwrap();
    let (_, payload) = crawl(&site, &["--no-sitemap-seed", "--release-meta", "release"]);
    let attestation = &payload["artifact"]["crawl_attestation"];
    assert_eq!(attestation["seeded_from"]["url_count"], 0);
    assert_eq!(
        *release_attestation(&payload),
        serde_json::json!({"source": "meta", "name": "release", "value": null,
            "pages_with_marker": 2, "pages_without_marker": 1, "distinct_values": ["r1", "r2"]})
    );
    let context = inventory_context(&site, &["/", "/a", "/b"], &[]);
    write(&temp.path().join("context.json"), &context.to_string());
    // The crawl's own directory is gone; its artifact is in the output.
    write(
        &temp.path().join("audit.json"),
        &payload["artifact"].to_string(),
    );
    let report = audit_link_report(temp.path(), "audit.json");
    let issues = report["issues"].to_string();
    for issue in [
        "audit_release_inconsistent",
        "audit_release_unattested",
        "audit_not_inventory_seeded",
    ] {
        assert!(issues.contains(issue), "{issue}: {report:#}");
    }
    // The caller's release stands when the crawl attests none.
    assert_eq!(report["identity"]["release"], "r1");
}

fn resume_with(url: &str, checkpoint: &Path, extra: &[&str]) -> Output {
    Command::new(bin())
        .args(["crawl", url, "--engine", "http", "--progress", "off"])
        .args(["--format", "json", "--resume", checkpoint.to_str().unwrap()])
        .args(extra)
        .current_dir(checkpoint.parent().unwrap())
        .output()
        .unwrap()
}

#[test]
fn a_resumed_crawl_keeps_its_start_and_reports_a_release_change() {
    let site = resumable_site();
    site.serve_header("X-Release", "r1");
    let temp = tempfile::tempdir().unwrap();
    let checkpoint = temp.path().join("checkpoint.json");
    let (_, payload) = crawl(
        &site,
        &[
            "--max-pages",
            "1",
            "--checkpoint",
            checkpoint.to_str().unwrap(),
            "--checkpoint-every",
            "1",
            "--release-header",
            "x-release",
        ],
    );
    assert_eq!(payload["artifact"]["status"], "partial");
    let mut saved: Value = serde_json::from_slice(&std::fs::read(&checkpoint).unwrap()).unwrap();
    assert!(saved["started_at"].is_i64(), "{saved:#}");
    assert_eq!(saved["release_marker"]["header"], "x-release");
    saved["started_at"] = 0.into();
    std::fs::write(&checkpoint, saved.to_string()).unwrap();

    // A resume must ask for the same marker.
    let refused = resume_with(&site.base_url, &checkpoint, &[]);
    assert_ne!(refused.status.code(), Some(0));
    let text = String::from_utf8_lossy(&refused.stdout).to_string()
        + &String::from_utf8_lossy(&refused.stderr);
    assert!(text.contains("different release marker"), "{text}");

    site.serve_header("X-Release", "r2");
    let output = resume_with(
        &site.base_url,
        &checkpoint,
        &["--release-header", "x-release"],
    );
    let payload = parse_json(&output.stdout);
    assert_eq!(payload["artifact"]["status"], "complete", "{payload:#}");
    let attestation = &payload["artifact"]["crawl_attestation"];
    assert_eq!(
        attestation["snapshot"]["started_at"],
        "1970-01-01T00:00:00Z"
    );
    assert_eq!(attestation["snapshot"]["resumed"], true);
    let release = release_attestation(&payload);
    assert_eq!(release["spans_segments"], true, "{payload:#}");
    assert_eq!(release["value"], Value::Null);
    assert_eq!(release["distinct_values"], serde_json::json!(["r1", "r2"]));
}

#[test]
fn a_checkpoint_from_before_release_markers_still_resumes_unchanged() {
    let site = resumable_site();
    let temp = tempfile::tempdir().unwrap();
    let checkpoint = temp.path().join("checkpoint.json");
    interrupted_crawl(&site, &checkpoint);
    let mut saved: Value = serde_json::from_slice(&std::fs::read(&checkpoint).unwrap()).unwrap();
    assert!(saved.get("release_marker").is_none());
    saved.as_object_mut().unwrap().remove("started_at");
    std::fs::write(&checkpoint, saved.to_string()).unwrap();
    let payload = parse_json(&resume_with(&site.base_url, &checkpoint, &[]).stdout);
    assert_eq!(payload["artifact"]["status"], "complete");
    assert!(payload["artifact"].get("crawl_attestation").is_none());
    assert!(!payload.to_string().contains("release_marker"));
}

#[test]
fn a_credential_header_is_refused_as_a_release_marker() {
    let site = resumable_site();
    let (output, _) = crawl(&site, &["--release-header", "Set-Cookie"]);
    assert_ne!(output.status.code(), Some(0));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("credentials")
            || String::from_utf8_lossy(&output.stdout).contains("credentials")
    );
    assert!(site.requests.lock().unwrap().is_empty());
}

#[test]
fn inventory_aliases_are_checked_against_what_the_seeded_crawl_observed() {
    let site = Site::start(vec![
        ("/", html(r#"<main><a href="/guide">Guide</a></main>"#)),
        (
            "/old",
            (
                "301 Moved Permanently\r\nLocation: /new",
                "text/html",
                String::new(),
            ),
        ),
        ("/new", html("<main>New</main>")),
        ("/guide", html("<main>Guide</main>")),
        (
            "/alias",
            html(r#"<link rel="canonical" href="guide#top"><main>Copy</main>"#),
        ),
        ("/fr/guide", html("<main>Guide</main>")),
    ]);
    let temp = tempfile::tempdir().unwrap();
    let base = &site.base_url;
    // `/new` is fetched before `/old` here. The snapshot keeps one capture per
    // final route, so a redirect fetched before a direct fetch of its target
    // loses its provenance and leaves the seed unfetched.
    let mut context = inventory_context(&site, &["/", "/new", "/guide"], &[]);
    let aliases = [
        ("/old", "redirect", "/new"),
        ("/alias", "canonical_alias", "/guide"),
        ("/fr/guide", "locale_fallback", "/guide"),
    ];
    for (path, disposition, target) in aliases {
        context["inventory"]["pages"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({
            "url": format!("{base}{path}"), "disposition": disposition,
            "target": format!("{base}{target}"), "locale": null}));
    }
    let (_, audit) = seeded_crawl(&site, temp.path(), &context, &[]);
    let report = audit_link_report(temp.path(), &audit);
    assert_eq!(report["crawl_complete"], true, "{report:#}");
    let alias_at = |url: String| {
        report["pages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|page| page["url"] == url)
            .unwrap_or_else(|| panic!("{report:#}"))["inventory_aliases"][0]
            .clone()
    };
    let redirect = alias_at(format!("{base}/new"));
    assert_eq!(redirect["url"], format!("{base}/old"), "{report:#}");
    assert_eq!(redirect["verified_by"], "redirect", "{report:#}");
    assert_eq!(
        alias_at(format!("{base}/alias"))["verified_by"],
        "canonical"
    );
    let fallback = alias_at(format!("{base}/fr/guide"));
    assert_eq!(
        fallback,
        serde_json::json!({"url": format!("{base}/fr/guide"), "disposition": "locale_fallback",
            "expected_target": format!("{base}/guide"), "verified_by": null,
            "issue": "inventory_alias_unobserved", "observed_url": format!("{base}/fr/guide")})
    );
    let reconciliation = &report["reconciliation"];
    assert_eq!(reconciliation["comparable"], false);
    assert_eq!(reconciliation["eligible_pages"], serde_json::json!([]));
    let issues = reconciliation["issues"].to_string();
    assert!(issues.contains("inventory_alias_unobserved"), "{report:#}");
    assert!(!issues.contains("redirect_observed"), "{report:#}");
    assert!(
        !issues.contains("canonical_target_unresolved"),
        "{report:#}"
    );

    // Once the fallback is published in its own right, the verified aliases
    // leave the inventory's own pages eligible, and never themselves.
    context["inventory"]["pages"][5]["disposition"] = "published".into();
    write(&temp.path().join("context.json"), &context.to_string());
    let report = audit_link_report(temp.path(), &audit);
    let reconciliation = &report["reconciliation"];
    assert_eq!(reconciliation["comparable"], true, "{report:#}");
    let eligible = ["/", "/fr/guide", "/guide", "/new"].map(|path| format!("{base}{path}"));
    assert_eq!(
        reconciliation["eligible_pages"],
        serde_json::json!(eligible)
    );
}

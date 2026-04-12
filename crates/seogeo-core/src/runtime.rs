mod graph;
mod http;
mod planner;
mod playwright;
mod snapshot;

use anyhow::Result;
use seogeo_contracts::Finding;
use std::collections::BTreeSet;

use crate::config::Config;
use crate::policy::apply_policy;
use crate::site::{Site, route_from_urlish};
use crate::static_check::run_checks_for_site;
use crate::verification::{DiffResult, diff_finding_sets};
use graph::{extract_internal_links, read_loc_values, response_report_path, should_enqueue_link};
use http::{fetch_with_http, host_for_url, is_html_content_type, same_site_host};
use planner::CrawlPlanner;
use playwright::{fetch_with_playwright, playwright_is_available};
use snapshot::RuntimeSnapshotBuilder;

#[derive(Debug, Clone)]
pub struct RuntimeAudit {
    pub site: Site,
    pub crawl_findings: Vec<Finding>,
    pub findings: Vec<Finding>,
}

fn resolve_runtime_engine(engine: &str) -> Result<&'static str> {
    match engine {
        "auto" => Ok(if playwright_is_available() {
            "playwright"
        } else {
            "http"
        }),
        "http" => Ok("http"),
        "playwright" => {
            if playwright_is_available() {
                Ok("playwright")
            } else {
                anyhow::bail!(
                    "runtime engine 'playwright' requires a local Playwright runtime; run `npm install` in the repo root or set SEOGEO_PLAYWRIGHT_EXECUTABLE"
                )
            }
        }
        other => anyhow::bail!("unknown runtime engine '{other}'"),
    }
}

fn materialize_runtime_site(
    base_url: &str,
    max_pages: usize,
    engine: &str,
    config: &Config,
) -> Result<(Site, Vec<Finding>)> {
    let runtime = config.runtime();
    let mut planner = CrawlPlanner::new(base_url, max_pages);
    let mut snapshot = RuntimeSnapshotBuilder::new();
    let mut crawl_findings = Vec::new();

    for seed in runtime.crawl_seeds {
        planner.seed_from_user_input(seed, &runtime);
    }

    if runtime.crawl_use_sitemap {
        let mut visited_sitemaps = BTreeSet::new();
        let sitemap_base = planner.normalized_base().to_string();
        for sitemap_name in ["sitemap.xml", "sitemap-index.xml", "sitemap_index.xml"] {
            seed_routes_from_sitemap(
                &mut planner,
                &format!("{}{}", sitemap_base, sitemap_name),
                &runtime,
                &mut visited_sitemaps,
            )?;
        }
    }

    while let Some(current) = planner.next_url(&runtime) {
        let fetched = fetch_runtime_page(engine, &current, &runtime)?;
        let Some(body) = fetched.body else {
            let route = route_from_urlish(&current).unwrap_or_default();
            crawl_findings.push(Finding {
                rule_id: "CRW001".to_string(),
                message: format!("failed to fetch URL: {}", current),
                path: response_report_path(&route),
                line: 1,
                column: 1,
                severity: "error".to_string(),
                suggestion: None,
            });
            continue;
        };
        if !is_html_content_type(fetched.content_type.as_deref()) {
            continue;
        }
        let effective_host = host_for_url(&fetched.effective_url);
        if !same_site_host(&effective_host, planner.base_host()) {
            continue;
        }
        planner.align_with_effective_url(&fetched.effective_url);
        let route = route_from_urlish(&fetched.effective_url).unwrap_or_default();
        snapshot.write_page(&route, &body, &fetched.headers)?;

        for target in extract_internal_links(&body, planner.base_host()) {
            if !should_enqueue_link(&target) {
                continue;
            }
            planner.discover_link_target(&target, &runtime);
        }
    }

    snapshot.capture_optional_artifacts(planner.normalized_base(), &runtime)?;
    let (site, snapshot_findings) = snapshot.finalize(
        planner.visited_count(),
        max_pages,
        planner.discovered_route_count(),
        planner.truncated(),
    )?;
    crawl_findings.extend(snapshot_findings);
    Ok((site, crawl_findings))
}

fn fetch_runtime_page(
    engine: &str,
    url: &str,
    runtime: &crate::config::RuntimeConfig<'_>,
) -> Result<http::FetchResult> {
    match engine {
        "http" => fetch_with_http(
            url,
            runtime.crawl_headers,
            runtime.crawl_cookies,
            runtime.crawl_basic_auth,
        ),
        "playwright" => fetch_with_playwright(url, runtime),
        other => anyhow::bail!("unsupported runtime engine '{other}'"),
    }
}

fn seed_routes_from_sitemap(
    planner: &mut CrawlPlanner,
    sitemap_url: &str,
    runtime: &crate::config::RuntimeConfig<'_>,
    visited_sitemaps: &mut BTreeSet<String>,
) -> Result<()> {
    if !visited_sitemaps.insert(sitemap_url.to_string()) {
        return Ok(());
    }
    let fetched = fetch_with_http(
        sitemap_url,
        runtime.crawl_headers,
        runtime.crawl_cookies,
        runtime.crawl_basic_auth,
    )?;
    if fetched.status_code.unwrap_or(500) >= 400
        || !fetched
            .content_type
            .as_deref()
            .unwrap_or_default()
            .contains("xml")
    {
        return Ok(());
    }
    let Some(body) = fetched.body else {
        return Ok(());
    };
    for loc in read_loc_values(&body) {
        if loc.trim().ends_with(".xml") {
            seed_routes_from_sitemap(planner, &loc, runtime, visited_sitemaps)?;
        } else {
            planner.seed_from_sitemap_loc(&loc, runtime);
        }
    }
    Ok(())
}

pub fn run_runtime_audit(
    base_url: &str,
    max_pages: usize,
    engine: &str,
    config: &Config,
) -> Result<RuntimeAudit> {
    let effective_engine = resolve_runtime_engine(engine)?;
    let (site, crawl_findings) =
        materialize_runtime_site(base_url, max_pages, effective_engine, config)?;
    let mut findings = crawl_findings.clone();
    findings.extend(run_checks_for_site(&site, config));
    findings = apply_policy(findings, config);
    Ok(RuntimeAudit {
        site,
        crawl_findings,
        findings,
    })
}

pub fn verify_runtime_audit(audit: &RuntimeAudit, baseline_findings: &[Finding]) -> DiffResult {
    diff_finding_sets(baseline_findings, &audit.findings)
}

#[cfg(test)]
mod tests {
    use super::{run_runtime_audit, verify_runtime_audit};
    use crate::config::{Config, default_rule_switches};
    use seogeo_contracts::Finding;
    use std::collections::{BTreeMap, BTreeSet};
    use std::io::ErrorKind;
    use std::io::{Read, Write};
    use std::net::{SocketAddr, TcpListener, TcpStream};
    use std::path::PathBuf;
    use std::thread;
    use std::time::{Duration, Instant};

    fn respond(
        mut stream: TcpStream,
        status: &str,
        content_type: &str,
        body: &str,
        extra_headers: &[(&str, &str)],
    ) {
        let mut response = format!(
            "HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n",
            status,
            content_type,
            body.len()
        );
        for (key, value) in extra_headers {
            response.push_str(&format!("{}: {}\r\n", key, value));
        }
        response.push_str("\r\n");
        response.push_str(body);
        stream.write_all(response.as_bytes()).unwrap();
        stream.flush().unwrap();
    }

    fn spawn_fixture_server<F>(min_requests: usize, handler: F) -> (String, thread::JoinHandle<()>)
    where
        F: Fn(TcpStream, String, SocketAddr) + Send + 'static,
    {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            let mut served = 0usize;
            let started = Instant::now();
            let mut last_request = Instant::now();
            loop {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream.set_nonblocking(false).unwrap();
                        let mut buffer = [0_u8; 4096];
                        let size = stream.read(&mut buffer).unwrap();
                        let request = String::from_utf8_lossy(&buffer[..size]).into_owned();
                        served += 1;
                        last_request = Instant::now();
                        handler(stream, request, address);
                    }
                    Err(error) if error.kind() == ErrorKind::WouldBlock => {
                        if (served >= min_requests.max(1)
                            && last_request.elapsed() > Duration::from_millis(150))
                            || started.elapsed() > Duration::from_secs(30)
                        {
                            break;
                        }
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) => panic!("server accept failed: {error}"),
                }
            }
        });
        (format!("http://{}", address), handle)
    }

    fn spawn_server(expected_requests: usize) -> (String, thread::JoinHandle<()>) {
        spawn_fixture_server(expected_requests, |stream, request, _address| {
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
                    &[],
                ),
                "/about" => respond(
                    stream,
                    "200 OK",
                    "text/html",
                    "<html><head><meta name=\"description\" content=\"About page\"></head><body><h1>About</h1></body></html>",
                    &[("X-Robots-Tag", "noindex")],
                ),
                "/robots.txt" => respond(
                    stream,
                    "200 OK",
                    "text/plain",
                    "User-agent: *\nAllow: /\n",
                    &[],
                ),
                "/llms.txt" => respond(
                    stream,
                    "200 OK",
                    "text/plain",
                    "# Site\n\n## Pages\n- [Home](/)\n",
                    &[],
                ),
                "/sitemap.xml" => respond(
                    stream,
                    "200 OK",
                    "application/xml",
                    "<urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\"><url><loc>http://example.test/</loc></url><url><loc>http://example.test/about</loc></url></urlset>",
                    &[],
                ),
                _ => respond(stream, "404 Not Found", "text/plain", "missing", &[]),
            }
        })
    }

    fn spawn_sitemap_index_server(expected_requests: usize) -> (String, thread::JoinHandle<()>) {
        spawn_fixture_server(expected_requests, move |stream, request, address| {
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
                    "<html><head><title>Home</title><meta name=\"description\" content=\"Root\"><link rel=\"canonical\" href=\"http://example.test/\"></head><body><h1>Home</h1></body></html>",
                    &[],
                ),
                "/from-sitemap" => respond(
                    stream,
                    "200 OK",
                    "text/html",
                    "<html><head><title>Indexed</title><meta name=\"description\" content=\"Indexed\"><link rel=\"canonical\" href=\"http://example.test/from-sitemap\"></head><body><h1>Indexed</h1></body></html>",
                    &[],
                ),
                "/robots.txt" => respond(
                    stream,
                    "200 OK",
                    "text/plain",
                    "User-agent: *\nAllow: /\n",
                    &[],
                ),
                "/llms.txt" => respond(stream, "404 Not Found", "text/plain", "missing", &[]),
                "/sitemap.xml" => {
                    respond(stream, "404 Not Found", "application/xml", "missing", &[])
                }
                "/sitemap-index.xml" => respond(
                    stream,
                    "200 OK",
                    "application/xml",
                    &format!(
                        "<sitemapindex xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\"><sitemap><loc>http://{}/nested-sitemap.xml</loc></sitemap></sitemapindex>",
                        address
                    ),
                    &[],
                ),
                "/nested-sitemap.xml" => respond(
                    stream,
                    "200 OK",
                    "application/xml",
                    &format!(
                        "<urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\"><url><loc>http://{}/from-sitemap</loc></url></urlset>",
                        address
                    ),
                    &[],
                ),
                "/sitemap_index.xml" => {
                    respond(stream, "404 Not Found", "application/xml", "missing", &[])
                }
                _ => respond(stream, "404 Not Found", "text/plain", "missing", &[]),
            }
        })
    }

    fn html_only_config() -> Config {
        let mut config = Config {
            checks: default_rule_switches()
                .into_iter()
                .map(|(key, value)| (key.to_string(), value))
                .collect(),
            ..Config::default()
        };
        for key in [
            "links",
            "sitemap",
            "robots",
            "social",
            "schema",
            "llm",
            "content",
            "structure",
        ] {
            config.checks.insert(key.to_string(), false);
        }
        config
    }

    #[test]
    fn runtime_audit_crawls_http_site() {
        let (base_url, handle) = spawn_server(6);
        let audit = run_runtime_audit(&base_url, 10, "http", &html_only_config()).unwrap();
        assert!(audit.site.route_pages.contains_key(""));
        assert!(audit.site.route_pages.contains_key("about"));
        assert!(
            audit
                .findings
                .iter()
                .any(|finding| finding.rule_id == "SEO001" || finding.rule_id == "SEO004")
        );
        handle.join().unwrap();
    }

    #[test]
    fn runtime_audit_accepts_html_charset_content_types() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            for _ in 0..8 {
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
                        "text/html; charset=utf-8",
                        "<html><head><meta name=\"description\" content=\"Root\"><link rel=\"canonical\" href=\"http://example.test/\"></head><body><h1>Home</h1><a href=\"/about\">About</a></body></html>",
                        &[],
                    ),
                    "/about" => respond(
                        stream,
                        "200 OK",
                        "text/html; charset=utf-8",
                        "<html><head><meta name=\"description\" content=\"About page\"></head><body><h1>About</h1></body></html>",
                        &[],
                    ),
                    "/robots.txt" => respond(
                        stream,
                        "200 OK",
                        "text/plain; charset=utf-8",
                        "User-agent: *\nAllow: /\n",
                        &[],
                    ),
                    "/llms.txt" => respond(
                        stream,
                        "404 Not Found",
                        "text/plain; charset=utf-8",
                        "missing",
                        &[],
                    ),
                    _ => respond(stream, "404 Not Found", "text/plain", "missing", &[]),
                }
            }
        });

        let audit = run_runtime_audit(
            &format!("http://{}", address),
            10,
            "http",
            &html_only_config(),
        )
        .unwrap();
        assert!(audit.site.route_pages.contains_key(""));
        assert!(audit.site.route_pages.contains_key("about"));
        assert!(
            audit
                .findings
                .iter()
                .any(|finding| finding.rule_id == "SEO001" || finding.rule_id == "SEO004")
        );
        handle.join().unwrap();
    }

    #[test]
    fn runtime_audit_seeds_from_sitemap_indexes() {
        let (base_url, handle) = spawn_sitemap_index_server(9);
        let audit = run_runtime_audit(&base_url, 10, "http", &html_only_config()).unwrap();
        assert!(audit.site.route_pages.contains_key(""));
        assert!(audit.site.route_pages.contains_key("from-sitemap"));
        handle.join().unwrap();
    }

    #[test]
    fn runtime_verify_reports_regressions() {
        let audit = super::RuntimeAudit {
            site: crate::site::Site {
                root: PathBuf::new(),
                pages: Vec::new(),
                route_pages: BTreeMap::new(),
                indexed_paths: BTreeSet::new(),
                inbound_links: BTreeMap::new(),
                llms_text: None,
                robots_text: None,
                sitemap_routes: BTreeSet::new(),
                sitemap_error: None,
                deployment_model: crate::site::DeploymentModel::RuntimeSnapshot,
                deployment_markers: Vec::new(),
                crawl_meta: None,
            },
            crawl_findings: Vec::new(),
            findings: vec![Finding {
                rule_id: "SEO001".to_string(),
                message: "missing <title>".to_string(),
                path: "crawl/about/index.html".to_string(),
                line: 1,
                column: 1,
                severity: "error".to_string(),
                suggestion: None,
            }],
        };
        let diff = verify_runtime_audit(&audit, &[]);
        assert_eq!(diff.new_findings.len(), 1);
    }

    #[test]
    fn runtime_audit_reports_truncated_crawl_coverage() {
        let (base_url, handle) = spawn_server(5);
        let mut config = html_only_config();
        config.checks.insert("links".to_string(), true);
        let audit = run_runtime_audit(&base_url, 1, "http", &config).unwrap();
        assert!(
            audit
                .findings
                .iter()
                .any(|finding| finding.rule_id == "CRW003")
        );
        assert!(
            !audit
                .findings
                .iter()
                .any(|finding| finding.rule_id == "LNK001")
        );
        handle.join().unwrap();
    }

    #[test]
    fn runtime_audit_handles_playwright_according_to_local_runtime() {
        if super::playwright_is_available() {
            let (base_url, handle) = spawn_server(5);
            let audit = run_runtime_audit(&base_url, 1, "playwright", &html_only_config())
                .expect("playwright should run when the local runtime is installed");
            assert!(
                audit
                    .findings
                    .iter()
                    .any(|finding| finding.path == "crawl/index.html")
            );
            handle.join().unwrap();
        } else {
            let error =
                run_runtime_audit("https://example.com", 10, "playwright", &Config::default())
                    .expect_err("playwright without a runner should fail");
            assert!(
                error
                    .to_string()
                    .contains("requires a local Playwright runtime")
            );
        }
    }

    #[test]
    fn runtime_audit_rejects_unknown_engine() {
        let error = run_runtime_audit("https://example.com", 10, "invalid", &Config::default())
            .expect_err("invalid engines should fail");
        assert!(error.to_string().contains("unknown runtime engine"));
    }
}

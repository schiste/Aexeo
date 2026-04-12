mod fetcher;
mod graph;
mod http;
mod planner;
mod playwright;
mod snapshot;

use anyhow::Result;
use seogeo_contracts::{AuditArtifact, AuditStatus, CrawlStats, Finding, FindingScope};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use crate::config::Config;
use crate::policy::apply_policy;
use crate::reporting::build_audit_artifact;
use crate::site::{Site, route_from_urlish};
use crate::static_check::run_checks_for_site;
use crate::verification::{DiffResult, diff_finding_sets};
use fetcher::RuntimeFetcher;
use graph::{extract_internal_links, read_loc_values, response_report_path, should_enqueue_link};
use http::{fetch_with_http, host_for_url, is_html_content_type, same_site_host};
use planner::{CrawlPlanner, CrawlPlannerState};
pub use playwright::PlaywrightDoctor;
use playwright::{playwright_is_available, probe_playwright_runtime};
use snapshot::{RuntimeSnapshotBuilder, RuntimeSnapshotState};

#[derive(Debug, Clone)]
pub struct RuntimeAudit {
    pub site: Site,
    pub crawl_findings: Vec<Finding>,
    pub findings: Vec<Finding>,
    pub status: AuditStatus,
    pub crawl_stats: CrawlStats,
    pub truncation_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeCheckpoint {
    pub(crate) version: u32,
    pub(crate) base_url: String,
    pub(crate) engine: String,
    pub(crate) planner: CrawlPlannerState,
    pub(crate) snapshot: RuntimeSnapshotState,
    pub(crate) crawl_findings: Vec<Finding>,
    pub(crate) crawl_stats: CrawlStats,
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeProgressEvent {
    pub phase: String,
    pub engine: String,
    pub current_url: Option<String>,
    pub visited_pages: usize,
    pub discovered_internal_routes: usize,
    pub queued_routes_remaining: usize,
    pub fetch_failures: usize,
    pub fetch_retries: usize,
    pub skipped_non_html: usize,
    pub truncated: bool,
}

pub struct RuntimeAuditOptions<'a> {
    pub checkpoint_path: Option<&'a Path>,
    pub checkpoint_every: usize,
    pub resume_from: Option<&'a Path>,
    pub fetch_retry_budget: usize,
    pub progress: RuntimeProgressMode<'a>,
    pub artifact_command: &'a str,
    pub partial_artifact: RuntimeArtifactMode<'a>,
}

#[derive(Default)]
pub enum RuntimeProgressMode<'a> {
    #[default]
    Off,
    Callback(&'a mut dyn FnMut(RuntimeProgressEvent)),
}

#[derive(Default)]
pub enum RuntimeArtifactMode<'a> {
    #[default]
    Off,
    Callback(&'a mut dyn FnMut(&AuditArtifact) -> Result<()>),
}

impl<'a> Default for RuntimeAuditOptions<'a> {
    fn default() -> Self {
        Self {
            checkpoint_path: None,
            checkpoint_every: 25,
            resume_from: None,
            fetch_retry_budget: 2,
            progress: RuntimeProgressMode::Off,
            artifact_command: "crawl",
            partial_artifact: RuntimeArtifactMode::Off,
        }
    }
}

impl RuntimeAudit {
    pub fn is_partial(&self) -> bool {
        self.status == AuditStatus::Partial
    }
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
            let doctor = probe_playwright_runtime();
            if doctor.available {
                Ok("playwright")
            } else {
                anyhow::bail!(
                    "runtime engine 'playwright' requires a local Playwright runtime; {}",
                    doctor.message
                )
            }
        }
        other => anyhow::bail!("unknown runtime engine '{other}'"),
    }
}

fn emit_progress(options: &mut RuntimeAuditOptions<'_>, event: RuntimeProgressEvent) {
    if let RuntimeProgressMode::Callback(callback) = &mut options.progress {
        callback(event);
    }
}

fn emit_partial_artifact(
    options: &mut RuntimeAuditOptions<'_>,
    artifact: &AuditArtifact,
) -> Result<()> {
    if let RuntimeArtifactMode::Callback(callback) = &mut options.partial_artifact {
        callback(artifact)?;
    }
    Ok(())
}

fn build_partial_runtime_artifact(
    command: &str,
    snapshot: &RuntimeSnapshotBuilder,
    planner: &CrawlPlanner,
    crawl_findings: &[Finding],
    crawl_stats: &CrawlStats,
    config: &Config,
) -> Result<AuditArtifact> {
    let (site, snapshot_findings) = snapshot.preview(
        planner.visited_count(),
        crawl_stats.max_pages,
        planner.discovered_route_count(),
        planner.truncated() || planner.queued_count() > 0,
    )?;
    let mut findings = crawl_findings.to_vec();
    findings.extend(snapshot_findings);
    findings.extend(run_checks_for_site(&site, config));
    findings = apply_policy(findings, config);
    let status = if planner.queued_count() > 0 || planner.truncated() {
        AuditStatus::Partial
    } else {
        AuditStatus::Complete
    };
    let truncation_reason = if planner.truncated() {
        Some(format!(
            "crawl stopped at max_pages={} after visiting {} pages while at least {} routes were discovered",
            crawl_stats.max_pages,
            planner.visited_count(),
            planner.discovered_route_count()
        ))
    } else if planner.queued_count() > 0 {
        Some(format!(
            "checkpoint after visiting {} pages with {} routes still queued",
            planner.visited_count(),
            planner.queued_count()
        ))
    } else {
        None
    };
    Ok(build_audit_artifact(
        command,
        &findings,
        status,
        Some(crawl_stats.clone()),
        truncation_reason,
    ))
}

fn checkpoint_state(
    checkpoint_path: &Path,
    base_url: &str,
    engine: &str,
    planner: &CrawlPlanner,
    snapshot: &RuntimeSnapshotBuilder,
    crawl_findings: &[Finding],
    crawl_stats: &CrawlStats,
) -> Result<()> {
    let checkpoint = RuntimeCheckpoint {
        version: 1,
        base_url: base_url.to_string(),
        engine: engine.to_string(),
        planner: planner.checkpoint_state(),
        snapshot: snapshot.checkpoint_state(),
        crawl_findings: crawl_findings.to_vec(),
        crawl_stats: crawl_stats.clone(),
    };
    fs::write(checkpoint_path, serde_json::to_string_pretty(&checkpoint)?)?;
    Ok(())
}

fn load_checkpoint(checkpoint_path: &Path, max_pages: usize) -> Result<RuntimeCheckpoint> {
    let text = fs::read_to_string(checkpoint_path)?;
    let mut checkpoint = serde_json::from_str::<RuntimeCheckpoint>(&text)?;
    checkpoint.planner.max_pages = max_pages;
    checkpoint.crawl_stats.max_pages = max_pages;
    Ok(checkpoint)
}

fn materialize_runtime_site(
    base_url: &str,
    max_pages: usize,
    engine: &str,
    config: &Config,
    options: &mut RuntimeAuditOptions<'_>,
) -> Result<(Site, Vec<Finding>, CrawlStats, Option<String>)> {
    let runtime = config.runtime();
    let (mut planner, mut snapshot, mut crawl_findings, mut crawl_stats) =
        if let Some(resume_from) = options.resume_from {
            let checkpoint = load_checkpoint(resume_from, max_pages)?;
            (
                CrawlPlanner::from_checkpoint(checkpoint.planner, max_pages),
                RuntimeSnapshotBuilder::from_state(checkpoint.snapshot),
                checkpoint.crawl_findings,
                checkpoint.crawl_stats,
            )
        } else {
            (
                CrawlPlanner::new(base_url, max_pages),
                RuntimeSnapshotBuilder::new(),
                Vec::new(),
                CrawlStats {
                    engine: engine.to_string(),
                    max_pages,
                    ..CrawlStats::default()
                },
            )
        };

    if options.resume_from.is_none() {
        for seed in runtime.crawl_seeds {
            planner.seed_from_user_input(seed, &runtime);
        }

        if runtime.crawl_use_sitemap {
            let mut visited_sitemaps = BTreeSet::new();
            let sitemap_base = planner.normalized_base().to_string();
            for sitemap_name in ["sitemap.xml", "sitemap-index.xml", "sitemap_index.xml"] {
                let _ = seed_routes_from_sitemap(
                    &mut planner,
                    &format!("{}{}", sitemap_base, sitemap_name),
                    &runtime,
                    &mut visited_sitemaps,
                );
            }
        }
    }

    let mut fetcher = RuntimeFetcher::new(engine, &runtime)?;
    while let Some(current) = planner.next_url(&runtime) {
        let fetched = match fetch_runtime_page(
            &mut fetcher,
            &current,
            options.fetch_retry_budget,
            &mut crawl_stats,
        ) {
            Ok(fetched) => fetched,
            Err(error) => {
                let route = route_from_urlish(&current).unwrap_or_default();
                crawl_stats.fetch_failures += 1;
                crawl_findings.push(Finding {
                    rule_id: "CRW001".to_string(),
                    message: format!("failed to fetch URL: {} ({})", current, error),
                    path: response_report_path(&route),
                    line: 1,
                    column: 1,
                    severity: "error".to_string(),
                    suggestion: None,
                    scope: FindingScope::Page,
                });
                continue;
            }
        };
        let Some(body) = fetched.body else {
            let route = route_from_urlish(&current).unwrap_or_default();
            crawl_stats.fetch_failures += 1;
            crawl_findings.push(Finding {
                rule_id: "CRW001".to_string(),
                message: format!("failed to fetch URL: {}", current),
                path: response_report_path(&route),
                line: 1,
                column: 1,
                severity: "error".to_string(),
                suggestion: None,
                scope: FindingScope::Page,
            });
            continue;
        };
        if !is_html_content_type(fetched.content_type.as_deref()) {
            crawl_stats.skipped_non_html += 1;
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

        crawl_stats.visited_pages = planner.visited_count();
        crawl_stats.discovered_internal_routes = planner.discovered_route_count();
        crawl_stats.queued_routes_remaining = planner.queued_count();
        crawl_stats.truncated = planner.truncated();
        emit_progress(
            options,
            RuntimeProgressEvent {
                phase: "progress".to_string(),
                engine: engine.to_string(),
                current_url: Some(fetched.effective_url),
                visited_pages: crawl_stats.visited_pages,
                discovered_internal_routes: crawl_stats.discovered_internal_routes,
                queued_routes_remaining: crawl_stats.queued_routes_remaining,
                fetch_failures: crawl_stats.fetch_failures,
                fetch_retries: crawl_stats.fetch_retries,
                skipped_non_html: crawl_stats.skipped_non_html,
                truncated: crawl_stats.truncated,
            },
        );
        if let Some(checkpoint_path) = options.checkpoint_path
            && crawl_stats.visited_pages % options.checkpoint_every.max(1) == 0
        {
            checkpoint_state(
                checkpoint_path,
                base_url,
                engine,
                &planner,
                &snapshot,
                &crawl_findings,
                &crawl_stats,
            )?;
        }
        if crawl_stats.visited_pages % options.checkpoint_every.max(1) == 0 {
            let artifact = build_partial_runtime_artifact(
                options.artifact_command,
                &snapshot,
                &planner,
                &crawl_findings,
                &crawl_stats,
                config,
            )?;
            emit_partial_artifact(options, &artifact)?;
        }
    }

    let _ = snapshot.capture_optional_artifacts(planner.normalized_base(), &runtime);
    let (site, snapshot_findings) = snapshot.finalize(
        planner.visited_count(),
        max_pages,
        planner.discovered_route_count(),
        planner.truncated(),
    )?;
    crawl_stats.visited_pages = planner.visited_count();
    crawl_stats.discovered_internal_routes = planner.discovered_route_count();
    crawl_stats.queued_routes_remaining = planner.queued_count();
    crawl_stats.truncated = planner.truncated();
    crawl_findings.extend(snapshot_findings);
    let truncation_reason = if planner.truncated() {
        Some(format!(
            "crawl stopped at max_pages={} after visiting {} pages while at least {} routes were discovered",
            max_pages,
            planner.visited_count(),
            planner.discovered_route_count()
        ))
    } else {
        None
    };
    if let Some(checkpoint_path) = options.checkpoint_path
        && site
            .crawl_meta
            .as_ref()
            .map(|meta| !meta.truncated)
            .unwrap_or(true)
    {
        let _ = fs::remove_file(checkpoint_path);
    }
    Ok((site, crawl_findings, crawl_stats, truncation_reason))
}

fn fetch_runtime_page(
    fetcher: &mut RuntimeFetcher,
    url: &str,
    fetch_retry_budget: usize,
    crawl_stats: &mut CrawlStats,
) -> Result<http::FetchResult> {
    let mut attempt = 0usize;
    loop {
        let result = fetcher.fetch(url);
        match result {
            Ok(fetched) => return Ok(fetched),
            Err(error) if attempt < fetch_retry_budget => {
                attempt += 1;
                crawl_stats.fetch_retries += 1;
                continue;
            }
            Err(error) => return Err(error),
        }
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
    run_runtime_audit_with_options(
        base_url,
        max_pages,
        engine,
        config,
        &mut RuntimeAuditOptions::default(),
    )
}

pub fn run_runtime_audit_with_options(
    base_url: &str,
    max_pages: usize,
    engine: &str,
    config: &Config,
    options: &mut RuntimeAuditOptions<'_>,
) -> Result<RuntimeAudit> {
    let effective_engine = resolve_runtime_engine(engine)?;
    let (site, crawl_findings, crawl_stats, truncation_reason) =
        materialize_runtime_site(base_url, max_pages, effective_engine, config, options)?;
    let mut findings = crawl_findings.clone();
    findings.extend(run_checks_for_site(&site, config));
    findings = apply_policy(findings, config);
    let status = if crawl_stats.truncated {
        AuditStatus::Partial
    } else {
        AuditStatus::Complete
    };
    emit_progress(
        options,
        RuntimeProgressEvent {
            phase: "complete".to_string(),
            engine: effective_engine.to_string(),
            current_url: None,
            visited_pages: crawl_stats.visited_pages,
            discovered_internal_routes: crawl_stats.discovered_internal_routes,
            queued_routes_remaining: crawl_stats.queued_routes_remaining,
            fetch_failures: crawl_stats.fetch_failures,
            fetch_retries: crawl_stats.fetch_retries,
            skipped_non_html: crawl_stats.skipped_non_html,
            truncated: crawl_stats.truncated,
        },
    );
    Ok(RuntimeAudit {
        site,
        crawl_findings,
        findings,
        status,
        crawl_stats,
        truncation_reason,
    })
}

pub fn verify_runtime_audit(audit: &RuntimeAudit, baseline_findings: &[Finding]) -> DiffResult {
    diff_finding_sets(baseline_findings, &audit.findings)
}

pub fn runtime_doctor() -> PlaywrightDoctor {
    probe_playwright_runtime()
}

#[cfg(test)]
mod tests {
    use super::{
        RuntimeArtifactMode, RuntimeAuditOptions, RuntimeProgressMode, run_runtime_audit,
        run_runtime_audit_with_options, verify_runtime_audit,
    };
    use crate::config::{Config, default_rule_switches};
    use seogeo_contracts::{AuditStatus, CrawlStats, Finding, FindingScope};
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
                scope: FindingScope::Page,
            }],
            status: AuditStatus::Complete,
            crawl_stats: CrawlStats {
                engine: "http".to_string(),
                max_pages: 10,
                ..CrawlStats::default()
            },
            truncation_reason: None,
        };
        let diff = verify_runtime_audit(&audit, &[]);
        assert_eq!(diff.new_findings.len(), 1);
    }

    #[test]
    fn runtime_audit_reports_truncated_crawl_coverage() {
        let (base_url, handle) = spawn_server(1);
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
    fn runtime_audit_emits_partial_artifacts_during_checkpoint_flushes() {
        let (base_url, handle) = spawn_server(6);
        let mut partial_statuses = Vec::new();
        let mut artifact_callback =
            |artifact: &seogeo_contracts::AuditArtifact| -> anyhow::Result<()> {
                partial_statuses.push(artifact.status);
                Ok(())
            };
        let mut options = RuntimeAuditOptions {
            checkpoint_every: 1,
            progress: RuntimeProgressMode::Off,
            artifact_command: "crawl",
            partial_artifact: RuntimeArtifactMode::Callback(&mut artifact_callback),
            ..RuntimeAuditOptions::default()
        };

        let audit =
            run_runtime_audit_with_options(&base_url, 1, "http", &html_only_config(), &mut options)
                .unwrap();

        assert_eq!(audit.status, AuditStatus::Partial);
        assert!(!partial_statuses.is_empty());
        assert!(
            partial_statuses
                .iter()
                .all(|status| *status == AuditStatus::Partial)
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
    fn runtime_audit_reuses_playwright_session_across_multiple_pages() {
        if !super::playwright_is_available() {
            return;
        }
        let (base_url, handle) = spawn_server(5);
        let audit = run_runtime_audit(&base_url, 2, "playwright", &html_only_config()).unwrap();
        assert!(audit.site.route_pages.contains_key(""));
        assert!(audit.site.route_pages.contains_key("about"));
        assert!(
            !audit
                .findings
                .iter()
                .any(|finding| finding.rule_id == "CRW001")
        );
        handle.join().unwrap();
    }

    #[test]
    fn runtime_audit_rejects_unknown_engine() {
        let error = run_runtime_audit("https://example.com", 10, "invalid", &Config::default())
            .expect_err("invalid engines should fail");
        assert!(error.to_string().contains("unknown runtime engine"));
    }
}

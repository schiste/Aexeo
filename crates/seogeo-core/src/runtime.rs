use anyhow::{Context, Result};
use seogeo_contracts::Finding;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::config::Config;
use crate::policy::apply_policy;
use crate::site::{
    CrawlMeta, DeploymentModel, Site, load_site, normalize_internal_href, route_from_urlish,
};
use crate::static_check::run_checks_for_site;
use crate::verification::{DiffResult, diff_finding_sets};

#[derive(Debug, Clone)]
pub struct RuntimeAudit {
    pub site: Site,
    pub crawl_findings: Vec<Finding>,
    pub findings: Vec<Finding>,
}

#[derive(Debug, Clone)]
struct FetchResult {
    status_code: Option<u16>,
    content_type: Option<String>,
    body: Option<String>,
    headers: BTreeMap<String, String>,
    effective_url: String,
}

const ASSET_EXTENSIONS: &[&str] = &[
    ".css", ".gif", ".html", ".ico", ".jpeg", ".jpg", ".js", ".json", ".mjs", ".png", ".svg",
    ".txt", ".webp", ".xml",
];

fn unique_runtime_dir() -> Result<PathBuf> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let path =
        std::env::temp_dir().join(format!("seogeo-runtime-{}-{}", std::process::id(), nonce));
    fs::create_dir_all(&path)?;
    Ok(path)
}

fn normalize_base_url(base_url: &str) -> String {
    format!("{}/", base_url.trim_end_matches('/'))
}

fn host_for_url(url: &str) -> String {
    let after_scheme = url.split_once("://").map(|(_, rest)| rest).unwrap_or(url);
    after_scheme
        .split('/')
        .next()
        .unwrap_or_default()
        .to_string()
}

fn response_report_path(route: &str) -> String {
    if route.is_empty() {
        "crawl/index.html".to_string()
    } else {
        format!("crawl/{}/index.html", route)
    }
}

fn snapshot_path_for_route(root: &Path, route: &str) -> PathBuf {
    if route.is_empty() {
        root.join("index.html")
    } else {
        root.join(route).join("index.html")
    }
}

fn attr_value(snippet: &str, attr: &str) -> Option<String> {
    let lower = snippet.to_ascii_lowercase();
    let marker = format!("{}=", attr.to_ascii_lowercase());
    let index = lower.find(&marker)?;
    let after = &snippet[index + marker.len()..];
    let mut chars = after.chars();
    let quote = chars.next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let rest = &after[quote.len_utf8()..];
    let end = rest.find(quote)?;
    Some(rest[..end].to_string())
}

fn extract_internal_links(raw: &str) -> Vec<String> {
    let mut links = Vec::new();
    let mut offset = 0;
    while let Some(index) = raw[offset..].find("<a") {
        let start = offset + index;
        let Some(open_end_rel) = raw[start..].find('>') else {
            break;
        };
        let open_end = start + open_end_rel;
        let snippet = &raw[start..=open_end];
        if let Some(href) = attr_value(snippet, "href")
            && let Some(target) = normalize_internal_href(&href)
        {
            links.push(target);
        }
        offset = open_end + 1;
    }
    links
}

fn should_enqueue_link(target: &str) -> bool {
    let suffix = Path::new(target)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| format!(".{}", ext.to_ascii_lowercase()))
        .unwrap_or_default();
    suffix.is_empty() || !ASSET_EXTENSIONS.contains(&suffix.as_str()) || suffix == ".html"
}

fn read_loc_values(text: &str) -> Vec<String> {
    let mut values = Vec::new();
    let mut offset = 0;
    while let Some(start_rel) = text[offset..].find("<loc>") {
        let start = offset + start_rel + 5;
        let Some(end_rel) = text[start..].find("</loc>") else {
            break;
        };
        let end = start + end_rel;
        values.push(text[start..end].trim().to_string());
        offset = end + 6;
    }
    values
}

fn route_matches_patterns(route: &str, patterns: &[String]) -> bool {
    patterns.is_empty()
        || patterns
            .iter()
            .any(|pattern| route.contains(pattern.trim_matches('/')))
}

fn route_is_excluded(route: &str, patterns: &[String]) -> bool {
    patterns
        .iter()
        .any(|pattern| route.contains(pattern.trim_matches('/')))
}

fn route_is_allowed(route: &str, config: &Config) -> bool {
    !route_is_excluded(route, &config.crawl_exclude_patterns)
        && route_matches_patterns(route, &config.crawl_include_patterns)
}

fn parse_headers(raw: &str) -> BTreeMap<String, String> {
    let mut headers = BTreeMap::new();
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("HTTP/") {
            continue;
        }
        if let Some((key, value)) = trimmed.split_once(':') {
            headers.insert(key.trim().to_ascii_lowercase(), value.trim().to_string());
        }
    }
    headers
}

fn fetch_with_curl(
    url: &str,
    headers: &BTreeMap<String, String>,
    basic_auth: &BTreeMap<String, String>,
) -> Result<FetchResult> {
    let body_path = unique_runtime_dir()?.join("body.txt");
    let headers_path = unique_runtime_dir()?.join("headers.txt");
    let mut command = ProcessCommand::new("curl");
    command.arg("-sS").arg("-L");
    command.arg("-D").arg(&headers_path);
    command.arg("-o").arg(&body_path);
    command
        .arg("-w")
        .arg("%{http_code}\n%{content_type}\n%{url_effective}");
    for (key, value) in headers {
        command.arg("-H").arg(format!("{}: {}", key, value));
    }
    if let (Some(username), Some(password)) =
        (basic_auth.get("username"), basic_auth.get("password"))
    {
        command.arg("-u").arg(format!("{}:{}", username, password));
    }
    command.arg(url);
    let output = command
        .output()
        .with_context(|| format!("failed to run curl for {}", url))?;
    let metadata = String::from_utf8_lossy(&output.stdout);
    let mut lines = metadata.lines();
    let status_code = lines.next().and_then(|value| value.parse::<u16>().ok());
    let content_type = lines
        .next()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let effective_url = lines.next().unwrap_or(url).trim().to_string();
    let body = fs::read_to_string(&body_path).ok();
    let headers_map = fs::read_to_string(&headers_path)
        .map(|raw| parse_headers(&raw))
        .unwrap_or_default();
    let _ = fs::remove_file(body_path);
    let _ = fs::remove_file(headers_path);
    if !output.status.success() && body.is_none() {
        return Ok(FetchResult {
            status_code,
            content_type: None,
            body: None,
            headers: BTreeMap::new(),
            effective_url,
        });
    }
    Ok(FetchResult {
        status_code,
        content_type,
        body,
        headers: headers_map,
        effective_url,
    })
}

fn write_optional_artifact(
    snapshot_root: &Path,
    base_url: &str,
    name: &str,
    headers: &BTreeMap<String, String>,
    basic_auth: &BTreeMap<String, String>,
) -> Result<()> {
    let artifact_url = format!("{}{}", base_url, name);
    let fetched = fetch_with_curl(&artifact_url, headers, basic_auth)?;
    if let Some(body) = fetched.body
        && fetched.status_code.unwrap_or(200) < 400
    {
        fs::write(snapshot_root.join(name), body)?;
    }
    Ok(())
}

fn materialize_runtime_site(
    base_url: &str,
    max_pages: usize,
    engine: &str,
    config: &Config,
) -> Result<(Site, Vec<Finding>)> {
    let normalized_base = normalize_base_url(base_url);
    let base_host = host_for_url(&normalized_base);
    let snapshot_root = unique_runtime_dir()?;
    let mut queue = VecDeque::from([normalized_base.clone()]);
    let mut visited = BTreeSet::new();
    let mut discovered_routes = BTreeSet::from([String::new()]);
    let mut response_headers = BTreeMap::<String, BTreeMap<String, String>>::new();
    let mut crawl_findings = Vec::new();
    let mut truncated = false;

    if engine == "playwright" {
        crawl_findings.push(Finding {
            rule_id: "CRW002".to_string(),
            message:
                "Playwright crawl requested but native Rust runtime currently uses HTTP fallback"
                    .to_string(),
            path: "crawl/index.html".to_string(),
            line: 1,
            column: 1,
            severity: "warning".to_string(),
            suggestion: None,
        });
    }

    for seed in &config.crawl_seeds {
        let Some(route) = route_from_urlish(seed).or_else(|| normalize_internal_href(seed)) else {
            continue;
        };
        if route_is_allowed(&route, config) {
            let seed_url = if route.is_empty() {
                normalized_base.clone()
            } else {
                format!("{}{}", normalized_base, route)
            };
            queue.push_back(seed_url);
            discovered_routes.insert(route);
        }
    }

    if config.crawl_use_sitemap {
        let sitemap_url = format!("{}sitemap.xml", normalized_base);
        let fetched = fetch_with_curl(
            &sitemap_url,
            &config.crawl_headers,
            &config.crawl_basic_auth,
        )?;
        if fetched.status_code.unwrap_or(500) < 400
            && fetched
                .content_type
                .as_deref()
                .unwrap_or_default()
                .contains("xml")
            && let Some(body) = fetched.body
        {
            for loc in read_loc_values(&body) {
                let Some(route) = route_from_urlish(&loc) else {
                    continue;
                };
                if route_is_allowed(&route, config) {
                    let seed_url = if route.is_empty() {
                        normalized_base.clone()
                    } else {
                        format!("{}{}", normalized_base, route)
                    };
                    queue.push_back(seed_url);
                    discovered_routes.insert(route);
                }
            }
        }
    }

    while let Some(current) = queue.pop_front() {
        if visited.len() >= max_pages {
            truncated = true;
            break;
        }
        if visited.contains(&current) {
            continue;
        }
        visited.insert(current.clone());
        let current_route = route_from_urlish(&current).unwrap_or_default();
        if !current_route.is_empty() && !route_is_allowed(&current_route, config) {
            continue;
        }
        let fetched = fetch_with_curl(&current, &config.crawl_headers, &config.crawl_basic_auth)?;
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
        if fetched.content_type.as_deref().unwrap_or_default() != "text/html" {
            continue;
        }
        let effective_host = host_for_url(&fetched.effective_url);
        if effective_host != base_host {
            continue;
        }
        let route = route_from_urlish(&fetched.effective_url).unwrap_or_default();
        let page_path = snapshot_path_for_route(&snapshot_root, &route);
        if let Some(parent) = page_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&page_path, &body)?;
        response_headers.insert(route.clone(), fetched.headers.clone());

        for target in extract_internal_links(&body) {
            if !should_enqueue_link(&target) {
                continue;
            }
            if !route_is_allowed(&target, config) {
                continue;
            }
            discovered_routes.insert(target.clone());
            let child = if target.is_empty() {
                normalized_base.clone()
            } else {
                format!("{}{}", normalized_base, target)
            };
            if host_for_url(&child) == base_host {
                queue.push_back(child);
            }
        }
    }

    write_optional_artifact(
        &snapshot_root,
        &normalized_base,
        "robots.txt",
        &config.crawl_headers,
        &config.crawl_basic_auth,
    )?;
    write_optional_artifact(
        &snapshot_root,
        &normalized_base,
        "llms.txt",
        &config.crawl_headers,
        &config.crawl_basic_auth,
    )?;
    write_optional_artifact(
        &snapshot_root,
        &normalized_base,
        "sitemap.xml",
        &config.crawl_headers,
        &config.crawl_basic_auth,
    )?;

    let mut site = load_site(&snapshot_root)?;
    site.root = PathBuf::from("crawl");
    site.deployment_model = DeploymentModel::RuntimeSnapshot;
    site.deployment_markers = vec!["runtime crawl snapshot".to_string()];
    site.crawl_meta = Some(CrawlMeta {
        visited_pages: visited.len(),
        max_pages,
        discovered_internal_routes: discovered_routes.len(),
        truncated,
    });
    for page in site.pages.iter_mut() {
        page.path = PathBuf::from(response_report_path(&page.route));
        if let Some(headers) = response_headers.get(&page.route) {
            page.response_headers = headers.clone();
        }
    }
    site.route_pages = site
        .pages
        .iter()
        .cloned()
        .map(|page| (page.route.clone(), page))
        .collect();
    if let Some(crawl_meta) = &site.crawl_meta
        && crawl_meta.truncated
    {
        crawl_findings.push(Finding {
            rule_id: "CRW003".to_string(),
            message: format!(
                "crawl stopped at max_pages={} after visiting {} pages; discovered at least {} internal routes, so graph-dependent findings may be incomplete",
                crawl_meta.max_pages, crawl_meta.visited_pages, crawl_meta.discovered_internal_routes
            ),
            path: "crawl/index.html".to_string(),
            line: 1,
            column: 1,
            severity: "warning".to_string(),
            suggestion: Some("increase --max-pages for a more complete runtime audit".to_string()),
        });
    }
    let _ = fs::remove_dir_all(&snapshot_root);
    Ok((site, crawl_findings))
}

pub fn run_runtime_audit(
    base_url: &str,
    max_pages: usize,
    engine: &str,
    config: &Config,
) -> Result<RuntimeAudit> {
    let effective_engine = if engine == "auto" { "http" } else { engine };
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
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::path::PathBuf;
    use std::thread;

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
            }
        });
        (format!("http://{}", address), handle)
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
        let (base_url, handle) = spawn_server(5);
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
        let (base_url, handle) = spawn_server(4);
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
}

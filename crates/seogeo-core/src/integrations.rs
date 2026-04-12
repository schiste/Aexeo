use anyhow::{Context, Result, bail};
use csv::ReaderBuilder;
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use std::time::Duration;
use url::Url;

use crate::config::{Config, load_config};
use crate::reporting::rule_group_name;
use crate::site::{Page, Site, build_page_from_source, load_site, route_from_urlish};
use crate::static_check::run_native_static_audit_with_config;
use crate::verification::load_audit_artifact;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SnippetInspection {
    pub target: String,
    pub route: String,
    pub canonical: Option<String>,
    pub meta_robots: Option<String>,
    pub x_robots_tag: Option<String>,
    pub directives: Vec<String>,
    pub data_nosnippet_blocks: usize,
    pub snippet_blocked: bool,
    pub restrictive_max_snippet: Option<String>,
    pub observations: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IndexNowValidation {
    pub site_url: String,
    pub host: String,
    pub key: String,
    pub key_location: String,
    pub key_file_path: Option<String>,
    pub key_file_present: bool,
    pub key_file_matches: bool,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IndexNowSubmission {
    pub endpoint: String,
    pub host: String,
    pub submitted_urls: usize,
    pub key_location: String,
    pub status_code: u16,
    pub success: bool,
    pub response_body: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BingAiRecord {
    pub url: String,
    pub route: String,
    pub query: Option<String>,
    pub citations: u64,
    pub clicks: Option<u64>,
    pub impressions: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BingAiUrlSummary {
    pub url: String,
    pub route: String,
    pub citations: u64,
    pub rows: usize,
    pub audit_findings: usize,
    pub audit_errors: usize,
    pub audit_warnings: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BingAiImportReport {
    pub rows_read: usize,
    pub urls_seen: usize,
    pub cited_urls: Vec<BingAiUrlSummary>,
    pub unmatched_urls: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SearchConsoleExportRow {
    pub route: String,
    pub url: Option<String>,
    pub findings: usize,
    pub errors: usize,
    pub warnings: usize,
    pub heuristic: usize,
    pub rule_groups: Vec<String>,
    pub rule_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PublishHookReport {
    pub changed_routes: Vec<String>,
    pub finding_count: usize,
    pub findings_by_route: BTreeMap<String, usize>,
    pub search_console_rows: Vec<SearchConsoleExportRow>,
    pub indexnow_validation: Option<IndexNowValidation>,
    pub indexnow_submission: Option<IndexNowSubmission>,
}

fn normalize_directives(page: &Page) -> BTreeSet<String> {
    page.metadata("robots")
        .into_iter()
        .chain(
            page.response_headers
                .get("x-robots-tag")
                .map(String::as_str),
        )
        .flat_map(|value| value.split(','))
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty())
        .collect()
}

fn restrictive_max_snippet(directives: &BTreeSet<String>) -> Option<String> {
    directives.iter().find_map(|directive| {
        let value = directive.strip_prefix("max-snippet:")?.trim();
        let parsed = value.parse::<i64>().ok()?;
        (parsed <= 0).then(|| directive.clone())
    })
}

fn normalize_audit_path_to_route(path: &str) -> Option<String> {
    let normalized = path.replace('\\', "/");
    let trimmed = normalized
        .strip_prefix("crawl/")
        .or_else(|| normalized.strip_prefix("./crawl/"))
        .unwrap_or(&normalized);
    if trimmed == "index.html" {
        return Some(String::new());
    }
    if let Some(prefix) = trimmed.strip_suffix("/index.html") {
        return Some(prefix.to_string());
    }
    if let Some(prefix) = trimmed.strip_suffix(".html") {
        return Some(prefix.to_string());
    }
    route_from_urlish(trimmed)
}

fn inspection_from_page(target: &str, page: &Page) -> SnippetInspection {
    let directives = normalize_directives(page);
    let restrictive = restrictive_max_snippet(&directives);
    let data_nosnippet_blocks = page.raw_text.matches("data-nosnippet").count();
    let snippet_blocked = directives.contains("nosnippet") || restrictive.is_some();
    let mut observations = Vec::new();
    if directives.contains("nosnippet") {
        observations.push("snippet reuse is explicitly blocked by nosnippet".to_string());
    }
    if let Some(value) = &restrictive {
        observations.push(format!("snippet length is restricted by {}", value));
    }
    if data_nosnippet_blocks > 0 {
        observations.push(format!(
            "{} block(s) opt out of snippet extraction via data-nosnippet",
            data_nosnippet_blocks
        ));
    }
    SnippetInspection {
        target: target.to_string(),
        route: page.route.clone(),
        canonical: page.canonical.clone(),
        meta_robots: page.metadata("robots").map(str::to_string),
        x_robots_tag: page.response_headers.get("x-robots-tag").cloned(),
        directives: directives.into_iter().collect(),
        data_nosnippet_blocks,
        snippet_blocked,
        restrictive_max_snippet: restrictive,
        observations,
    }
}

pub fn inspect_snippet_controls_site(site: &Site, route: &str) -> Result<SnippetInspection> {
    let page = site
        .page(route)
        .ok_or_else(|| anyhow::anyhow!("route '{}' was not found in the loaded site", route))?;
    Ok(inspection_from_page(route, page))
}

pub fn inspect_snippet_controls_path(
    root: &Path,
    explicit_config_path: Option<&Path>,
    route: &str,
) -> Result<SnippetInspection> {
    let config = load_config(root, explicit_config_path)?;
    inspect_snippet_controls_with_config(root, &config, route)
}

pub fn inspect_snippet_controls_with_config(
    root: &Path,
    config: &Config,
    route: &str,
) -> Result<SnippetInspection> {
    let site_root = crate::adapter::resolve_static_site_root(root, config)?;
    let site = load_site(&site_root)?;
    inspect_snippet_controls_site(&site, route)
}

pub fn inspect_snippet_controls_url(url: &str) -> Result<SnippetInspection> {
    let client = Client::builder().timeout(Duration::from_secs(30)).build()?;
    let response = client
        .get(url)
        .send()
        .with_context(|| format!("failed to fetch URL: {url}"))?;
    let effective_url = response.url().to_string();
    let headers = response
        .headers()
        .iter()
        .filter_map(|(key, value)| {
            Some((
                key.as_str().to_ascii_lowercase(),
                value.to_str().ok()?.to_string(),
            ))
        })
        .collect::<BTreeMap<_, _>>();
    let body = response.text().unwrap_or_default();
    let route = route_from_urlish(&effective_url).unwrap_or_default();
    let audit_path = if route.is_empty() {
        "index.html".to_string()
    } else {
        format!("{route}/index.html")
    };
    let page = build_page_from_source(
        Path::new("crawl").join(&audit_path),
        audit_path,
        body,
        headers,
    );
    Ok(inspection_from_page(url, &page))
}

fn indexnow_host(site_url: &str) -> Result<String> {
    let parsed = Url::parse(site_url)?;
    parsed
        .host_str()
        .map(str::to_string)
        .ok_or_else(|| anyhow::anyhow!("site URL '{}' is missing a host", site_url))
}

fn validate_indexnow_key_format(key: &str) -> Vec<String> {
    let mut errors = Vec::new();
    if key.trim().is_empty() {
        errors.push("IndexNow key must not be empty".to_string());
    }
    if !key
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    {
        errors.push("IndexNow key must use URL-safe ASCII characters only".to_string());
    }
    errors
}

pub fn validate_indexnow(
    site_url: &str,
    key: &str,
    root: Option<&Path>,
) -> Result<IndexNowValidation> {
    let host = indexnow_host(site_url)?;
    let key_location = format!("{}/{}.txt", site_url.trim_end_matches('/'), key);
    let key_file_path = root.map(|value| value.join(format!("{key}.txt")));
    let (key_file_present, key_file_matches, file_warning) = if let Some(path) = &key_file_path {
        if !path.exists() {
            (
                false,
                false,
                Some(format!("missing key file at {}", path.display())),
            )
        } else {
            let content = fs::read_to_string(path).unwrap_or_default();
            (true, content.trim() == key, None)
        }
    } else {
        (false, false, None)
    };
    let mut errors = validate_indexnow_key_format(key);
    let mut warnings = Vec::new();
    if let Some(warning) = file_warning {
        warnings.push(warning);
    }
    if key_file_present && !key_file_matches {
        errors.push(
            "IndexNow key file exists but its contents do not match the provided key".to_string(),
        );
    }
    Ok(IndexNowValidation {
        site_url: site_url.to_string(),
        host,
        key: key.to_string(),
        key_location,
        key_file_path: key_file_path.map(|path| path.to_string_lossy().into_owned()),
        key_file_present,
        key_file_matches,
        errors,
        warnings,
    })
}

pub fn submit_indexnow(
    endpoint: &str,
    site_url: &str,
    key: &str,
    urls: &[String],
) -> Result<IndexNowSubmission> {
    if urls.is_empty() {
        bail!("at least one URL is required for IndexNow submission");
    }
    let validation = validate_indexnow(site_url, key, None)?;
    if !validation.errors.is_empty() {
        bail!(
            "IndexNow validation failed: {}",
            validation.errors.join("; ")
        );
    }
    let client = Client::builder().timeout(Duration::from_secs(30)).build()?;
    let payload = json!({
        "host": validation.host,
        "key": key,
        "keyLocation": validation.key_location,
        "urlList": urls,
    });
    let response = client
        .post(endpoint)
        .header("content-type", "application/json")
        .body(payload.to_string())
        .send()
        .with_context(|| format!("failed to submit IndexNow payload to {}", endpoint))?;
    let status_code = response.status().as_u16();
    let response_body = response
        .text()
        .ok()
        .filter(|text: &String| !text.trim().is_empty());
    Ok(IndexNowSubmission {
        endpoint: endpoint.to_string(),
        host: validation.host,
        submitted_urls: urls.len(),
        key_location: validation.key_location,
        status_code,
        success: (200..300).contains(&status_code),
        response_body,
    })
}

fn normalize_export_key(key: &str) -> String {
    key.to_ascii_lowercase()
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .collect()
}

fn first_string<'a>(record: &'a BTreeMap<String, String>, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|key| record.get(&normalize_export_key(key)).map(String::as_str))
        .filter(|value| !value.trim().is_empty())
}

fn first_u64(record: &BTreeMap<String, String>, keys: &[&str]) -> Option<u64> {
    first_string(record, keys).and_then(|value| value.trim().parse::<u64>().ok())
}

fn parse_bing_record(map: &BTreeMap<String, String>) -> Option<BingAiRecord> {
    let url = first_string(map, &["url", "page", "landing_page", "source_url"])?;
    let route = route_from_urlish(url)?;
    Some(BingAiRecord {
        url: url.to_string(),
        route,
        query: first_string(map, &["query", "prompt", "question"]).map(str::to_string),
        citations: first_u64(
            map,
            &["citations", "citation_count", "answers", "appearances"],
        )
        .unwrap_or(1),
        clicks: first_u64(map, &["clicks"]),
        impressions: first_u64(map, &["impressions", "views"]),
    })
}

fn parse_bing_csv(path: &Path) -> Result<Vec<BingAiRecord>> {
    let mut reader = ReaderBuilder::new().flexible(true).from_path(path)?;
    let headers = reader
        .headers()?
        .iter()
        .map(normalize_export_key)
        .collect::<Vec<_>>();
    let mut records = Vec::new();
    for row in reader.records() {
        let row = row?;
        let mut map = BTreeMap::new();
        for (index, value) in row.iter().enumerate() {
            if let Some(header) = headers.get(index) {
                map.insert(header.clone(), value.to_string());
            }
        }
        if let Some(record) = parse_bing_record(&map) {
            records.push(record);
        }
    }
    Ok(records)
}

fn parse_bing_json(path: &Path) -> Result<Vec<BingAiRecord>> {
    let payload = serde_json::from_str::<Value>(&fs::read_to_string(path)?)?;
    let items = match payload {
        Value::Array(items) => items,
        Value::Object(map) => map
            .get("rows")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default(),
        _ => Vec::new(),
    };
    let mut records = Vec::new();
    for item in items {
        let Value::Object(map) = item else {
            continue;
        };
        let normalized = map
            .into_iter()
            .map(|(key, value)| {
                (
                    normalize_export_key(&key),
                    match value {
                        Value::String(text) => text,
                        other => other.to_string(),
                    },
                )
            })
            .collect::<BTreeMap<_, _>>();
        if let Some(record) = parse_bing_record(&normalized) {
            records.push(record);
        }
    }
    Ok(records)
}

pub fn import_bing_ai_export(path: &Path, audit_path: Option<&Path>) -> Result<BingAiImportReport> {
    let records = match path.extension().and_then(|ext| ext.to_str()) {
        Some("json") => parse_bing_json(path)?,
        _ => parse_bing_csv(path)?,
    };
    let mut audit_index = BTreeMap::<String, (usize, usize, usize)>::new();
    if let Some(audit_path) = audit_path {
        let artifact = load_audit_artifact(audit_path)?;
        for finding in artifact.findings {
            if let Some(route) = normalize_audit_path_to_route(&finding.path) {
                let entry = audit_index.entry(route).or_insert((0, 0, 0));
                entry.0 += 1;
                if finding.is_error() {
                    entry.1 += 1;
                } else {
                    entry.2 += 1;
                }
            }
        }
    }

    let mut per_url = BTreeMap::<String, BingAiUrlSummary>::new();
    for record in &records {
        let audit_counts = audit_index.get(&record.route).copied().unwrap_or((0, 0, 0));
        let entry = per_url
            .entry(record.url.clone())
            .or_insert(BingAiUrlSummary {
                url: record.url.clone(),
                route: record.route.clone(),
                citations: 0,
                rows: 0,
                audit_findings: audit_counts.0,
                audit_errors: audit_counts.1,
                audit_warnings: audit_counts.2,
            });
        entry.citations = entry.citations.saturating_add(record.citations);
        entry.rows += 1;
    }
    let unmatched_urls = per_url
        .values()
        .filter(|summary| !audit_index.contains_key(&summary.route))
        .map(|summary| summary.url.clone())
        .collect::<Vec<_>>();
    let mut cited_urls = per_url.into_values().collect::<Vec<_>>();
    cited_urls.sort_by(|left, right| {
        right
            .citations
            .cmp(&left.citations)
            .then_with(|| left.url.cmp(&right.url))
    });
    Ok(BingAiImportReport {
        rows_read: records.len(),
        urls_seen: cited_urls.len(),
        cited_urls,
        unmatched_urls,
    })
}

pub fn export_search_console_rows(
    audit_path: &Path,
    site_url: Option<&str>,
) -> Result<Vec<SearchConsoleExportRow>> {
    let artifact = load_audit_artifact(audit_path)?;
    let mut rows = BTreeMap::<String, SearchConsoleExportRow>::new();
    for finding in artifact.findings {
        let Some(route) = normalize_audit_path_to_route(&finding.path) else {
            continue;
        };
        let row = rows
            .entry(route.clone())
            .or_insert_with(|| SearchConsoleExportRow {
                route: route.clone(),
                url: site_url.map(|base| {
                    if route.is_empty() {
                        format!("{}/", base.trim_end_matches('/'))
                    } else {
                        format!("{}/{}", base.trim_end_matches('/'), route)
                    }
                }),
                findings: 0,
                errors: 0,
                warnings: 0,
                heuristic: 0,
                rule_groups: Vec::new(),
                rule_ids: Vec::new(),
            });
        row.findings += 1;
        if finding.is_error() {
            row.errors += 1;
        } else {
            row.warnings += 1;
        }
        if finding.rule_id.starts_with("GEO") {
            row.heuristic += 1;
        }
        let group = rule_group_name(&finding.rule_id).to_string();
        if !row.rule_groups.contains(&group) {
            row.rule_groups.push(group);
        }
        if !row.rule_ids.contains(&finding.rule_id) {
            row.rule_ids.push(finding.rule_id);
        }
    }
    let mut rows = rows.into_values().collect::<Vec<_>>();
    rows.sort_by(|left, right| left.route.cmp(&right.route));
    Ok(rows)
}

pub fn build_publish_hook_report(
    root: &Path,
    explicit_config_path: Option<&Path>,
    changed_urls: &[String],
    indexnow_key: Option<&str>,
    submit_indexnow_request: bool,
    indexnow_endpoint: &str,
) -> Result<PublishHookReport> {
    let config = load_config(root, explicit_config_path)?;
    build_publish_hook_report_with_config(
        root,
        &config,
        changed_urls,
        indexnow_key,
        submit_indexnow_request,
        indexnow_endpoint,
    )
}

pub fn build_publish_hook_report_with_config(
    root: &Path,
    config: &Config,
    changed_urls: &[String],
    indexnow_key: Option<&str>,
    submit_indexnow_request: bool,
    indexnow_endpoint: &str,
) -> Result<PublishHookReport> {
    let findings = run_native_static_audit_with_config(root, config)?;
    let changed_routes = changed_urls
        .iter()
        .filter_map(|url| route_from_urlish(url))
        .collect::<Vec<_>>();
    let findings_by_route = changed_routes
        .iter()
        .map(|route| {
            let count = findings
                .iter()
                .filter(|finding| {
                    normalize_audit_path_to_route(&finding.path)
                        .is_some_and(|value| value == *route)
                })
                .count();
            (route.clone(), count)
        })
        .collect::<BTreeMap<_, _>>();
    let publish_artifact = crate::reporting::build_audit_artifact(
        "publish-hook",
        &findings,
        seogeo_contracts::AuditStatus::Complete,
        None,
        None,
    );
    let audit_path = crate::reporting::write_audit_artifact(
        &publish_artifact,
        root,
        "publish-hook",
        config.output().audit_log_limit,
    )?;
    let search_console_rows = export_search_console_rows(&audit_path, config.site().site_url)?
        .into_iter()
        .filter(|row| changed_routes.iter().any(|route| route == &row.route))
        .collect::<Vec<_>>();
    let indexnow_validation = match (config.site().site_url, indexnow_key) {
        (Some(site_url), Some(key)) => Some(validate_indexnow(site_url, key, Some(root))?),
        _ => None,
    };
    let indexnow_submission =
        if submit_indexnow_request && !changed_urls.is_empty() && indexnow_validation.is_some() {
            Some(submit_indexnow(
                indexnow_endpoint,
                config.site().site_url.unwrap_or_default(),
                indexnow_key.unwrap_or_default(),
                changed_urls,
            )?)
        } else {
            None
        };
    Ok(PublishHookReport {
        changed_routes,
        finding_count: findings.len(),
        findings_by_route,
        search_console_rows,
        indexnow_validation,
        indexnow_submission,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        build_publish_hook_report, export_search_console_rows, import_bing_ai_export,
        inspect_snippet_controls_path, submit_indexnow, validate_indexnow,
    };
    use std::fs;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    fn write(path: &std::path::Path, text: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, text).unwrap();
    }

    #[test]
    fn validates_indexnow_key_files() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        write(&root.join("abc123.txt"), "abc123");
        let result = validate_indexnow("https://example.com", "abc123", Some(root)).unwrap();
        assert!(result.errors.is_empty());
        assert!(result.key_file_present);
        assert!(result.key_file_matches);
    }

    #[test]
    fn imports_bing_ai_csv_exports() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("bing-ai.csv");
        write(
            &path,
            "url,query,citations\nhttps://example.com/docs,what is docs,3\nhttps://example.com/docs,another,2\n",
        );
        let report = import_bing_ai_export(&path, None).unwrap();
        assert_eq!(report.rows_read, 2);
        assert_eq!(report.urls_seen, 1);
        assert_eq!(report.cited_urls[0].citations, 5);
    }

    #[test]
    fn inspects_snippet_controls_from_static_site() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        write(
            &root.join("index.html"),
            "<html><head><title>x</title><meta name=\"description\" content=\"y\"><meta name=\"robots\" content=\"nosnippet,max-snippet:0\"></head><body><h1>x</h1><div data-nosnippet=\"true\">x</div></body></html>",
        );
        let inspection = inspect_snippet_controls_path(root, None, "").unwrap();
        assert!(inspection.snippet_blocked);
        assert_eq!(inspection.data_nosnippet_blocks, 1);
    }

    #[test]
    fn exports_search_console_rows_from_audit() {
        let temp_dir = tempfile::tempdir().unwrap();
        let audit_path = temp_dir.path().join("audit.json");
        fs::write(
            &audit_path,
            r#"{"version":2,"command":"check","status":"complete","generated_at":0,"summary":{"total":1,"errors":1,"warnings":0,"actionable":1,"heuristic":0},"findings":[{"rule_id":"SEO001","message":"missing <title>","path":"crawl/about/index.html","line":1,"column":1,"severity":"error","scope":"page"}]}"#,
        )
        .unwrap();
        let rows = export_search_console_rows(&audit_path, Some("https://example.com")).unwrap();
        assert_eq!(rows[0].route, "about");
        assert_eq!(rows[0].errors, 1);
    }

    #[test]
    fn builds_publish_hook_reports() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        write(
            &root.join("seogeo.toml"),
            "version = 1\n[site]\nurl = \"https://example.com\"\nsource_dir = \".\"\n",
        );
        write(
            &root.join("index.html"),
            "<html><head><meta name=\"description\" content=\"x\"></head><body><h1>x</h1></body></html>",
        );
        write(&root.join("abc123.txt"), "abc123");
        let report = build_publish_hook_report(
            root,
            None,
            &["https://example.com/".to_string()],
            Some("abc123"),
            false,
            "https://api.indexnow.org/indexnow",
        )
        .unwrap();
        assert_eq!(report.changed_routes, vec![String::new()]);
        assert!(report.indexnow_validation.is_some());
    }

    #[test]
    fn submits_indexnow_payloads() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buffer = [0_u8; 4096];
            let size = stream.read(&mut buffer).unwrap();
            let request = String::from_utf8_lossy(&buffer[..size]);
            assert!(request.contains("\"host\":\"example.com\""));
            assert!(request.contains("\"urlList\":[\"https://example.com/a\"]"));
            let response = "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok";
            stream.write_all(response.as_bytes()).unwrap();
            stream.flush().unwrap();
        });
        let submission = submit_indexnow(
            &format!("http://{address}"),
            "https://example.com",
            "abc123",
            &["https://example.com/a".to_string()],
        )
        .unwrap();
        assert!(submission.success);
        assert_eq!(submission.status_code, 200);
        handle.join().unwrap();
    }
}

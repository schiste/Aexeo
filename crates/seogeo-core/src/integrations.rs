use anyhow::{Context, Result, bail};
use csv::{ReaderBuilder, Writer};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
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
pub struct BingAiOpportunity {
    pub url: String,
    pub route: String,
    pub citations: u64,
    pub audit_findings: usize,
    pub audit_errors: usize,
    pub audit_warnings: usize,
    pub score: u64,
    pub priority: String,
    pub rationale: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BingAiOpportunityReport {
    pub rows_read: usize,
    pub urls_seen: usize,
    pub clean_cited_urls: usize,
    pub opportunities: Vec<BingAiOpportunity>,
    pub unmatched_urls: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BingAiTrendRoute {
    pub url: String,
    pub route: String,
    pub citations: u64,
    pub audit_findings: usize,
    pub audit_errors: usize,
    pub audit_warnings: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BingAiTrendSnapshot {
    pub imported_at: u64,
    pub source_path: String,
    pub audit_path: Option<String>,
    pub rows_read: usize,
    pub urls_seen: usize,
    pub total_citations: u64,
    pub routes: Vec<BingAiTrendRoute>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BingAiTrendDelta {
    pub url: String,
    pub route: String,
    pub previous_citations: u64,
    pub current_citations: u64,
    pub citation_delta: i64,
    pub audit_findings: usize,
    pub audit_errors: usize,
    pub audit_warnings: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BingAiTrendReport {
    pub snapshots: usize,
    pub current: Option<BingAiTrendSnapshot>,
    pub previous: Option<BingAiTrendSnapshot>,
    pub increased: Vec<BingAiTrendDelta>,
    pub decreased: Vec<BingAiTrendDelta>,
    pub newly_cited: Vec<BingAiTrendDelta>,
    pub no_longer_cited: Vec<BingAiTrendDelta>,
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
    pub audit_path: String,
    pub search_console_export_path: String,
    pub indexnow_ledger_path: Option<String>,
    pub search_console_rows: Vec<SearchConsoleExportRow>,
    pub indexnow_validation: Option<IndexNowValidation>,
    pub indexnow_submission: Option<IndexNowSubmission>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IndexNowLedgerEntry {
    pub submitted_at: u64,
    pub attempt: usize,
    pub endpoint: String,
    pub site_url: String,
    pub host: String,
    pub key_location: String,
    pub urls: Vec<String>,
    pub submitted_urls: usize,
    pub status_code: Option<u16>,
    pub success: bool,
    pub retryable: bool,
    pub response_body: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct IndexNowLedger {
    pub entries: Vec<IndexNowLedgerEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IndexNowRetryReport {
    pub ledger_path: String,
    pub attempted: usize,
    pub succeeded: usize,
    pub failed: usize,
    pub entries: Vec<IndexNowLedgerEntry>,
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

fn now_epoch_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn ensure_reports_dir(root: &Path) -> Result<PathBuf> {
    let reports_dir = root.join(".seogeo-reports");
    fs::create_dir_all(&reports_dir)?;
    Ok(reports_dir)
}

fn indexnow_ledger_path(root: &Path) -> PathBuf {
    root.join(".seogeo-reports/indexnow-ledger.json")
}

fn bing_ai_trend_path(root: &Path) -> PathBuf {
    root.join(".seogeo-reports/bing-ai-trends.json")
}

fn publish_hook_search_console_path(root: &Path) -> PathBuf {
    root.join(".seogeo-reports/publish-hook-search-console.csv")
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

pub fn load_indexnow_ledger(root: &Path) -> Result<IndexNowLedger> {
    let path = indexnow_ledger_path(root);
    if !path.exists() {
        return Ok(IndexNowLedger::default());
    }
    Ok(serde_json::from_str(&fs::read_to_string(path)?)?)
}

fn write_indexnow_ledger(root: &Path, ledger: &IndexNowLedger) -> Result<PathBuf> {
    let path = indexnow_ledger_path(root);
    ensure_reports_dir(root)?;
    fs::write(&path, serde_json::to_string_pretty(ledger)?)?;
    Ok(path)
}

fn is_retryable_indexnow_status(status_code: u16) -> bool {
    status_code == 429 || status_code >= 500
}

fn next_attempt_number(
    ledger: &IndexNowLedger,
    endpoint: &str,
    site_url: &str,
    urls: &[String],
) -> usize {
    ledger
        .entries
        .iter()
        .filter(|entry| {
            entry.endpoint == endpoint && entry.site_url == site_url && entry.urls == urls
        })
        .map(|entry| entry.attempt)
        .max()
        .unwrap_or(0)
        + 1
}

pub fn submit_indexnow_with_ledger(
    root: &Path,
    endpoint: &str,
    site_url: &str,
    key: &str,
    urls: &[String],
) -> Result<IndexNowLedgerEntry> {
    let validation = validate_indexnow(site_url, key, Some(root))?;
    if !validation.errors.is_empty() {
        bail!(
            "IndexNow validation failed: {}",
            validation.errors.join("; ")
        );
    }
    let mut ledger = load_indexnow_ledger(root)?;
    let attempt = next_attempt_number(&ledger, endpoint, site_url, urls);
    let entry = match submit_indexnow(endpoint, site_url, key, urls) {
        Ok(submission) => IndexNowLedgerEntry {
            submitted_at: now_epoch_seconds(),
            attempt,
            endpoint: endpoint.to_string(),
            site_url: site_url.to_string(),
            host: submission.host,
            key_location: submission.key_location,
            urls: urls.to_vec(),
            submitted_urls: submission.submitted_urls,
            status_code: Some(submission.status_code),
            success: submission.success,
            retryable: is_retryable_indexnow_status(submission.status_code),
            response_body: submission.response_body,
            error: None,
        },
        Err(error) => IndexNowLedgerEntry {
            submitted_at: now_epoch_seconds(),
            attempt,
            endpoint: endpoint.to_string(),
            site_url: site_url.to_string(),
            host: validation.host,
            key_location: validation.key_location,
            urls: urls.to_vec(),
            submitted_urls: urls.len(),
            status_code: None,
            success: false,
            retryable: true,
            response_body: None,
            error: Some(error.to_string()),
        },
    };
    ledger.entries.push(entry.clone());
    let _ = write_indexnow_ledger(root, &ledger)?;
    Ok(entry)
}

pub fn retry_indexnow_submissions(
    root: &Path,
    key: &str,
    limit: usize,
) -> Result<IndexNowRetryReport> {
    let ledger = load_indexnow_ledger(root)?;
    let ledger_path = indexnow_ledger_path(root);
    let mut latest_by_batch = BTreeMap::<(String, String, Vec<String>), IndexNowLedgerEntry>::new();
    for entry in &ledger.entries {
        let key = (
            entry.endpoint.clone(),
            entry.site_url.clone(),
            entry.urls.clone(),
        );
        let should_replace = latest_by_batch
            .get(&key)
            .is_none_or(|existing| entry.attempt > existing.attempt);
        if should_replace {
            latest_by_batch.insert(key, entry.clone());
        }
    }
    let pending = latest_by_batch
        .into_values()
        .filter(|entry| !entry.success && entry.retryable)
        .take(limit)
        .collect::<Vec<_>>();
    let mut retried = Vec::new();
    let mut succeeded = 0usize;
    let mut failed = 0usize;
    for entry in pending {
        let result =
            submit_indexnow_with_ledger(root, &entry.endpoint, &entry.site_url, key, &entry.urls)?;
        if result.success {
            succeeded += 1;
        } else {
            failed += 1;
        }
        retried.push(result);
    }
    Ok(IndexNowRetryReport {
        ledger_path: ledger_path.to_string_lossy().into_owned(),
        attempted: retried.len(),
        succeeded,
        failed,
        entries: retried,
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

fn priority_label(score: u64) -> String {
    if score >= 200 {
        "critical".to_string()
    } else if score >= 80 {
        "high".to_string()
    } else if score >= 40 {
        "medium".to_string()
    } else {
        "low".to_string()
    }
}

fn opportunity_score(summary: &BingAiUrlSummary, unmatched: bool) -> (u64, Vec<String>) {
    let mut score = summary.citations.saturating_mul(5);
    let mut rationale = Vec::new();
    if summary.citations > 0 {
        rationale.push(format!("{} Bing AI citations", summary.citations));
    }
    if summary.audit_errors > 0 {
        score = score.saturating_add((summary.audit_errors as u64).saturating_mul(50));
        rationale.push(format!("{} audit errors", summary.audit_errors));
    }
    if summary.audit_warnings > 0 {
        score = score.saturating_add((summary.audit_warnings as u64).saturating_mul(10));
        rationale.push(format!("{} audit warnings", summary.audit_warnings));
    }
    if unmatched {
        score = score.saturating_add(75);
        rationale.push("URL is cited by Bing AI but missing from the audit artifact".to_string());
    }
    (score, rationale)
}

pub fn build_bing_ai_opportunity_report(
    path: &Path,
    audit_path: &Path,
) -> Result<BingAiOpportunityReport> {
    let imported = import_bing_ai_export(path, Some(audit_path))?;
    let unmatched = imported
        .unmatched_urls
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let opportunities = imported
        .cited_urls
        .iter()
        .filter_map(|summary| {
            let is_unmatched = unmatched.contains(&summary.url);
            let (score, rationale) = opportunity_score(summary, is_unmatched);
            if score == 0 {
                return None;
            }
            Some(BingAiOpportunity {
                url: summary.url.clone(),
                route: summary.route.clone(),
                citations: summary.citations,
                audit_findings: summary.audit_findings,
                audit_errors: summary.audit_errors,
                audit_warnings: summary.audit_warnings,
                score,
                priority: priority_label(score),
                rationale,
            })
        })
        .collect::<Vec<_>>();
    let mut opportunities = opportunities;
    opportunities.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| left.url.cmp(&right.url))
    });
    Ok(BingAiOpportunityReport {
        rows_read: imported.rows_read,
        urls_seen: imported.urls_seen,
        clean_cited_urls: imported
            .cited_urls
            .iter()
            .filter(|summary| summary.audit_findings == 0)
            .count(),
        opportunities,
        unmatched_urls: imported.unmatched_urls,
    })
}

fn build_bing_ai_trend_snapshot(
    source_path: &Path,
    audit_path: Option<&Path>,
) -> Result<BingAiTrendSnapshot> {
    let imported = import_bing_ai_export(source_path, audit_path)?;
    let routes = imported
        .cited_urls
        .iter()
        .map(|summary| BingAiTrendRoute {
            url: summary.url.clone(),
            route: summary.route.clone(),
            citations: summary.citations,
            audit_findings: summary.audit_findings,
            audit_errors: summary.audit_errors,
            audit_warnings: summary.audit_warnings,
        })
        .collect::<Vec<_>>();
    Ok(BingAiTrendSnapshot {
        imported_at: now_epoch_seconds(),
        source_path: source_path.to_string_lossy().into_owned(),
        audit_path: audit_path.map(|path| path.to_string_lossy().into_owned()),
        rows_read: imported.rows_read,
        urls_seen: imported.urls_seen,
        total_citations: routes.iter().map(|route| route.citations).sum(),
        routes,
    })
}

pub fn record_bing_ai_trend(
    root: &Path,
    source_path: &Path,
    audit_path: Option<&Path>,
) -> Result<BingAiTrendSnapshot> {
    let mut history = load_bing_ai_trends(root)?;
    let snapshot = build_bing_ai_trend_snapshot(source_path, audit_path)?;
    history.push(snapshot.clone());
    ensure_reports_dir(root)?;
    fs::write(
        bing_ai_trend_path(root),
        serde_json::to_string_pretty(&history)?,
    )?;
    Ok(snapshot)
}

pub fn load_bing_ai_trends(root: &Path) -> Result<Vec<BingAiTrendSnapshot>> {
    let path = bing_ai_trend_path(root);
    if !path.exists() {
        return Ok(Vec::new());
    }
    Ok(serde_json::from_str(&fs::read_to_string(path)?)?)
}

fn compare_bing_ai_routes(
    current: &BingAiTrendSnapshot,
    previous: Option<&BingAiTrendSnapshot>,
) -> BingAiTrendReport {
    let previous_routes = previous
        .map(|snapshot| {
            snapshot
                .routes
                .iter()
                .map(|route| (route.route.clone(), route.clone()))
                .collect::<BTreeMap<_, _>>()
        })
        .unwrap_or_default();
    let current_routes = current
        .routes
        .iter()
        .map(|route| (route.route.clone(), route.clone()))
        .collect::<BTreeMap<_, _>>();

    let mut increased = Vec::new();
    let mut decreased = Vec::new();
    let mut newly_cited = Vec::new();
    let mut no_longer_cited = Vec::new();

    for (route, current_route) in &current_routes {
        match previous_routes.get(route) {
            Some(previous_route) => {
                let delta = current_route.citations as i64 - previous_route.citations as i64;
                if delta > 0 {
                    increased.push(BingAiTrendDelta {
                        url: current_route.url.clone(),
                        route: route.clone(),
                        previous_citations: previous_route.citations,
                        current_citations: current_route.citations,
                        citation_delta: delta,
                        audit_findings: current_route.audit_findings,
                        audit_errors: current_route.audit_errors,
                        audit_warnings: current_route.audit_warnings,
                    });
                } else if delta < 0 {
                    decreased.push(BingAiTrendDelta {
                        url: current_route.url.clone(),
                        route: route.clone(),
                        previous_citations: previous_route.citations,
                        current_citations: current_route.citations,
                        citation_delta: delta,
                        audit_findings: current_route.audit_findings,
                        audit_errors: current_route.audit_errors,
                        audit_warnings: current_route.audit_warnings,
                    });
                }
            }
            None => newly_cited.push(BingAiTrendDelta {
                url: current_route.url.clone(),
                route: route.clone(),
                previous_citations: 0,
                current_citations: current_route.citations,
                citation_delta: current_route.citations as i64,
                audit_findings: current_route.audit_findings,
                audit_errors: current_route.audit_errors,
                audit_warnings: current_route.audit_warnings,
            }),
        }
    }

    for (route, previous_route) in &previous_routes {
        if !current_routes.contains_key(route) {
            no_longer_cited.push(BingAiTrendDelta {
                url: previous_route.url.clone(),
                route: route.clone(),
                previous_citations: previous_route.citations,
                current_citations: 0,
                citation_delta: -(previous_route.citations as i64),
                audit_findings: previous_route.audit_findings,
                audit_errors: previous_route.audit_errors,
                audit_warnings: previous_route.audit_warnings,
            });
        }
    }

    for deltas in [
        &mut increased,
        &mut decreased,
        &mut newly_cited,
        &mut no_longer_cited,
    ] {
        deltas.sort_by(|left, right| {
            right
                .citation_delta
                .abs()
                .cmp(&left.citation_delta.abs())
                .then_with(|| left.route.cmp(&right.route))
        });
    }

    BingAiTrendReport {
        snapshots: if previous.is_some() { 2 } else { 1 },
        current: Some(current.clone()),
        previous: previous.cloned(),
        increased,
        decreased,
        newly_cited,
        no_longer_cited,
    }
}

pub fn build_bing_ai_trend_report(root: &Path) -> Result<BingAiTrendReport> {
    let history = load_bing_ai_trends(root)?;
    let current = history.last().cloned();
    let previous = history.iter().rev().nth(1).cloned();
    Ok(match current {
        Some(snapshot) => compare_bing_ai_routes(&snapshot, previous.as_ref()),
        None => BingAiTrendReport {
            snapshots: 0,
            current: None,
            previous: None,
            increased: Vec::new(),
            decreased: Vec::new(),
            newly_cited: Vec::new(),
            no_longer_cited: Vec::new(),
        },
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

fn render_search_console_rows_csv(rows: &[SearchConsoleExportRow]) -> Result<String> {
    let mut writer = Writer::from_writer(Vec::new());
    writer.write_record([
        "route",
        "url",
        "findings",
        "errors",
        "warnings",
        "heuristic",
        "rule_groups",
        "rule_ids",
    ])?;
    for row in rows {
        writer.write_record([
            row.route.as_str(),
            row.url.as_deref().unwrap_or(""),
            &row.findings.to_string(),
            &row.errors.to_string(),
            &row.warnings.to_string(),
            &row.heuristic.to_string(),
            &row.rule_groups.join("|"),
            &row.rule_ids.join("|"),
        ])?;
    }
    Ok(String::from_utf8(writer.into_inner()?)?)
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
    ensure_reports_dir(root)?;
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
    let search_console_csv = render_search_console_rows_csv(&search_console_rows)?;
    let search_console_export_path = publish_hook_search_console_path(root);
    fs::write(&search_console_export_path, search_console_csv)?;
    let indexnow_validation = match (config.site().site_url, indexnow_key) {
        (Some(site_url), Some(key)) => Some(validate_indexnow(site_url, key, Some(root))?),
        _ => None,
    };
    let (indexnow_submission, indexnow_ledger_path) =
        if submit_indexnow_request && !changed_urls.is_empty() && indexnow_validation.is_some() {
            let ledger_entry = submit_indexnow_with_ledger(
                root,
                indexnow_endpoint,
                config.site().site_url.unwrap_or_default(),
                indexnow_key.unwrap_or_default(),
                changed_urls,
            )?;
            (
                Some(IndexNowSubmission {
                    endpoint: ledger_entry.endpoint.clone(),
                    host: ledger_entry.host.clone(),
                    submitted_urls: ledger_entry.submitted_urls,
                    key_location: ledger_entry.key_location.clone(),
                    status_code: ledger_entry.status_code.unwrap_or_default(),
                    success: ledger_entry.success,
                    response_body: ledger_entry
                        .response_body
                        .clone()
                        .or(ledger_entry.error.clone()),
                }),
                Some(indexnow_ledger_path(root).to_string_lossy().into_owned()),
            )
        } else {
            (None, None)
        };
    Ok(PublishHookReport {
        changed_routes,
        finding_count: findings.len(),
        findings_by_route,
        audit_path: audit_path.to_string_lossy().into_owned(),
        search_console_export_path: search_console_export_path.to_string_lossy().into_owned(),
        indexnow_ledger_path,
        search_console_rows,
        indexnow_validation,
        indexnow_submission,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        build_bing_ai_opportunity_report, build_bing_ai_trend_report, build_publish_hook_report,
        export_search_console_rows, import_bing_ai_export, inspect_snippet_controls_path,
        load_indexnow_ledger, record_bing_ai_trend, retry_indexnow_submissions, submit_indexnow,
        submit_indexnow_with_ledger, validate_indexnow,
    };
    use std::fs;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::path::Path;
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
    fn builds_bing_ai_opportunity_reports() {
        let temp_dir = tempfile::tempdir().unwrap();
        let export_path = temp_dir.path().join("bing-ai.csv");
        let audit_path = temp_dir.path().join("audit.json");
        write(
            &export_path,
            "url,query,citations\nhttps://example.com/docs,what is docs,5\n",
        );
        fs::write(
            &audit_path,
            r#"{"version":2,"command":"check","status":"complete","generated_at":0,"summary":{"total":2,"errors":1,"warnings":1,"actionable":2,"heuristic":0},"findings":[{"rule_id":"SEO001","message":"missing <title>","path":"crawl/docs/index.html","line":1,"column":1,"severity":"error","scope":"page"},{"rule_id":"SEO002","message":"missing meta description","path":"crawl/docs/index.html","line":1,"column":1,"severity":"warning","scope":"page"}]}"#,
        )
        .unwrap();
        let report = build_bing_ai_opportunity_report(&export_path, &audit_path).unwrap();
        assert_eq!(report.opportunities.len(), 1);
        assert!(report.opportunities[0].score > 0);
        assert_eq!(report.opportunities[0].priority, "high");
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
        assert!(Path::new(&report.audit_path).exists());
        assert!(Path::new(&report.search_console_export_path).exists());
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

    #[test]
    fn writes_and_retries_indexnow_ledger_entries() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        write(&root.join("abc123.txt"), "abc123");
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            for status in ["429 Too Many Requests", "200 OK"] {
                let (mut stream, _) = listener.accept().unwrap();
                let mut buffer = [0_u8; 4096];
                let _ = stream.read(&mut buffer).unwrap();
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok"
                );
                stream.write_all(response.as_bytes()).unwrap();
                stream.flush().unwrap();
            }
        });
        let first = submit_indexnow_with_ledger(
            root,
            &format!("http://{address}"),
            "https://example.com",
            "abc123",
            &["https://example.com/a".to_string()],
        )
        .unwrap();
        assert!(!first.success);
        assert!(first.retryable);
        let ledger = load_indexnow_ledger(root).unwrap();
        assert_eq!(ledger.entries.len(), 1);

        let retry = retry_indexnow_submissions(root, "abc123", 10).unwrap();
        assert_eq!(retry.attempted, 1);
        assert_eq!(retry.succeeded, 1);
        let ledger = load_indexnow_ledger(root).unwrap();
        assert_eq!(ledger.entries.len(), 2);
        handle.join().unwrap();
    }

    #[test]
    fn records_and_compares_bing_ai_trends() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        let audit_path = root.join("audit.json");
        fs::write(
            &audit_path,
            r#"{"version":2,"command":"check","status":"complete","generated_at":0,"summary":{"total":1,"errors":0,"warnings":1,"actionable":1,"heuristic":0},"findings":[{"rule_id":"SEO002","message":"missing meta description","path":"crawl/docs/index.html","line":1,"column":1,"severity":"warning","scope":"page"}]}"#,
        )
        .unwrap();
        let export_one = root.join("bing-one.csv");
        let export_two = root.join("bing-two.csv");
        write(
            &export_one,
            "url,query,citations\nhttps://example.com/docs,what is docs,2\n",
        );
        write(
            &export_two,
            "url,query,citations\nhttps://example.com/docs,what is docs,5\nhttps://example.com/new,what is new,1\n",
        );
        record_bing_ai_trend(root, &export_one, Some(&audit_path)).unwrap();
        record_bing_ai_trend(root, &export_two, Some(&audit_path)).unwrap();
        let report = build_bing_ai_trend_report(root).unwrap();
        assert_eq!(report.snapshots, 2);
        assert_eq!(report.increased[0].route, "docs");
        assert_eq!(report.newly_cited[0].route, "new");
    }
}

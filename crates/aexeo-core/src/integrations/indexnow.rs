//! IndexNow integration: key-file validation, URL ownership checks, batch
//! submission, and the on-disk ledger that makes retries idempotent.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use reqwest::blocking::Client;

use super::{ensure_reports_dir, now_epoch_seconds};
use url::Url;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IndexNowValidation {
    pub site_url: String,
    pub host: String,
    pub key: String,
    pub key_location: String,
    pub validation_mode: String,
    pub key_file_path: Option<String>,
    pub key_file_present: bool,
    pub key_file_matches: bool,
    pub remote_status_code: Option<u16>,
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
pub struct IndexNowPlan {
    pub endpoint: String,
    pub site_url: String,
    pub host: String,
    pub key_location: String,
    pub validation: IndexNowValidation,
    pub urls: Vec<String>,
    pub submitted_urls: usize,
    pub can_submit: bool,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    pub notes: Vec<String>,
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

pub(super) fn indexnow_ledger_path(root: &Path) -> PathBuf {
    root.join(".aexeo-reports/indexnow-ledger.json")
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
    let (validation_mode, key_file_present, key_file_matches, remote_status_code, file_warning) =
        if let Some(path) = &key_file_path {
            if !path.exists() {
                (
                    "local".to_string(),
                    false,
                    false,
                    None,
                    Some(format!("missing key file at {}", path.display())),
                )
            } else {
                let content = fs::read_to_string(path).unwrap_or_default();
                ("local".to_string(), true, content.trim() == key, None, None)
            }
        } else {
            let client = Client::builder().timeout(Duration::from_secs(15)).build()?;
            match client.get(&key_location).send() {
                Ok(response) => {
                    let status_code = response.status().as_u16();
                    if !response.status().is_success() {
                        (
                            "remote".to_string(),
                            false,
                            false,
                            Some(status_code),
                            Some(format!(
                                "remote key file check returned HTTP {} from {}",
                                status_code, key_location
                            )),
                        )
                    } else {
                        let content = response.text().unwrap_or_default();
                        (
                            "remote".to_string(),
                            true,
                            content.trim() == key,
                            Some(status_code),
                            None,
                        )
                    }
                }
                Err(error) => (
                    "remote".to_string(),
                    false,
                    false,
                    None,
                    Some(format!(
                        "failed to fetch remote key file {}: {}",
                        key_location, error
                    )),
                ),
            }
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
        validation_mode,
        key_file_path: key_file_path.map(|path| path.to_string_lossy().into_owned()),
        key_file_present,
        key_file_matches,
        remote_status_code,
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

fn normalize_indexnow_urls(urls: &[String]) -> Vec<String> {
    let mut normalized = BTreeSet::new();
    for url in urls {
        let trimmed = url.trim();
        if !trimmed.is_empty() {
            normalized.insert(trimmed.to_string());
        }
    }
    normalized.into_iter().collect()
}

fn validate_indexnow_url_ownership(host: &str, urls: &[String]) -> Vec<String> {
    let mut errors = Vec::new();
    for url in urls {
        let parsed = match Url::parse(url) {
            Ok(parsed) => parsed,
            Err(error) => {
                errors.push(format!("invalid URL '{}': {}", url, error));
                continue;
            }
        };
        if !matches!(parsed.scheme(), "http" | "https") {
            errors.push(format!("URL '{}' must use http or https", url));
        }
        if parsed.host_str() != Some(host) {
            errors.push(format!(
                "URL '{}' is not owned by IndexNow host '{}'",
                url, host
            ));
        }
    }
    errors
}

pub fn plan_indexnow_submission(
    endpoint: &str,
    site_url: &str,
    key: &str,
    root: Option<&Path>,
    urls: &[String],
) -> Result<IndexNowPlan> {
    let validation = validate_indexnow(site_url, key, root)?;
    let normalized_urls = normalize_indexnow_urls(urls);
    let mut errors = validation.errors.clone();
    let mut warnings = validation.warnings.clone();
    if !validation.key_file_present {
        errors.push(format!(
            "IndexNow key file must be present before submission: {}",
            validation.key_location
        ));
    }
    if normalized_urls.is_empty() {
        errors.push("at least one URL is required for IndexNow submission".to_string());
    }
    if normalized_urls.len() > 10_000 {
        errors.push(format!(
            "IndexNow payload contains {} URLs; split submissions into batches of at most 10,000 URLs",
            normalized_urls.len()
        ));
    }
    errors.extend(validate_indexnow_url_ownership(
        &validation.host,
        &normalized_urls,
    ));
    if validation.validation_mode == "remote" {
        warnings.push(
            "remote validation checks the live key file; use --path for local pre-publish validation"
                .to_string(),
        );
    }
    let can_submit = errors.is_empty();
    Ok(IndexNowPlan {
        endpoint: endpoint.to_string(),
        site_url: site_url.to_string(),
        host: validation.host.clone(),
        key_location: validation.key_location.clone(),
        validation,
        submitted_urls: normalized_urls.len(),
        urls: normalized_urls,
        can_submit,
        errors,
        warnings,
        notes: vec![
            "This is a dry-run plan; no search engine was notified.".to_string(),
            "IndexNow notifies participating engines about URL changes but does not guarantee indexing or ranking.".to_string(),
            "Persist real submissions through the ledger by using `indexnow submit --path <root>` or the publish hook.".to_string(),
        ],
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    fn write(path: &Path, text: &str) {
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
        assert_eq!(result.validation_mode, "local");
        assert!(result.key_file_present);
        assert!(result.key_file_matches);
        assert_eq!(result.remote_status_code, None);
    }
    #[test]
    fn plans_indexnow_submission_without_network_submit() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        write(&root.join("abc123.txt"), "abc123");
        let urls = vec![
            "https://example.com/a".to_string(),
            "https://example.com/a".to_string(),
            "https://example.com/b".to_string(),
        ];
        let plan = plan_indexnow_submission(
            "https://api.indexnow.org/indexnow",
            "https://example.com",
            "abc123",
            Some(root),
            &urls,
        )
        .unwrap();
        assert!(plan.can_submit);
        assert_eq!(plan.submitted_urls, 2);
        assert!(plan.errors.is_empty());
        assert_eq!(plan.validation.validation_mode, "local");
    }
    #[test]
    fn blocks_indexnow_urls_for_other_hosts() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        write(&root.join("abc123.txt"), "abc123");
        let plan = plan_indexnow_submission(
            "https://api.indexnow.org/indexnow",
            "https://example.com",
            "abc123",
            Some(root),
            &["https://other.example/page".to_string()],
        )
        .unwrap();
        assert!(!plan.can_submit);
        assert!(
            plan.errors
                .iter()
                .any(|error| error.contains("not owned by IndexNow host"))
        );
    }
    #[test]
    fn validates_indexnow_remote_key_files() {
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
        let result = validate_indexnow(&format!("http://{}", address), "abc123", None).unwrap();
        assert!(result.errors.is_empty());
        assert_eq!(result.validation_mode, "remote");
        assert!(result.key_file_present);
        assert!(result.key_file_matches);
        assert_eq!(result.remote_status_code, Some(200));
        handle.join().unwrap();
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
}

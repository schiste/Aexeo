//! Publish-hook reporting: pre-computes the Search Console upload a deploy
//! hook would perform, so a release can show the delta before it ships.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use super::indexnow::indexnow_ledger_path;
use super::indexnow::{IndexNowSubmission, IndexNowValidation};
use super::indexnow::{submit_indexnow_with_ledger, validate_indexnow};
use super::search_console::{
    SearchConsoleExportRow, export_search_console_rows, render_search_console_rows_csv,
};
use super::{ensure_reports_dir, normalize_audit_path_to_route};
use crate::config::{Config, load_config};
use crate::site::route_from_urlish;
use crate::static_check::run_native_static_audit_with_config;

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

fn publish_hook_search_console_path(root: &Path) -> PathBuf {
    root.join(".aexeo-reports/publish-hook-search-console.csv")
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
        aexeo_contracts::AuditStatus::Complete,
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
    fn write(path: &Path, text: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, text).unwrap();
    }

    use super::*;
    use std::fs;
    use std::path::Path;

    #[test]
    fn builds_publish_hook_reports() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        write(
            &root.join("aexeo.toml"),
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
}

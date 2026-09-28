//! External service integrations.
//!
//! Five unrelated third-party surfaces used to share one flat module:
//! snippet-control inspection, IndexNow, Bing AI, Search Console, and the
//! publish hook. Nothing in it referenced anything else except the small
//! set of helpers below, so each is now its own file with its own imports
//! and its own tests.
//!
//! The shared helpers stay here because more than one integration needs
//! them; everything else is re-exported so `aexeo_core::integrations::X`
//! and the flat `aexeo_core::X` re-exports in `lib.rs` keep working
//! unchanged.

use anyhow::Result;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::site::{Page, route_from_urlish};

mod bing_ai;
mod indexnow;
mod publish_hook;
mod search_console;
mod snippet;

pub use bing_ai::{
    BingAiImportReport, BingAiOpportunity, BingAiOpportunityReport, BingAiRecord, BingAiTrendDelta,
    BingAiTrendReport, BingAiTrendRoute, BingAiTrendSnapshot, BingAiUrlSummary,
    build_bing_ai_opportunity_report, build_bing_ai_trend_report, import_bing_ai_export,
    load_bing_ai_trends, record_bing_ai_trend,
};
pub use indexnow::{
    IndexNowLedger, IndexNowLedgerEntry, IndexNowPlan, IndexNowRetryReport, IndexNowSubmission,
    IndexNowValidation, load_indexnow_ledger, plan_indexnow_submission, retry_indexnow_submissions,
    submit_indexnow, submit_indexnow_with_ledger, validate_indexnow,
};
pub use publish_hook::{
    PublishHookReport, build_publish_hook_report, build_publish_hook_report_with_config,
};
pub use search_console::{SearchConsoleExportRow, export_search_console_rows};
pub use snippet::{
    SnippetInspection, inspect_snippet_controls_path, inspect_snippet_controls_site,
    inspect_snippet_controls_url, inspect_snippet_controls_with_config,
};

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
    let reports_dir = root.join(".aexeo-reports");
    fs::create_dir_all(&reports_dir)?;
    Ok(reports_dir)
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

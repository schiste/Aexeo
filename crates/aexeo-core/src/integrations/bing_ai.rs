//! Bing AI import and trend analysis: parses a Bing export, scores
//! opportunities against the current audit, and diffs route trends over time.

use anyhow::Result;
use csv::ReaderBuilder;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use super::{ensure_reports_dir, normalize_audit_path_to_route, now_epoch_seconds};
use crate::site::route_from_urlish;
use crate::verification::load_audit_artifact;

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

fn bing_ai_trend_path(root: &Path) -> PathBuf {
    root.join(".aexeo-reports/bing-ai-trends.json")
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;

    fn write(path: &Path, text: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, text).unwrap();
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

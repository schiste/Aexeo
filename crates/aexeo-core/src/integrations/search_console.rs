//! Search Console CSV export: flattens an audit artifact into the
//! column/row shape the Search Console API expects.

use anyhow::Result;
use csv::Writer;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

use super::normalize_audit_path_to_route;
use crate::reporting::rule_group_name;
use crate::verification::load_audit_artifact;

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

pub(super) fn render_search_console_rows_csv(rows: &[SearchConsoleExportRow]) -> Result<String> {
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

#[cfg(test)]
mod tests {

    use super::*;
    use std::fs;

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
}

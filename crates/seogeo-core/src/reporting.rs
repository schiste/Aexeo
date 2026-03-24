use anyhow::Result;
use serde_json::json;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::registry::rule_metadata_for_id;
use seogeo_contracts::Finding;
use seogeo_contracts::RuleClass;

pub const DEFAULT_AUDIT_LOG_LIMIT: usize = 5;

fn now_epoch_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn confidence_buckets() -> BTreeMap<&'static str, usize> {
    BTreeMap::from([("high", 0), ("medium", 0), ("low", 0)])
}

fn class_buckets() -> BTreeMap<&'static str, usize> {
    BTreeMap::from([("hard", 0), ("policy", 0), ("heuristic", 0)])
}

#[derive(Debug, Clone)]
struct FindingCluster {
    rule_id: String,
    message: String,
    count: usize,
    examples: Vec<String>,
}

impl FindingCluster {
    fn new(finding: &Finding) -> Self {
        Self {
            rule_id: finding.rule_id.clone(),
            message: finding.message.clone(),
            count: 1,
            examples: vec![format!(
                "{}:{}:{}",
                finding.path, finding.line, finding.column
            )],
        }
    }

    fn merge(&mut self, finding: &Finding) {
        self.count += 1;
        let example = format!("{}:{}:{}", finding.path, finding.line, finding.column);
        if self.examples.len() < 3 && !self.examples.contains(&example) {
            self.examples.push(example);
        }
    }

    fn confidence_tag(&self) -> String {
        rule_metadata_for_id(&self.rule_id).render_tag()
    }
}

fn cluster_findings(findings: &[Finding]) -> BTreeMap<String, Vec<FindingCluster>> {
    let mut grouped: BTreeMap<String, BTreeMap<(String, String, String), FindingCluster>> =
        BTreeMap::new();
    for finding in findings {
        let group_name = rule_group_name(&finding.rule_id).to_string();
        let key = (
            finding.rule_id.clone(),
            finding.message.clone(),
            finding.severity.clone(),
        );
        grouped
            .entry(group_name)
            .or_default()
            .entry(key)
            .and_modify(|cluster| cluster.merge(finding))
            .or_insert_with(|| FindingCluster::new(finding));
    }

    grouped
        .into_iter()
        .map(|(group_name, clusters)| {
            let mut clusters = clusters.into_values().collect::<Vec<_>>();
            clusters.sort_by(|a, b| {
                b.count
                    .cmp(&a.count)
                    .then_with(|| a.rule_id.cmp(&b.rule_id))
            });
            (group_name, clusters)
        })
        .collect()
}

fn summarize_metadata(
    findings: &[Finding],
) -> (BTreeMap<&'static str, usize>, BTreeMap<&'static str, usize>) {
    let mut confidence = confidence_buckets();
    let mut classes = class_buckets();
    for finding in findings {
        let metadata = rule_metadata_for_id(&finding.rule_id);
        *confidence.entry(metadata.confidence.as_str()).or_default() += 1;
        *classes.entry(metadata.class.as_str()).or_default() += 1;
    }
    (confidence, classes)
}

fn is_heuristic_finding(finding: &Finding) -> bool {
    matches!(
        rule_metadata_for_id(&finding.rule_id).class,
        RuleClass::Heuristic
    )
}

fn section_title(is_heuristic: bool) -> &'static str {
    if is_heuristic {
        "Heuristic Observations"
    } else {
        "Actionable Findings"
    }
}

fn section_findings(findings: &[Finding], is_heuristic: bool) -> Vec<Finding> {
    findings
        .iter()
        .filter(|finding| is_heuristic_finding(finding) == is_heuristic)
        .cloned()
        .collect()
}

fn repeated_issue_summary(findings: &[Finding], is_heuristic: bool) -> Option<String> {
    let mut repeated: Vec<((String, String), usize)> = findings
        .iter()
        .filter(|finding| is_heuristic_finding(finding) == is_heuristic)
        .fold(
            BTreeMap::<(String, String), usize>::new(),
            |mut acc, finding| {
                *acc.entry((finding.rule_id.clone(), finding.message.clone()))
                    .or_default() += 1;
                acc
            },
        )
        .into_iter()
        .filter(|(_, count)| *count > 1)
        .collect();
    if repeated.is_empty() {
        return None;
    }
    repeated.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let label = if is_heuristic {
        "Heuristic observations"
    } else {
        "Actionable issues"
    };
    let top_repeats = repeated
        .into_iter()
        .take(3)
        .map(|((rule_id, message), count)| format!("{} {} x{}", rule_id, message, count))
        .collect::<Vec<_>>()
        .join(", ");
    Some(format!(
        "- Most repeated {}: {}",
        label.to_lowercase(),
        top_repeats
    ))
}

pub fn rule_group_name(rule_id: &str) -> &'static str {
    let prefix: String = rule_id
        .chars()
        .take_while(|ch| ch.is_ascii_uppercase())
        .collect();
    match prefix.as_str() {
        "SEO" => "HTML Metadata",
        "LNK" => "Internal Links",
        "MAP" => "Sitemaps",
        "ROB" => "Robots",
        "SOC" => "Social Metadata",
        "SCH" => "Structured Data",
        "LLM" => "LLM Artifacts",
        "CNT" => "Content Policy",
        "GEO" => "Retrieval Structure",
        "CRW" => "Runtime Crawl",
        "DEP" => "Deployment Model",
        "QLT" => "Internal Quality",
        _ => "Other",
    }
}

pub fn build_recap_lines(findings: &[Finding]) -> Vec<String> {
    let total = findings.len();
    let error_count = findings.iter().filter(|finding| finding.is_error()).count();
    let warning_count = total.saturating_sub(error_count);
    let (confidence, classes) = summarize_metadata(findings);
    let actionable_count = findings
        .iter()
        .filter(|finding| !is_heuristic_finding(finding))
        .count();
    let heuristic_count = total.saturating_sub(actionable_count);
    let mut by_group: BTreeMap<String, usize> = BTreeMap::new();
    for finding in findings {
        *by_group
            .entry(rule_group_name(&finding.rule_id).to_string())
            .or_default() += 1;
    }
    let mut ranked_groups: Vec<(String, usize)> = by_group.into_iter().collect();
    ranked_groups.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let mut lines = vec![
        "Recap".to_string(),
        format!("- Actionable findings: {}", actionable_count),
        format!("- Heuristic observations: {}", heuristic_count),
        format!("- Total findings: {}", total),
        format!("- Errors: {}", error_count),
        format!("- Warnings: {}", warning_count),
    ];
    if !ranked_groups.is_empty() {
        let sections = ranked_groups
            .into_iter()
            .take(5)
            .map(|(name, count)| format!("{} ({})", name, count))
            .collect::<Vec<_>>()
            .join(", ");
        lines.push(format!("- Largest sections: {}", sections));
    }
    lines.push(format!(
        "- Confidence mix: high {}, medium {}, low {}",
        confidence.get("high").copied().unwrap_or(0),
        confidence.get("medium").copied().unwrap_or(0),
        confidence.get("low").copied().unwrap_or(0)
    ));
    lines.push(format!(
        "- Rule classes: hard {}, policy {}, heuristic {}",
        classes.get("hard").copied().unwrap_or(0),
        classes.get("policy").copied().unwrap_or(0),
        classes.get("heuristic").copied().unwrap_or(0)
    ));
    if let Some(summary) = repeated_issue_summary(findings, false) {
        lines.push(summary);
    }
    if let Some(summary) = repeated_issue_summary(findings, true) {
        lines.push(summary);
    }
    lines
}

pub fn render_text(
    findings: &[Finding],
    success_message: &str,
    audit_path: Option<&Path>,
) -> String {
    if findings.is_empty() {
        let mut lines = vec![success_message.to_string()];
        if let Some(path) = audit_path {
            lines.push(String::new());
            lines.push(format!("Audit results: {}", path.display()));
        }
        return lines.join("\n");
    }

    let mut lines = vec!["Audit Report".to_string(), String::new()];
    for is_heuristic in [false, true] {
        let section_findings = section_findings(findings, is_heuristic);
        if section_findings.is_empty() {
            continue;
        }
        let grouped = cluster_findings(&section_findings);
        let section_count = section_findings.len();
        lines.push(format!(
            "{} ({})",
            section_title(is_heuristic),
            section_count
        ));
        for (group_name, group_clusters) in grouped {
            let group_count = group_clusters
                .iter()
                .map(|cluster| cluster.count)
                .sum::<usize>();
            lines.push(format!("{} ({})", group_name, group_count));
            for cluster in group_clusters {
                let mut line = format!(
                    "- {} {} [{}] {}",
                    cluster.examples.first().cloned().unwrap_or_default(),
                    cluster.rule_id,
                    cluster.confidence_tag(),
                    cluster.message
                );
                if cluster.count > 1 {
                    line.push_str(&format!(" ({} occurrences)", cluster.count));
                    if !cluster.examples.is_empty() {
                        line.push_str(&format!("; examples: {}", cluster.examples.join(", ")));
                    }
                }
                lines.push(line);
            }
            lines.push(String::new());
        }
    }
    lines.extend(build_recap_lines(findings));
    if let Some(path) = audit_path {
        lines.push(String::new());
        lines.push(format!("Audit results: {}", path.display()));
    }
    lines.join("\n")
}

pub fn render_json(findings: &[Finding]) -> Result<String> {
    Ok(serde_json::to_string_pretty(findings)?)
}

pub fn render_sarif(findings: &[Finding], tool_name: &str) -> Result<String> {
    let mut rules = BTreeMap::new();
    let results: Vec<_> = findings
        .iter()
        .map(|finding| {
            rules.entry(finding.rule_id.clone()).or_insert_with(|| {
                json!({
                    "id": finding.rule_id,
                    "name": finding.rule_id,
                    "shortDescription": {"text": finding.rule_id},
                })
            });
            json!({
                "ruleId": finding.rule_id,
                "level": if finding.severity == "warning" { "warning" } else { "error" },
                "message": { "text": finding.message },
                "locations": [{
                    "physicalLocation": {
                        "artifactLocation": { "uri": finding.path },
                        "region": { "startLine": finding.line, "startColumn": finding.column }
                    }
                }]
            })
        })
        .collect();

    Ok(serde_json::to_string_pretty(&json!({
        "version": "2.1.0",
        "$schema": "https://json.schemastore.org/sarif-2.1.0.json",
        "runs": [{
            "tool": { "driver": { "name": tool_name, "rules": rules.into_values().collect::<Vec<_>>() } },
            "results": results,
        }]
    }))?)
}

pub fn prune_old_audit_logs(artifact_dir: &Path, command_name: &str, keep: usize) -> Result<()> {
    let mut history_logs: Vec<PathBuf> = fs::read_dir(artifact_dir)?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .map(|name| {
                    name.starts_with(&format!("{}-", command_name))
                        && name.ends_with(".json")
                        && name != format!("{}-latest.json", command_name)
                        && name != format!("{}-trends.json", command_name)
                })
                .unwrap_or(false)
        })
        .collect();
    history_logs.sort_by_key(|path| fs::metadata(path).and_then(|m| m.modified()).ok());
    history_logs.reverse();
    for path in history_logs.into_iter().skip(keep.saturating_sub(1)) {
        let _ = fs::remove_file(path);
    }
    Ok(())
}

pub fn update_trend_history(
    artifact_dir: &Path,
    command_name: &str,
    findings: &[Finding],
) -> Result<()> {
    let trend_path = artifact_dir.join(format!("{}-trends.json", command_name));
    let mut payload: Vec<serde_json::Value> = fs::read_to_string(&trend_path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default();
    payload.push(json!({
        "timestamp": now_epoch_seconds(),
        "total": findings.len(),
        "errors": findings.iter().filter(|finding| finding.is_error()).count(),
        "warnings": findings.iter().filter(|finding| !finding.is_error()).count(),
    }));
    let slice_start = payload.len().saturating_sub(50);
    fs::write(
        trend_path,
        serde_json::to_string_pretty(&payload[slice_start..])?,
    )?;
    Ok(())
}

pub fn write_audit_artifact(
    findings: &[Finding],
    base_dir: &Path,
    command_name: &str,
    keep: usize,
) -> Result<PathBuf> {
    let artifact_dir = base_dir.join(".seogeo-reports");
    fs::create_dir_all(&artifact_dir)?;
    let timestamp = now_epoch_seconds();
    let history_path = artifact_dir.join(format!("{}-{}.json", command_name, timestamp));
    let latest_path = artifact_dir.join(format!("{}-latest.json", command_name));
    let payload = render_json(findings)?;
    fs::write(&history_path, &payload)?;
    fs::write(&latest_path, payload)?;
    prune_old_audit_logs(&artifact_dir, command_name, keep)?;
    update_trend_history(&artifact_dir, command_name, findings)?;
    Ok(latest_path)
}

#[cfg(test)]
mod tests {
    use super::{build_recap_lines, render_sarif, render_text, write_audit_artifact};
    use seogeo_contracts::Finding;

    fn sample_finding() -> Finding {
        Finding {
            rule_id: "SEO001".into(),
            message: "missing title".into(),
            path: "index.html".into(),
            line: 1,
            column: 1,
            severity: "error".into(),
            suggestion: None,
        }
    }

    #[test]
    fn renders_text_and_recap() {
        let finding = sample_finding();
        let text = render_text(&[finding], "ok", None);
        assert!(text.contains("Audit Report"));
        assert!(
            build_recap_lines(&[sample_finding()])
                .iter()
                .any(|line| line.contains("Total findings"))
        );
    }

    #[test]
    fn renders_sarif() {
        let payload = render_sarif(&[sample_finding()], "seogeo").unwrap();
        assert!(payload.contains("sarif"));
        assert!(payload.contains("SEO001"));
    }

    #[test]
    fn writes_audit_artifacts() {
        let temp_dir = tempfile::tempdir().unwrap();
        let latest =
            write_audit_artifact(&[sample_finding()], temp_dir.path(), "check", 5).unwrap();
        assert!(latest.exists());
    }

    #[test]
    fn clusters_repeated_findings_in_text_output() {
        let finding_a = sample_finding();
        let finding_b = Finding {
            path: "about.html".into(),
            ..sample_finding()
        };
        let text = render_text(&[finding_a, finding_b], "ok", None);
        assert!(text.contains("2 occurrences"));
        assert!(text.contains("Confidence mix"));
        assert!(text.contains("Rule classes"));
    }

    #[test]
    fn separates_actionable_and_heuristic_sections() {
        let actionable = sample_finding();
        let heuristic = Finding {
            rule_id: "GEO007".into(),
            message: "thin block".into(),
            path: "page.html".into(),
            line: 1,
            column: 1,
            severity: "warning".into(),
            suggestion: None,
        };
        let text = render_text(&[actionable, heuristic], "ok", None);
        let actionable_pos = text.find("Actionable Findings").unwrap();
        let heuristic_pos = text.find("Heuristic Observations").unwrap();
        assert!(actionable_pos < heuristic_pos);
        assert!(text.contains("Actionable findings: 1"));
        assert!(text.contains("Heuristic observations: 1"));
    }
}

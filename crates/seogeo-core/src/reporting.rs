use anyhow::Result;
use serde_json::json;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::registry::rule_metadata_for_id;
use seogeo_contracts::{
    AuditArtifact, AuditStatus, AuditSummary, CrawlStats, Finding, FindingScope, RuleClass,
};

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
        if self.examples.len() < 5 && !self.examples.contains(&example) {
            self.examples.push(example);
        }
    }

    fn confidence_tag(&self) -> String {
        rule_metadata_for_id(&self.rule_id).render_tag()
    }
}

pub fn summarize_findings(findings: &[Finding]) -> AuditSummary {
    let errors = findings.iter().filter(|finding| finding.is_error()).count();
    let actionable = findings
        .iter()
        .filter(|finding| !is_heuristic_finding(finding))
        .count();
    AuditSummary {
        total: findings.len(),
        errors,
        warnings: findings.len().saturating_sub(errors),
        actionable,
        heuristic: findings.len().saturating_sub(actionable),
    }
}

fn rule_is_sitewide(rule_id: &str) -> bool {
    matches!(
        rule_id,
        id if id.starts_with("MAP")
            || id.starts_with("ROB")
            || id.starts_with("LLM")
            || id.starts_with("DEP")
            || id.starts_with("QLT")
            || matches!(id, "SCH009" | "SEO011" | "SEO017" | "CRW003")
    )
}

pub fn normalize_finding_scopes(findings: &[Finding]) -> Vec<Finding> {
    let repeated: BTreeMap<(String, String), usize> =
        findings
            .iter()
            .fold(BTreeMap::new(), |mut counts, finding| {
                *counts
                    .entry((finding.rule_id.clone(), finding.message.clone()))
                    .or_default() += 1;
                counts
            });
    findings
        .iter()
        .cloned()
        .map(|mut finding| {
            if finding.scope != FindingScope::Page {
                return finding;
            }
            finding.scope = if rule_is_sitewide(&finding.rule_id) {
                FindingScope::Sitewide
            } else if repeated
                .get(&(finding.rule_id.clone(), finding.message.clone()))
                .copied()
                .unwrap_or(0)
                >= 3
            {
                FindingScope::Template
            } else {
                FindingScope::Page
            };
            finding
        })
        .collect()
}

fn completion_ratio(crawl: &CrawlStats) -> Option<String> {
    if crawl.discovered_internal_routes == 0 {
        return None;
    }
    Some(format!(
        "{:.1}%",
        (crawl.visited_pages as f64 / crawl.discovered_internal_routes as f64) * 100.0
    ))
}

pub fn build_audit_artifact(
    command: &str,
    findings: &[Finding],
    status: AuditStatus,
    crawl: Option<CrawlStats>,
    truncation_reason: Option<String>,
) -> AuditArtifact {
    let findings = normalize_finding_scopes(findings);
    AuditArtifact {
        version: 2,
        command: command.to_string(),
        status,
        generated_at: now_epoch_seconds(),
        summary: summarize_findings(&findings),
        completion_ratio: crawl.as_ref().and_then(completion_ratio),
        truncation_reason,
        crawl,
        findings,
        performance: None,
        site: None,
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

fn scope_title(scope: FindingScope) -> &'static str {
    match scope {
        FindingScope::Sitewide => "Sitewide Findings",
        FindingScope::Template => "Template Findings",
        FindingScope::Page => "Page Findings",
    }
}

fn scope_findings(findings: &[Finding], scope: FindingScope, is_heuristic: bool) -> Vec<Finding> {
    findings
        .iter()
        .filter(|finding| finding.scope == scope && is_heuristic_finding(finding) == is_heuristic)
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
        "heuristic observations"
    } else {
        "actionable issues"
    };
    let top_repeats = repeated
        .into_iter()
        .take(3)
        .map(|((rule_id, message), count)| format!("{} {} x{}", rule_id, message, count))
        .collect::<Vec<_>>()
        .join(", ");
    Some(format!("- Most repeated {}: {}", label, top_repeats))
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
    let summary = summarize_findings(findings);
    let (confidence, classes) = summarize_metadata(findings);
    let mut by_group: BTreeMap<String, usize> = BTreeMap::new();
    let mut by_scope: BTreeMap<&'static str, usize> =
        BTreeMap::from([("sitewide", 0), ("template", 0), ("page", 0)]);
    for finding in findings {
        *by_group
            .entry(rule_group_name(&finding.rule_id).to_string())
            .or_default() += 1;
        let scope_key = match finding.scope {
            FindingScope::Sitewide => "sitewide",
            FindingScope::Template => "template",
            FindingScope::Page => "page",
        };
        *by_scope.entry(scope_key).or_default() += 1;
    }
    let mut ranked_groups: Vec<(String, usize)> = by_group.into_iter().collect();
    ranked_groups.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let mut lines = vec![
        "Recap".to_string(),
        format!("- Actionable findings: {}", summary.actionable),
        format!("- Heuristic observations: {}", summary.heuristic),
        format!("- Total findings: {}", summary.total),
        format!("- Errors: {}", summary.errors),
        format!("- Warnings: {}", summary.warnings),
        format!(
            "- Finding scopes: sitewide {}, template {}, page {}",
            by_scope.get("sitewide").copied().unwrap_or(0),
            by_scope.get("template").copied().unwrap_or(0),
            by_scope.get("page").copied().unwrap_or(0)
        ),
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

fn artifact_status_lines(artifact: &AuditArtifact) -> Vec<String> {
    let mut lines = Vec::new();
    if artifact.status != AuditStatus::Complete {
        lines.push(format!(
            "- Status: {}",
            artifact_status_label(artifact.status)
        ));
    }
    if let Some(ratio) = artifact.completion_ratio.as_deref() {
        lines.push(format!("- Completion ratio: {}", ratio));
    }
    if let Some(reason) = artifact.truncation_reason.as_deref() {
        lines.push(format!("- Truncation reason: {}", reason));
    }
    if let Some(crawl) = artifact.crawl.as_ref() {
        lines.push(format!(
            "- Crawl stats: engine={} visited={} discovered={} queued={} retries={} fetch_failures={} skipped_non_html={} elapsed_ms={} pages_per_minute={}",
            crawl.engine,
            crawl.visited_pages,
            crawl.discovered_internal_routes,
            crawl.queued_routes_remaining,
            crawl.fetch_retries,
            crawl.fetch_failures,
            crawl.skipped_non_html,
            crawl.elapsed_ms,
            crawl.pages_per_minute
        ));
        lines.push(format!(
            "- Runtime timings: avg_fetch_ms={} avg_page_process_ms={} avg_partial_audit_ms={} checkpoints_written={} partial_artifacts_written={}",
            crawl.average_fetch_ms,
            crawl.average_page_process_ms,
            crawl.average_partial_audit_ms,
            crawl.checkpoints_written,
            crawl.partial_artifacts_written
        ));
        if !crawl.slowest_paths.is_empty() {
            let slowest = crawl
                .slowest_paths
                .iter()
                .map(|path| {
                    format!(
                        "{} (fetch={}ms process={}ms)",
                        path.url, path.fetch_ms, path.process_ms
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            lines.push(format!("- Slowest paths: {}", slowest));
        }
    }
    if let Some(performance) = artifact.performance.as_ref() {
        if !performance.phases.is_empty() {
            let phases = performance
                .phases
                .iter()
                .map(|phase| format!("{}={}ms", phase.name, phase.elapsed_us / 1_000))
                .collect::<Vec<_>>()
                .join(", ");
            lines.push(format!("- Phase timings: {}", phases));
        }
        if !performance.rule_groups.is_empty() {
            let groups = performance
                .rule_groups
                .iter()
                .take(5)
                .map(|timing| {
                    format!(
                        "{}={}ms/{} findings",
                        timing.group,
                        timing.elapsed_us / 1_000,
                        timing.findings
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            lines.push(format!("- Slowest rule groups: {}", groups));
        }
        if !performance.bottlenecks.is_empty() {
            let bottlenecks = performance
                .bottlenecks
                .iter()
                .take(5)
                .map(|item| {
                    format!(
                        "{}:{}={}ms/{}",
                        item.kind,
                        item.name,
                        item.elapsed_us / 1_000,
                        format_basis_points(item.share_basis_points)
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            lines.push(format!("- Performance bottlenecks: {}", bottlenecks));
        }
    }
    lines
}

fn artifact_status_label(status: AuditStatus) -> &'static str {
    match status {
        AuditStatus::Complete => "complete",
        AuditStatus::Partial => "partial",
        AuditStatus::Failed => "failed",
    }
}

fn format_basis_points(value: u32) -> String {
    format!("{}.{:02}%", value / 100, value % 100)
}

pub fn render_text_artifact(
    artifact: &AuditArtifact,
    success_message: &str,
    audit_path: Option<&Path>,
) -> String {
    if artifact.findings.is_empty() && artifact.status == AuditStatus::Complete {
        let mut lines = vec![success_message.to_string()];
        if let Some(path) = audit_path {
            lines.push(String::new());
            lines.push(format!("Audit results: {}", path.display()));
        }
        return lines.join("\n");
    }

    let mut lines = vec!["Audit Report".to_string(), String::new()];
    lines.extend(artifact_status_lines(artifact));
    if !lines.last().is_some_and(|line| line.is_empty()) {
        lines.push(String::new());
    }

    for scope in [
        FindingScope::Sitewide,
        FindingScope::Template,
        FindingScope::Page,
    ] {
        let actionable = scope_findings(&artifact.findings, scope, false);
        let heuristic = scope_findings(&artifact.findings, scope, true);
        let scope_total = actionable.len() + heuristic.len();
        if scope_total == 0 {
            continue;
        }
        lines.push(format!("{} ({})", scope_title(scope), scope_total));
        for (is_heuristic, section_findings) in [(false, actionable), (true, heuristic)] {
            if section_findings.is_empty() {
                continue;
            }
            lines.push(format!(
                "{} ({})",
                section_title(is_heuristic),
                section_findings.len()
            ));
            for (group_name, group_clusters) in cluster_findings(&section_findings) {
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
    }
    lines.extend(build_recap_lines(&artifact.findings));
    if let Some(path) = audit_path {
        lines.push(String::new());
        lines.push(format!("Audit results: {}", path.display()));
    }
    lines.join("\n")
}

pub fn render_markdown_artifact(artifact: &AuditArtifact, audit_path: Option<&Path>) -> String {
    let mut lines = vec!["# Audit Report".to_string(), String::new()];
    lines.push(format!("- Command: `{}`", artifact.command));
    lines.push(format!(
        "- Status: `{}`",
        artifact_status_label(artifact.status)
    ));
    if let Some(ratio) = artifact.completion_ratio.as_deref() {
        lines.push(format!("- Completion ratio: `{}`", ratio));
    }
    if let Some(reason) = artifact.truncation_reason.as_deref() {
        lines.push(format!("- Truncation reason: {}", reason));
    }
    if let Some(path) = audit_path {
        lines.push(format!("- Audit artifact: `{}`", path.display()));
    }
    lines.push(String::new());
    lines.push("## Summary".to_string());
    lines.push(format!("- Total findings: `{}`", artifact.summary.total));
    lines.push(format!("- Errors: `{}`", artifact.summary.errors));
    lines.push(format!("- Warnings: `{}`", artifact.summary.warnings));
    lines.push(format!("- Actionable: `{}`", artifact.summary.actionable));
    lines.push(format!("- Heuristic: `{}`", artifact.summary.heuristic));
    if let Some(crawl) = artifact.crawl.as_ref() {
        lines.push(format!(
            "- Crawl: engine=`{}` visited=`{}` discovered=`{}` queued=`{}` elapsed_ms=`{}` pages_per_minute=`{}`",
            crawl.engine,
            crawl.visited_pages,
            crawl.discovered_internal_routes,
            crawl.queued_routes_remaining,
            crawl.elapsed_ms,
            crawl.pages_per_minute
        ));
        lines.push(format!(
            "- Timings: avg_fetch_ms=`{}` avg_page_process_ms=`{}` avg_partial_audit_ms=`{}` checkpoints_written=`{}` partial_artifacts_written=`{}`",
            crawl.average_fetch_ms,
            crawl.average_page_process_ms,
            crawl.average_partial_audit_ms,
            crawl.checkpoints_written,
            crawl.partial_artifacts_written
        ));
        if !crawl.slowest_paths.is_empty() {
            lines.push("- Slowest paths:".to_string());
            for path in &crawl.slowest_paths {
                lines.push(format!(
                    "  - `{}` fetch=`{}ms` process=`{}ms`",
                    path.url,
                    path.fetch_us / 1_000,
                    path.process_us / 1_000
                ));
            }
        }
    }
    if let Some(performance) = artifact.performance.as_ref() {
        if !performance.phases.is_empty() {
            lines.push("- Phase timings:".to_string());
            for phase in &performance.phases {
                lines.push(format!(
                    "  - `{}` elapsed=`{}ms`",
                    phase.name,
                    phase.elapsed_us / 1_000
                ));
            }
        }
        if !performance.rule_groups.is_empty() {
            lines.push("- Slowest rule groups:".to_string());
            for timing in performance.rule_groups.iter().take(10) {
                lines.push(format!(
                    "  - `{}` elapsed=`{}ms` findings=`{}`",
                    timing.group,
                    timing.elapsed_us / 1_000,
                    timing.findings
                ));
            }
        }
        if !performance.bottlenecks.is_empty() {
            lines.push("- Runtime bottlenecks:".to_string());
            for bottleneck in performance.bottlenecks.iter().take(8) {
                let mut line = format!(
                    "  - `{}` `{}` elapsed=`{}ms` share=`{}`",
                    bottleneck.kind,
                    bottleneck.name,
                    bottleneck.elapsed_us / 1_000,
                    format_basis_points(bottleneck.share_basis_points)
                );
                if let Some(findings) = bottleneck.findings {
                    line.push_str(&format!(" findings=`{}`", findings));
                }
                if let Some(recommendation) = &bottleneck.recommendation {
                    line.push_str(&format!(" recommendation=`{}`", recommendation));
                }
                lines.push(line);
            }
        }
        if !performance.observations.is_empty() {
            lines.push("- Performance observations:".to_string());
            for observation in &performance.observations {
                lines.push(format!("  - {}", observation));
            }
        }
    }
    lines.push(String::new());

    for scope in [
        FindingScope::Sitewide,
        FindingScope::Template,
        FindingScope::Page,
    ] {
        let scoped = artifact
            .findings
            .iter()
            .filter(|finding| finding.scope == scope)
            .cloned()
            .collect::<Vec<_>>();
        if scoped.is_empty() {
            continue;
        }
        lines.push(format!("## {}", scope_title(scope)));
        for (group_name, group_clusters) in cluster_findings(&scoped) {
            lines.push(format!("### {}", group_name));
            for cluster in group_clusters {
                let mut line = format!(
                    "- `{}` `{}` {}",
                    cluster.examples.first().cloned().unwrap_or_default(),
                    cluster.rule_id,
                    cluster.message
                );
                if cluster.count > 1 {
                    line.push_str(&format!(" (`{} occurrences`)", cluster.count));
                }
                lines.push(line);
            }
            lines.push(String::new());
        }
    }
    lines.join("\n")
}

pub fn render_text(
    findings: &[Finding],
    success_message: &str,
    audit_path: Option<&Path>,
) -> String {
    let artifact = build_audit_artifact("audit", findings, AuditStatus::Complete, None, None);
    render_text_artifact(&artifact, success_message, audit_path)
}

pub fn render_json(findings: &[Finding]) -> Result<String> {
    Ok(serde_json::to_string_pretty(findings)?)
}

pub fn render_audit_artifact_json(artifact: &AuditArtifact) -> Result<String> {
    Ok(serde_json::to_string_pretty(artifact)?)
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
    artifact: &AuditArtifact,
) -> Result<()> {
    let trend_path = artifact_dir.join(format!("{}-trends.json", command_name));
    let mut payload: Vec<serde_json::Value> = fs::read_to_string(&trend_path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default();
    payload.push(json!({
        "timestamp": artifact.generated_at,
        "status": artifact_status_label(artifact.status),
        "total": artifact.summary.total,
        "errors": artifact.summary.errors,
        "warnings": artifact.summary.warnings,
        "actionable": artifact.summary.actionable,
        "heuristic": artifact.summary.heuristic,
        "elapsed_ms": artifact.crawl.as_ref().map(|crawl| crawl.elapsed_us / 1_000),
        "pages_per_minute": artifact.crawl.as_ref().map(|crawl| crawl.pages_per_minute),
        "avg_fetch_ms": artifact.crawl.as_ref().map(|crawl| crawl.average_fetch_us / 1_000),
        "avg_partial_audit_ms": artifact
            .crawl
            .as_ref()
            .map(|crawl| crawl.average_partial_audit_us / 1_000),
    }));
    let slice_start = payload.len().saturating_sub(50);
    fs::write(
        trend_path,
        serde_json::to_string_pretty(&payload[slice_start..])?,
    )?;
    Ok(())
}

pub fn write_audit_artifact(
    artifact: &AuditArtifact,
    base_dir: &Path,
    command_name: &str,
    keep: usize,
) -> Result<PathBuf> {
    let artifact_dir = base_dir.join(".seogeo-reports");
    fs::create_dir_all(&artifact_dir)?;
    let timestamp = artifact.generated_at;
    let history_path = artifact_dir.join(format!("{}-{}.json", command_name, timestamp));
    let latest_path = artifact_dir.join(format!("{}-latest.json", command_name));
    let payload = render_audit_artifact_json(artifact)?;
    fs::write(&history_path, &payload)?;
    fs::write(&latest_path, payload)?;
    prune_old_audit_logs(&artifact_dir, command_name, keep)?;
    update_trend_history(&artifact_dir, command_name, artifact)?;
    Ok(latest_path)
}

pub fn write_partial_audit_artifact(
    artifact: &AuditArtifact,
    base_dir: &Path,
    command_name: &str,
) -> Result<PathBuf> {
    let artifact_dir = base_dir.join(".seogeo-reports");
    fs::create_dir_all(&artifact_dir)?;
    let latest_path = artifact_dir.join(format!("{}-latest.json", command_name));
    fs::write(&latest_path, render_audit_artifact_json(artifact)?)?;
    Ok(latest_path)
}

pub fn write_progress_audit_artifact(
    artifact: &AuditArtifact,
    base_dir: &Path,
    command_name: &str,
) -> Result<PathBuf> {
    let artifact_dir = base_dir.join(".seogeo-reports");
    fs::create_dir_all(&artifact_dir)?;
    let latest_path = artifact_dir.join(format!("{}-progress-latest.json", command_name));
    fs::write(&latest_path, render_audit_artifact_json(artifact)?)?;
    Ok(latest_path)
}

#[cfg(test)]
mod tests {
    use super::{
        build_audit_artifact, build_recap_lines, render_audit_artifact_json,
        render_markdown_artifact, render_sarif, render_text, render_text_artifact,
        write_audit_artifact, write_partial_audit_artifact, write_progress_audit_artifact,
    };
    use seogeo_contracts::{AuditStatus, CrawlStats, Finding, FindingScope};

    fn sample_finding() -> Finding {
        Finding {
            rule_id: "SEO001".into(),
            message: "missing title".into(),
            path: "index.html".into(),
            line: 1,
            column: 1,
            severity: "error".into(),
            suggestion: None,
            scope: FindingScope::Page,
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
        let artifact = build_audit_artifact(
            "check",
            &[sample_finding()],
            AuditStatus::Complete,
            None,
            None,
        );
        let latest = write_audit_artifact(&artifact, temp_dir.path(), "check", 5).unwrap();
        assert!(latest.exists());
    }

    #[test]
    fn writes_partial_audit_artifact_without_history() {
        let temp_dir = tempfile::tempdir().unwrap();
        let artifact = build_audit_artifact(
            "crawl",
            &[sample_finding()],
            AuditStatus::Partial,
            None,
            Some("checkpoint".into()),
        );
        let latest = write_partial_audit_artifact(&artifact, temp_dir.path(), "crawl").unwrap();
        assert!(latest.exists());
        assert!(
            !temp_dir
                .path()
                .join(".seogeo-reports/crawl-trends.json")
                .exists()
        );
    }

    #[test]
    fn writes_progress_audit_artifact_to_progress_path() {
        let temp_dir = tempfile::tempdir().unwrap();
        let artifact = build_audit_artifact(
            "crawl",
            &[sample_finding()],
            AuditStatus::Partial,
            None,
            Some("checkpoint".into()),
        );
        let latest = write_progress_audit_artifact(&artifact, temp_dir.path(), "crawl").unwrap();
        assert!(latest.exists());
        assert!(latest.ends_with(".seogeo-reports/crawl-progress-latest.json"));
    }

    #[test]
    fn clusters_repeated_findings_in_text_output() {
        let finding_a = sample_finding();
        let finding_b = Finding {
            path: "about.html".into(),
            ..sample_finding()
        };
        let text = render_text(&[finding_a, finding_b], "ok", None);
        assert!(text.contains("Finding scopes"));
        assert!(text.contains("Confidence mix"));
        assert!(text.contains("Rule classes"));
    }

    #[test]
    fn separates_sitewide_template_and_page_sections() {
        let actionable = sample_finding();
        let heuristic = Finding {
            rule_id: "GEO007".into(),
            message: "thin block".into(),
            path: "page.html".into(),
            line: 1,
            column: 1,
            severity: "warning".into(),
            suggestion: None,
            scope: FindingScope::Page,
        };
        let text = render_text(&[actionable, heuristic], "ok", None);
        assert!(text.contains("Page Findings"));
        assert!(text.contains("Actionable findings: 1"));
        assert!(text.contains("Heuristic observations: 1"));
    }

    #[test]
    fn renders_partial_artifact_metadata() {
        let artifact = build_audit_artifact(
            "crawl",
            &[sample_finding()],
            AuditStatus::Partial,
            Some(CrawlStats {
                engine: "http".into(),
                visited_pages: 10,
                discovered_internal_routes: 40,
                queued_routes_remaining: 30,
                max_pages: 10,
                fetch_failures: 1,
                fetch_retries: 2,
                skipped_non_html: 3,
                truncated: true,
                elapsed_ms: 1_200,
                elapsed_us: 1_200_000,
                pages_per_minute: 500,
                checkpoints_written: 2,
                progress_artifacts_written: 1,
                partial_artifacts_written: 1,
                total_fetch_ms: 400,
                total_fetch_us: 400_000,
                average_fetch_ms: 40,
                average_fetch_us: 40_000,
                total_page_process_ms: 300,
                total_page_process_us: 300_000,
                average_page_process_ms: 30,
                average_page_process_us: 30_000,
                total_partial_audit_ms: 80,
                total_partial_audit_us: 80_000,
                average_partial_audit_ms: 80,
                average_partial_audit_us: 80_000,
                total_sitemap_seed_us: 9_000,
                total_optional_artifact_fetch_us: 10_000,
                total_snapshot_build_us: 20_000,
                total_snapshot_write_us: 15_000,
                total_queue_selection_us: 5_000,
                total_planner_update_us: 7_000,
                total_link_extraction_us: 30_000,
                total_progress_callback_us: 4_000,
                total_checkpoint_write_us: 11_000,
                total_progress_artifact_write_us: 6_000,
                total_partial_audit_build_us: 70_000,
                total_partial_artifact_write_us: 10_000,
                total_rule_evaluation_us: 40_000,
                total_policy_apply_us: 10_000,
                total_final_audit_us: 50_000,
                total_overhead_us: 260_000,
                slowest_paths: vec![seogeo_contracts::SlowCrawlPath {
                    url: "https://example.test/about".into(),
                    fetch_us: 55_000,
                    process_us: 44_000,
                    fetch_ms: 55,
                    process_ms: 44,
                }],
            }),
            Some("hit max-pages budget".into()),
        );
        let text = render_text_artifact(&artifact, "ok", None);
        assert!(text.contains("Status: partial"));
        assert!(text.contains("Completion ratio"));
        assert!(text.contains("Runtime timings"));
        let markdown = render_markdown_artifact(&artifact, None);
        assert!(markdown.contains("## Summary"));
        assert!(markdown.contains("Slowest paths"));
        let json = render_audit_artifact_json(&artifact).unwrap();
        assert!(json.contains("\"status\": \"partial\""));
    }
}

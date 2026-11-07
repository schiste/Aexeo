use aexeo_contracts::AuditArtifact;
use aexeo_core::DiffResult;
use aexeo_core::config::ConfigWarning;
use anyhow::Result;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
struct DiffSummary {
    new: usize,
    resolved: usize,
    unchanged: usize,
}

#[derive(Debug, Clone, Serialize)]
struct AuditCommandOutput<'a> {
    command: &'a str,
    success: bool,
    audit_path: Option<String>,
    summary: &'a aexeo_contracts::AuditSummary,
    findings: &'a [aexeo_contracts::Finding],
    artifact: &'a AuditArtifact,
    priorities: Vec<aexeo_core::PriorityIssue>,
    warnings: Vec<ConfigWarning>,
    #[serde(skip_serializing_if = "Option::is_none")]
    link_graph_summary: Option<&'a serde_json::Value>,
}

/// The page HTML is in the saved artifact at `audit_path`. Repeating it here made
/// a 100-page crawl print about 15 MB.
fn without_page_html(artifact: &AuditArtifact) -> Option<AuditArtifact> {
    let site = artifact.site.as_ref()?;
    if site.pages.iter().all(|page| page.raw_html.is_empty()) {
        return None;
    }
    let mut slim = artifact.clone();
    if let Some(site) = slim.site.as_mut() {
        for page in &mut site.pages {
            page.raw_html.clear();
        }
    }
    Some(slim)
}

#[derive(Debug, Clone, Serialize)]
struct DiffCommandOutput<'a> {
    command: &'a str,
    success: bool,
    summary: DiffSummary,
    diff: &'a DiffResult,
    warnings: Vec<ConfigWarning>,
}

#[derive(Debug, Clone, Serialize)]
struct FailedCommandOutput {
    command: String,
    success: bool,
    error: String,
    warnings: Vec<ConfigWarning>,
}

fn diff_summary(diff: &DiffResult) -> DiffSummary {
    DiffSummary {
        new: diff.new_findings.len(),
        resolved: diff.resolved_findings.len(),
        unchanged: diff.unchanged_findings.len(),
    }
}

pub fn render_audit_command_json(
    command: &str,
    artifact: &AuditArtifact,
    success: bool,
    audit_path: Option<String>,
    warnings: Vec<ConfigWarning>,
) -> Result<String> {
    render_audit_command_json_with_graph(command, artifact, success, audit_path, warnings, None)
}

pub fn render_audit_command_json_with_graph(
    command: &str,
    artifact: &AuditArtifact,
    success: bool,
    audit_path: Option<String>,
    warnings: Vec<ConfigWarning>,
    link_graph_summary: Option<&serde_json::Value>,
) -> Result<String> {
    let slim = without_page_html(artifact);
    Ok(serde_json::to_string_pretty(&AuditCommandOutput {
        command,
        success,
        audit_path,
        summary: &artifact.summary,
        findings: &artifact.findings,
        artifact: slim.as_ref().unwrap_or(artifact),
        priorities: aexeo_core::prioritize_findings(&artifact.findings, 10),
        warnings,
        link_graph_summary,
    })?)
}

pub fn render_diff_command_json(
    command: &str,
    diff: &DiffResult,
    success: bool,
    warnings: Vec<ConfigWarning>,
) -> Result<String> {
    Ok(serde_json::to_string_pretty(&DiffCommandOutput {
        command,
        success,
        summary: diff_summary(diff),
        diff,
        warnings,
    })?)
}

pub fn render_failed_command_json(
    command: &str,
    error: impl Into<String>,
    warnings: Vec<ConfigWarning>,
) -> Result<String> {
    Ok(serde_json::to_string_pretty(&FailedCommandOutput {
        command: command.to_string(),
        success: false,
        error: error.into(),
        warnings,
    })?)
}

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
    warnings: Vec<ConfigWarning>,
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
    Ok(serde_json::to_string_pretty(&AuditCommandOutput {
        command,
        success,
        audit_path,
        summary: &artifact.summary,
        findings: &artifact.findings,
        artifact,
        warnings,
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

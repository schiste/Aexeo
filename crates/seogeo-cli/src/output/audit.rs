use anyhow::Result;
use seogeo_contracts::Finding;
use seogeo_core::DiffResult;
use seogeo_core::config::ConfigWarning;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
struct FindingSummary {
    total: usize,
    errors: usize,
    warnings: usize,
}

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
    summary: FindingSummary,
    findings: &'a [Finding],
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

fn finding_summary(findings: &[Finding]) -> FindingSummary {
    let errors = findings.iter().filter(|finding| finding.is_error()).count();
    FindingSummary {
        total: findings.len(),
        errors,
        warnings: findings.len().saturating_sub(errors),
    }
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
    findings: &[Finding],
    success: bool,
    audit_path: Option<String>,
    warnings: Vec<ConfigWarning>,
) -> Result<String> {
    Ok(serde_json::to_string_pretty(&AuditCommandOutput {
        command,
        success,
        audit_path,
        summary: finding_summary(findings),
        findings,
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

use anyhow::Result;
use seogeo_contracts::Finding;
use seogeo_core::DiffResult;
use seogeo_core::PluginManifestCheck;
use seogeo_core::config::ConfigWarning;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct FindingSummary {
    total: usize,
    errors: usize,
    warnings: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiffSummary {
    new: usize,
    resolved: usize,
    unchanged: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct AuditCommandOutput<'a> {
    command: &'a str,
    success: bool,
    audit_path: Option<String>,
    summary: FindingSummary,
    findings: &'a [Finding],
    warnings: Vec<ConfigWarning>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiffCommandOutput<'a> {
    command: &'a str,
    success: bool,
    summary: DiffSummary,
    diff: &'a DiffResult,
    warnings: Vec<ConfigWarning>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConfigCommandOutput<T> {
    command: &'static str,
    success: bool,
    warnings: Vec<ConfigWarning>,
    config: T,
}

#[derive(Debug, Clone, Serialize)]
pub struct ListCommandOutput<T> {
    command: &'static str,
    success: bool,
    items: T,
}

#[derive(Debug, Clone, Serialize)]
pub struct TextCommandOutput {
    command: &'static str,
    success: bool,
    kind: String,
    output: String,
    warnings: Vec<ConfigWarning>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PathsCommandOutput {
    command: &'static str,
    success: bool,
    action: String,
    paths: Vec<String>,
    warnings: Vec<ConfigWarning>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PathCommandOutput {
    command: &'static str,
    success: bool,
    path: String,
    warnings: Vec<ConfigWarning>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PluginCheckCommandOutput {
    command: &'static str,
    success: bool,
    plugin: PluginManifestCheck,
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

pub fn render_config_command_json<T: Serialize>(
    command: &'static str,
    config: T,
    warnings: Vec<ConfigWarning>,
) -> Result<String> {
    Ok(serde_json::to_string_pretty(&ConfigCommandOutput {
        command,
        success: true,
        warnings,
        config,
    })?)
}

pub fn render_list_command_json<T: Serialize>(command: &'static str, items: T) -> Result<String> {
    Ok(serde_json::to_string_pretty(&ListCommandOutput {
        command,
        success: true,
        items,
    })?)
}

pub fn render_text_command_json(
    command: &'static str,
    kind: &str,
    output: String,
    warnings: Vec<ConfigWarning>,
) -> Result<String> {
    Ok(serde_json::to_string_pretty(&TextCommandOutput {
        command,
        success: true,
        kind: kind.to_string(),
        output,
        warnings,
    })?)
}

pub fn render_paths_command_json(
    command: &'static str,
    action: &str,
    success: bool,
    paths: Vec<String>,
    warnings: Vec<ConfigWarning>,
) -> Result<String> {
    Ok(serde_json::to_string_pretty(&PathsCommandOutput {
        command,
        success,
        action: action.to_string(),
        paths,
        warnings,
    })?)
}

pub fn render_path_command_json(
    command: &'static str,
    success: bool,
    path: String,
    warnings: Vec<ConfigWarning>,
) -> Result<String> {
    Ok(serde_json::to_string_pretty(&PathCommandOutput {
        command,
        success,
        path,
        warnings,
    })?)
}

pub fn render_plugin_check_command_json(plugin: PluginManifestCheck) -> Result<String> {
    Ok(serde_json::to_string_pretty(&PluginCheckCommandOutput {
        command: "plugin-check",
        success: true,
        plugin,
    })?)
}

pub fn emit_config_warnings(warnings: &[ConfigWarning]) {
    for warning in warnings {
        eprintln!("{} {}", warning.code, warning.message);
    }
}

use crate::cli::render_cli_reference;
use crate::output::{
    render_audit_command_json, render_diff_command_json, render_paths_command_json,
};
use aexeo_contracts::AuditStatus;
use aexeo_core::{
    build_audit_artifact, find_reference_doc_drift, load_audit_artifact, load_findings_from_audit,
    render_audit_artifact_json, render_diff_text, render_markdown_artifact, render_sarif,
    render_text_artifact, run_repo_quality_checks, write_audit_artifact, write_reference_documents,
};
use anyhow::{Result, anyhow};
use std::fs;
use std::path::{Path, PathBuf};

fn canonicalize_or_keep(path: &str) -> PathBuf {
    PathBuf::from(path)
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from(path))
}

pub fn command_quality(path: &str, output_format: &str) -> Result<i32> {
    let root = canonicalize_or_keep(path);
    let cli_reference = render_cli_reference()?;
    let findings = run_repo_quality_checks(&root, &cli_reference)?;
    let audit_artifact =
        build_audit_artifact("quality", &findings, AuditStatus::Complete, None, None);
    let audit_path = write_audit_artifact(&audit_artifact, &root, "quality", 5)?;
    match output_format {
        "json" => println!(
            "{}",
            render_audit_command_json(
                "quality",
                &audit_artifact,
                !findings.iter().any(|finding| finding.is_error()),
                Some(audit_path.display().to_string()),
                Vec::new(),
            )?
        ),
        "sarif" => println!("{}", render_sarif(&findings, "aexeo")?),
        _ => println!(
            "{}",
            render_text_artifact(
                &audit_artifact,
                "All quality checks passed.",
                Some(&audit_path)
            )
        ),
    }
    Ok(if findings.iter().any(|finding| finding.is_error()) {
        1
    } else {
        0
    })
}

pub fn command_docs(action: &str, path: &str, output_format: &str) -> Result<i32> {
    let root = canonicalize_or_keep(path);
    let cli_reference = render_cli_reference()?;
    if action == "generate" {
        let changed = write_reference_documents(&root, cli_reference)?;
        let changed_paths = changed
            .iter()
            .map(|item| item.display().to_string())
            .collect::<Vec<_>>();
        match output_format {
            "json" => println!(
                "{}",
                render_paths_command_json("docs", action, true, changed_paths, Vec::new())?
            ),
            _ if changed.is_empty() => println!("Generated docs already up to date."),
            _ => {
                for item in changed {
                    println!("{}", item.display());
                }
            }
        }
        return Ok(0);
    }
    let drifted = find_reference_doc_drift(&root, cli_reference)?;
    let success = drifted.is_empty();
    let drifted_paths = drifted
        .iter()
        .map(|item| item.display().to_string())
        .collect::<Vec<_>>();
    match output_format {
        "json" => println!(
            "{}",
            render_paths_command_json("docs", action, success, drifted_paths, Vec::new())?
        ),
        _ if success => println!("Generated docs are up to date."),
        _ => {
            for item in drifted {
                println!("{}", item.display());
            }
        }
    }
    Ok(if success { 0 } else { 1 })
}

pub fn command_diff(baseline: &str, current: &str, output_format: &str) -> Result<i32> {
    let baseline_findings = load_findings_from_audit(Path::new(baseline))?;
    let current_findings = load_findings_from_audit(Path::new(current))?;
    let diff = aexeo_core::diff_finding_sets(&baseline_findings, &current_findings);
    match output_format {
        "json" => println!(
            "{}",
            render_diff_command_json("diff", &diff, diff.new_findings.is_empty(), Vec::new())?
        ),
        _ => println!("{}", render_diff_text(&diff)),
    }
    Ok(if diff.new_findings.is_empty() { 0 } else { 1 })
}

pub fn command_trend(command_name: &str, path: &str, output_format: &str) -> Result<i32> {
    let root = canonicalize_or_keep(path);
    let trend_path = root
        .join(".aexeo-reports")
        .join(format!("{}-trends.json", command_name));
    if !trend_path.exists() {
        println!("No trend history found.");
        return Ok(0);
    }
    let text = fs::read_to_string(&trend_path)?;
    let payload: serde_json::Value = serde_json::from_str(&text)?;
    if output_format == "json" {
        println!("{}", serde_json::to_string_pretty(&payload)?);
        return Ok(0);
    }
    let entries = payload
        .as_array()
        .ok_or_else(|| anyhow!("trend payload must be a JSON array"))?;
    println!("Trend Report");
    println!();
    println!("Command: {}", command_name);
    println!("Entries: {}", entries.len());
    for entry in entries.iter().rev().take(10).rev() {
        println!(
            "- ts={} total={} errors={} warnings={}",
            entry
                .get("timestamp")
                .and_then(|v| v.as_u64())
                .unwrap_or_default(),
            entry
                .get("total")
                .and_then(|v| v.as_u64())
                .unwrap_or_default(),
            entry
                .get("errors")
                .and_then(|v| v.as_u64())
                .unwrap_or_default(),
            entry
                .get("warnings")
                .and_then(|v| v.as_u64())
                .unwrap_or_default(),
        );
    }
    Ok(0)
}

pub fn command_report_render(audit: &str, output_format: &str) -> Result<i32> {
    let artifact = load_audit_artifact(Path::new(audit))?;
    match output_format {
        "json" => println!("{}", render_audit_artifact_json(&artifact)?),
        "md" => println!(
            "{}",
            render_markdown_artifact(&artifact, Some(Path::new(audit)))
        ),
        "sarif" => println!("{}", render_sarif(&artifact.findings, "aexeo")?),
        _ => println!(
            "{}",
            render_text_artifact(&artifact, "All checks passed.", Some(Path::new(audit)))
        ),
    }
    Ok(0)
}

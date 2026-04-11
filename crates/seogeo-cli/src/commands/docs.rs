use crate::cli::render_cli_reference;
use anyhow::{Result, anyhow};
use seogeo_core::{
    find_reference_doc_drift, load_findings_from_audit, render_diff_text, render_json,
    render_sarif, render_text, run_repo_quality_checks, write_audit_artifact,
    write_reference_documents,
};
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
    let audit_path = write_audit_artifact(&findings, &root, "quality", 5)?;
    match output_format {
        "json" => println!("{}", render_json(&findings)?),
        "sarif" => println!("{}", render_sarif(&findings, "seogeo")?),
        _ => println!(
            "{}",
            render_text(&findings, "All quality checks passed.", Some(&audit_path))
        ),
    }
    Ok(if findings.iter().any(|finding| finding.is_error()) {
        1
    } else {
        0
    })
}

pub fn command_docs(action: &str, path: &str) -> Result<i32> {
    let root = canonicalize_or_keep(path);
    let cli_reference = render_cli_reference()?;
    if action == "generate" {
        let changed = write_reference_documents(&root, cli_reference)?;
        if changed.is_empty() {
            println!("Generated docs already up to date.");
            return Ok(0);
        }
        for item in changed {
            println!("{}", item.display());
        }
        return Ok(0);
    }
    let drifted = find_reference_doc_drift(&root, cli_reference)?;
    if drifted.is_empty() {
        println!("Generated docs are up to date.");
        return Ok(0);
    }
    for item in drifted {
        println!("{}", item.display());
    }
    Ok(1)
}

pub fn command_diff(baseline: &str, current: &str, output_format: &str) -> Result<i32> {
    let baseline_findings = load_findings_from_audit(Path::new(baseline))?;
    let current_findings = load_findings_from_audit(Path::new(current))?;
    let diff = seogeo_core::diff_finding_sets(&baseline_findings, &current_findings);
    match output_format {
        "json" => println!("{}", serde_json::to_string_pretty(&diff)?),
        _ => println!("{}", render_diff_text(&diff)),
    }
    Ok(if diff.new_findings.is_empty() { 0 } else { 1 })
}

pub fn command_trend(command_name: &str, path: &str, output_format: &str) -> Result<i32> {
    let root = canonicalize_or_keep(path);
    let trend_path = root
        .join(".seogeo-reports")
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

use anyhow::{Result, bail};
use clap::ArgMatches;
use seogeo_core::{
    GroundingCoverageGap, GroundingSiteAnalysis, TrustSurfaceReconciliation, TruthAssessment,
    TruthStructuredSource, assess_truth_layer, discover_truth_manifest,
    import_trust_surface_records, load_site, map_grounding_queries, reconcile_trust_surfaces,
};
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};

use crate::commands::common::{canonicalize_or_keep, required_arg};
use crate::output::{render_data_command_json, render_failed_command_json};

pub fn command_intelligence(submatches: &ArgMatches) -> Result<i32> {
    match submatches.subcommand() {
        Some(("grounding-map", map_matches)) => command_grounding_map(map_matches),
        Some(("truth", truth_matches)) => match truth_matches.subcommand() {
            Some(("assess", assess_matches)) => command_truth_assess(assess_matches),
            Some((other, _)) => bail!("unsupported truth command: {}", other),
            None => bail!("missing truth subcommand"),
        },
        Some(("trust-surface", trust_matches)) => match trust_matches.subcommand() {
            Some(("import", import_matches)) => command_trust_surface_import(import_matches),
            Some(("reconcile", reconcile_matches)) => {
                command_trust_surface_reconcile(reconcile_matches)
            }
            Some((other, _)) => bail!("unsupported trust-surface command: {}", other),
            None => bail!("missing trust-surface subcommand"),
        },
        Some((other, _)) => bail!("unsupported intelligence command: {}", other),
        None => bail!("missing intelligence subcommand"),
    }
}

fn command_grounding_map(submatches: &ArgMatches) -> Result<i32> {
    let format = required_arg(submatches, "format")?;
    let root = PathBuf::from(required_arg(submatches, "path")?);
    match load_site(&root).map(|site| map_grounding_queries(&site)) {
        Ok(report) => {
            let report_path = write_report(&root, "grounding-map-latest.json", &report)?;
            match format {
                "json" => println!(
                    "{}",
                    render_data_command_json(
                        "intelligence grounding-map",
                        true,
                        serde_json::json!({
                            "report_path": report_path.to_string_lossy(),
                            "analysis": report,
                        }),
                        Vec::new()
                    )?
                ),
                _ => println!("{}", grounding_text(&report, &report_path)),
            }
            Ok(0)
        }
        Err(error) => emit_failure("intelligence grounding-map", format, error),
    }
}

fn command_truth_assess(submatches: &ArgMatches) -> Result<i32> {
    let format = required_arg(submatches, "format")?;
    let root = PathBuf::from(required_arg(submatches, "path")?);
    let manifest_path = submatches.get_one::<String>("manifest").map(PathBuf::from);
    let manifest = discover_truth_manifest(&root, manifest_path.as_deref())?;
    match load_site(&root)
        .map(|site| assess_truth_layer(&site, manifest.as_ref().map(|(_, item)| item)))
    {
        Ok(report) => {
            let report_path = write_report(&root, "truth-layer-latest.json", &report)?;
            let manifest_path = manifest.as_ref().map(|(path, _)| {
                canonicalize_or_keep(&path.to_string_lossy())
                    .display()
                    .to_string()
            });
            match format {
                "json" => println!(
                    "{}",
                    render_data_command_json(
                        "intelligence truth assess",
                        true,
                        serde_json::json!({
                            "manifest_path": manifest_path,
                            "report_path": report_path.to_string_lossy(),
                            "assessment": report,
                        }),
                        Vec::new()
                    )?
                ),
                _ => println!(
                    "{}",
                    truth_text(&report, manifest_path.as_deref(), &report_path)
                ),
            }
            Ok(0)
        }
        Err(error) => emit_failure("intelligence truth assess", format, error),
    }
}

fn command_trust_surface_import(submatches: &ArgMatches) -> Result<i32> {
    let format = required_arg(submatches, "format")?;
    let path = PathBuf::from(required_arg(submatches, "path")?);
    let root = submatches.get_one::<String>("root").map(PathBuf::from);
    match import_trust_surface_records(&path) {
        Ok(records) => {
            let report_path = if let Some(root) = root.as_ref() {
                Some(write_report(
                    root,
                    "trust-surface-import-latest.json",
                    &records,
                )?)
            } else {
                None
            };
            match format {
                "json" => println!(
                    "{}",
                    render_data_command_json(
                        "intelligence trust-surface import",
                        true,
                        serde_json::json!({
                            "rows_read": records.len(),
                            "source_types": unique_source_types(&records),
                            "report_path": report_path.as_ref().map(|item| item.to_string_lossy().to_string()),
                            "records": records,
                        }),
                        Vec::new()
                    )?
                ),
                _ => println!("{}", trust_import_text(&records, report_path.as_deref())),
            }
            Ok(0)
        }
        Err(error) => emit_failure("intelligence trust-surface import", format, error),
    }
}

fn command_trust_surface_reconcile(submatches: &ArgMatches) -> Result<i32> {
    let format = required_arg(submatches, "format")?;
    let input_path = PathBuf::from(required_arg(submatches, "input")?);
    let root = PathBuf::from(required_arg(submatches, "path")?);
    let manifest_path = submatches.get_one::<String>("manifest").map(PathBuf::from);
    let site_url = submatches.get_one::<String>("site_url").map(String::as_str);

    let manifest = discover_truth_manifest(&root, manifest_path.as_deref())?;
    let site = match load_site(&root) {
        Ok(site) => site,
        Err(error) => return emit_failure("intelligence trust-surface reconcile", format, error),
    };
    match import_trust_surface_records(&input_path).map(|records| {
        reconcile_trust_surfaces(
            &records,
            &site,
            site_url,
            manifest.as_ref().map(|(_, item)| item),
        )
    }) {
        Ok(report) => {
            let report_path = write_report(&root, "trust-surface-reconcile-latest.json", &report)?;
            match format {
                "json" => println!(
                    "{}",
                    render_data_command_json(
                        "intelligence trust-surface reconcile",
                        true,
                        serde_json::json!({
                            "report_path": report_path.to_string_lossy(),
                            "reconciliation": report,
                        }),
                        Vec::new()
                    )?
                ),
                _ => println!("{}", trust_reconcile_text(&report, &report_path)),
            }
            Ok(0)
        }
        Err(error) => emit_failure("intelligence trust-surface reconcile", format, error),
    }
}

fn emit_failure(command: &str, format: &str, error: anyhow::Error) -> Result<i32> {
    match format {
        "json" => println!(
            "{}",
            render_failed_command_json(command, error.to_string(), Vec::new())?
        ),
        _ => eprintln!("{}", error),
    }
    Ok(1)
}

fn write_report<T: Serialize>(root: &Path, file_name: &str, payload: &T) -> Result<PathBuf> {
    let reports_dir = root.join(".seogeo-reports");
    fs::create_dir_all(&reports_dir)?;
    let path = reports_dir.join(file_name);
    fs::write(&path, serde_json::to_string_pretty(payload)?)?;
    Ok(path)
}

fn grounding_text(report: &GroundingSiteAnalysis, report_path: &Path) -> String {
    let mut lines = vec![
        "Grounding Map".to_string(),
        String::new(),
        format!("Pages analyzed: {}", report.pages_analyzed),
        format!("Routes with topics: {}", report.routes_with_topics),
        format!("Elapsed: {}ms", report.elapsed_us / 1000),
        format!("Report: {}", report_path.display()),
    ];
    if !report.intent_distribution.is_empty() {
        lines.push(String::new());
        lines.push("Intent distribution:".to_string());
        for (intent, count) in &report.intent_distribution {
            lines.push(format!("- {} {}", intent, count));
        }
    }
    lines.push(String::new());
    lines.push("Top routes:".to_string());
    for route in report.routes.iter().take(10) {
        lines.push(format!(
            "- /{} topic='{}' intents={} gaps={}",
            route.route,
            route.primary_topic,
            route
                .intents
                .iter()
                .map(|intent| format!("{:?}", intent).to_ascii_lowercase())
                .collect::<Vec<_>>()
                .join(","),
            route
                .coverage_gaps
                .iter()
                .map(gap_label)
                .collect::<Vec<_>>()
                .join(",")
        ));
    }
    lines.join("\n")
}

fn gap_label(gap: &GroundingCoverageGap) -> &'static str {
    match gap {
        GroundingCoverageGap::MissingDirectAnswer => "missing_direct_answer",
        GroundingCoverageGap::ThinAnswerCoverage => "thin_answer_coverage",
        GroundingCoverageGap::WeakComparisonStructure => "weak_comparison_structure",
        GroundingCoverageGap::MissingPricingSignals => "missing_pricing_signals",
        GroundingCoverageGap::MissingProceduralSignals => "missing_procedural_signals",
    }
}

fn truth_text(report: &TruthAssessment, manifest_path: Option<&str>, report_path: &Path) -> String {
    let mut lines = vec![
        "Truth Assessment".to_string(),
        String::new(),
        format!(
            "Structured source: {}",
            trust_source_label(&report.structured_truth_source)
        ),
        format!(
            "Structured prerequisite met: {}",
            report.structured_truth_prerequisite_met
        ),
        format!("Score: {}/{}", report.score, report.score_ceiling),
        format!("Pages analyzed: {}", report.pages_analyzed),
        format!("Pages with schema: {}", report.pages_with_schema),
        format!("Manifest present: {}", report.manifest_present),
        format!("Preferred term hits: {}", report.preferred_term_hits),
        format!("Forbidden term hits: {}", report.forbidden_term_hits),
        format!("Elapsed: {}ms", report.elapsed_us / 1000),
        format!("Report: {}", report_path.display()),
    ];
    if let Some(path) = manifest_path {
        lines.push(format!("Manifest: {}", path));
    }
    if !report.mismatches.is_empty() {
        lines.push(String::new());
        lines.push("Mismatches:".to_string());
        for mismatch in report.mismatches.iter().take(10) {
            lines.push(format!(
                "- {} [{}] expected='{}' observed='{}' route={}",
                mismatch.field,
                mismatch.source,
                mismatch.expected,
                mismatch.observed,
                mismatch.route
            ));
        }
    }
    lines.join("\n")
}

fn trust_source_label(source: &TruthStructuredSource) -> &'static str {
    match source {
        TruthStructuredSource::Manifest => "manifest",
        TruthStructuredSource::Schema => "schema",
        TruthStructuredSource::SchemaAndManifest => "schema_and_manifest",
        TruthStructuredSource::None => "none",
    }
}

fn trust_import_text(
    records: &[seogeo_core::TrustSurfaceRecord],
    report_path: Option<&Path>,
) -> String {
    let mut lines = vec![
        "Trust Surface Import".to_string(),
        String::new(),
        format!("Rows read: {}", records.len()),
        format!("Source types: {}", unique_source_types(records).join(",")),
    ];
    if let Some(path) = report_path {
        lines.push(format!("Report: {}", path.display()));
    }
    for record in records.iter().take(10) {
        lines.push(format!("- [{}] {}", record.source_type, record.url));
    }
    lines.join("\n")
}

fn trust_reconcile_text(report: &TrustSurfaceReconciliation, report_path: &Path) -> String {
    let mut lines = vec![
        "Trust Surface Reconciliation".to_string(),
        String::new(),
        format!("Rows read: {}", report.rows_read),
        format!(
            "Matched first-party routes: {}",
            report.matched_first_party_routes
        ),
        format!("Offsite mentions: {}", report.offsite_mentions),
        format!("Issues: {}", report.issues.len()),
        format!("Elapsed: {}ms", report.elapsed_us / 1000),
        format!("Report: {}", report_path.display()),
    ];
    if !report.source_summaries.is_empty() {
        lines.push(String::new());
        lines.push("Sources:".to_string());
        for source in &report.source_summaries {
            lines.push(format!("- {} {}", source.source_type, source.rows));
        }
    }
    if !report.issues.is_empty() {
        lines.push(String::new());
        lines.push("Issues:".to_string());
        for issue in report.issues.iter().take(10) {
            lines.push(format!(
                "- [{}] {} {}",
                issue.source_type, issue.url, issue.message
            ));
        }
    }
    lines.join("\n")
}

fn unique_source_types(records: &[seogeo_core::TrustSurfaceRecord]) -> Vec<String> {
    let mut sources = records
        .iter()
        .map(|item| item.source_type.clone())
        .collect::<Vec<_>>();
    sources.sort();
    sources.dedup();
    sources
}

use anyhow::{Result, bail};
use clap::ArgMatches;
use seogeo_core::{
    EvidenceSiteAssessment, GroundingCoverageGap, GroundingSiteAnalysis, SiteIntelligenceScore,
    TrustSurfaceReconciliation, TruthAssessment, TruthManifestValidation, TruthStructuredSource,
    assess_evidence_coverage, assess_truth_layer, discover_truth_manifest,
    import_trust_surface_records, load_site, map_grounding_queries, reconcile_trust_surfaces,
    score_intelligence, validate_truth_manifest,
};
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};

use crate::commands::common::{canonicalize_or_keep, required_arg};
use crate::output::{render_data_command_json, render_failed_command_json};

pub fn command_intelligence(submatches: &ArgMatches) -> Result<i32> {
    match submatches.subcommand() {
        Some(("grounding-map", map_matches)) => command_grounding_map(map_matches),
        Some(("evidence", evidence_matches)) => match evidence_matches.subcommand() {
            Some(("assess", assess_matches)) => command_evidence_assess(assess_matches),
            Some((other, _)) => bail!("unsupported evidence command: {}", other),
            None => bail!("missing evidence subcommand"),
        },
        Some(("truth", truth_matches)) => match truth_matches.subcommand() {
            Some(("validate", validate_matches)) => command_truth_validate(validate_matches),
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
        Some(("score", score_matches)) => command_intelligence_score(score_matches),
        Some((other, _)) => bail!("unsupported intelligence command: {}", other),
        None => bail!("missing intelligence subcommand"),
    }
}

fn command_evidence_assess(submatches: &ArgMatches) -> Result<i32> {
    let format = required_arg(submatches, "format")?;
    let root = PathBuf::from(required_arg(submatches, "path")?);
    match load_site(&root).map(|site| assess_evidence_coverage(&site)) {
        Ok(report) => {
            let report_path = write_report(&root, "evidence-latest.json", &report)?;
            match format {
                "json" => println!(
                    "{}",
                    render_data_command_json(
                        "intelligence evidence assess",
                        true,
                        serde_json::json!({
                            "report_path": report_path.to_string_lossy(),
                            "assessment": report,
                        }),
                        Vec::new()
                    )?
                ),
                _ => println!("{}", evidence_text(&report, &report_path)),
            }
            Ok(0)
        }
        Err(error) => emit_failure("intelligence evidence assess", format, error),
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

fn command_truth_validate(submatches: &ArgMatches) -> Result<i32> {
    let format = required_arg(submatches, "format")?;
    let root = PathBuf::from(required_arg(submatches, "path")?);
    let manifest_path = submatches.get_one::<String>("manifest").map(PathBuf::from);
    match discover_truth_manifest(&root, manifest_path.as_deref())? {
        Some((path, manifest)) => {
            let validation = validate_truth_manifest(&manifest);
            match format {
                "json" => println!(
                    "{}",
                    render_data_command_json(
                        "intelligence truth validate",
                        validation.valid,
                        serde_json::json!({
                            "manifest_path": canonicalize_or_keep(&path.to_string_lossy()).display().to_string(),
                            "validation": validation,
                        }),
                        Vec::new()
                    )?
                ),
                _ => println!(
                    "{}",
                    truth_manifest_text(
                        &validation,
                        &canonicalize_or_keep(&path.to_string_lossy()),
                    )
                ),
            }
            Ok(if validation.valid { 0 } else { 1 })
        }
        None => emit_failure(
            "intelligence truth validate",
            format,
            anyhow::anyhow!("no truth manifest found"),
        ),
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
            let manifest_validation = manifest
                .as_ref()
                .map(|(_, item)| validate_truth_manifest(item));
            match format {
                "json" => println!(
                    "{}",
                    render_data_command_json(
                        "intelligence truth assess",
                        true,
                        serde_json::json!({
                            "manifest_path": manifest_path,
                            "manifest_validation": manifest_validation,
                            "report_path": report_path.to_string_lossy(),
                            "assessment": report,
                        }),
                        Vec::new()
                    )?
                ),
                _ => println!(
                    "{}",
                    truth_text(
                        &report,
                        manifest_validation.as_ref(),
                        manifest_path.as_deref(),
                        &report_path,
                    )
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

fn command_intelligence_score(submatches: &ArgMatches) -> Result<i32> {
    let format = required_arg(submatches, "format")?;
    let root = PathBuf::from(required_arg(submatches, "path")?);
    let manifest_path = submatches.get_one::<String>("manifest").map(PathBuf::from);
    let trust_surface_path = submatches
        .get_one::<String>("trust-surfaces")
        .map(PathBuf::from);
    let site_url = submatches.get_one::<String>("site-url").map(String::as_str);

    let site = match load_site(&root) {
        Ok(site) => site,
        Err(error) => return emit_failure("intelligence score", format, error),
    };
    let manifest = discover_truth_manifest(&root, manifest_path.as_deref())?;
    let grounding = map_grounding_queries(&site);
    let evidence = assess_evidence_coverage(&site);
    let truth = assess_truth_layer(&site, manifest.as_ref().map(|(_, item)| item));
    let trust = match trust_surface_path.as_ref() {
        Some(path) => Some(reconcile_trust_surfaces(
            &import_trust_surface_records(path)?,
            &site,
            site_url,
            manifest.as_ref().map(|(_, item)| item),
        )),
        None => None,
    };
    let score = score_intelligence(&grounding, &truth, &evidence, trust.as_ref());
    let report_path = write_report(&root, "intelligence-score-latest.json", &score)?;

    match format {
        "json" => println!(
            "{}",
            render_data_command_json(
                "intelligence score",
                true,
                serde_json::json!({
                    "report_path": report_path.to_string_lossy(),
                    "score": score,
                }),
                Vec::new()
            )?
        ),
        _ => println!("{}", score_text(&score, &report_path)),
    }
    Ok(0)
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

fn evidence_text(report: &EvidenceSiteAssessment, report_path: &Path) -> String {
    let mut lines = vec![
        "Evidence Assessment".to_string(),
        String::new(),
        format!("Pages analyzed: {}", report.pages_analyzed),
        format!("Routes with claims: {}", report.routes_with_claims),
        format!("Claims detected: {}", report.claim_count),
        format!("Unsupported claims: {}", report.unsupported_claims),
        format!("Evidence density: {}", report.evidence_density_score),
        format!("Citation readiness: {}", report.citation_readiness_score),
        format!(
            "Average fidelity risk: {}",
            report.average_fidelity_risk_score
        ),
        format!("Elapsed: {}ms", report.elapsed_us / 1000),
        format!("Report: {}", report_path.display()),
    ];
    if !report.claim_kind_distribution.is_empty() {
        lines.push(String::new());
        lines.push("Claim kinds:".to_string());
        for (kind, count) in &report.claim_kind_distribution {
            lines.push(format!("- {} {}", kind, count));
        }
    }
    lines.push(String::new());
    lines.push("Top risky routes:".to_string());
    for route in report.routes.iter().take(10) {
        lines.push(format!(
            "- /{} claims={} unsupported={} fidelity_risk={} citation_readiness={}",
            route.route,
            route.claim_count,
            route.unsupported_claims,
            route.fidelity_risk_score,
            route.citation_readiness_score
        ));
    }
    lines.join("\n")
}

fn score_text(report: &SiteIntelligenceScore, report_path: &Path) -> String {
    let mut lines = vec![
        "Intelligence Score".to_string(),
        String::new(),
        format!("Overall score: {}", report.overall_score),
        format!("Citation readiness: {}", report.citation_readiness_score),
        format!("Truth consistency: {}", report.truth_consistency_score),
        format!("Answer pack: {}", report.answer_pack_score),
        format!(
            "External trust alignment: {}",
            report
                .external_trust_alignment_score
                .map(|value| value.to_string())
                .unwrap_or_else(|| "n/a".to_string())
        ),
        format!("Elapsed: {}ms", report.elapsed_us / 1000),
        format!("Report: {}", report_path.display()),
    ];
    if !report.blockers.is_empty() {
        lines.push(String::new());
        lines.push("Top blockers:".to_string());
        for blocker in report.blockers.iter().take(10) {
            lines.push(format!(
                "- [{}] {} route={}",
                blocker.category,
                blocker.message,
                blocker.route.as_deref().unwrap_or("(sitewide)")
            ));
        }
    }
    lines.push(String::new());
    lines.push("Lowest scoring routes:".to_string());
    for route in report.route_scores.iter().take(10) {
        lines.push(format!(
            "- /{} overall={} citation={} truth={} answer_pack={} trust={}",
            route.route,
            route.overall_score,
            route.citation_readiness_score,
            route.truth_consistency_score,
            route.answer_pack_score,
            route
                .external_trust_alignment_score
                .map(|value| value.to_string())
                .unwrap_or_else(|| "n/a".to_string())
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

fn truth_text(
    report: &TruthAssessment,
    manifest_validation: Option<&TruthManifestValidation>,
    manifest_path: Option<&str>,
    report_path: &Path,
) -> String {
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
    if let Some(validation) = manifest_validation {
        lines.push(format!("Manifest valid: {}", validation.valid));
        if !validation.errors.is_empty() {
            lines.push(format!(
                "Manifest errors: {}",
                validation.errors.join(" | ")
            ));
        }
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

fn truth_manifest_text(report: &TruthManifestValidation, manifest_path: &Path) -> String {
    let mut lines = vec![
        "Truth Manifest Validation".to_string(),
        String::new(),
        format!("Manifest: {}", manifest_path.display()),
        format!("Valid: {}", report.valid),
        format!("Version: {}", report.version),
        format!("Organization present: {}", report.organization_present),
        format!("Products: {}", report.product_count),
        format!("Preferred terms: {}", report.preferred_term_count),
        format!("Forbidden terms: {}", report.forbidden_term_count),
        format!("Elapsed: {}ms", report.elapsed_us / 1000),
    ];
    if !report.warnings.is_empty() {
        lines.push(String::new());
        lines.push("Warnings:".to_string());
        for warning in &report.warnings {
            lines.push(format!("- {}", warning));
        }
    }
    if !report.errors.is_empty() {
        lines.push(String::new());
        lines.push("Errors:".to_string());
        for error in &report.errors {
            lines.push(format!("- {}", error));
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

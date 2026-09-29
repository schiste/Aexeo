use aexeo_contracts::AuditStatus;
use aexeo_core::intelligence::presence::{
    SOURCE_ORDER, SourceResult, SourceStatus, check_all_sources, entity_from_manifest, source_label,
};
use aexeo_core::{
    AnswerFanoutReport, EvidenceSiteAssessment, GroundingCoverageGap, GroundingSiteAnalysis,
    MachineSurfaceGraph, MachineSurfaceOptions, PageIdentity, SiteIntelligenceScore,
    TrustSurfaceReconciliation, TruthAssessment, TruthManifestGeneration, TruthManifestValidation,
    TruthStructuredSource, assess_answer_fanout, assess_evidence_coverage, assess_truth_layer,
    compute_page_identity, discover_machine_surface_graph, discover_truth_manifest,
    generate_truth_manifest_with_options, import_trust_surface_records, load_site,
    load_site_from_audit_artifact, looks_like_generic_entity_name, map_grounding_queries,
    reconcile_trust_surfaces, score_intelligence, validate_truth_manifest,
};
use anyhow::{Context, Result, bail};
use clap::ArgMatches;
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};

use super::exit_code::{EXIT_FINDINGS, EXIT_SUCCESS, EXIT_UNSUPPORTED};
use crate::commands::common::{canonicalize_or_keep, required_arg};
use crate::output::{render_data_command_json, render_failed_command_json};

#[derive(Debug, Clone, Serialize)]
struct IntelligenceInputMetadata {
    source: String,
    path: String,
    report_root: String,
    site_url: Option<String>,
    status: Option<String>,
    partial: bool,
    pages: usize,
    warning: Option<String>,
}

struct IntelligenceInput {
    site: aexeo_core::Site,
    report_root: PathBuf,
    metadata: IntelligenceInputMetadata,
}

pub fn command_intelligence(submatches: &ArgMatches) -> Result<i32> {
    match submatches.subcommand() {
        Some(("grounding-map", map_matches)) => command_grounding_map(map_matches),
        Some(("identity", identity_matches)) => command_identity(identity_matches),
        Some(("evidence", evidence_matches)) => match evidence_matches.subcommand() {
            Some(("assess", assess_matches)) => command_evidence_assess(assess_matches),
            Some((other, _)) => bail!("unsupported evidence command: {}", other),
            None => bail!("missing evidence subcommand"),
        },
        Some(("fanout", fanout_matches)) => match fanout_matches.subcommand() {
            Some(("assess", assess_matches)) => command_fanout_assess(assess_matches),
            Some((other, _)) => bail!("unsupported fanout command: {}", other),
            None => bail!("missing fanout subcommand"),
        },
        Some(("facts", truth_matches)) | Some(("truth", truth_matches)) => {
            match truth_matches.subcommand() {
                Some(("validate", validate_matches)) => command_truth_validate(validate_matches),
                Some(("generate", generate_matches)) => command_truth_generate(generate_matches),
                Some(("assess", assess_matches)) => command_truth_assess(assess_matches),
                Some((other, _)) => bail!("unsupported facts command: {}", other),
                None => bail!("missing facts subcommand"),
            }
        }
        Some(("trust-surface", trust_matches)) => match trust_matches.subcommand() {
            Some(("import", import_matches)) => command_trust_surface_import(import_matches),
            Some(("reconcile", reconcile_matches)) => {
                command_trust_surface_reconcile(reconcile_matches)
            }
            Some((other, _)) => bail!("unsupported trust-surface command: {}", other),
            None => bail!("missing trust-surface subcommand"),
        },
        Some(("surfaces", surfaces_matches)) => match surfaces_matches.subcommand() {
            Some(("discover", discover_matches)) => command_surfaces_discover(discover_matches),
            Some((other, _)) => bail!("unsupported surfaces command: {}", other),
            None => bail!("missing surfaces subcommand"),
        },
        Some(("score", score_matches)) => command_intelligence_score(score_matches),
        Some(("presence", presence_matches)) => command_intelligence_presence(presence_matches),
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
            Ok(EXIT_SUCCESS)
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
            Ok(EXIT_SUCCESS)
        }
        Err(error) => emit_failure("intelligence grounding-map", format, error),
    }
}

fn command_fanout_assess(submatches: &ArgMatches) -> Result<i32> {
    let format = required_arg(submatches, "format")?;
    let input = match load_intelligence_input(submatches) {
        Ok(input) => input,
        Err(error) => return emit_failure("intelligence fanout assess", format, error),
    };
    let report = assess_answer_fanout(&input.site);
    let report_path = write_report(&input.report_root, "answer-fanout-latest.json", &report)?;
    match format {
        "json" => println!(
            "{}",
            render_data_command_json(
                "intelligence fanout assess",
                true,
                serde_json::json!({
                    "input": input.metadata,
                    "report_path": report_path.to_string_lossy(),
                    "assessment": report,
                }),
                Vec::new()
            )?
        ),
        _ => println!(
            "{}",
            with_input_text(fanout_text(&report, &report_path), &input.metadata)
        ),
    }
    Ok(EXIT_SUCCESS)
}

fn command_surfaces_discover(submatches: &ArgMatches) -> Result<i32> {
    let format = required_arg(submatches, "format")?;
    let input = match load_intelligence_input(submatches) {
        Ok(input) => input,
        Err(error) => return emit_failure("intelligence surfaces discover", format, error),
    };
    let explicit_site_url = submatches.get_one::<String>("site-url").map(String::as_str);
    let snapshot_site_url = input.metadata.site_url.as_deref();
    let site_url = explicit_site_url.or(snapshot_site_url);
    let report = discover_machine_surface_graph(&input.site, MachineSurfaceOptions::new(site_url));
    let report_path = write_report(&input.report_root, "machine-surfaces-latest.json", &report)?;
    match format {
        "json" => println!(
            "{}",
            render_data_command_json(
                "intelligence surfaces discover",
                true,
                serde_json::json!({
                    "input": input.metadata,
                    "report_path": report_path.to_string_lossy(),
                    "graph": report,
                }),
                Vec::new()
            )?
        ),
        _ => println!(
            "{}",
            with_input_text(surfaces_text(&report, &report_path), &input.metadata)
        ),
    }
    Ok(EXIT_SUCCESS)
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
                        "intelligence facts validate",
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
            "intelligence facts validate",
            format,
            anyhow::anyhow!("no facts manifest found"),
        ),
    }
}

fn command_truth_generate(submatches: &ArgMatches) -> Result<i32> {
    let format = required_arg(submatches, "format")?;
    let root = PathBuf::from(required_arg(submatches, "path")?);
    let curate = submatches.get_flag("curate");
    let deploy_location = submatches
        .get_one::<String>("deploy-location")
        .map(String::as_str);
    let explicit_write = submatches.get_one::<String>("write").map(PathBuf::from);

    match load_site(&root).map(|site| generate_truth_manifest_with_options(&site, curate)) {
        Ok(report) => {
            let report_path = write_report(&root, "facts-manifest-generated.json", &report)?;
            let write_path = if let Some(path) = explicit_write {
                Some(path)
            } else {
                deploy_location.map(|location| match location {
                    "well-known" => root.join(".well-known/facts.json"),
                    _ => root.join("facts.json"),
                })
            };
            if let Some(path) = write_path.as_ref() {
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::write(path, serde_json::to_string_pretty(&report.manifest)?)?;
            }
            match format {
                "json" => println!(
                    "{}",
                    render_data_command_json(
                        "intelligence facts generate",
                        report.validation.valid,
                        serde_json::json!({
                            "report_path": report_path.to_string_lossy(),
                            "write_path": write_path.as_ref().map(|path| canonicalize_or_keep(&path.to_string_lossy()).display().to_string()),
                            "generation": report,
                        }),
                        Vec::new()
                    )?
                ),
                _ => println!(
                    "{}",
                    truth_generate_text(&report, write_path.as_deref(), &report_path,)
                ),
            }
            Ok(if report.validation.valid { 0 } else { 1 })
        }
        Err(error) => emit_failure("intelligence facts generate", format, error),
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
            let report_path = write_report(&root, "facts-layer-latest.json", &report)?;
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
                        "intelligence facts assess",
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
            Ok(EXIT_SUCCESS)
        }
        Err(error) => emit_failure("intelligence facts assess", format, error),
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
            Ok(EXIT_SUCCESS)
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
            Ok(EXIT_SUCCESS)
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
    Ok(EXIT_SUCCESS)
}

fn status_label(status: AuditStatus) -> &'static str {
    match status {
        AuditStatus::Complete => "complete",
        AuditStatus::Partial => "partial",
        AuditStatus::Failed => "failed",
    }
}

fn report_root_for_artifact(path: &Path) -> PathBuf {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    if parent.file_name().and_then(|item| item.to_str()) == Some(".aexeo-reports") {
        let root = parent
            .parent()
            .filter(|candidate| !candidate.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        return root.to_path_buf();
    }
    if parent.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        parent.to_path_buf()
    }
}

fn load_intelligence_input(submatches: &ArgMatches) -> Result<IntelligenceInput> {
    if let Some(path) = submatches.get_one::<String>("from-crawl-artifact") {
        let artifact_path = PathBuf::from(path);
        let (artifact, site) =
            load_site_from_audit_artifact(&artifact_path).with_context(|| {
                format!(
                    "failed to load crawl artifact input from {}",
                    artifact_path.display()
                )
            })?;
        let snapshot = artifact
            .site
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("audit artifact does not include a site snapshot"))?;
        let report_root = report_root_for_artifact(&artifact_path);
        let partial = artifact.is_partial();
        let warning = if partial {
            Some(
                "input crawl artifact is partial; downstream intelligence may miss routes"
                    .to_string(),
            )
        } else {
            None
        };
        return Ok(IntelligenceInput {
            site,
            metadata: IntelligenceInputMetadata {
                source: "crawl_artifact".to_string(),
                path: artifact_path.to_string_lossy().to_string(),
                report_root: report_root.to_string_lossy().to_string(),
                site_url: snapshot.site_url.clone(),
                status: Some(status_label(artifact.status).to_string()),
                partial,
                pages: snapshot.pages.len(),
                warning,
            },
            report_root,
        });
    }

    let root = PathBuf::from(required_arg(submatches, "path")?);
    if root.is_file() {
        bail!(
            "{} is a file; pass --from-crawl-artifact for crawl audit artifacts or provide a site directory",
            root.display()
        );
    }
    let site = load_site(&root)?;
    Ok(IntelligenceInput {
        metadata: IntelligenceInputMetadata {
            source: "directory".to_string(),
            path: root.to_string_lossy().to_string(),
            report_root: root.to_string_lossy().to_string(),
            site_url: None,
            status: None,
            partial: false,
            pages: site.pages.len(),
            warning: None,
        },
        report_root: root,
        site,
    })
}

fn with_input_text(mut text: String, metadata: &IntelligenceInputMetadata) -> String {
    text.push_str("\n\nInput\n");
    text.push_str(&format!("- Source: {}\n", metadata.source));
    text.push_str(&format!("- Path: {}\n", metadata.path));
    text.push_str(&format!("- Pages: {}\n", metadata.pages));
    if let Some(status) = metadata.status.as_deref() {
        text.push_str(&format!("- Crawl status: {status}\n"));
    }
    if let Some(warning) = metadata.warning.as_deref() {
        text.push_str(&format!("- Warning: {warning}\n"));
    }
    text
}

fn emit_failure(command: &str, format: &str, error: anyhow::Error) -> Result<i32> {
    match format {
        "json" => println!(
            "{}",
            render_failed_command_json(command, error.to_string(), Vec::new())?
        ),
        _ => eprintln!("{}", error),
    }
    Ok(EXIT_FINDINGS)
}

fn write_report<T: Serialize>(root: &Path, file_name: &str, payload: &T) -> Result<PathBuf> {
    let reports_dir = root.join(".aexeo-reports");
    fs::create_dir_all(&reports_dir)?;
    let path = reports_dir.join(file_name);
    fs::write(&path, serde_json::to_string_pretty(payload)?)?;
    Ok(path)
}

fn surfaces_text(report: &MachineSurfaceGraph, report_path: &Path) -> String {
    let coverage = &report.coverage;
    let mut lines = vec![
        "Machine Surface Graph".to_string(),
        String::new(),
        format!("Site root: {}", report.site_root),
        format!("Site URL: {}", report.site_url.as_deref().unwrap_or("-")),
        format!("Surfaces discovered: {}", report.surfaces.len()),
        format!("Routes analyzed: {}", coverage.total_routes),
        format!(
            "Markdown mirror coverage: {}/{}",
            coverage.routes_with_markdown_mirror, coverage.total_routes
        ),
        format!(
            "Schema coverage: {}/{}",
            coverage.routes_with_schema, coverage.total_routes
        ),
        format!("Static machine links: {}", coverage.static_machine_links),
        format!("llms.txt links: {}", coverage.llms_index_links),
        format!("Convention mirror hits: {}", coverage.convention_probe_hits),
        format!("Facts present: {}", coverage.facts_present),
        format!("llms.txt present: {}", coverage.llms_present),
        format!("sitemap.xml present: {}", coverage.sitemap_present),
        format!("robots.txt present: {}", coverage.robots_present),
        format!("Report: {}", report_path.display()),
    ];
    if !report.recommendations.is_empty() {
        lines.push(String::new());
        lines.push("Recommendations:".to_string());
        for recommendation in &report.recommendations {
            lines.push(format!("- {}", recommendation));
        }
    }
    let routes_with_issues = report
        .routes
        .iter()
        .filter(|route| !route.issues.is_empty())
        .take(10)
        .collect::<Vec<_>>();
    if !routes_with_issues.is_empty() {
        lines.push(String::new());
        lines.push("Routes needing attention:".to_string());
        for route in routes_with_issues {
            lines.push(format!(
                "- /{} schema={} markdown={} static_links={} issues={}",
                route.route,
                route.schema_types.len(),
                route.markdown_mirrors.len(),
                route.static_machine_links.len(),
                route.issues.join("; ")
            ));
        }
    }
    lines.join("\n")
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

fn fanout_text(report: &AnswerFanoutReport, report_path: &Path) -> String {
    let mut lines = vec![
        "Answer Fan-out Assessment".to_string(),
        String::new(),
        format!("Routes analyzed: {}", report.routes_analyzed),
        format!("Queries generated: {}", report.query_count),
        format!("Covered queries: {}", report.covered_queries),
        format!("Coverage score: {}", report.coverage_score),
        format!("Elapsed: {}ms", report.elapsed_us / 1000),
        format!("Report: {}", report_path.display()),
    ];
    let weak_queries = report
        .queries
        .iter()
        .filter(|query| query.coverage_score < 60 || !query.gaps.is_empty())
        .take(10)
        .collect::<Vec<_>>();
    if !weak_queries.is_empty() {
        lines.push(String::new());
        lines.push("Weak fan-out queries:".to_string());
        for query in weak_queries {
            let best = query
                .matched_routes
                .first()
                .map(|item| format!("/{} ({})", item.route, item.score))
                .unwrap_or_else(|| "none".to_string());
            lines.push(format!(
                "- [{}] '{}' score={} best={} expected={}",
                query.family, query.query, query.coverage_score, best, query.expected_surface
            ));
            if !query.gaps.is_empty() {
                lines.push(format!("  gaps: {}", query.gaps.join("; ")));
            }
        }
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
        format!("Evidence quality: {}", report.evidence_quality_score),
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
        format!("Facts consistency: {}", report.truth_consistency_score),
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
            "- /{} overall={} citation={} facts={} answer_pack={} trust={}",
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
        "Facts Assessment".to_string(),
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
        "Facts Manifest Validation".to_string(),
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

fn truth_generate_text(
    report: &TruthManifestGeneration,
    write_path: Option<&Path>,
    report_path: &Path,
) -> String {
    let mut lines = vec![
        "Facts Manifest Generation".to_string(),
        String::new(),
        format!("Curated: {}", report.curated),
        format!("Generated manifest valid: {}", report.validation.valid),
        format!(
            "Organization: {}",
            report
                .manifest
                .organization
                .as_ref()
                .map(|entity| entity.name.as_str())
                .unwrap_or("(missing)")
        ),
        format!("Products: {}", report.manifest.products.len()),
        format!("Elapsed: {}ms", report.elapsed_us / 1000),
        format!("Report: {}", report_path.display()),
    ];
    if let Some(path) = write_path {
        lines.push(format!("Manifest written to: {}", path.display()));
    } else {
        lines.push(format!(
            "Suggested deploy paths: {}",
            report.suggested_deploy_paths.join(", ")
        ));
    }
    if !report.warnings.is_empty() {
        lines.push(String::new());
        lines.push("Warnings:".to_string());
        for warning in &report.warnings {
            lines.push(format!("- {}", warning));
        }
    }
    if !report.provenance.is_empty() {
        lines.push(String::new());
        lines.push("Provenance:".to_string());
        for (field, sources) in report.provenance.iter().take(10) {
            lines.push(format!("- {} <- {}", field, sources.join(", ")));
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
    records: &[aexeo_core::TrustSurfaceRecord],
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

fn unique_source_types(records: &[aexeo_core::TrustSurfaceRecord]) -> Vec<String> {
    let mut sources = records
        .iter()
        .map(|item| item.source_type.clone())
        .collect::<Vec<_>>();
    sources.sort();
    sources.dedup();
    sources
}

fn command_identity(submatches: &ArgMatches) -> Result<i32> {
    let raw_route = required_arg(submatches, "route")?;
    // Convention: an empty string and "/" both mean the home page.
    // Internally the site keys use the empty string for home; strip a
    // leading "/" so users can pass either form.
    let normalized_route = if raw_route == "/" {
        "".to_string()
    } else {
        raw_route.trim_start_matches('/').to_string()
    };
    let path = canonicalize_or_keep(
        submatches
            .get_one::<String>("path")
            .map(String::as_str)
            .unwrap_or("."),
    );
    let format = submatches
        .get_one::<String>("format")
        .map(String::as_str)
        .unwrap_or("text");

    let site = load_site(&path)?;
    let Some(identity) = compute_page_identity(&site, &normalized_route) else {
        match format {
            "json" => {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "route": raw_route,
                        "found": false,
                    }))?
                );
            }
            _ => {
                println!(
                    "identity: route '{}' not found in {}",
                    raw_route,
                    path.display()
                );
            }
        }
        return Ok(EXIT_UNSUPPORTED);
    };

    match format {
        "json" => {
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "route": &identity.route,
                    "canonical_title": &identity.canonical_title,
                    "canonical_source": &identity.canonical_source,
                    "agrees": identity.agrees,
                    "sources": &identity.sources,
                    "drift": &identity.drift,
                }))?
            );
        }
        _ => print_identity_text(&identity),
    }

    Ok(if identity.agrees { 0 } else { 1 })
}

fn print_identity_text(identity: &PageIdentity) {
    println!(
        "identity: /{}",
        if identity.route.is_empty() {
            ""
        } else {
            &identity.route
        }
    );
    match &identity.canonical_title {
        Some(title) => println!(
            "  canonical: \"{}\"  (source: {})",
            title, identity.canonical_source
        ),
        None => println!("  canonical: <none — no title, H1, or schema name found>"),
    }
    println!();
    println!("  sources:");
    println!(
        "    title:      {}",
        identity
            .sources
            .title
            .as_deref()
            .map(|t| format!("\"{}\"", t))
            .unwrap_or_else(|| "<missing>".to_string())
    );
    println!(
        "    h1:         {}",
        identity
            .sources
            .first_h1
            .as_deref()
            .map(|t| format!("\"{}\"", t))
            .unwrap_or_else(|| "<missing>".to_string())
    );
    println!(
        "    og:title:   {}",
        identity
            .sources
            .og_title
            .as_deref()
            .map(|t| format!("\"{}\"", t))
            .unwrap_or_else(|| "<missing>".to_string())
    );
    if identity.sources.schema_names.is_empty() {
        println!("    schema name:  <missing>");
    } else {
        println!(
            "    schema name:  {}",
            identity
                .sources
                .schema_names
                .iter()
                .map(|n| format!("\"{}\"", n))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    if !identity.sources.schema_headlines.is_empty() {
        println!(
            "    headline:   {}",
            identity
                .sources
                .schema_headlines
                .iter()
                .map(|n| format!("\"{}\"", n))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    println!();
    if identity.agrees {
        println!("  status: ✓ all signals agree with the canonical");
    } else {
        println!("  status: ✗ {} drift dimension(s):", identity.drift.len());
        for entry in &identity.drift {
            println!(
                "    - {}: \"{}\" disagrees with canonical \"{}\"",
                entry.dimension, entry.observed, entry.canonical
            );
        }
        println!();
        println!(
            "  fix: build your templates from the canonical so the four signals can't drift independently."
        );
    }
}

fn command_intelligence_presence(submatches: &ArgMatches) -> Result<i32> {
    let format = required_arg(submatches, "format")?;
    let root = PathBuf::from(required_arg(submatches, "path")?);
    let manifest_path = submatches.get_one::<String>("manifest").map(PathBuf::from);

    let (manifest_path_resolved, manifest) = match discover_truth_manifest(
        &root,
        manifest_path.as_deref(),
    ) {
        Ok(Some(found)) => found,
        Ok(None) => {
            return emit_failure(
                "intelligence presence",
                format,
                anyhow::anyhow!(
                    "no truth manifest found at {} — run `aexeo-cli intelligence facts generate --write facts.json` first",
                    root.display()
                ),
            );
        }
        Err(error) => return emit_failure("intelligence presence", format, error),
    };

    let entity = match entity_from_manifest(&manifest) {
        Some(entity) => entity,
        None => {
            return emit_failure(
                "intelligence presence",
                format,
                anyhow::anyhow!(
                    "manifest at {} has no organization — add `organization.name` (and ideally `organization.website`) and re-run",
                    manifest_path_resolved.display()
                ),
            );
        }
    };

    // Warn-but-proceed when the entity name looks like a generic
    // listing label or 404 title — those produce misleading
    // Wikipedia/Wikidata hits (the generic concept of "blog" rather
    // than the actual brand). Symptom Aeptus reported on /blog and
    // "Page not found" with v0.0.8's generated facts.json.
    let suspicion = if looks_like_generic_entity_name(&entity.name) {
        Some(format!(
            "manifest's organization.name '{}' looks like a generic listing or error-page label; presence results below are likely matching the generic concept rather than the brand. Curate facts.json before relying on these results.",
            entity.name
        ))
    } else {
        None
    };
    if let Some(message) = &suspicion {
        eprintln!("warning: {message}");
    }

    let results = check_all_sources(&entity);
    let report_path = write_report(
        &root,
        "presence-latest.json",
        &serde_json::json!({
            "entity_name": entity.name,
            "website": entity.website,
            "results": results,
            "suspicion_warning": suspicion,
        }),
    )?;

    match format {
        "json" => println!(
            "{}",
            render_data_command_json(
                "intelligence presence",
                true,
                serde_json::json!({
                    "report_path": report_path.to_string_lossy(),
                    "entity_name": entity.name,
                    "website": entity.website,
                    "results": results,
                    "suspicion_warning": suspicion,
                }),
                Vec::new(),
            )?
        ),
        _ => println!(
            "{}",
            presence_text(
                &entity.name,
                entity.website.as_deref(),
                &results,
                &report_path,
                suspicion.as_deref(),
            )
        ),
    }
    Ok(EXIT_SUCCESS)
}

fn presence_text(
    entity_name: &str,
    website: Option<&str>,
    results: &[SourceResult],
    report_path: &Path,
    suspicion_warning: Option<&str>,
) -> String {
    let mut lines = vec![
        "Entity Presence (layer 4 — surfaced, not scored)".to_string(),
        String::new(),
        format!("Entity: {entity_name}"),
        format!("Website: {}", website.unwrap_or("-")),
        String::new(),
    ];
    if let Some(warning) = suspicion_warning {
        lines.push("⚠ Suspicion warning:".to_string());
        lines.push(format!("  {warning}"));
        lines.push(String::new());
    }

    let ordered = order_results(results);
    for result in &ordered {
        lines.push(format_source_line(result));
        if let Some(label) = &result.label {
            lines.push(format!("    {label}"));
        }
        if let Some(extra) = &result.extra {
            lines.push(format!("    {extra}"));
        }
        if let Some(url) = &result.url {
            lines.push(format!("    -> {url}"));
        }
        if let Some(error) = &result.error
            && matches!(
                result.status,
                SourceStatus::Unreachable | SourceStatus::Skipped
            )
        {
            lines.push(format!("    ({error})"));
        }
    }

    lines.push(String::new());
    lines.push(format!(
        "Report: {}",
        canonicalize_or_keep(&report_path.to_string_lossy()).display()
    ));
    lines.join("\n")
}

fn order_results(results: &[SourceResult]) -> Vec<SourceResult> {
    let mut ordered: Vec<SourceResult> = SOURCE_ORDER
        .iter()
        .filter_map(|name| results.iter().find(|r| r.source == *name).cloned())
        .collect();
    for result in results {
        if !SOURCE_ORDER.contains(&result.source.as_str()) {
            ordered.push(result.clone());
        }
    }
    ordered
}

fn format_source_line(result: &SourceResult) -> String {
    let symbol = match result.status {
        SourceStatus::Found => "[+]",
        SourceStatus::NotFound => "[ ]",
        SourceStatus::Unreachable => "[!]",
        SourceStatus::Skipped => "[-]",
    };
    let status = match result.status {
        SourceStatus::Found => "found",
        SourceStatus::NotFound => "no record",
        SourceStatus::Unreachable => "couldn't reach",
        SourceStatus::Skipped => "skipped",
    };
    format!(
        "  {symbol} {label:<22} {status}",
        label = source_label(&result.source)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use aexeo_core::{GroundingRouteAnalysis, TrustSurfaceRecord};

    fn path(name: &str) -> &Path {
        Path::new(name)
    }

    // --- status_label --------------------------------------------------

    #[test]
    fn status_label_covers_every_audit_status() {
        assert_eq!(status_label(AuditStatus::Complete), "complete");
        assert_eq!(status_label(AuditStatus::Partial), "partial");
        assert_eq!(status_label(AuditStatus::Failed), "failed");
    }

    // --- report_root_for_artifact ---------------------------------------

    /// Artifacts written under `.aexeo-reports/` should resolve back to the
    /// site root, one level above the reports directory, so downstream
    /// intelligence writes its own output next to the input rather than
    /// nested inside it.
    #[test]
    fn report_root_strips_the_aexeo_reports_directory() {
        assert_eq!(
            report_root_for_artifact(path("/site/.aexeo-reports/crawl.json")),
            PathBuf::from("/site")
        );
    }

    /// A report written anywhere else keeps its containing directory as the
    /// root; only the literal `.aexeo-reports` name triggers the strip.
    #[test]
    fn report_root_keeps_the_parent_for_any_other_directory() {
        assert_eq!(
            report_root_for_artifact(path("/site/out/crawl.json")),
            PathBuf::from("/site/out")
        );
        // A near-miss name is not the sentinel.
        assert_eq!(
            report_root_for_artifact(path("/site/.aexeo-reports-old/crawl.json")),
            PathBuf::from("/site/.aexeo-reports-old")
        );
    }

    /// A bare filename has no parent, and must degrade to the current
    /// directory rather than an empty path that would write into `""`.
    #[test]
    fn report_root_handles_a_bare_filename_and_a_reports_dir_at_the_root() {
        assert_eq!(
            report_root_for_artifact(path("crawl.json")),
            PathBuf::from(".")
        );
        // `.aexeo-reports` directly under root: the grandparent is empty, so
        // it must fall back to "." rather than producing an empty path.
        assert_eq!(
            report_root_for_artifact(path(".aexeo-reports/crawl.json")),
            PathBuf::from(".")
        );
    }

    // --- gap_label ------------------------------------------------------

    #[test]
    fn gap_label_names_every_coverage_gap() {
        use GroundingCoverageGap::*;
        let pairs = [
            (MissingDirectAnswer, "missing_direct_answer"),
            (ThinAnswerCoverage, "thin_answer_coverage"),
            (WeakComparisonStructure, "weak_comparison_structure"),
            (MissingPricingSignals, "missing_pricing_signals"),
            (MissingProceduralSignals, "missing_procedural_signals"),
        ];
        let mut seen = std::collections::BTreeSet::new();
        for (gap, expected) in &pairs {
            assert_eq!(gap_label(gap), *expected);
            seen.insert(gap_label(gap));
        }
        assert_eq!(seen.len(), pairs.len(), "two gaps share a label");
    }

    // --- grounding_text -------------------------------------------------

    fn grounding_route(
        route: &str,
        topic: &str,
        gaps: Vec<GroundingCoverageGap>,
    ) -> GroundingRouteAnalysis {
        serde_json::from_value(serde_json::json!({
            "route": route,
            "page_kind": "detail",
            "primary_topic": topic,
            "secondary_topics": [],
            "primary_intent": "definition",
            "secondary_intents": [],
            "intents": ["definition", "procedural"],
            "intent_matches": [],
            "schema_types": [],
            "signals": [],
            "coverage_gaps": gaps,
            "answer_blocks": 1,
            "heading_count": 2,
        }))
        .expect("grounding route fixture")
    }

    /// The grounding map is the operator's view of "which pages could a model
    /// actually answer from", so a route's topic, intents, and gaps must all
    /// appear, and gaps must be rendered as their stable machine labels.
    #[test]
    fn grounding_text_reports_route_topics_intents_and_gaps() {
        let report = GroundingSiteAnalysis {
            pages_analyzed: 3,
            routes_with_topics: 1,
            intent_distribution: [("definition".to_string(), 2)].into_iter().collect(),
            primary_intent_distribution: [("definition".to_string(), 1)].into_iter().collect(),
            topic_clusters: [("pricing".to_string(), vec!["pricing".to_string()])]
                .into_iter()
                .collect(),
            routes: vec![grounding_route(
                "pricing",
                "plans",
                vec![
                    GroundingCoverageGap::MissingPricingSignals,
                    GroundingCoverageGap::ThinAnswerCoverage,
                ],
            )],
            elapsed_us: 3_000_000,
        };

        let text = grounding_text(&report, path("/site/.aexeo-reports/grounding.json"));

        assert!(text.contains("Pages analyzed: 3"), "{text}");
        assert!(text.contains("Routes with topics: 1"), "{text}");
        assert!(text.contains("Elapsed: 3000ms"), "{text}");
        assert!(text.contains("grounding.json"), "{text}");
        assert!(text.contains("/pricing"), "{text}");
        assert!(text.contains("topic='plans'"), "{text}");
        // Intents render lowercase, gaps as stable snake_case labels.
        assert!(text.contains("intents=definition,procedural"), "{text}");
        assert!(
            text.contains("gaps=missing_pricing_signals,thin_answer_coverage"),
            "{text}"
        );
        assert!(text.contains("Intent distribution:"), "{text}");
    }

    /// A route with no gaps must render an empty gap list rather than omit the
    /// field, so the column alignment an operator scans stays stable.
    #[test]
    fn grounding_text_shows_an_empty_gap_list_for_a_well_covered_route() {
        let report = GroundingSiteAnalysis {
            pages_analyzed: 1,
            routes_with_topics: 1,
            intent_distribution: Default::default(),
            primary_intent_distribution: Default::default(),
            topic_clusters: Default::default(),
            routes: vec![grounding_route("well-covered", "topic", vec![])],
            elapsed_us: 0,
        };
        let text = grounding_text(&report, path("r.json"));
        assert!(text.contains("gaps="), "{text}");
        assert!(
            !text.contains("Intent distribution:"),
            "an empty intent distribution must not print an empty heading: {text}"
        );
    }

    // --- surfaces_text --------------------------------------------------

    fn surface_graph() -> MachineSurfaceGraph {
        serde_json::from_value(serde_json::json!({
            "site_root": "/site",
            "site_url": "https://example.com",
            "surfaces": [],
            "coverage": {
                "total_routes": 4,
                "routes_with_schema": 2,
                "routes_with_markdown_mirror": 3,
                "routes_with_static_machine_link": 1,
                "routes_in_sitemap": 4,
                "llms_present": true,
                "llms_full_present": false,
                "facts_present": true,
                "robots_present": true,
                "sitemap_present": true,
                "static_machine_links": 1,
                "llms_index_links": 2,
                "convention_probe_hits": 1,
                "convention_probe_misses": 0,
            },
            "routes": [{
                "route": "pricing",
                "canonical": null,
                "schema_types": ["Product"],
                "markdown_mirrors": ["pricing.md.txt"],
                "static_machine_links": [],
                "sitemap_listed": true,
                "issues": ["no schema on markdown mirror"],
            }],
            "recommendations": ["publish llms-full.txt"],
        }))
        .expect("surface graph fixture")
    }

    /// The surface graph is the "is my site machine-readable" report, so every
    /// coverage ratio must be printed, and routes needing attention must carry
    /// their issues so the operator knows what to fix.
    #[test]
    fn surfaces_text_reports_coverage_ratios_and_flagged_routes() {
        let text = surfaces_text(&surface_graph(), path("/site/surfaces.json"));

        assert!(text.contains("Routes analyzed: 4"), "{text}");
        assert!(text.contains("Markdown mirror coverage: 3/4"), "{text}");
        assert!(text.contains("Schema coverage: 2/4"), "{text}");
        assert!(text.contains("llms.txt present: true"), "{text}");
        assert!(text.contains("Routes needing attention:"), "{text}");
        assert!(text.contains("/pricing"), "{text}");
        assert!(text.contains("no schema on markdown mirror"), "{text}");
        assert!(text.contains("publish llms-full.txt"), "{text}");
    }

    /// A missing `site_url` is common on a pre-deploy site and must render as
    /// a placeholder rather than `None` or crashing the formatter.
    #[test]
    fn surfaces_text_handles_an_absent_site_url() {
        let mut graph = surface_graph();
        graph.site_url = None;
        let text = surfaces_text(&graph, path("s.json"));
        assert!(
            text.contains("Site URL: -"),
            "expected a placeholder for a missing site URL: {text}"
        );
    }

    /// A clean graph with no issues and no recommendations must not print
    /// empty "needs attention" / "recommendations" headings.
    #[test]
    fn surfaces_text_omits_empty_recommendation_and_issue_sections() {
        let mut graph = surface_graph();
        graph.recommendations.clear();
        graph.routes.clear();
        let text = surfaces_text(&graph, path("s.json"));
        assert!(!text.contains("Recommendations:"), "{text}");
        assert!(!text.contains("Routes needing attention:"), "{text}");
    }

    // --- fanout_text ----------------------------------------------------

    fn fanout_query(
        family: &str,
        query: &str,
        score: u8,
        gaps: Vec<&str>,
    ) -> aexeo_core::AnswerFanoutQuery {
        serde_json::from_value(serde_json::json!({
            "query": query,
            "family": family,
            "topic": "pricing",
            "expected_surface": "product",
            "coverage_score": score,
            "matched_routes": [{
                "route": "pricing",
                "score": score,
                "reasons": [],
            }],
            "gaps": gaps,
        }))
        .expect("fanout query fixture")
    }

    /// A query is "weak" if it scores below 60 **or** carries gaps, and the
    /// report must surface both kinds — a low-scoring query with no gaps and a
    /// high-scoring query with gaps are both actionable.
    #[test]
    fn fanout_text_surfaces_low_scoring_and_gapped_queries() {
        let report = AnswerFanoutReport {
            routes_analyzed: 2,
            query_count: 2,
            covered_queries: 0,
            coverage_score: 0,
            elapsed_us: 1_000_000,
            queries: vec![
                fanout_query("pricing", "how much", 30, vec![]),
                fanout_query("definition", "what is", 90, vec!["no direct answer"]),
            ],
        };

        let text = fanout_text(&report, path("fanout.json"));

        assert!(text.contains("Routes analyzed: 2"), "{text}");
        assert!(text.contains("Weak fan-out queries:"), "{text}");
        // Low score (no gaps) is weak.
        assert!(text.contains("'how much'"), "{text}");
        // High score but with a gap is *also* weak — a gap is disqualifying
        // on its own.
        assert!(text.contains("'what is'"), "{text}");
        assert!(text.contains("gaps: no direct answer"), "{text}");
        assert!(text.contains("best=/pricing"), "{text}");
    }

    /// A fully-covered query must not be listed as weak.
    #[test]
    fn fanout_text_omits_a_strong_query() {
        let report = AnswerFanoutReport {
            routes_analyzed: 1,
            query_count: 1,
            covered_queries: 1,
            coverage_score: 100,
            elapsed_us: 0,
            queries: vec![fanout_query("definition", "strong", 95, vec![])],
        };
        let text = fanout_text(&report, path("f.json"));
        assert!(!text.contains("Weak fan-out queries:"), "{text}");
    }

    // --- score_text -----------------------------------------------------

    /// The external-trust score is optional (it needs network data). When
    /// absent it must read `n/a`, never a misleading `0`, which would look
    /// like a failing score.
    #[test]
    fn score_text_prints_n_a_for_a_missing_external_trust_score() {
        let report: SiteIntelligenceScore = serde_json::from_value(serde_json::json!({
            "overall_score": 70,
            "citation_readiness_score": 60,
            "truth_consistency_score": 80,
            "answer_pack_score": 70,
            "external_trust_alignment_score": null,
            "elapsed_us": 0,
            "blockers": [],
            "route_scores": [],
        }))
        .expect("score fixture");

        let text = score_text(&report, path("score.json"));
        assert!(text.contains("External trust alignment: n/a"), "{text}");
        assert!(!text.contains("External trust alignment: 0"), "{text}");
    }

    /// Blockers are the most actionable output of the score report: a
    /// site-wide blocker has no route and must say so rather than printing an
    /// empty route field.
    #[test]
    fn score_text_marks_a_sitewide_blocker() {
        let report: SiteIntelligenceScore = serde_json::from_value(serde_json::json!({
            "overall_score": 40,
            "citation_readiness_score": 30,
            "truth_consistency_score": 50,
            "answer_pack_score": 40,
            "external_trust_alignment_score": 60,
            "elapsed_us": 0,
            "blockers": [{
                "category": "truth",
                "route": null,
                "severity": 80,
                "message": "no structured facts",
            }],
            "route_scores": [],
        }))
        .expect("score fixture");

        let text = score_text(&report, path("score.json"));
        assert!(text.contains("no structured facts"), "{text}");
        assert!(
            text.contains("route=(sitewide)"),
            "a blocker with no route must be marked site-wide: {text}"
        );
    }

    // --- presence: ordering and formatting ------------------------------

    fn source_result(source: &str, status: SourceStatus) -> SourceResult {
        serde_json::from_value(serde_json::json!({
            "source": source,
            "status": status,
            "checkedAt": "2024-01-01T00:00:00Z",
        }))
        .expect("source result fixture")
    }

    /// The presence report is read as a checklist, so results must appear in
    /// the canonical `SOURCE_ORDER` regardless of the order they were checked
    /// in — the ordering is what makes two runs comparable.
    #[test]
    fn order_results_sorts_into_the_canonical_source_order() {
        let results = vec![
            source_result("github", SourceStatus::Found),
            source_result("wikipedia", SourceStatus::Found),
            source_result("rdap", SourceStatus::Found),
        ];
        let ordered = order_results(&results);
        let names: Vec<_> = ordered.iter().map(|r| r.source.as_str()).collect();
        assert_eq!(names, vec!["wikipedia", "github", "rdap"]);
    }

    /// An unrecognised source is not dropped — it is appended after the known
    /// ones so a newly-added source still shows up in the report.
    #[test]
    fn order_results_appends_unknown_sources_rather_than_dropping_them() {
        let results = vec![
            source_result("brand_new_source", SourceStatus::Found),
            source_result("wikipedia", SourceStatus::Found),
        ];
        let ordered = order_results(&results);
        let names: Vec<_> = ordered.iter().map(|r| r.source.as_str()).collect();
        assert_eq!(names, vec!["wikipedia", "brand_new_source"]);
    }

    /// `Unreachable` and `NotFound` mean very different things to a reader —
    /// "we couldn't check" is not "it's absent" — so each status needs its
    /// own symbol and wording.
    #[test]
    fn format_source_line_distinguishes_every_status() {
        for (status, symbol, word) in [
            (SourceStatus::Found, "[+]", "found"),
            (SourceStatus::NotFound, "[ ]", "no record"),
            (SourceStatus::Unreachable, "[!]", "couldn't reach"),
            (SourceStatus::Skipped, "[-]", "skipped"),
        ] {
            let line = format_source_line(&source_result("wikipedia", status));
            assert!(line.contains(symbol), "status {status:?}: {line}");
            assert!(line.contains(word), "status {status:?}: {line}");
            assert!(line.contains("Wikipedia"), "status {status:?}: {line}");
        }
    }

    // --- unique_source_types -------------------------------------------

    /// Trust-surface records can repeat a source type; the summary must
    /// de-duplicate and sort so the count reflects distinct types.
    #[test]
    fn unique_source_types_sorts_and_dedups() {
        let record = |source_type: &str| TrustSurfaceRecord {
            source_type: source_type.to_string(),
            url: "https://example.com".to_string(),
            title: None,
            snippet: None,
            entity: None,
            observed_at: None,
            metrics: Default::default(),
        };
        let types = unique_source_types(&[record("github"), record("rdap"), record("github")]);
        assert_eq!(types, vec!["github".to_string(), "rdap".to_string()]);
    }

    #[test]
    fn unique_source_types_is_empty_for_no_records() {
        assert!(unique_source_types(&[]).is_empty());
    }
}

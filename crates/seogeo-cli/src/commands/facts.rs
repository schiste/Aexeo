//! `seogeo-cli facts ...` commands.
//!
//! Today this hosts the `validate` subcommand, which loads a candidate
//! `facts.json`, validates its shape via `validate_truth_manifest`, then runs
//! `assess_truth_layer` against the site to surface mismatches. The audit half
//! is what makes this command useful beyond a JSON-schema check: it tells the
//! editor whether what they (or an LLM) wrote actually agrees with what's on
//! the site.

use anyhow::{Result, bail};
use clap::ArgMatches;
use seogeo_core::adapter::resolve_static_site_root;
use seogeo_core::config::load_config_with_diagnostics;
use seogeo_core::{
    TruthMismatchSeverity, assess_truth_layer, load_site, load_truth_manifest,
    validate_truth_manifest,
};
use std::path::PathBuf;

use crate::commands::common::{canonicalize_or_keep, required_arg};
use crate::output::emit_config_warnings;

pub fn command_facts(submatches: &ArgMatches) -> Result<i32> {
    match submatches.subcommand() {
        Some(("validate", validate_matches)) => command_facts_validate(validate_matches),
        Some((other, _)) => bail!("unsupported facts subcommand: {}", other),
        None => bail!("missing facts subcommand"),
    }
}

fn command_facts_validate(submatches: &ArgMatches) -> Result<i32> {
    let manifest_path = PathBuf::from(required_arg(submatches, "path")?);
    let site_root = canonicalize_or_keep(
        submatches
            .get_one::<String>("site-path")
            .map(String::as_str)
            .unwrap_or("."),
    );
    let format = submatches
        .get_one::<String>("format")
        .map(String::as_str)
        .unwrap_or("text");

    let manifest = load_truth_manifest(&manifest_path)?;
    let validation = validate_truth_manifest(&manifest);

    let explicit_config = submatches.get_one::<String>("config").map(PathBuf::from);
    let loaded = load_config_with_diagnostics(&site_root, explicit_config.as_deref())?;
    let site = load_site(&resolve_static_site_root(&site_root, &loaded.config)?)?;
    let assessment = assess_truth_layer(&site, Some(&manifest));

    let error_count = validation.errors.len()
        + assessment
            .mismatches
            .iter()
            .filter(|m| matches!(m.severity, TruthMismatchSeverity::Error))
            .count();

    match format {
        "json" => {
            let payload = serde_json::json!({
                "manifest_path": manifest_path.display().to_string(),
                "validation": validation,
                "assessment": assessment,
                "error_count": error_count,
                "ok": error_count == 0,
            });
            println!("{}", serde_json::to_string_pretty(&payload)?);
        }
        _ => {
            emit_config_warnings(&loaded.warnings);
            print_text_report(&manifest_path, &validation, &assessment);
        }
    }

    Ok(if error_count == 0 { 0 } else { 1 })
}

fn print_text_report(
    manifest_path: &std::path::Path,
    validation: &seogeo_core::TruthManifestValidation,
    assessment: &seogeo_core::TruthAssessment,
) {
    println!("facts validate: {}", manifest_path.display());
    println!();
    println!(
        "  shape: {} (version {})",
        if validation.valid { "valid" } else { "invalid" },
        validation.version
    );
    println!(
        "  organization: {}, products: {}",
        if validation.organization_present {
            "present"
        } else {
            "missing"
        },
        validation.product_count
    );
    println!(
        "  terminology: {} preferred, {} forbidden",
        validation.preferred_term_count, validation.forbidden_term_count
    );

    if !validation.errors.is_empty() {
        println!();
        println!("  schema errors:");
        for error in &validation.errors {
            println!("    - {}", error);
        }
    }
    if !validation.warnings.is_empty() {
        println!();
        println!("  schema warnings:");
        for warning in &validation.warnings {
            println!("    - {}", warning);
        }
    }

    println!();
    println!(
        "  assessment: {}/{} score, source = {:?}",
        assessment.score, assessment.score_ceiling, assessment.structured_truth_source
    );
    println!(
        "  pages analyzed: {}, with schema: {}, manifest present: {}",
        assessment.pages_analyzed, assessment.pages_with_schema, assessment.manifest_present
    );
    println!(
        "  preferred-term hits: {}, forbidden-term hits: {}",
        assessment.preferred_term_hits, assessment.forbidden_term_hits
    );

    if !assessment.mismatches.is_empty() {
        println!();
        println!("  mismatches:");
        for mismatch in &assessment.mismatches {
            let sev = match mismatch.severity {
                TruthMismatchSeverity::Error => "error",
                TruthMismatchSeverity::Warning => "warn",
            };
            println!(
                "    [{}] route={} field={} expected={:?} observed={:?} ({})",
                sev,
                mismatch.route,
                mismatch.field,
                mismatch.expected,
                mismatch.observed,
                mismatch.source
            );
        }
    }
}

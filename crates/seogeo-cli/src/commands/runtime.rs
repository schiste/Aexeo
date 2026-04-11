use anyhow::{Result, bail};
use clap::ArgMatches;
use seogeo_core::config::load_config_with_diagnostics;
use seogeo_core::{
    diff_finding_sets, load_findings_from_audit, render_diff_text, render_sarif, render_text,
    run_runtime_audit, verify_runtime_audit, write_audit_artifact,
};
use std::path::{Path, PathBuf};

use crate::output::{emit_config_warnings, render_audit_command_json, render_diff_command_json};

fn apply_runtime_cli_overrides(config: &mut seogeo_core::Config, submatches: &ArgMatches) {
    if let Some(values) = submatches.get_many::<String>("seed") {
        config.crawl_seeds = values.cloned().collect();
    }
    if let Some(values) = submatches.get_many::<String>("include-pattern") {
        config.crawl_include_patterns = values.cloned().collect();
    }
    if let Some(values) = submatches.get_many::<String>("exclude-pattern") {
        config.crawl_exclude_patterns = values.cloned().collect();
    }
    if submatches.get_flag("no-sitemap-seed") {
        config.crawl_use_sitemap = false;
    }
}

fn selected_runtime_engine<'a>(
    config: &'a seogeo_core::Config,
    submatches: &'a ArgMatches,
) -> &'a str {
    submatches
        .get_one::<String>("engine")
        .map(String::as_str)
        .unwrap_or(config.runtime().browser_engine)
}

pub fn command_crawl(submatches: &ArgMatches) -> Result<i32> {
    let cwd = std::env::current_dir()?;
    let explicit_config = submatches.get_one::<String>("config").map(PathBuf::from);
    let loaded = load_config_with_diagnostics(&cwd, explicit_config.as_deref())?;
    let mut config = loaded.config;
    let warnings = loaded.warnings;
    apply_runtime_cli_overrides(&mut config, submatches);
    let baseline_path = submatches.get_one::<String>("baseline");
    let regressions_only = submatches.get_flag("regressions-only");
    if regressions_only && baseline_path.is_none() {
        bail!("--regressions-only requires --baseline");
    }
    let audit = run_runtime_audit(
        submatches.get_one::<String>("url").unwrap(),
        *submatches.get_one::<usize>("max-pages").unwrap_or(&200),
        selected_runtime_engine(&config, submatches),
        &config,
    )?;
    let audit_path = write_audit_artifact(
        &audit.findings,
        &cwd,
        "crawl",
        config.output().audit_log_limit,
    )?;
    let findings_to_render = if let Some(baseline_path) = baseline_path {
        let baseline_findings = load_findings_from_audit(Path::new(baseline_path))?;
        let diff = diff_finding_sets(&baseline_findings, &audit.findings);
        if regressions_only {
            diff.new_findings
        } else {
            audit.findings.clone()
        }
    } else {
        audit.findings.clone()
    };
    let success = if regressions_only {
        findings_to_render.is_empty()
    } else {
        !findings_to_render.iter().any(|finding| finding.is_error())
    };

    match submatches
        .get_one::<String>("format")
        .map(String::as_str)
        .unwrap_or("text")
    {
        "json" => println!(
            "{}",
            render_audit_command_json(
                "crawl",
                &findings_to_render,
                success,
                Some(audit_path.display().to_string()),
                warnings,
            )?
        ),
        "sarif" => println!("{}", render_sarif(&findings_to_render, "seogeo")?),
        _ => {
            emit_config_warnings(&warnings);
            println!(
                "{}",
                render_text(
                    &findings_to_render,
                    "All runtime checks passed.",
                    Some(&audit_path)
                )
            );
        }
    }
    let exit_code = if success { 0 } else { 1 };
    Ok(exit_code)
}

pub fn command_verify(submatches: &ArgMatches) -> Result<i32> {
    let cwd = std::env::current_dir()?;
    let explicit_config = submatches.get_one::<String>("config").map(PathBuf::from);
    let loaded = load_config_with_diagnostics(&cwd, explicit_config.as_deref())?;
    let mut config = loaded.config;
    let warnings = loaded.warnings;
    apply_runtime_cli_overrides(&mut config, submatches);
    let audit = run_runtime_audit(
        submatches.get_one::<String>("url").unwrap(),
        *submatches.get_one::<usize>("max-pages").unwrap_or(&200),
        selected_runtime_engine(&config, submatches),
        &config,
    )?;
    let baseline_path = submatches
        .get_one::<String>("baseline")
        .map(PathBuf::from)
        .unwrap_or_else(|| cwd.join(config.output().baseline_file));
    let baseline_findings = if baseline_path.exists() {
        load_findings_from_audit(&baseline_path)?
    } else {
        Vec::new()
    };
    let diff = verify_runtime_audit(&audit, &baseline_findings);
    match submatches
        .get_one::<String>("format")
        .map(String::as_str)
        .unwrap_or("text")
    {
        "json" => println!(
            "{}",
            render_diff_command_json("verify", &diff, diff.new_findings.is_empty(), warnings)?
        ),
        _ => {
            emit_config_warnings(&warnings);
            println!("{}", render_diff_text(&diff));
        }
    }
    Ok(if diff.new_findings.is_empty() { 0 } else { 1 })
}

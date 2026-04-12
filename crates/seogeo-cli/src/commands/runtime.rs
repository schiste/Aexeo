use anyhow::{Result, bail};
use clap::ArgMatches;
use seogeo_contracts::AuditStatus;
use seogeo_core::config::load_config_with_diagnostics;
use seogeo_core::{
    RuntimeAudit, RuntimeAuditOptions, RuntimeProgressEvent, RuntimeProgressMode,
    build_audit_artifact, diff_finding_sets, load_audit_artifact, load_findings_from_audit,
    render_diff_text, render_sarif, render_text_artifact, run_runtime_audit_with_options,
    runtime_doctor, verify_runtime_audit, write_audit_artifact, write_partial_audit_artifact,
};
use std::path::{Path, PathBuf};

use crate::commands::common::required_arg;
use crate::output::{
    emit_config_warnings, render_audit_command_json, render_diff_command_json,
    render_failed_command_json,
};

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

fn runtime_output_artifact(
    command: &str,
    audit: &RuntimeAudit,
    findings: &[seogeo_contracts::Finding],
) -> seogeo_contracts::AuditArtifact {
    build_audit_artifact(
        command,
        findings,
        audit.status,
        Some(audit.crawl_stats.clone()),
        audit.truncation_reason.clone(),
    )
}

fn print_progress(mode: &str, event: RuntimeProgressEvent) {
    match mode {
        "json" => eprintln!(
            "{}",
            serde_json::to_string(&event)
                .unwrap_or_else(|_| "{\"phase\":\"progress_error\"}".to_string())
        ),
        "plain" => {
            if event.phase == "complete" {
                eprintln!(
                    "crawl complete: engine={} visited={} discovered={} queued={} retries={} failures={} skipped_non_html={} truncated={} elapsed_ms={} ppm={} avg_fetch_ms={} avg_process_ms={} avg_partial_audit_ms={} checkpoints={} partial_artifacts={}",
                    event.engine,
                    event.visited_pages,
                    event.discovered_internal_routes,
                    event.queued_routes_remaining,
                    event.fetch_retries,
                    event.fetch_failures,
                    event.skipped_non_html,
                    event.truncated,
                    event.elapsed_ms,
                    event.pages_per_minute,
                    event.average_fetch_ms,
                    event.average_page_process_ms,
                    event.average_partial_audit_ms,
                    event.checkpoints_written,
                    event.partial_artifacts_written
                );
            } else {
                eprintln!(
                    "crawl progress: visited={} discovered={} queued={} ppm={} avg_fetch_ms={} avg_process_ms={} avg_partial_audit_ms={} checkpoints={} partial_artifacts={} url={}",
                    event.visited_pages,
                    event.discovered_internal_routes,
                    event.queued_routes_remaining,
                    event.pages_per_minute,
                    event.average_fetch_ms,
                    event.average_page_process_ms,
                    event.average_partial_audit_ms,
                    event.checkpoints_written,
                    event.partial_artifacts_written,
                    event.current_url.as_deref().unwrap_or("-")
                );
            }
        }
        _ => {}
    }
}

fn runtime_options_from_cli<'a>(
    submatches: &'a ArgMatches,
    progress: RuntimeProgressMode<'a>,
    artifact_command: &'a str,
    partial_artifact: seogeo_core::runtime::RuntimeArtifactMode<'a>,
) -> RuntimeAuditOptions<'a> {
    RuntimeAuditOptions {
        checkpoint_path: submatches.get_one::<String>("checkpoint").map(Path::new),
        checkpoint_every: *submatches
            .get_one::<usize>("checkpoint-every")
            .unwrap_or(&25),
        partial_artifact_every: *submatches.get_one::<usize>("artifact-every").unwrap_or(&25),
        partial_artifact_min_interval_ms: *submatches
            .get_one::<u64>("artifact-min-interval-ms")
            .unwrap_or(&15_000),
        resume_from: submatches.get_one::<String>("resume").map(Path::new),
        fetch_retry_budget: *submatches.get_one::<usize>("retry-budget").unwrap_or(&2),
        progress,
        artifact_command,
        partial_artifact,
    }
}

fn emit_runtime_failure(
    command: &str,
    format: &str,
    error: &anyhow::Error,
    warnings: Vec<seogeo_core::config::ConfigWarning>,
) -> Result<i32> {
    match format {
        "json" => println!(
            "{}",
            render_failed_command_json(command, error.to_string(), warnings)?
        ),
        _ => {
            emit_config_warnings(&warnings);
            eprintln!("Runtime audit failed: {}", error);
        }
    }
    Ok(2)
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
    let format = submatches
        .get_one::<String>("format")
        .map(String::as_str)
        .unwrap_or("text");
    let progress_mode = submatches
        .get_one::<String>("progress")
        .map(String::as_str)
        .unwrap_or("plain");
    let mut callback = |event: RuntimeProgressEvent| print_progress(progress_mode, event);
    let mut partial_writer = |artifact: &seogeo_contracts::AuditArtifact| -> Result<()> {
        let _ = write_partial_audit_artifact(artifact, &cwd, "crawl")?;
        Ok(())
    };
    let mut options = runtime_options_from_cli(
        submatches,
        match progress_mode {
            "off" => RuntimeProgressMode::Off,
            _ => RuntimeProgressMode::Callback(&mut callback),
        },
        "crawl",
        seogeo_core::runtime::RuntimeArtifactMode::Callback(&mut partial_writer),
    );

    let audit = match run_runtime_audit_with_options(
        required_arg(submatches, "url")?,
        *submatches.get_one::<usize>("max-pages").unwrap_or(&200),
        selected_runtime_engine(&config, submatches),
        &config,
        &mut options,
    ) {
        Ok(audit) => audit,
        Err(error) => return emit_runtime_failure("crawl", format, &error, warnings),
    };
    let full_audit_artifact = runtime_output_artifact("crawl", &audit, &audit.findings);
    let audit_path = write_audit_artifact(
        &full_audit_artifact,
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
    let render_artifact = runtime_output_artifact("crawl", &audit, &findings_to_render);
    let success = if regressions_only {
        findings_to_render.is_empty() && audit.status != AuditStatus::Failed
    } else {
        !findings_to_render.iter().any(|finding| finding.is_error())
            && audit.status != AuditStatus::Failed
    };

    match format {
        "json" => println!(
            "{}",
            render_audit_command_json(
                "crawl",
                &render_artifact,
                success,
                Some(audit_path.display().to_string()),
                warnings,
            )?
        ),
        "sarif" => {
            emit_config_warnings(&warnings);
            println!("{}", render_sarif(&findings_to_render, "seogeo")?);
        }
        _ => {
            emit_config_warnings(&warnings);
            println!(
                "{}",
                render_text_artifact(
                    &render_artifact,
                    "All runtime checks passed.",
                    Some(&audit_path)
                )
            );
        }
    }
    Ok(if success { 0 } else { 1 })
}

pub fn command_verify(submatches: &ArgMatches) -> Result<i32> {
    let cwd = std::env::current_dir()?;
    let explicit_config = submatches.get_one::<String>("config").map(PathBuf::from);
    let loaded = load_config_with_diagnostics(&cwd, explicit_config.as_deref())?;
    let mut config = loaded.config;
    let warnings = loaded.warnings;
    apply_runtime_cli_overrides(&mut config, submatches);
    let format = submatches
        .get_one::<String>("format")
        .map(String::as_str)
        .unwrap_or("text");
    let progress_mode = submatches
        .get_one::<String>("progress")
        .map(String::as_str)
        .unwrap_or("plain");
    let mut callback = |event: RuntimeProgressEvent| print_progress(progress_mode, event);
    let mut partial_writer = |artifact: &seogeo_contracts::AuditArtifact| -> Result<()> {
        let _ = write_partial_audit_artifact(artifact, &cwd, "verify")?;
        Ok(())
    };
    let mut options = runtime_options_from_cli(
        submatches,
        match progress_mode {
            "off" => RuntimeProgressMode::Off,
            _ => RuntimeProgressMode::Callback(&mut callback),
        },
        "verify",
        seogeo_core::runtime::RuntimeArtifactMode::Callback(&mut partial_writer),
    );
    let audit = match run_runtime_audit_with_options(
        required_arg(submatches, "url")?,
        *submatches.get_one::<usize>("max-pages").unwrap_or(&200),
        selected_runtime_engine(&config, submatches),
        &config,
        &mut options,
    ) {
        Ok(audit) => audit,
        Err(error) => return emit_runtime_failure("verify", format, &error, warnings),
    };
    let baseline_path = submatches
        .get_one::<String>("baseline")
        .map(PathBuf::from)
        .unwrap_or_else(|| cwd.join(config.output().baseline_file));
    let baseline_artifact = if baseline_path.exists() {
        load_audit_artifact(&baseline_path)?
    } else {
        build_audit_artifact("baseline", &[], AuditStatus::Complete, None, None)
    };
    if baseline_artifact.is_partial() && !submatches.get_flag("allow-partial-baseline") {
        bail!(
            "baseline audit is partial; rerun the baseline or pass --allow-partial-baseline to compare against an incomplete crawl"
        );
    }
    let diff = verify_runtime_audit(&audit, &baseline_artifact.findings);
    match format {
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

pub fn command_doctor(submatches: &ArgMatches) -> Result<i32> {
    let target = required_arg(submatches, "target")?;
    let format = submatches
        .get_one::<String>("format")
        .map(String::as_str)
        .unwrap_or("text");
    match target {
        "runtime" => {
            let doctor = runtime_doctor();
            match format {
                "json" => println!("{}", serde_json::to_string_pretty(&doctor)?),
                _ => {
                    println!("Runtime Doctor");
                    println!();
                    println!("- Playwright available: {}", doctor.available);
                    println!("- Mode: {}", doctor.mode);
                    println!("- Executable: {}", doctor.executable);
                    println!("- Detail: {}", doctor.message);
                }
            }
            Ok(if doctor.available { 0 } else { 1 })
        }
        other => bail!("unsupported doctor target: {}", other),
    }
}

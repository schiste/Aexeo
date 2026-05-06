use aexeo_contracts::AuditStatus;
use aexeo_contracts::PerformanceBudget;
use aexeo_contracts::PerformanceDiffThresholds;
use aexeo_core::config::load_config_with_diagnostics;
use aexeo_core::{
    RuntimeAudit, RuntimeAuditOptions, RuntimeProgressEvent, RuntimeProgressMode,
    audit_site_snapshot, build_audit_artifact, diff_finding_sets, diff_performance_artifacts,
    evaluate_performance_budget, load_audit_artifact, load_findings_from_audit, render_diff_text,
    render_performance_diff_text, render_sarif, render_text_artifact,
    run_runtime_audit_with_options, runtime_doctor, verify_runtime_audit, write_audit_artifact,
    write_partial_audit_artifact, write_progress_audit_artifact,
};
use anyhow::{Result, bail};
use clap::ArgMatches;
use std::fs;
use std::path::{Path, PathBuf};

use crate::commands::common::required_arg;
use crate::output::{
    emit_config_warnings, render_audit_command_json, render_diff_command_json,
    render_failed_command_json,
};

fn apply_runtime_cli_overrides(config: &mut aexeo_core::Config, submatches: &ArgMatches) {
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
    // try_get_one because not every runtime subcommand registers
    // --a11y-strict (only `crawl` does today). get_flag would panic.
    if matches!(
        submatches.try_get_one::<bool>("a11y-strict"),
        Ok(Some(true))
    ) {
        config.accessibility.strict = true;
    }
    apply_cf_access_credentials(config, submatches);
}

/// Inject Cloudflare Access service-token headers into the runtime
/// audit's crawl_headers so every fetch passes the Access gate.
///
/// Resolution order: explicit CLI flags first, then env vars
/// (CF_ACCESS_CLIENT_ID / CF_ACCESS_CLIENT_SECRET), then nothing —
/// in which case the request goes through unchanged. Both the ID
/// and the secret must be present together; if only one resolves,
/// neither header gets set (better to fail visibly at the Access
/// gate than to send a half-credential).
fn apply_cf_access_credentials(config: &mut aexeo_core::Config, submatches: &ArgMatches) {
    // try_get_one rather than get_one because not every runtime
    // subcommand registers these flags (only `crawl` does today).
    // get_one would panic on the others; try_get_one returns Ok(None)
    // which falls through to the env-var path cleanly.
    let cli_id = submatches
        .try_get_one::<String>("cf-access-id")
        .ok()
        .flatten()
        .cloned();
    let cli_secret = submatches
        .try_get_one::<String>("cf-access-secret")
        .ok()
        .flatten()
        .cloned();
    let id = cli_id.or_else(|| std::env::var("CF_ACCESS_CLIENT_ID").ok());
    let secret = cli_secret.or_else(|| std::env::var("CF_ACCESS_CLIENT_SECRET").ok());
    if let (Some(id), Some(secret)) = (id, secret) {
        config
            .crawl_headers
            .insert("CF-Access-Client-Id".to_string(), id);
        config
            .crawl_headers
            .insert("CF-Access-Client-Secret".to_string(), secret);
    }
}

fn selected_runtime_engine<'a>(
    config: &'a aexeo_core::Config,
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
    findings: &[aexeo_contracts::Finding],
    site_url: Option<&str>,
) -> aexeo_contracts::AuditArtifact {
    let mut artifact = build_audit_artifact(
        command,
        findings,
        audit.status,
        Some(audit.crawl_stats.clone()),
        audit.truncation_reason.clone(),
    );
    artifact.performance = audit.performance.clone();
    artifact.site = Some(audit_site_snapshot(&audit.site, site_url));
    artifact
}

fn load_performance_budget(
    submatches: &ArgMatches,
) -> Result<Option<(PathBuf, PerformanceBudget)>> {
    let Some(path) = submatches.get_one::<String>("perf-budget") else {
        return Ok(None);
    };
    load_performance_budget_path(PathBuf::from(path)).map(Some)
}

fn load_performance_budget_path(path: PathBuf) -> Result<(PathBuf, PerformanceBudget)> {
    let text = fs::read_to_string(&path)?;
    let budget = serde_json::from_str::<PerformanceBudget>(&text)?;
    Ok((path, budget))
}

fn load_performance_budget_or_default(
    submatches: &ArgMatches,
    cwd: &Path,
    config: &aexeo_core::Config,
) -> Result<Option<(PathBuf, PerformanceBudget)>> {
    if let Some(explicit) = load_performance_budget(submatches)? {
        return Ok(Some(explicit));
    }
    let configured = cwd.join(config.performance_budget_file.clone());
    if configured.exists() {
        return load_performance_budget_path(configured).map(Some);
    }
    Ok(None)
}

fn apply_performance_budget(
    artifact: &mut aexeo_contracts::AuditArtifact,
    budget: Option<&(PathBuf, PerformanceBudget)>,
) -> bool {
    let Some((path, budget)) = budget else {
        return true;
    };
    let report =
        evaluate_performance_budget(artifact, budget.clone(), Some(path.display().to_string()));
    let passed = report.passed;
    if let Some(performance) = artifact.performance.as_mut() {
        performance.budget = Some(report);
    }
    passed
}

fn copy_performance_budget(
    target: &mut aexeo_contracts::AuditArtifact,
    source: &aexeo_contracts::AuditArtifact,
) {
    let Some(source_budget) = source
        .performance
        .as_ref()
        .and_then(|performance| performance.budget.clone())
    else {
        return;
    };
    if let Some(target_performance) = target.performance.as_mut() {
        target_performance.budget = Some(source_budget);
    }
}

fn sanitize_baseline_name(name: &str) -> String {
    let mut sanitized = String::new();
    let mut last_was_separator = false;
    for character in name.chars() {
        let next = if character.is_ascii_alphanumeric() {
            last_was_separator = false;
            Some(character.to_ascii_lowercase())
        } else if matches!(character, '.' | '_' | '-') {
            if last_was_separator {
                None
            } else {
                last_was_separator = true;
                Some(character)
            }
        } else if last_was_separator {
            None
        } else {
            last_was_separator = true;
            Some('-')
        };
        if let Some(next) = next {
            sanitized.push(next);
        }
    }
    let sanitized = sanitized.trim_matches(['.', '_', '-']).to_string();
    if sanitized.is_empty() {
        "runtime".to_string()
    } else {
        sanitized
    }
}

fn runtime_baseline_latest_path(cwd: &Path, name: &str) -> PathBuf {
    cwd.join(".aexeo-reports")
        .join(format!("{name}-runtime-baseline-latest.json"))
}

fn runtime_baseline_timestamped_path(cwd: &Path, name: &str, generated_at: u64) -> PathBuf {
    cwd.join(".aexeo-reports")
        .join(format!("{name}-runtime-baseline-{generated_at}.json"))
}

fn write_runtime_baseline_artifact(
    artifact: &aexeo_contracts::AuditArtifact,
    cwd: &Path,
    name: &str,
    update_latest: bool,
) -> Result<(PathBuf, PathBuf)> {
    let reports_dir = cwd.join(".aexeo-reports");
    fs::create_dir_all(&reports_dir)?;
    let latest_path = runtime_baseline_latest_path(cwd, name);
    let timestamped_path = runtime_baseline_timestamped_path(cwd, name, artifact.generated_at);
    let payload = serde_json::to_string_pretty(artifact)?;
    fs::write(&timestamped_path, &payload)?;
    if update_latest {
        fs::write(&latest_path, payload)?;
    }
    Ok((latest_path, timestamped_path))
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
                    "crawl complete: engine={} visited={} discovered={} queued={} retries={} failures={} skipped_non_html={} truncated={} elapsed_ms={} ppm={} avg_fetch_ms={} avg_process_ms={} avg_partial_audit_ms={} checkpoints={} progress_artifacts={} partial_artifacts={}",
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
                    event.progress_artifacts_written,
                    event.partial_artifacts_written
                );
            } else {
                eprintln!(
                    "crawl progress: visited={} discovered={} queued={} ppm={} avg_fetch_ms={} avg_process_ms={} avg_partial_audit_ms={} checkpoints={} progress_artifacts={} partial_artifacts={} url={}",
                    event.visited_pages,
                    event.discovered_internal_routes,
                    event.queued_routes_remaining,
                    event.pages_per_minute,
                    event.average_fetch_ms,
                    event.average_page_process_ms,
                    event.average_partial_audit_ms,
                    event.checkpoints_written,
                    event.progress_artifacts_written,
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
    progress_artifact: aexeo_core::runtime::RuntimeArtifactMode<'a>,
    partial_artifact: aexeo_core::runtime::RuntimeArtifactMode<'a>,
) -> RuntimeAuditOptions<'a> {
    RuntimeAuditOptions {
        checkpoint_path: submatches
            .try_get_one::<String>("checkpoint")
            .ok()
            .flatten()
            .map(Path::new),
        checkpoint_every: *submatches
            .try_get_one::<usize>("checkpoint-every")
            .ok()
            .flatten()
            .unwrap_or(&25),
        progress_artifact_every: *submatches
            .try_get_one::<usize>("artifact-every")
            .ok()
            .flatten()
            .unwrap_or(&25),
        progress_artifact_min_interval_ms: *submatches
            .try_get_one::<u64>("artifact-min-interval-ms")
            .ok()
            .flatten()
            .unwrap_or(&15_000),
        partial_audit_every: *submatches
            .try_get_one::<usize>("partial-audit-every")
            .ok()
            .flatten()
            .unwrap_or(&100),
        partial_audit_min_interval_ms: *submatches
            .try_get_one::<u64>("partial-audit-min-interval-ms")
            .ok()
            .flatten()
            .unwrap_or(&60_000),
        resume_from: submatches
            .try_get_one::<String>("resume")
            .ok()
            .flatten()
            .map(Path::new),
        fetch_retry_budget: *submatches
            .try_get_one::<usize>("retry-budget")
            .ok()
            .flatten()
            .unwrap_or(&2),
        progress,
        artifact_command,
        progress_artifact,
        partial_audit_artifact: partial_artifact,
    }
}

fn emit_runtime_failure(
    command: &str,
    format: &str,
    error: &anyhow::Error,
    warnings: Vec<aexeo_core::config::ConfigWarning>,
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
    let performance_budget = load_performance_budget(submatches)?;
    let format = submatches
        .get_one::<String>("format")
        .map(String::as_str)
        .unwrap_or("text");
    let progress_mode = submatches
        .get_one::<String>("progress")
        .map(String::as_str)
        .unwrap_or("plain");
    let mut callback = |event: RuntimeProgressEvent| print_progress(progress_mode, event);
    let mut progress_writer = |artifact: &aexeo_contracts::AuditArtifact| -> Result<()> {
        let _ = write_progress_audit_artifact(artifact, &cwd, "crawl")?;
        Ok(())
    };
    let mut partial_writer = |artifact: &aexeo_contracts::AuditArtifact| -> Result<()> {
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
        aexeo_core::runtime::RuntimeArtifactMode::Callback(&mut progress_writer),
        aexeo_core::runtime::RuntimeArtifactMode::Callback(&mut partial_writer),
    );
    let target_url = required_arg(submatches, "url")?;

    let audit = match run_runtime_audit_with_options(
        target_url,
        *submatches.get_one::<usize>("max-pages").unwrap_or(&200),
        selected_runtime_engine(&config, submatches),
        &config,
        &mut options,
    ) {
        Ok(audit) => audit,
        Err(error) => return emit_runtime_failure("crawl", format, &error, warnings),
    };
    let mut full_audit_artifact =
        runtime_output_artifact("crawl", &audit, &audit.findings, Some(target_url));
    let performance_budget_passed =
        apply_performance_budget(&mut full_audit_artifact, performance_budget.as_ref());
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
    let mut render_artifact =
        runtime_output_artifact("crawl", &audit, &findings_to_render, Some(target_url));
    copy_performance_budget(&mut render_artifact, &full_audit_artifact);
    let findings_success = if regressions_only {
        findings_to_render.is_empty() && audit.status != AuditStatus::Failed
    } else {
        !findings_to_render.iter().any(|finding| finding.is_error())
            && audit.status != AuditStatus::Failed
    };
    let success = findings_success && performance_budget_passed;

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
            println!("{}", render_sarif(&findings_to_render, "aexeo")?);
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
    let mut progress_writer = |artifact: &aexeo_contracts::AuditArtifact| -> Result<()> {
        let _ = write_progress_audit_artifact(artifact, &cwd, "verify")?;
        Ok(())
    };
    let mut partial_writer = |artifact: &aexeo_contracts::AuditArtifact| -> Result<()> {
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
        aexeo_core::runtime::RuntimeArtifactMode::Callback(&mut progress_writer),
        aexeo_core::runtime::RuntimeArtifactMode::Callback(&mut partial_writer),
    );
    let target_url = required_arg(submatches, "url")?;
    let audit = match run_runtime_audit_with_options(
        target_url,
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

pub fn command_profile_runtime(submatches: &ArgMatches) -> Result<i32> {
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
    let performance_budget = load_performance_budget(submatches)?;
    let mut options = runtime_options_from_cli(
        submatches,
        RuntimeProgressMode::Off,
        "profile",
        aexeo_core::runtime::RuntimeArtifactMode::Off,
        aexeo_core::runtime::RuntimeArtifactMode::Off,
    );
    let target_url = required_arg(submatches, "url")?;
    let audit = run_runtime_audit_with_options(
        target_url,
        *submatches.get_one::<usize>("max-pages").unwrap_or(&20),
        selected_runtime_engine(&config, submatches),
        &config,
        &mut options,
    )?;
    let mut artifact =
        runtime_output_artifact("profile", &audit, &audit.findings, Some(target_url));
    let performance_budget_passed =
        apply_performance_budget(&mut artifact, performance_budget.as_ref());
    match format {
        "json" => {
            println!("{}", serde_json::to_string_pretty(&artifact)?);
        }
        _ => {
            emit_config_warnings(&warnings);
            let crawl = &audit.crawl_stats;
            println!("Runtime Profile");
            println!();
            println!("- Engine: {}", crawl.engine);
            println!("- Status: {:?}", audit.status);
            println!("- Visited pages: {}", crawl.visited_pages);
            println!("- Discovered routes: {}", crawl.discovered_internal_routes);
            println!("- Queued routes: {}", crawl.queued_routes_remaining);
            println!("- Elapsed: {}ms", crawl.elapsed_ms);
            println!("- Throughput: {} pages/min", crawl.pages_per_minute);
            println!("- Average fetch: {}ms", crawl.average_fetch_ms);
            println!(
                "- Average page process: {}ms",
                crawl.average_page_process_ms
            );
            println!(
                "- Average partial audit: {}ms",
                crawl.average_partial_audit_ms
            );
            println!();
            if let Some(performance) = audit.performance.as_ref() {
                if performance.wall_clock_us > 0 || performance.cumulative_tracked_us > 0 {
                    println!(
                        "Performance Basis: wall_clock={}ms cumulative_tracked={}ms",
                        performance.wall_clock_us / 1_000,
                        performance.cumulative_tracked_us / 1_000
                    );
                    println!();
                }
                if !performance.phases.is_empty() {
                    println!("Phase Timings");
                    for phase in &performance.phases {
                        let basis = if phase.basis.is_empty() {
                            "unknown"
                        } else {
                            &phase.basis
                        };
                        println!(
                            "- {}: {}ms basis={} cumulative_share={}.{:02}% wall_share={}.{:02}% p95={}ms samples={}",
                            phase.name,
                            phase.elapsed_us / 1_000,
                            basis,
                            phase.cumulative_share_basis_points / 100,
                            phase.cumulative_share_basis_points % 100,
                            phase.wall_share_basis_points / 100,
                            phase.wall_share_basis_points % 100,
                            phase.p95_us / 1_000,
                            phase.sample_count
                        );
                    }
                    println!();
                }
                if !performance.rule_groups.is_empty() {
                    println!("Slowest Rule Groups");
                    for group in performance.rule_groups.iter().take(10) {
                        println!(
                            "- {}: {}ms ({} findings)",
                            group.group,
                            group.elapsed_us / 1_000,
                            group.findings
                        );
                    }
                    println!();
                }
                if !performance.bottlenecks.is_empty() {
                    println!("Runtime Bottlenecks");
                    for bottleneck in performance.bottlenecks.iter().take(8) {
                        println!(
                            "- {} {}: {}ms share={}.{:02}% wall={}.{:02}% cumulative={}.{:02}%{}",
                            bottleneck.kind,
                            bottleneck.name,
                            bottleneck.elapsed_us / 1_000,
                            bottleneck.share_basis_points / 100,
                            bottleneck.share_basis_points % 100,
                            bottleneck.wall_share_basis_points / 100,
                            bottleneck.wall_share_basis_points % 100,
                            bottleneck.cumulative_share_basis_points / 100,
                            bottleneck.cumulative_share_basis_points % 100,
                            bottleneck
                                .findings
                                .map(|value| format!(" findings={}", value))
                                .unwrap_or_default()
                        );
                        if let Some(recommendation) = &bottleneck.recommendation {
                            println!("  recommendation: {}", recommendation);
                        }
                    }
                    println!();
                }
                if !performance.observations.is_empty() {
                    println!("Performance Observations");
                    for observation in &performance.observations {
                        println!("- {}", observation);
                    }
                    println!();
                }
                if let Some(budget) = artifact
                    .performance
                    .as_ref()
                    .and_then(|performance| performance.budget.as_ref())
                {
                    println!("Performance Budget");
                    println!("- Passed: {}", budget.passed);
                    if let Some(path) = budget.budget_path.as_deref() {
                        println!("- Path: {}", path);
                    }
                    for violation in &budget.violations {
                        println!("- {}", violation.message);
                    }
                    for warning in &budget.warnings {
                        println!("- Warning: {}", warning);
                    }
                    println!();
                }
            }
            if !crawl.slowest_paths.is_empty() {
                println!("Slowest Paths");
                for path in &crawl.slowest_paths {
                    println!(
                        "- {}: fetch={}ms process={}ms",
                        path.url, path.fetch_ms, path.process_ms
                    );
                }
            }
        }
    }
    Ok(
        if audit.status == AuditStatus::Failed || !performance_budget_passed {
            1
        } else {
            0
        },
    )
}

fn performance_diff_thresholds_from_cli(submatches: &ArgMatches) -> PerformanceDiffThresholds {
    let relative_threshold_pct = *submatches
        .get_one::<u32>("regression-threshold-pct")
        .unwrap_or(&10);
    PerformanceDiffThresholds {
        relative_threshold_basis_points: relative_threshold_pct.saturating_mul(100),
        absolute_threshold: *submatches
            .get_one::<u64>("absolute-threshold-ms")
            .unwrap_or(&0),
    }
}

pub fn command_perf_baseline(submatches: &ArgMatches) -> Result<i32> {
    let cwd = std::env::current_dir()?;
    let explicit_config = submatches.get_one::<String>("config").map(PathBuf::from);
    let loaded = load_config_with_diagnostics(&cwd, explicit_config.as_deref())?;
    let mut config = loaded.config;
    let warnings = loaded.warnings;
    apply_runtime_cli_overrides(&mut config, submatches);
    let target_url = required_arg(submatches, "url")?;
    let format = submatches
        .get_one::<String>("format")
        .map(String::as_str)
        .unwrap_or("text");
    let baseline_name = sanitize_baseline_name(
        submatches
            .get_one::<String>("name")
            .map(String::as_str)
            .unwrap_or("runtime"),
    );
    let performance_budget = load_performance_budget_or_default(submatches, &cwd, &config)?;
    let latest_path = runtime_baseline_latest_path(&cwd, &baseline_name);
    let compare_path = submatches
        .get_one::<String>("compare-to")
        .map(PathBuf::from)
        .or_else(|| latest_path.exists().then_some(latest_path.clone()));
    let compare_artifact = compare_path
        .as_ref()
        .map(|path| load_audit_artifact(path))
        .transpose()?;
    let mut options = runtime_options_from_cli(
        submatches,
        RuntimeProgressMode::Off,
        "perf-baseline",
        aexeo_core::runtime::RuntimeArtifactMode::Off,
        aexeo_core::runtime::RuntimeArtifactMode::Off,
    );
    let audit = run_runtime_audit_with_options(
        target_url,
        *submatches.get_one::<usize>("max-pages").unwrap_or(&200),
        selected_runtime_engine(&config, submatches),
        &config,
        &mut options,
    )?;
    let mut artifact =
        runtime_output_artifact("perf-baseline", &audit, &audit.findings, Some(target_url));
    let performance_budget_passed =
        apply_performance_budget(&mut artifact, performance_budget.as_ref());
    let audit_path = write_audit_artifact(
        &artifact,
        &cwd,
        "perf-baseline",
        config.output().audit_log_limit,
    )?;
    let timestamped_path =
        runtime_baseline_timestamped_path(&cwd, &baseline_name, artifact.generated_at);
    let diff_report = compare_artifact.as_ref().map(|baseline| {
        diff_performance_artifacts(
            baseline,
            &artifact,
            compare_path.as_ref().map(|path| path.display().to_string()),
            Some(timestamped_path.display().to_string()),
            performance_diff_thresholds_from_cli(submatches),
        )
    });
    let diff_passed = diff_report
        .as_ref()
        .map(|report| report.summary.regressions == 0)
        .unwrap_or(true);
    let success = audit.status != AuditStatus::Failed && performance_budget_passed && diff_passed;
    let latest_updated = success || submatches.get_flag("promote-on-regression");
    let (latest_path, timestamped_path) =
        write_runtime_baseline_artifact(&artifact, &cwd, &baseline_name, latest_updated)?;

    match format {
        "json" => println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "command": "perf baseline",
                "success": success,
                "name": baseline_name,
                "url": target_url,
                "audit_path": audit_path.display().to_string(),
                "baseline_path": timestamped_path.display().to_string(),
                "latest_path": latest_path.display().to_string(),
                "latest_updated": latest_updated,
                "budget": artifact.performance.as_ref().and_then(|performance| performance.budget.as_ref()),
                "diff": diff_report,
                "warnings": warnings,
                "artifact": artifact,
            }))?
        ),
        _ => {
            emit_config_warnings(&warnings);
            println!("Performance Baseline");
            println!();
            println!("- Name: {}", baseline_name);
            println!("- URL: {}", target_url);
            println!("- Status: {:?}", audit.status);
            println!("- Pages: {}", audit.crawl_stats.visited_pages);
            println!("- Elapsed: {}ms", audit.crawl_stats.elapsed_ms);
            println!(
                "- Throughput: {} pages/min",
                audit.crawl_stats.pages_per_minute
            );
            println!("- Average fetch: {}ms", audit.crawl_stats.average_fetch_ms);
            println!("- Audit results: {}", audit_path.display());
            println!("- Baseline: {}", timestamped_path.display());
            println!("- Latest: {}", latest_path.display());
            println!("- Latest updated: {}", latest_updated);
            if let Some(budget) = artifact
                .performance
                .as_ref()
                .and_then(|performance| performance.budget.as_ref())
            {
                println!("- Budget passed: {}", budget.passed);
                if let Some(path) = budget.budget_path.as_deref() {
                    println!("- Budget path: {}", path);
                }
                for violation in &budget.violations {
                    println!("- Budget violation: {}", violation.message);
                }
            } else {
                println!("- Budget: not configured");
            }
            if let Some(report) = diff_report.as_ref() {
                println!(
                    "- Diff: regressions={} improvements={} unchanged={} missing={}",
                    report.summary.regressions,
                    report.summary.improvements,
                    report.summary.unchanged,
                    report.summary.missing
                );
                for warning in &report.warnings {
                    println!("- Diff warning: {}", warning);
                }
            } else {
                println!("- Diff: no previous baseline");
            }
        }
    }
    Ok(if success { 0 } else { 1 })
}

pub fn command_perf_diff(submatches: &ArgMatches) -> Result<i32> {
    let baseline_path = required_arg(submatches, "baseline")?;
    let current_path = required_arg(submatches, "current")?;
    let format = submatches
        .get_one::<String>("format")
        .map(String::as_str)
        .unwrap_or("text");
    let thresholds = performance_diff_thresholds_from_cli(submatches);
    let baseline = load_audit_artifact(Path::new(baseline_path))?;
    let current = load_audit_artifact(Path::new(current_path))?;
    let report = diff_performance_artifacts(
        &baseline,
        &current,
        Some(baseline_path.to_string()),
        Some(current_path.to_string()),
        thresholds,
    );
    let success = report.summary.regressions == 0;
    match format {
        "json" => println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "command": "perf diff",
                "success": success,
                "report": report,
            }))?
        ),
        _ => println!("{}", render_performance_diff_text(&report)),
    }
    Ok(if success { 0 } else { 1 })
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

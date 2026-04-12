use anyhow::{Result, anyhow, bail};
use clap::ArgMatches;
use csv::Writer;
use seogeo_core::config::load_config_with_diagnostics;
use seogeo_core::{
    BingAiImportReport, BingAiOpportunityReport, BingAiTrendReport, BingAiTrendSnapshot,
    IndexNowLedger, IndexNowRetryReport, PublishHookReport, SearchConsoleExportRow,
    SnippetInspection, build_bing_ai_opportunity_report, build_bing_ai_trend_report,
    build_publish_hook_report_with_config, export_search_console_rows, import_bing_ai_export,
    inspect_snippet_controls_path, inspect_snippet_controls_url, load_indexnow_ledger,
    record_bing_ai_trend, retry_indexnow_submissions, submit_indexnow, submit_indexnow_with_ledger,
    validate_indexnow,
};
use std::path::{Path, PathBuf};

use crate::commands::common::{canonicalize_or_keep, required_arg};
use crate::output::{emit_config_warnings, render_data_command_json, render_failed_command_json};

pub fn command_snippet(submatches: &ArgMatches) -> Result<i32> {
    match submatches.subcommand() {
        Some(("inspect", inspect_matches)) => command_snippet_inspect(inspect_matches),
        Some((other, _)) => bail!("unsupported snippet command: {}", other),
        None => bail!("missing snippet subcommand"),
    }
}

pub fn command_indexnow(submatches: &ArgMatches) -> Result<i32> {
    match submatches.subcommand() {
        Some(("validate", validate_matches)) => command_indexnow_validate(validate_matches),
        Some(("submit", submit_matches)) => command_indexnow_submit(submit_matches),
        Some(("ledger", ledger_matches)) => command_indexnow_ledger(ledger_matches),
        Some(("retry", retry_matches)) => command_indexnow_retry(retry_matches),
        Some((other, _)) => bail!("unsupported indexnow command: {}", other),
        None => bail!("missing indexnow subcommand"),
    }
}

pub fn command_bing_ai(submatches: &ArgMatches) -> Result<i32> {
    match submatches.subcommand() {
        Some(("import", import_matches)) => command_bing_ai_import(import_matches),
        Some(("opportunities", opportunities_matches)) => {
            command_bing_ai_opportunities(opportunities_matches)
        }
        Some(("trend", trend_matches)) => match trend_matches.subcommand() {
            Some(("import", import_matches)) => command_bing_ai_trend_import(import_matches),
            Some(("show", show_matches)) => command_bing_ai_trend_show(show_matches),
            Some((other, _)) => bail!("unsupported bing-ai trend command: {}", other),
            None => bail!("missing bing-ai trend subcommand"),
        },
        Some((other, _)) => bail!("unsupported bing-ai command: {}", other),
        None => bail!("missing bing-ai subcommand"),
    }
}

pub fn command_search_console(submatches: &ArgMatches) -> Result<i32> {
    match submatches.subcommand() {
        Some(("export", export_matches)) => command_search_console_export(export_matches),
        Some((other, _)) => bail!("unsupported search-console command: {}", other),
        None => bail!("missing search-console subcommand"),
    }
}

pub fn command_publish_hook(submatches: &ArgMatches) -> Result<i32> {
    match submatches.subcommand() {
        Some(("run", run_matches)) => command_publish_hook_run(run_matches),
        Some((other, _)) => bail!("unsupported publish-hook command: {}", other),
        None => bail!("missing publish-hook subcommand"),
    }
}

fn emit_integration_failure(command: &str, format: &str, error: anyhow::Error) -> Result<i32> {
    match format {
        "json" => println!(
            "{}",
            render_failed_command_json(command, error.to_string(), Vec::new())?
        ),
        _ => eprintln!("{}", error),
    }
    Ok(1)
}

fn snippet_text(inspection: &SnippetInspection) -> String {
    let mut lines = vec![
        "Snippet Inspection".to_string(),
        String::new(),
        format!("Target: {}", inspection.target),
        format!("Route: {}", inspection.route),
        format!(
            "Snippet blocked: {}",
            if inspection.snippet_blocked {
                "yes"
            } else {
                "no"
            }
        ),
        format!(
            "Meta robots: {}",
            inspection.meta_robots.as_deref().unwrap_or("-")
        ),
        format!(
            "X-Robots-Tag: {}",
            inspection.x_robots_tag.as_deref().unwrap_or("-")
        ),
        format!(
            "Canonical: {}",
            inspection.canonical.as_deref().unwrap_or("-")
        ),
        format!(
            "data-nosnippet blocks: {}",
            inspection.data_nosnippet_blocks
        ),
    ];
    if !inspection.directives.is_empty() {
        lines.push(format!("Directives: {}", inspection.directives.join(", ")));
    }
    if !inspection.observations.is_empty() {
        lines.push(String::new());
        lines.push("Observations:".to_string());
        for observation in &inspection.observations {
            lines.push(format!("- {}", observation));
        }
    }
    lines.join("\n")
}

fn indexnow_validation_text(validation: &seogeo_core::IndexNowValidation) -> String {
    let mut lines = vec![
        "IndexNow Validation".to_string(),
        String::new(),
        format!("Site: {}", validation.site_url),
        format!("Host: {}", validation.host),
        format!("Validation mode: {}", validation.validation_mode),
        format!("Key location: {}", validation.key_location),
        format!(
            "Key file: {}",
            validation.key_file_path.as_deref().unwrap_or("-")
        ),
        format!("Key file present: {}", validation.key_file_present),
        format!("Key file matches: {}", validation.key_file_matches),
    ];
    if let Some(status_code) = validation.remote_status_code {
        lines.push(format!("Remote status code: {}", status_code));
    }
    if !validation.warnings.is_empty() {
        lines.push(String::new());
        lines.push("Warnings:".to_string());
        for warning in &validation.warnings {
            lines.push(format!("- {}", warning));
        }
    }
    if !validation.errors.is_empty() {
        lines.push(String::new());
        lines.push("Errors:".to_string());
        for error in &validation.errors {
            lines.push(format!("- {}", error));
        }
    }
    lines.join("\n")
}

fn indexnow_submission_text(submission: &seogeo_core::IndexNowSubmission) -> String {
    let mut lines = vec![
        "IndexNow Submission".to_string(),
        String::new(),
        format!("Endpoint: {}", submission.endpoint),
        format!("Host: {}", submission.host),
        format!("Submitted URLs: {}", submission.submitted_urls),
        format!("Key location: {}", submission.key_location),
        format!("Status code: {}", submission.status_code),
        format!("Success: {}", submission.success),
    ];
    if let Some(body) = &submission.response_body {
        lines.push(format!("Response body: {}", body));
    }
    lines.join("\n")
}

fn indexnow_ledger_text(ledger: &IndexNowLedger) -> String {
    let mut lines = vec![
        "IndexNow Ledger".to_string(),
        String::new(),
        format!("Entries: {}", ledger.entries.len()),
    ];
    for entry in ledger.entries.iter().rev().take(10) {
        lines.push(format!(
            "- ts={} attempt={} success={} retryable={} status={} urls={} endpoint={}",
            entry.submitted_at,
            entry.attempt,
            entry.success,
            entry.retryable,
            entry
                .status_code
                .map(|value| value.to_string())
                .unwrap_or_else(|| "transport_error".to_string()),
            entry.submitted_urls,
            entry.endpoint
        ));
    }
    lines.join("\n")
}

fn indexnow_retry_text(report: &IndexNowRetryReport) -> String {
    let mut lines = vec![
        "IndexNow Retry".to_string(),
        String::new(),
        format!("Ledger: {}", report.ledger_path),
        format!("Attempted: {}", report.attempted),
        format!("Succeeded: {}", report.succeeded),
        format!("Failed: {}", report.failed),
    ];
    for entry in &report.entries {
        lines.push(format!(
            "- attempt={} success={} status={} urls={}",
            entry.attempt,
            entry.success,
            entry
                .status_code
                .map(|value| value.to_string())
                .unwrap_or_else(|| "transport_error".to_string()),
            entry.submitted_urls
        ));
    }
    lines.join("\n")
}

fn bing_ai_import_text(report: &BingAiImportReport) -> String {
    let mut lines = vec![
        "Bing AI Import".to_string(),
        String::new(),
        format!("Rows read: {}", report.rows_read),
        format!("URLs seen: {}", report.urls_seen),
        format!("Unmatched URLs: {}", report.unmatched_urls.len()),
    ];
    if !report.cited_urls.is_empty() {
        lines.push(String::new());
        lines.push("Top cited URLs:".to_string());
        for summary in report.cited_urls.iter().take(10) {
            lines.push(format!(
                "- {} citations={} rows={} findings={} errors={} warnings={}",
                summary.url,
                summary.citations,
                summary.rows,
                summary.audit_findings,
                summary.audit_errors,
                summary.audit_warnings
            ));
        }
    }
    if !report.unmatched_urls.is_empty() {
        lines.push(String::new());
        lines.push("Unmatched URLs:".to_string());
        for url in report.unmatched_urls.iter().take(10) {
            lines.push(format!("- {}", url));
        }
    }
    lines.join("\n")
}

fn bing_ai_opportunities_text(report: &BingAiOpportunityReport) -> String {
    let mut lines = vec![
        "Bing AI Opportunities".to_string(),
        String::new(),
        format!("Rows read: {}", report.rows_read),
        format!("URLs seen: {}", report.urls_seen),
        format!("Clean cited URLs: {}", report.clean_cited_urls),
        format!("Unmatched URLs: {}", report.unmatched_urls.len()),
    ];
    if !report.opportunities.is_empty() {
        lines.push(String::new());
        lines.push("Top opportunities:".to_string());
        for item in report.opportunities.iter().take(10) {
            lines.push(format!(
                "- {} score={} priority={} citations={} errors={} warnings={} findings={}",
                item.url,
                item.score,
                item.priority,
                item.citations,
                item.audit_errors,
                item.audit_warnings,
                item.audit_findings
            ));
        }
    }
    lines.join("\n")
}

fn bing_ai_snapshot_text(snapshot: &BingAiTrendSnapshot) -> String {
    let mut lines = vec![
        "Bing AI Trend Snapshot".to_string(),
        String::new(),
        format!("Imported at: {}", snapshot.imported_at),
        format!("Source: {}", snapshot.source_path),
        format!("Rows read: {}", snapshot.rows_read),
        format!("URLs seen: {}", snapshot.urls_seen),
        format!("Total citations: {}", snapshot.total_citations),
    ];
    for route in snapshot.routes.iter().take(10) {
        lines.push(format!(
            "- {} citations={} findings={} errors={} warnings={}",
            route.url,
            route.citations,
            route.audit_findings,
            route.audit_errors,
            route.audit_warnings
        ));
    }
    lines.join("\n")
}

fn bing_ai_trend_text(report: &BingAiTrendReport) -> String {
    let mut lines = vec![
        "Bing AI Trend".to_string(),
        String::new(),
        format!("Snapshots: {}", report.snapshots),
    ];
    if let Some(current) = &report.current {
        lines.push(format!("Current imported_at: {}", current.imported_at));
        lines.push(format!(
            "Current total citations: {}",
            current.total_citations
        ));
    }
    if let Some(previous) = &report.previous {
        lines.push(format!("Previous imported_at: {}", previous.imported_at));
        lines.push(format!(
            "Previous total citations: {}",
            previous.total_citations
        ));
    }
    lines.push(format!("Increased routes: {}", report.increased.len()));
    lines.push(format!("Decreased routes: {}", report.decreased.len()));
    lines.push(format!("Newly cited routes: {}", report.newly_cited.len()));
    lines.push(format!(
        "No longer cited routes: {}",
        report.no_longer_cited.len()
    ));
    for label in ["Increased", "Newly cited"] {
        let items = if label == "Increased" {
            &report.increased
        } else {
            &report.newly_cited
        };
        if !items.is_empty() {
            lines.push(String::new());
            lines.push(format!("{label}:"));
            for item in items.iter().take(10) {
                lines.push(format!(
                    "- {} delta={} current={} findings={} errors={} warnings={}",
                    item.url,
                    item.citation_delta,
                    item.current_citations,
                    item.audit_findings,
                    item.audit_errors,
                    item.audit_warnings
                ));
            }
        }
    }
    lines.join("\n")
}

fn render_search_console_csv(rows: &[SearchConsoleExportRow]) -> Result<String> {
    let mut writer = Writer::from_writer(Vec::new());
    writer.write_record([
        "route",
        "url",
        "findings",
        "errors",
        "warnings",
        "heuristic",
        "rule_groups",
        "rule_ids",
    ])?;
    for row in rows {
        writer.write_record([
            row.route.as_str(),
            row.url.as_deref().unwrap_or(""),
            &row.findings.to_string(),
            &row.errors.to_string(),
            &row.warnings.to_string(),
            &row.heuristic.to_string(),
            &row.rule_groups.join("|"),
            &row.rule_ids.join("|"),
        ])?;
    }
    Ok(String::from_utf8(writer.into_inner()?)?)
}

fn search_console_text(rows: &[SearchConsoleExportRow]) -> String {
    let mut lines = vec![
        "Search Console Export".to_string(),
        String::new(),
        format!("Rows: {}", rows.len()),
    ];
    if !rows.is_empty() {
        lines.push(String::new());
        lines.push("Top routes by findings:".to_string());
        let mut ranked = rows.to_vec();
        ranked.sort_by(|left, right| {
            right
                .findings
                .cmp(&left.findings)
                .then_with(|| left.route.cmp(&right.route))
        });
        for row in ranked.iter().take(10) {
            lines.push(format!(
                "- {} findings={} errors={} warnings={} groups={}",
                if row.route.is_empty() {
                    "/"
                } else {
                    &row.route
                },
                row.findings,
                row.errors,
                row.warnings,
                row.rule_groups.join(",")
            ));
        }
    }
    lines.join("\n")
}

fn publish_hook_text(report: &PublishHookReport) -> String {
    let mut lines = vec![
        "Publish Hook Report".to_string(),
        String::new(),
        format!("Changed routes: {}", report.changed_routes.len()),
        format!("Finding count: {}", report.finding_count),
        format!("Audit artifact: {}", report.audit_path),
        format!(
            "Search Console export: {}",
            report.search_console_export_path
        ),
    ];
    if !report.findings_by_route.is_empty() {
        lines.push(String::new());
        lines.push("Findings by route:".to_string());
        for (route, count) in &report.findings_by_route {
            lines.push(format!(
                "- {} {}",
                if route.is_empty() { "/" } else { route },
                count
            ));
        }
    }
    if let Some(validation) = &report.indexnow_validation {
        lines.push(String::new());
        lines.push(format!(
            "IndexNow validation: {}",
            if validation.errors.is_empty() {
                "ok"
            } else {
                "failed"
            }
        ));
    }
    if let Some(submission) = &report.indexnow_submission {
        lines.push(format!(
            "IndexNow submission: status={} success={}",
            submission.status_code, submission.success
        ));
    }
    if let Some(ledger_path) = &report.indexnow_ledger_path {
        lines.push(format!("IndexNow ledger: {}", ledger_path));
    }
    lines.join("\n")
}

fn snippet_route_arg(submatches: &ArgMatches) -> &str {
    submatches
        .get_one::<String>("route")
        .map(String::as_str)
        .unwrap_or("")
}

fn command_snippet_inspect(submatches: &ArgMatches) -> Result<i32> {
    let format = required_arg(submatches, "format")?;
    let from_url = submatches.get_one::<String>("url").map(String::as_str);
    let from_path = submatches.get_one::<String>("path").map(String::as_str);
    let route = snippet_route_arg(submatches);
    let result = match (from_url, from_path) {
        (Some(url), None) => inspect_snippet_controls_url(url),
        (None, Some(path)) => inspect_snippet_controls_path(
            &canonicalize_or_keep(path),
            submatches.get_one::<String>("config").map(Path::new),
            route,
        ),
        (Some(_), Some(_)) => bail!("choose either --url or --path"),
        (None, None) => bail!("one of --url or --path is required"),
    };
    let inspection = match result {
        Ok(inspection) => inspection,
        Err(error) => return emit_integration_failure("snippet inspect", format, error),
    };
    match format {
        "json" => println!(
            "{}",
            render_data_command_json("snippet inspect", true, inspection, Vec::new())?
        ),
        _ => println!("{}", snippet_text(&inspection)),
    }
    Ok(0)
}

fn command_indexnow_validate(submatches: &ArgMatches) -> Result<i32> {
    let format = required_arg(submatches, "format")?;
    let root = submatches
        .get_one::<String>("path")
        .map(|path| canonicalize_or_keep(path));
    let validation = match validate_indexnow(
        required_arg(submatches, "site_url")?,
        required_arg(submatches, "key")?,
        root.as_deref(),
    ) {
        Ok(validation) => validation,
        Err(error) => return emit_integration_failure("indexnow validate", format, error),
    };
    let success = validation.errors.is_empty();
    match format {
        "json" => println!(
            "{}",
            render_data_command_json("indexnow validate", success, validation.clone(), Vec::new())?
        ),
        _ => println!("{}", indexnow_validation_text(&validation)),
    }
    Ok(if success { 0 } else { 1 })
}

fn command_indexnow_submit(submatches: &ArgMatches) -> Result<i32> {
    let format = required_arg(submatches, "format")?;
    let urls = submatches
        .get_many::<String>("url")
        .ok_or_else(|| anyhow!("missing required CLI argument 'url'"))?
        .cloned()
        .collect::<Vec<_>>();
    let root = submatches
        .get_one::<String>("path")
        .map(|path| canonicalize_or_keep(path));
    let submission_result = match root.as_deref() {
        Some(root) => submit_indexnow_with_ledger(
            root,
            required_arg(submatches, "endpoint")?,
            required_arg(submatches, "site_url")?,
            required_arg(submatches, "key")?,
            &urls,
        )
        .map(|entry| seogeo_core::IndexNowSubmission {
            endpoint: entry.endpoint,
            host: entry.host,
            submitted_urls: entry.submitted_urls,
            key_location: entry.key_location,
            status_code: entry.status_code.unwrap_or_default(),
            success: entry.success,
            response_body: entry.response_body.or(entry.error),
        }),
        None => submit_indexnow(
            required_arg(submatches, "endpoint")?,
            required_arg(submatches, "site_url")?,
            required_arg(submatches, "key")?,
            &urls,
        ),
    };
    let submission = match submission_result {
        Ok(submission) => submission,
        Err(error) => return emit_integration_failure("indexnow submit", format, error),
    };
    let success = submission.success;
    match format {
        "json" => println!(
            "{}",
            render_data_command_json("indexnow submit", success, submission.clone(), Vec::new())?
        ),
        _ => println!("{}", indexnow_submission_text(&submission)),
    }
    Ok(if success { 0 } else { 1 })
}

fn command_indexnow_ledger(submatches: &ArgMatches) -> Result<i32> {
    let root = canonicalize_or_keep(required_arg(submatches, "path")?);
    let format = required_arg(submatches, "format")?;
    let ledger = match load_indexnow_ledger(&root) {
        Ok(ledger) => ledger,
        Err(error) => return emit_integration_failure("indexnow ledger", format, error),
    };
    match format {
        "json" => println!(
            "{}",
            render_data_command_json("indexnow ledger", true, ledger.clone(), Vec::new())?
        ),
        _ => println!("{}", indexnow_ledger_text(&ledger)),
    }
    Ok(0)
}

fn command_indexnow_retry(submatches: &ArgMatches) -> Result<i32> {
    let root = canonicalize_or_keep(required_arg(submatches, "path")?);
    let format = required_arg(submatches, "format")?;
    let limit = *submatches.get_one::<usize>("limit").unwrap_or(&10);
    let report = match retry_indexnow_submissions(&root, required_arg(submatches, "key")?, limit) {
        Ok(report) => report,
        Err(error) => return emit_integration_failure("indexnow retry", format, error),
    };
    let success = report.failed == 0;
    match format {
        "json" => println!(
            "{}",
            render_data_command_json("indexnow retry", success, report.clone(), Vec::new())?
        ),
        _ => println!("{}", indexnow_retry_text(&report)),
    }
    Ok(if success { 0 } else { 1 })
}

fn command_bing_ai_import(submatches: &ArgMatches) -> Result<i32> {
    let format = required_arg(submatches, "format")?;
    let audit_path = submatches.get_one::<String>("audit").map(Path::new);
    let report =
        match import_bing_ai_export(Path::new(required_arg(submatches, "path")?), audit_path) {
            Ok(report) => report,
            Err(error) => return emit_integration_failure("bing-ai import", format, error),
        };
    match format {
        "json" => println!(
            "{}",
            render_data_command_json("bing-ai import", true, report.clone(), Vec::new())?
        ),
        _ => println!("{}", bing_ai_import_text(&report)),
    }
    Ok(0)
}

fn command_bing_ai_opportunities(submatches: &ArgMatches) -> Result<i32> {
    let format = required_arg(submatches, "format")?;
    let report = match build_bing_ai_opportunity_report(
        Path::new(required_arg(submatches, "path")?),
        Path::new(required_arg(submatches, "audit")?),
    ) {
        Ok(report) => report,
        Err(error) => return emit_integration_failure("bing-ai opportunities", format, error),
    };
    match format {
        "json" => println!(
            "{}",
            render_data_command_json("bing-ai opportunities", true, report.clone(), Vec::new())?
        ),
        _ => println!("{}", bing_ai_opportunities_text(&report)),
    }
    Ok(0)
}

fn command_bing_ai_trend_import(submatches: &ArgMatches) -> Result<i32> {
    let root = canonicalize_or_keep(required_arg(submatches, "root")?);
    let format = required_arg(submatches, "format")?;
    let snapshot = match record_bing_ai_trend(
        &root,
        Path::new(required_arg(submatches, "path")?),
        submatches.get_one::<String>("audit").map(Path::new),
    ) {
        Ok(snapshot) => snapshot,
        Err(error) => return emit_integration_failure("bing-ai trend import", format, error),
    };
    match format {
        "json" => println!(
            "{}",
            render_data_command_json("bing-ai trend import", true, snapshot.clone(), Vec::new())?
        ),
        _ => println!("{}", bing_ai_snapshot_text(&snapshot)),
    }
    Ok(0)
}

fn command_bing_ai_trend_show(submatches: &ArgMatches) -> Result<i32> {
    let root = canonicalize_or_keep(required_arg(submatches, "path")?);
    let format = required_arg(submatches, "format")?;
    let report = match build_bing_ai_trend_report(&root) {
        Ok(report) => report,
        Err(error) => return emit_integration_failure("bing-ai trend show", format, error),
    };
    match format {
        "json" => println!(
            "{}",
            render_data_command_json("bing-ai trend show", true, report.clone(), Vec::new())?
        ),
        _ => println!("{}", bing_ai_trend_text(&report)),
    }
    Ok(0)
}

fn command_search_console_export(submatches: &ArgMatches) -> Result<i32> {
    let format = required_arg(submatches, "format")?;
    let rows = match export_search_console_rows(
        Path::new(required_arg(submatches, "audit")?),
        submatches.get_one::<String>("site_url").map(String::as_str),
    ) {
        Ok(rows) => rows,
        Err(error) => return emit_integration_failure("search-console export", format, error),
    };
    match format {
        "json" => println!(
            "{}",
            render_data_command_json("search-console export", true, rows.clone(), Vec::new())?
        ),
        "csv" => println!("{}", render_search_console_csv(&rows)?),
        _ => println!("{}", search_console_text(&rows)),
    }
    Ok(0)
}

fn command_publish_hook_run(submatches: &ArgMatches) -> Result<i32> {
    let root = canonicalize_or_keep(required_arg(submatches, "path")?);
    let format = required_arg(submatches, "format")?;
    let explicit_config = submatches.get_one::<String>("config").map(PathBuf::from);
    let loaded = match load_config_with_diagnostics(&root, explicit_config.as_deref()) {
        Ok(loaded) => loaded,
        Err(error) => return emit_integration_failure("publish-hook run", format, error),
    };
    let changed_urls = submatches
        .get_many::<String>("changed-url")
        .map(|values| values.cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    let report = match build_publish_hook_report_with_config(
        &root,
        &loaded.config,
        &changed_urls,
        submatches
            .get_one::<String>("indexnow-key")
            .map(String::as_str),
        submatches.get_flag("submit-indexnow"),
        submatches
            .get_one::<String>("indexnow-endpoint")
            .map(String::as_str)
            .unwrap_or("https://api.indexnow.org/indexnow"),
    ) {
        Ok(report) => report,
        Err(error) => return emit_integration_failure("publish-hook run", format, error),
    };
    let success = report
        .indexnow_validation
        .as_ref()
        .is_none_or(|validation| validation.errors.is_empty())
        && report
            .indexnow_submission
            .as_ref()
            .is_none_or(|submission| submission.success);
    match format {
        "json" => println!(
            "{}",
            render_data_command_json("publish-hook run", success, report.clone(), loaded.warnings)?
        ),
        _ => {
            emit_config_warnings(&loaded.warnings);
            println!("{}", publish_hook_text(&report));
        }
    }
    Ok(if success { 0 } else { 1 })
}

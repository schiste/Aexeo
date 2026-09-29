use aexeo_core::config::load_config_with_diagnostics;
use aexeo_core::{
    BingAiImportReport, BingAiOpportunityReport, BingAiTrendReport, BingAiTrendSnapshot,
    IndexNowLedger, IndexNowRetryReport, PublishHookReport, SearchConsoleExportRow,
    SnippetInspection, build_bing_ai_opportunity_report, build_bing_ai_trend_report,
    build_publish_hook_report_with_config, export_search_console_rows, import_bing_ai_export,
    inspect_snippet_controls_path, inspect_snippet_controls_url, load_indexnow_ledger,
    plan_indexnow_submission, record_bing_ai_trend, retry_indexnow_submissions, submit_indexnow,
    submit_indexnow_with_ledger, validate_indexnow,
};
use anyhow::{Result, anyhow, bail};
use clap::ArgMatches;
use csv::Writer;
use std::path::{Path, PathBuf};

use super::exit_code::{EXIT_FINDINGS, EXIT_SUCCESS};
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
        Some(("plan", plan_matches)) => command_indexnow_plan(plan_matches),
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
    Ok(EXIT_FINDINGS)
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

fn indexnow_validation_text(validation: &aexeo_core::IndexNowValidation) -> String {
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

fn indexnow_submission_text(submission: &aexeo_core::IndexNowSubmission) -> String {
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

fn indexnow_plan_text(plan: &aexeo_core::IndexNowPlan) -> String {
    let mut lines = vec![
        "IndexNow Plan".to_string(),
        String::new(),
        format!("Endpoint: {}", plan.endpoint),
        format!("Site: {}", plan.site_url),
        format!("Host: {}", plan.host),
        format!("Key location: {}", plan.key_location),
        format!("URLs: {}", plan.submitted_urls),
        format!("Can submit: {}", plan.can_submit),
    ];
    if !plan.warnings.is_empty() {
        lines.push(String::new());
        lines.push("Warnings:".to_string());
        for warning in &plan.warnings {
            lines.push(format!("- {}", warning));
        }
    }
    if !plan.errors.is_empty() {
        lines.push(String::new());
        lines.push("Errors:".to_string());
        for error in &plan.errors {
            lines.push(format!("- {}", error));
        }
    }
    if !plan.notes.is_empty() {
        lines.push(String::new());
        lines.push("Notes:".to_string());
        for note in &plan.notes {
            lines.push(format!("- {}", note));
        }
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
    Ok(EXIT_SUCCESS)
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

fn command_indexnow_plan(submatches: &ArgMatches) -> Result<i32> {
    let format = required_arg(submatches, "format")?;
    let urls = submatches
        .get_many::<String>("url")
        .ok_or_else(|| anyhow!("missing required CLI argument 'url'"))?
        .cloned()
        .collect::<Vec<_>>();
    let root = submatches
        .get_one::<String>("path")
        .map(|path| canonicalize_or_keep(path));
    let plan = match plan_indexnow_submission(
        required_arg(submatches, "endpoint")?,
        required_arg(submatches, "site_url")?,
        required_arg(submatches, "key")?,
        root.as_deref(),
        &urls,
    ) {
        Ok(plan) => plan,
        Err(error) => return emit_integration_failure("indexnow plan", format, error),
    };
    let success = plan.can_submit;
    match format {
        "json" => println!(
            "{}",
            render_data_command_json("indexnow plan", success, plan.clone(), Vec::new())?
        ),
        _ => println!("{}", indexnow_plan_text(&plan)),
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
        .map(|entry| aexeo_core::IndexNowSubmission {
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
    Ok(EXIT_SUCCESS)
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
    Ok(EXIT_SUCCESS)
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
    Ok(EXIT_SUCCESS)
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
    Ok(EXIT_SUCCESS)
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
    Ok(EXIT_SUCCESS)
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
    Ok(EXIT_SUCCESS)
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

#[cfg(test)]
mod tests {
    use super::*;
    use aexeo_core::{
        BingAiOpportunity, BingAiTrendDelta, BingAiTrendRoute, BingAiUrlSummary,
        IndexNowLedgerEntry, IndexNowPlan, IndexNowSubmission, IndexNowValidation,
    };

    // --- render_search_console_csv --------------------------------------

    fn sc_row(route: &str, findings: usize) -> SearchConsoleExportRow {
        SearchConsoleExportRow {
            route: route.to_string(),
            url: Some(format!("https://example.com/{route}")),
            findings,
            errors: findings / 2,
            warnings: findings - findings / 2,
            heuristic: 0,
            rule_groups: vec!["html".to_string(), "seo".to_string()],
            rule_ids: vec!["SEO001".to_string()],
        }
    }

    /// The CSV is consumed by a spreadsheet, so it needs a header row and the
    /// list-valued columns joined with a separator that a cell can hold.
    #[test]
    fn search_console_csv_has_a_header_and_pipe_joined_lists() {
        let csv = render_search_console_csv(&[sc_row("pricing", 4)]).expect("csv renders");
        let mut lines = csv.lines();
        assert_eq!(
            lines.next(),
            Some("route,url,findings,errors,warnings,heuristic,rule_groups,rule_ids")
        );
        let row = lines.next().expect("one data row");
        assert!(row.contains("https://example.com/pricing"), "{csv}");
        assert!(
            row.contains("html|seo"),
            "rule_groups must be pipe-joined: {csv}"
        );
        assert!(row.contains("SEO001"), "{csv}");
        assert_eq!(lines.next(), None, "only one data row expected");
    }

    /// A route with no URL still needs a row — an empty cell, not a dropped
    /// record, or the row count stops matching the findings count.
    #[test]
    fn search_console_csv_emits_an_empty_cell_for_a_missing_url() {
        let mut row = sc_row("pricing", 1);
        row.url = None;
        let csv = render_search_console_csv(&[row]).expect("csv renders");
        let data = csv.lines().nth(1).expect("data row");
        assert!(data.starts_with("pricing,"), "{csv}");
    }

    /// Values containing a comma or a quote would break the column alignment
    /// if they were not quoted, so this pins that `csv::Writer` is doing the
    /// escaping.
    #[test]
    fn search_console_csv_quotes_values_containing_delimiters() {
        let mut row = sc_row("pricing", 1);
        row.rule_ids = vec!["SEO001,with,commas".to_string()];
        let csv = render_search_console_csv(&[row]).expect("csv renders");
        assert!(
            csv.contains("\"SEO001,with,commas\""),
            "an unquoted comma would shift every later column: {csv}"
        );
    }

    /// An empty export is a header and nothing else — not zero bytes, which a
    /// spreadsheet would read as an empty file.
    #[test]
    fn search_console_csv_of_no_rows_is_still_a_header() {
        let csv = render_search_console_csv(&[]).expect("csv renders");
        assert_eq!(
            csv.lines().count(),
            1,
            "expected only the header, got: {csv}"
        );
    }

    // --- search_console_text --------------------------------------------

    /// Routes are ranked by findings descending so the worst offenders lead,
    /// with route name as a stable tiebreak so equal counts do not reorder
    /// between runs.
    #[test]
    fn search_console_text_ranks_by_findings_then_route() {
        let rows = vec![sc_row("zeta", 1), sc_row("alpha", 5), sc_row("beta", 5)];
        let text = search_console_text(&rows);

        let alpha = text.find("- alpha").expect("alpha row");
        let beta = text.find("- beta").expect("beta row");
        let zeta = text.find("- zeta").expect("zeta row");
        assert!(alpha < beta && beta < zeta, "wrong ranking order: {text}");
    }

    /// The home route is stored as an empty string; it must render as `/` so
    /// it is clickable rather than looking like a formatting bug.
    #[test]
    fn search_console_text_renders_the_empty_route_as_root() {
        let mut row = sc_row("", 3);
        row.url = None;
        let text = search_console_text(&[row]);
        assert!(text.contains("- / findings=3"), "{text}");
    }

    #[test]
    fn search_console_text_omits_the_ranking_for_an_empty_export() {
        let text = search_console_text(&[]);
        assert!(text.contains("Rows: 0"), "{text}");
        assert!(!text.contains("Top routes by findings:"), "{text}");
    }

    // --- snippet_text ----------------------------------------------------

    fn inspection() -> SnippetInspection {
        serde_json::from_value(serde_json::json!({
            "target": "https://example.com/pricing",
            "route": "pricing",
            "canonical": "https://example.com/pricing",
            "meta_robots": "noindex",
            "x_robots_tag": null,
            "directives": ["max-snippet:-1"],
            "data_nosnippet_blocks": 2,
            "snippet_blocked": true,
            "restrictive_max_snippet": "-1",
            "observations": ["x-robots-tag not sent by origin"],
        }))
        .expect("snippet fixture")
    }

    /// Every snippet control that can suppress a preview must appear, since a
    /// missing one is exactly what the operator is looking for.
    #[test]
    fn snippet_text_reports_every_snippet_control() {
        let text = snippet_text(&inspection());
        assert!(text.contains("Snippet blocked: yes"), "{text}");
        assert!(text.contains("Meta robots: noindex"), "{text}");
        assert!(
            text.contains("Canonical: https://example.com/pricing"),
            "{text}"
        );
        assert!(text.contains("data-nosnippet blocks: 2"), "{text}");
        assert!(text.contains("Directives: max-snippet:-1"), "{text}");
        assert!(text.contains("Observations:"), "{text}");
    }

    /// An absent header renders as `-` so the reader can tell "not sent" from
    /// an empty value, and the blocked flag still reads `no`.
    #[test]
    fn snippet_text_marks_absent_headers_with_a_dash() {
        let mut insp = inspection();
        insp.x_robots_tag = None;
        insp.snippet_blocked = false;
        insp.canonical = None;
        insp.directives.clear();
        insp.observations.clear();
        let text = snippet_text(&insp);
        assert!(text.contains("X-Robots-Tag: -"), "{text}");
        assert!(text.contains("Canonical: -"), "{text}");
        assert!(text.contains("Snippet blocked: no"), "{text}");
        // Empty collections must not print dangling headings.
        assert!(!text.contains("Directives:"), "{text}");
        assert!(!text.contains("Observations:"), "{text}");
    }

    // --- indexnow_validation_text ---------------------------------------

    fn validation() -> IndexNowValidation {
        serde_json::from_value(serde_json::json!({
            "site_url": "https://example.com",
            "host": "example.com",
            "key": "abc123",
            "key_location": "https://example.com/aexeo-indexnow-key.txt",
            "validation_mode": "local",
            "key_file_path": "/site/aexeo-indexnow-key.txt",
            "key_file_present": true,
            "key_file_matches": true,
            "remote_status_code": 200,
            "errors": [],
            "warnings": ["remote check skipped in local mode"],
        }))
        .expect("indexnow validation fixture")
    }

    /// The validation report is what decides whether a submission can go out,
    /// so the key file's presence and match state are the two load-bearing
    /// facts and both must print.
    #[test]
    fn indexnow_validation_text_reports_key_file_state() {
        let text = indexnow_validation_text(&validation());
        assert!(text.contains("Key file present: true"), "{text}");
        assert!(text.contains("Key file matches: true"), "{text}");
        assert!(text.contains("Remote status code: 200"), "{text}");
        assert!(text.contains("Validation mode: local"), "{text}");
        assert!(
            text.contains("remote check skipped in local mode"),
            "{text}"
        );
    }

    /// A local-mode validation performs no remote request, so there is no
    /// status code to print — the line must be absent, not zero, which would
    /// read as a failing HTTP response.
    #[test]
    fn indexnow_validation_text_omits_the_status_line_when_no_request_was_made() {
        let mut v = validation();
        v.remote_status_code = None;
        v.key_file_path = None;
        let text = indexnow_validation_text(&v);
        assert!(!text.contains("Remote status code:"), "{text}");
        assert!(text.contains("Key file: -"), "{text}");
    }

    /// Errors are printed as their own section, distinct from warnings, so a
    /// blocking failure is never filed under an advisory heading.
    #[test]
    fn indexnow_validation_text_separates_errors_from_warnings() {
        let mut v = validation();
        v.errors = vec!["key file content does not match".to_string()];
        let text = indexnow_validation_text(&v);
        assert!(text.contains("Warnings:"), "{text}");
        assert!(text.contains("Errors:"), "{text}");
        let warnings = text.find("Warnings:").expect("warnings");
        let errors = text.find("Errors:").expect("errors");
        assert!(warnings < errors, "errors should follow warnings: {text}");
    }

    // --- indexnow_submission_text ---------------------------------------

    #[test]
    fn indexnow_submission_text_reports_status_and_optional_body() {
        let submission = IndexNowSubmission {
            endpoint: "https://api.indexnow.org/indexnow".to_string(),
            host: "example.com".to_string(),
            submitted_urls: 3,
            key_location: "https://example.com/key.txt".to_string(),
            status_code: 202,
            success: true,
            response_body: Some("accepted".to_string()),
        };
        let text = indexnow_submission_text(&submission);
        assert!(text.contains("Status code: 202"), "{text}");
        assert!(text.contains("Success: true"), "{text}");
        assert!(text.contains("Response body: accepted"), "{text}");
    }

    #[test]
    fn indexnow_submission_text_omits_an_absent_response_body() {
        let submission = IndexNowSubmission {
            endpoint: "https://api.indexnow.org/indexnow".to_string(),
            host: "example.com".to_string(),
            submitted_urls: 0,
            key_location: "https://example.com/key.txt".to_string(),
            status_code: 422,
            success: false,
            response_body: None,
        };
        let text = indexnow_submission_text(&submission);
        assert!(!text.contains("Response body:"), "{text}");
    }

    // --- indexnow_plan_text ---------------------------------------------

    fn plan(can_submit: bool) -> IndexNowPlan {
        serde_json::from_value(serde_json::json!({
            "endpoint": "https://api.indexnow.org/indexnow",
            "site_url": "https://example.com",
            "host": "example.com",
            "key_location": "https://example.com/key.txt",
            "validation": validation(),
            "urls": ["https://example.com/a", "https://example.com/b"],
            "submitted_urls": 2,
            "can_submit": can_submit,
            "errors": if can_submit { vec![] } else { vec!["key file missing".to_string()] },
            "warnings": ["2 URLs exceed the 10,000 batch limit is false".to_string()],
            "notes": ["batch mode not requested".to_string()],
        }))
        .expect("indexnow plan fixture")
    }

    /// The plan is a dry run, so `can_submit` and the URL count are the two
    /// things being asked about; notes and warnings are supporting detail.
    #[test]
    fn indexnow_plan_text_reports_can_submit_and_counts() {
        let text = indexnow_plan_text(&plan(true));
        assert!(text.contains("URLs: 2"), "{text}");
        assert!(text.contains("Can submit: true"), "{text}");
        assert!(text.contains("Notes:"), "{text}");
        assert!(text.contains("batch mode not requested"), "{text}");
    }

    /// A plan that cannot be submitted must state why in an errors section.
    #[test]
    fn indexnow_plan_text_surfaces_blocking_errors() {
        let text = indexnow_plan_text(&plan(false));
        assert!(text.contains("Can submit: false"), "{text}");
        assert!(text.contains("Errors:"), "{text}");
        assert!(text.contains("key file missing"), "{text}");
    }

    /// A clean plan must not print empty Warnings/Errors/Notes headings.
    #[test]
    fn indexnow_plan_text_omits_empty_sections() {
        let mut p = plan(true);
        p.warnings.clear();
        p.errors.clear();
        p.notes.clear();
        let text = indexnow_plan_text(&p);
        assert!(!text.contains("Warnings:"), "{text}");
        assert!(!text.contains("Errors:"), "{text}");
        assert!(!text.contains("Notes:"), "{text}");
    }

    // --- indexnow_ledger_text / retry_text ------------------------------

    fn ledger_entry(submitted_at: u64, status_code: Option<u16>) -> IndexNowLedgerEntry {
        serde_json::from_value(serde_json::json!({
            "submitted_at": submitted_at,
            "attempt": 1,
            "endpoint": "https://api.indexnow.org/indexnow",
            "site_url": "https://example.com",
            "host": "example.com",
            "key_location": "https://example.com/key.txt",
            "urls": ["https://example.com/a"],
            "submitted_urls": 1,
            "status_code": status_code,
            "success": status_code.map(|code| code < 300).unwrap_or(false),
            "retryable": true,
            "response_body": null,
            "error": null,
        }))
        .expect("ledger entry fixture")
    }

    /// The ledger is an append-only history, so it renders newest-first —
    /// the entry someone wants is the most recent one.
    #[test]
    fn indexnow_ledger_text_lists_the_newest_entries_first() {
        let ledger = IndexNowLedger {
            entries: vec![ledger_entry(100, Some(202)), ledger_entry(200, Some(200))],
        };
        let text = indexnow_ledger_text(&ledger);
        let older = text.find("ts=100").expect("older entry");
        let newer = text.find("ts=200").expect("newer entry");
        assert!(newer < older, "the ledger must read newest-first: {text}");
    }

    /// A submission that never got an HTTP response has no status code. The
    /// ledger records `transport_error` so it is distinguishable from a real
    /// HTTP failure rather than showing a blank.
    #[test]
    fn indexnow_ledger_text_labels_a_missing_status_code_as_a_transport_error() {
        let ledger = IndexNowLedger {
            entries: vec![ledger_entry(100, None)],
        };
        let text = indexnow_ledger_text(&ledger);
        assert!(text.contains("status=transport_error"), "{text}");
        assert!(!text.contains("status=0"), "{text}");
    }

    #[test]
    fn indexnow_retry_text_summarises_the_attempt_counts() {
        let report = IndexNowRetryReport {
            ledger_path: "/site/.aexeo-reports/indexnow-ledger.json".to_string(),
            attempted: 2,
            succeeded: 1,
            failed: 1,
            entries: vec![ledger_entry(100, Some(200)), ledger_entry(200, None)],
        };
        let text = indexnow_retry_text(&report);
        assert!(text.contains("Attempted: 2"), "{text}");
        assert!(text.contains("Succeeded: 1"), "{text}");
        assert!(text.contains("Failed: 1"), "{text}");
        assert!(text.contains("indexnow-ledger.json"), "{text}");
        assert!(text.contains("status=transport_error"), "{text}");
    }

    // --- bing ai --------------------------------------------------------

    #[test]
    fn bing_ai_import_text_lists_cited_and_unmatched_urls() {
        let report = BingAiImportReport {
            rows_read: 100,
            urls_seen: 4,
            cited_urls: vec![BingAiUrlSummary {
                url: "https://example.com/pricing".to_string(),
                route: "pricing".to_string(),
                citations: 12,
                rows: 30,
                audit_findings: 2,
                audit_errors: 1,
                audit_warnings: 1,
            }],
            unmatched_urls: vec!["https://other.example/x".to_string()],
        };
        let text = bing_ai_import_text(&report);
        assert!(text.contains("Rows read: 100"), "{text}");
        assert!(text.contains("Top cited URLs:"), "{text}");
        assert!(text.contains("citations=12"), "{text}");
        assert!(text.contains("Unmatched URLs:"), "{text}");
        assert!(text.contains("https://other.example/x"), "{text}");
    }

    #[test]
    fn bing_ai_import_text_omits_empty_sections() {
        let report = BingAiImportReport {
            rows_read: 0,
            urls_seen: 0,
            cited_urls: vec![],
            unmatched_urls: vec![],
        };
        let text = bing_ai_import_text(&report);
        assert!(!text.contains("Top cited URLs:"), "{text}");
        // `Unmatched URLs: 0` is the always-present counter, so asserting on
        // the heading alone would match the count line. What must not appear
        // is the *section*: a blank line followed by the heading.
        assert!(!text.contains("\n\nUnmatched URLs:\n"), "{text}");
    }

    #[test]
    fn bing_ai_opportunities_text_reports_score_and_priority() {
        let report = BingAiOpportunityReport {
            rows_read: 50,
            urls_seen: 2,
            clean_cited_urls: 1,
            opportunities: vec![BingAiOpportunity {
                url: "https://example.com/pricing".to_string(),
                route: "pricing".to_string(),
                citations: 5,
                audit_findings: 3,
                audit_errors: 2,
                audit_warnings: 1,
                score: 90,
                priority: "high".to_string(),
                rationale: vec!["cited with errors".to_string()],
            }],
            unmatched_urls: vec![],
        };
        let text = bing_ai_opportunities_text(&report);
        assert!(text.contains("Clean cited URLs: 1"), "{text}");
        assert!(text.contains("Top opportunities:"), "{text}");
        assert!(text.contains("score=90"), "{text}");
        assert!(text.contains("priority=high"), "{text}");
    }

    /// The trend report is only meaningful when there is a previous snapshot
    /// to compare against. With one snapshot the counters must still print,
    /// but no route may appear under Increased or Newly cited, because a
    /// delta against nothing is not an increase.
    #[test]
    fn bing_ai_trend_text_with_a_single_snapshot_reports_no_deltas() {
        let snapshot = BingAiTrendSnapshot {
            imported_at: 200,
            source_path: "/site/trends.json".to_string(),
            audit_path: None,
            rows_read: 10,
            urls_seen: 1,
            total_citations: 7,
            routes: vec![BingAiTrendRoute {
                url: "https://example.com/a".to_string(),
                route: "a".to_string(),
                citations: 7,
                audit_findings: 0,
                audit_errors: 0,
                audit_warnings: 0,
            }],
        };
        let report = BingAiTrendReport {
            snapshots: 1,
            current: Some(snapshot.clone()),
            previous: None,
            increased: vec![],
            decreased: vec![],
            newly_cited: vec![],
            no_longer_cited: vec![],
        };
        let text = bing_ai_trend_text(&report);
        assert!(text.contains("Snapshots: 1"), "{text}");
        assert!(text.contains("Current total citations: 7"), "{text}");
        assert!(text.contains("Increased routes: 0"), "{text}");
        assert!(text.contains("Newly cited routes: 0"), "{text}");
        assert!(!text.contains("Previous total citations"), "{text}");
        // No delta sections when there is nothing to compare.
        assert!(!text.contains("Increased:"), "{text}");
        assert!(!text.contains("Newly cited:"), "{text}");
    }

    /// A real delta must render with both the signed change and the absolute
    /// current count, and negative deltas must not be mistaken for increases.
    #[test]
    fn bing_ai_trend_text_renders_signed_deltas() {
        let snapshot = |at: u64, citations: u64| BingAiTrendSnapshot {
            imported_at: at,
            source_path: "/site/trends.json".to_string(),
            audit_path: None,
            rows_read: 10,
            urls_seen: 1,
            total_citations: citations,
            routes: vec![],
        };
        let delta = |url: &str, d: i64| BingAiTrendDelta {
            url: url.to_string(),
            route: url.to_string(),
            previous_citations: 0,
            current_citations: 0,
            citation_delta: d,
            audit_findings: 0,
            audit_errors: 0,
            audit_warnings: 0,
        };
        let report = BingAiTrendReport {
            snapshots: 2,
            current: Some(snapshot(200, 20)),
            previous: Some(snapshot(100, 10)),
            increased: vec![delta("https://example.com/a", 5)],
            decreased: vec![delta("https://example.com/b", -3)],
            newly_cited: vec![delta("https://example.com/c", 2)],
            no_longer_cited: vec![],
        };
        let text = bing_ai_trend_text(&report);
        assert!(text.contains("Previous total citations: 10"), "{text}");
        assert!(text.contains("Increased routes: 1"), "{text}");
        assert!(text.contains("Decreased routes: 1"), "{text}");
        assert!(text.contains("Newly cited routes: 1"), "{text}");
        assert!(text.contains("No longer cited routes: 0"), "{text}");
        // Only the two positive-momentum sections get detail listings — the
        // renderer deliberately does not break out the declining routes, so
        // a loss is visible as a count and not as a route you can act on.
        assert!(text.contains("Increased:"), "{text}");
        assert!(text.contains("delta=5"), "{text}");
        assert!(text.contains("Newly cited:"), "{text}");
        assert!(text.contains("delta=2"), "{text}");
        assert!(
            !text.contains("https://example.com/b"),
            "decreased routes are counted but not listed: {text}"
        );
    }

    #[test]
    fn bing_ai_snapshot_text_lists_routes() {
        let snapshot = BingAiTrendSnapshot {
            imported_at: 42,
            source_path: "/site/trends.json".to_string(),
            audit_path: None,
            rows_read: 3,
            urls_seen: 1,
            total_citations: 9,
            routes: vec![BingAiTrendRoute {
                url: "https://example.com/a".to_string(),
                route: "a".to_string(),
                citations: 9,
                audit_findings: 1,
                audit_errors: 1,
                audit_warnings: 0,
            }],
        };
        let text = bing_ai_snapshot_text(&snapshot);
        assert!(text.contains("Imported at: 42"), "{text}");
        assert!(text.contains("https://example.com/a"), "{text}");
        assert!(text.contains("citations=9"), "{text}");
    }

    // --- publish_hook_text ----------------------------------------------

    fn publish_report() -> PublishHookReport {
        serde_json::from_value(serde_json::json!({
            "changed_routes": ["pricing", "about"],
            "finding_count": 5,
            "findings_by_route": {"": 1, "pricing": 4},
            "audit_path": "/site/.aexeo-reports/audit.json",
            "search_console_export_path": "/site/.aexeo-reports/search-console.csv",
            "indexnow_ledger_path": "/site/.aexeo-reports/indexnow-ledger.json",
            "search_console_rows": [],
            "indexnow_validation": validation(),
            "indexnow_submission": {
                "endpoint": "https://api.indexnow.org/indexnow",
                "host": "example.com",
                "submitted_urls": 2,
                "key_location": "https://example.com/key.txt",
                "status_code": 202,
                "success": true,
                "response_body": null,
            },
        }))
        .expect("publish hook report fixture")
    }

    /// The publish-hook report is the summary an operator reads after a deploy,
    /// so the changed-route count, the finding total, and where the artifacts
    /// landed all need to be present.
    #[test]
    fn publish_hook_text_summarises_the_deploy() {
        let text = publish_hook_text(&publish_report());
        assert!(text.contains("Changed routes: 2"), "{text}");
        assert!(text.contains("Finding count: 5"), "{text}");
        assert!(text.contains("search-console.csv"), "{text}");
        assert!(text.contains("IndexNow validation: ok"), "{text}");
        assert!(text.contains("IndexNow ledger:"), "{text}");
    }

    /// The home route is stored as an empty string and must render as `/`
    /// alongside the named routes rather than as a blank bullet.
    #[test]
    fn publish_hook_text_renders_the_empty_route_as_root() {
        let text = publish_hook_text(&publish_report());
        assert!(text.contains("- / 1"), "{text}");
        assert!(text.contains("- pricing 4"), "{text}");
    }

    /// A validation with errors must read `failed`, not `ok` — this is the
    /// line that tells an operator whether the IndexNow push actually ran.
    #[test]
    fn publish_hook_text_reports_a_failed_indexnow_validation() {
        let mut report = publish_report();
        let mut v = validation();
        v.errors = vec!["key mismatch".to_string()];
        report.indexnow_validation = Some(v);
        let text = publish_hook_text(&report);
        assert!(text.contains("IndexNow validation: failed"), "{text}");
        assert!(!text.contains("IndexNow validation: ok"), "{text}");
    }

    /// With no IndexNow step configured at all, the validation/submission
    /// lines must be absent rather than claiming a status that never happened.
    #[test]
    fn publish_hook_text_omits_indexnow_lines_when_it_did_not_run() {
        let mut report = publish_report();
        report.indexnow_validation = None;
        report.indexnow_submission = None;
        report.indexnow_ledger_path = None;
        let text = publish_hook_text(&report);
        assert!(!text.contains("IndexNow validation:"), "{text}");
        assert!(!text.contains("IndexNow submission:"), "{text}");
        assert!(!text.contains("IndexNow ledger:"), "{text}");
    }

    /// A publish run with no findings must not print an empty "Findings by
    /// route" heading.
    #[test]
    fn publish_hook_text_omits_an_empty_findings_breakdown() {
        let mut report = publish_report();
        report.findings_by_route.clear();
        let text = publish_hook_text(&report);
        assert!(!text.contains("Findings by route:"), "{text}");
    }
}

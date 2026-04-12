use anyhow::{Result, anyhow, bail};
use clap::ArgMatches;
use csv::Writer;
use seogeo_core::config::load_config_with_diagnostics;
use seogeo_core::{
    BingAiImportReport, PublishHookReport, SearchConsoleExportRow, SnippetInspection,
    build_publish_hook_report_with_config, export_search_console_rows, import_bing_ai_export,
    inspect_snippet_controls_path, inspect_snippet_controls_url, submit_indexnow,
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
        Some((other, _)) => bail!("unsupported indexnow command: {}", other),
        None => bail!("missing indexnow subcommand"),
    }
}

pub fn command_bing_ai(submatches: &ArgMatches) -> Result<i32> {
    match submatches.subcommand() {
        Some(("import", import_matches)) => command_bing_ai_import(import_matches),
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
        format!("Key location: {}", validation.key_location),
        format!(
            "Key file: {}",
            validation.key_file_path.as_deref().unwrap_or("-")
        ),
        format!("Key file present: {}", validation.key_file_present),
        format!("Key file matches: {}", validation.key_file_matches),
    ];
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
    let submission = match submit_indexnow(
        required_arg(submatches, "endpoint")?,
        required_arg(submatches, "site_url")?,
        required_arg(submatches, "key")?,
        &urls,
    ) {
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

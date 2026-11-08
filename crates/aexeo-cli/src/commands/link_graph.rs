use std::fs;
use std::time::Instant;

use aexeo_core::error::UserContext;
use aexeo_core::link_report::{
    AuditLinkContext, LinkReport, LinkReportInput, build_link_report,
    build_link_report_from_audit_with_static_urls,
};
use anyhow::Result;
use clap::ArgMatches;

use crate::commands::common::required_arg;

pub fn command_link_graph(matches: &ArgMatches) -> Result<i32> {
    let started = Instant::now();
    let path = required_arg(matches, "input")?;
    let data = fs::read(path).user_context(format!("cannot read link graph input: {path}"))?;
    let report = if matches.get_flag("audit") {
        let artifact = serde_json::from_slice(&data).user_context("invalid audit artifact JSON")?;
        let context = matches
            .get_one::<String>("context")
            .map(|path| read_audit_link_context(path))
            .transpose()?;
        let static_urls = matches
            .get_one::<String>("static-url-map")
            .map(
                |path| -> Result<std::collections::BTreeMap<String, String>> {
                    let data = fs::read(path)
                        .user_context(format!("cannot read static URL map: {path}"))?;
                    serde_json::from_slice(&data).user_context("invalid static URL map JSON")
                },
            )
            .transpose()?;
        build_link_report_from_audit_with_static_urls(
            &artifact,
            context.as_ref(),
            static_urls.as_ref(),
        )
        .user_context("invalid audit link graph input")?
    } else {
        let input: LinkReportInput =
            serde_json::from_slice(&data).user_context("invalid link graph input JSON")?;
        build_link_report(&input).user_context("invalid link graph input")?
    };
    if matches.get_flag("telemetry") {
        emit_link_graph_telemetry(&report, started);
    }
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(0)
}

/// Read an audit link context file, for `link-graph --audit --context` and
/// `crawl --link-graph-context`.
pub(super) fn read_audit_link_context(path: &str) -> Result<AuditLinkContext> {
    let data = fs::read(path).user_context(format!("cannot read audit link context: {path}"))?;
    serde_json::from_slice(&data).user_context("invalid audit link context JSON")
}

pub(super) fn emit_link_graph_telemetry(report: &LinkReport, started: Instant) {
    let observed = report
        .links
        .destinations
        .iter()
        .map(|destination| destination.evidence.len())
        .sum::<usize>()
        + report.links.same_document.len()
        + report.links.external.len()
        + report.links.unsupported.len()
        + report.links.indeterminate.len();
    let telemetry = serde_json::json!({
        "event": "link_graph_analysis",
        "version": 1,
        "elapsed_us": started.elapsed().as_micros(),
        "page_count": report.pages.len(),
        "link_count": observed,
        "indeterminate_count": report.links.indeterminate.len(),
        "issue_count": report.issues.len(),
    });
    eprintln!("{telemetry}");
}

use std::fs;
use std::time::Instant;

use aexeo_core::link_report::{
    AuditLinkContext, LinkReportInput, build_link_report, build_link_report_from_audit_with_context,
};
use anyhow::{Context, Result};
use clap::ArgMatches;

use crate::commands::common::required_arg;

pub fn command_link_graph(matches: &ArgMatches) -> Result<i32> {
    let started = Instant::now();
    let path = required_arg(matches, "input")?;
    let data = fs::read(path).with_context(|| format!("cannot read link graph input: {path}"))?;
    let report = if matches.get_flag("audit") {
        let artifact = serde_json::from_slice(&data).context("invalid audit artifact JSON")?;
        let context = matches
            .get_one::<String>("context")
            .map(|path| -> Result<AuditLinkContext> {
                let data = fs::read(path)
                    .with_context(|| format!("cannot read audit link context: {path}"))?;
                serde_json::from_slice(&data).context("invalid audit link context JSON")
            })
            .transpose()?;
        build_link_report_from_audit_with_context(&artifact, context.as_ref())?
    } else {
        let input: LinkReportInput =
            serde_json::from_slice(&data).context("invalid link graph input JSON")?;
        build_link_report(&input)?
    };
    if matches.get_flag("telemetry") {
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
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(0)
}

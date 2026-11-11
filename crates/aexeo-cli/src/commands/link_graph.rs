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
    let audit_mode = matches.get_flag("audit");
    let static_mode = matches.get_flag("static-site");
    anyhow::ensure!(
        audit_mode
            || static_mode
            || (matches.get_one::<String>("context").is_none()
                && matches.get_one::<String>("static-url-map").is_none()),
        "--context and --static-url-map require --audit or --static-site"
    );
    let report = if audit_mode || static_mode {
        let artifact = if static_mode {
            let site = aexeo_core::load_site(std::path::Path::new(path))?;
            let mut artifact = aexeo_core::build_audit_artifact(
                "link-graph",
                &[],
                aexeo_contracts::AuditStatus::Complete,
                None,
                None,
            );
            artifact.site = Some(aexeo_core::audit_site_snapshot(
                &site,
                Some(required_arg(matches, "site-origin")?),
            ));
            artifact
        } else {
            let data =
                fs::read(path).user_context(format!("cannot read link graph input: {path}"))?;
            serde_json::from_slice(&data).user_context("invalid audit artifact JSON")?
        };
        let context = matches
            .get_one::<String>("context")
            .map(|path| read_audit_link_context(path))
            .transpose()?;
        let static_urls = matches
            .get_one::<String>("static-url-map")
            .map(|path| read_static_url_map(path))
            .transpose()?;
        build_link_report_from_audit_with_static_urls(
            &artifact,
            context.as_ref(),
            static_urls.as_ref(),
        )
        .user_context("invalid audit link graph input")?
    } else {
        let data = fs::read(path).user_context(format!("cannot read link graph input: {path}"))?;
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

pub(super) fn read_static_url_map(
    path: &str,
) -> Result<std::collections::BTreeMap<String, String>> {
    let data = fs::read(path).user_context(format!("cannot read static URL map: {path}"))?;
    serde_json::from_slice(&data).user_context("invalid static URL map JSON")
}

pub(super) fn link_graph_summary(graph: &LinkReport) -> serde_json::Value {
    serde_json::json!({
        "version": 1,
        "crawl_complete": graph.crawl_complete,
        "page_count": graph.pages.len(),
        "destination_count": graph.links.destinations.len(),
        "same_document_count": graph.links.same_document.len(),
        "indeterminate_count": graph.links.indeterminate.len(),
        "issues": graph.issues,
        "legacy_link_findings": aexeo_core::link_findings::count_conclusions(&graph.legacy_findings),
        "suppressed_legacy_findings": graph.suppressed_legacy_findings,
    })
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
    let (conditional, unclassified, hidden, hidden_unobserved, collapsed, collapsed_unobserved) =
        report
            .links
            .destinations
            .iter()
            .flat_map(|destination| &destination.evidence)
            .chain(&report.links.same_document)
            .chain(&report.links.external)
            .chain(&report.links.unsupported)
            .chain(&report.links.indeterminate)
            .fold(
                (0, 0, 0, 0, 0, 0),
                |(
                    conditional,
                    unclassified,
                    hidden,
                    hidden_unobserved,
                    collapsed,
                    collapsed_unobserved,
                ),
                 link| {
                    (
                        conditional + usize::from(link.conditional_markup == Some(true)),
                        unclassified + usize::from(link.conditional_markup.is_none()),
                        hidden + usize::from(link.hidden_attribute == Some(true)),
                        hidden_unobserved + usize::from(link.hidden_attribute.is_none()),
                        collapsed + usize::from(link.collapsed_details == Some(true)),
                        collapsed_unobserved + usize::from(link.collapsed_details.is_none()),
                    )
                },
            );
    let mut telemetry = serde_json::json!({
        "event": "link_graph_analysis",
        "version": 1,
        "elapsed_us": started.elapsed().as_micros(),
        "page_count": report.pages.len(),
        "link_count": observed,
        "conditional_link_count": conditional,
        "unclassified_markup_link_count": unclassified,
        "hidden_attribute_link_count": hidden,
        "hidden_attribute_unobserved_link_count": hidden_unobserved,
        "collapsed_details_link_count": collapsed,
        "collapsed_details_unobserved_link_count": collapsed_unobserved,
        "indeterminate_count": report.links.indeterminate.len(),
        "issue_count": report.issues.len(),
    });
    if let Some(mapping) = &report.static_url_mapping {
        telemetry["static_mapping_matched_count"] = mapping.matched_count.into();
        telemetry["static_mapping_missing_count"] = mapping.missing_paths.len().into();
        telemetry["static_mapping_unused_count"] = mapping.unused_paths.len().into();
    }
    eprintln!("{telemetry}");
}

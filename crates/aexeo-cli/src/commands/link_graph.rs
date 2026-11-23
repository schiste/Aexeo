use std::fs;
use std::io::{self, Write};
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
    let mut report = if audit_mode || static_mode {
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
    // The static-site adapter uses an internal synthetic audit which is not
    // persisted and must not be presented as a reusable audit binding.
    if static_mode {
        report.audit_sha256 = None;
    }
    if matches.get_flag("telemetry") {
        emit_link_graph_telemetry(&report, started, matches.get_flag("compact"));
    }
    let stdout = io::stdout();
    let writer = io::BufWriter::new(stdout.lock());
    write_graph_json(writer, &report, matches.get_flag("compact"), true)?;
    Ok(0)
}

// Stream directly to the output instead of allocating another report-sized
// string. Flush explicitly so buffered write failures reach the caller.
fn write_graph_json<W: Write, T: serde::Serialize>(
    mut writer: W,
    value: &T,
    compact: bool,
    trailing_newline: bool,
) -> Result<()> {
    if compact {
        serde_json::to_writer(&mut writer, value)?;
    } else {
        serde_json::to_writer_pretty(&mut writer, value)?;
    }
    if trailing_newline {
        writer.write_all(b"\n")?;
    }
    writer.flush()?;
    Ok(())
}

/// Stream the complete report without allocating a second report-sized buffer.
/// A failed write can leave a partial destination; caller path checks are unchanged.
pub(super) fn write_link_graph_file(path: &str, report: &LinkReport, compact: bool) -> Result<()> {
    let file = fs::File::create(path)?;
    write_graph_json(io::BufWriter::new(file), report, compact, false)
}

/// Read an audit link context file, for `link-graph --audit --context` and
/// `crawl --link-graph-context`.
pub(super) fn read_audit_link_context(path: &str) -> Result<AuditLinkContext> {
    let data = fs::read(path).user_context(format!("cannot read audit link context: {path}"))?;
    AuditLinkContext::from_context_document(&data).user_context("invalid audit link context JSON")
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

pub(super) fn emit_link_graph_telemetry(report: &LinkReport, started: Instant, compact: bool) {
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
    #[derive(Default)]
    struct Counts {
        conditional: usize,
        unclassified: usize,
        hidden: usize,
        hidden_unobserved: usize,
        inline_hidden: usize,
        inline_hidden_unobserved: usize,
        collapsed: usize,
        collapsed_unobserved: usize,
        anchor_with_label: usize,
        anchor_empty: usize,
    }
    let counts = report
        .links
        .destinations
        .iter()
        .flat_map(|destination| &destination.evidence)
        .chain(&report.links.same_document)
        .chain(&report.links.external)
        .chain(&report.links.unsupported)
        .chain(&report.links.indeterminate)
        .fold(Counts::default(), |mut counts, link| {
            counts.conditional += usize::from(link.conditional_markup == Some(true));
            counts.unclassified += usize::from(link.conditional_markup.is_none());
            counts.hidden += usize::from(link.hidden_attribute == Some(true));
            counts.hidden_unobserved += usize::from(link.hidden_attribute.is_none());
            counts.inline_hidden += usize::from(link.hidden_inline_style == Some(true));
            counts.inline_hidden_unobserved += usize::from(link.hidden_inline_style.is_none());
            counts.collapsed += usize::from(link.collapsed_details == Some(true));
            counts.collapsed_unobserved += usize::from(link.collapsed_details.is_none());
            if link.anchor.is_empty() {
                counts.anchor_empty += 1;
            } else {
                counts.anchor_with_label += 1;
            }
            counts
        });
    let mut telemetry = serde_json::json!({
        "event": "link_graph_analysis",
        "version": 1,
        "audit_bound": report.audit_sha256.is_some(),
        "output_compact": compact,
        "elapsed_us": started.elapsed().as_micros(),
        "page_count": report.pages.len(),
        "link_count": observed,
        "conditional_link_count": counts.conditional,
        "unclassified_markup_link_count": counts.unclassified,
        "hidden_attribute_link_count": counts.hidden,
        "hidden_attribute_unobserved_link_count": counts.hidden_unobserved,
        "hidden_inline_style_link_count": counts.inline_hidden,
        "hidden_inline_style_unobserved_link_count": counts.inline_hidden_unobserved,
        "collapsed_details_link_count": counts.collapsed,
        "collapsed_details_unobserved_link_count": counts.collapsed_unobserved,
        "anchor_with_label_count": counts.anchor_with_label,
        "anchor_empty_count": counts.anchor_empty,
        "indeterminate_count": report.links.indeterminate.len(),
        "locale_unverified_issue_present": report.issues.iter().any(|issue| issue == "inventory_document_locale_unverified"),
        "issue_count": report.issues.len(),
    });
    if let Some(mapping) = &report.static_url_mapping {
        telemetry["static_mapping_matched_count"] = mapping.matched_count.into();
        telemetry["static_mapping_missing_count"] = mapping.missing_paths.len().into();
        telemetry["static_mapping_unused_count"] = mapping.unused_paths.len().into();
    }
    eprintln!("{telemetry}");
}

#[cfg(test)]
mod output_tests {
    use super::write_graph_json;
    use std::io::{self, Write};

    #[test]
    fn streamed_output_matches_existing_json_and_compact_json() {
        let value = serde_json::json!({"pages": [{"name": "café", "values": [1, 2]}]});
        for compact in [false, true] {
            let mut output = Vec::new();
            write_graph_json(&mut output, &value, compact, true).unwrap();
            let expected = if compact {
                serde_json::to_string(&value).unwrap()
            } else {
                serde_json::to_string_pretty(&value).unwrap()
            };
            assert_eq!(output, format!("{expected}\n").as_bytes());
        }
    }

    #[test]
    fn large_report_is_written_incrementally() {
        struct BoundedWrites(usize);
        impl Write for BoundedWrites {
            fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
                if buf.len() > 8192 {
                    return Err(io::Error::other("oversized write"));
                }
                self.0 += buf.len();
                Ok(buf.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let value = vec!["link evidence"; 20_000];
        let mut output = BoundedWrites(0);
        write_graph_json(&mut output, &value, false, true).unwrap();
        assert!(output.0 > 200_000);
    }

    #[test]
    fn output_write_failure_is_reported() {
        struct WriteFailure;
        impl Write for WriteFailure {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Err(io::Error::other("write failed"))
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let error = write_graph_json(WriteFailure, &serde_json::json!({}), true, true).unwrap_err();
        assert!(error.to_string().contains("write failed"));
    }

    #[test]
    fn file_output_matches_legacy_pretty_bytes_without_newline() {
        let value = serde_json::json!({"evidence": [{"label":"café", "count": 2}]});
        let mut output = Vec::new();
        write_graph_json(&mut output, &value, false, false).unwrap();
        assert_eq!(output, serde_json::to_vec_pretty(&value).unwrap());
    }

    struct FlushFailure;
    impl Write for FlushFailure {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Err(io::Error::other("flush failed"))
        }
    }

    #[test]
    fn output_flush_failure_is_reported() {
        let error =
            write_graph_json(FlushFailure, &serde_json::json!({}), false, true).unwrap_err();
        assert!(error.to_string().contains("flush failed"));
    }
}

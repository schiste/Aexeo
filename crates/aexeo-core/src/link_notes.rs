//! What the text and Markdown audit reports show of a run's link report: the
//! qualification of each legacy `LNK002`/`LNK004` finding and a short coverage
//! summary. Built only when a run has a report; a report without one renders
//! exactly as before.
use std::collections::BTreeMap;

use crate::link_findings::LegacyLinkConclusion;
use crate::link_report::LinkReport;

/// How many report issues the summary names before counting the rest.
const SHOWN_ISSUES: usize = 3;

/// A link report reduced to what the audit reports print.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkFindingNotes {
    crawl_complete: bool,
    issues: Vec<String>,
    /// Why a `not_eligible` finding cannot be verified, from the report's
    /// coverage: the most decisive conditions first.
    unverifiable_reason: String,
    conclusions: BTreeMap<(String, String), LegacyLinkConclusion>,
    suppressed: usize,
    graph_path: Option<String>,
}

/// How many of a group's findings the report confirms and how many it cannot
/// verify. Findings it does not qualify count in neither.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
pub struct LinkFindingCounts {
    pub confirmed: usize,
    pub not_verifiable: usize,
}

impl LinkFindingNotes {
    /// `graph_path` is where the report was written, if anywhere.
    pub fn from_report(report: &LinkReport, graph_path: Option<&str>) -> Self {
        let mut reasons = Vec::new();
        if !report.crawl_complete {
            reasons.push("crawl incomplete");
        }
        match &report.reconciliation {
            None => reasons.push("no inventory"),
            Some(reconciliation) if !reconciliation.comparable => {
                reasons.push("inventory not comparable");
            }
            Some(_) => {}
        }
        let unverifiable_reason = if reasons.is_empty() {
            "page not eligible".to_string()
        } else {
            reasons.join(" / ")
        };
        Self {
            crawl_complete: report.crawl_complete,
            issues: report.issues.clone(),
            unverifiable_reason,
            conclusions: report
                .legacy_findings
                .iter()
                .map(|finding| {
                    (
                        (finding.rule_id.clone(), finding.path.clone()),
                        finding.conclusion,
                    )
                })
                .collect(),
            suppressed: report.suppressed_legacy_findings.len(),
            graph_path: graph_path.map(str::to_string),
        }
    }

    /// Tally the qualified findings among `(rule_id, path)` pairs.
    pub fn count<'a>(
        &self,
        findings: impl IntoIterator<Item = (&'a str, &'a str)>,
    ) -> LinkFindingCounts {
        let mut counts = LinkFindingCounts::default();
        for (rule_id, path) in findings {
            match self
                .conclusions
                .get(&(rule_id.to_string(), path.to_string()))
            {
                Some(LegacyLinkConclusion::Confirmed) => counts.confirmed += 1,
                Some(LegacyLinkConclusion::NotEligible | LegacyLinkConclusion::NotInReport) => {
                    counts.not_verifiable += 1;
                }
                // Disproven findings are suppressed before rendering.
                Some(_) | None => {}
            }
        }
        counts
    }

    /// A short qualification for a group of `rule_id` findings, such as
    /// `confirmed orphan` or `not verifiable: crawl incomplete / no inventory`;
    /// `None` when the report qualifies none of them.
    pub fn describe(&self, rule_id: &str, counts: LinkFindingCounts) -> Option<String> {
        let reason = &self.unverifiable_reason;
        match (counts.confirmed, counts.not_verifiable) {
            (0, 0) => None,
            (1, 0) if rule_id == "LNK002" => Some("confirmed orphan".to_string()),
            (1, 0) => Some("confirmed".to_string()),
            (confirmed, 0) => Some(format!("{confirmed} confirmed")),
            (0, 1) => Some(format!("not verifiable: {reason}")),
            (0, unverified) => Some(format!("{unverified} not verifiable: {reason}")),
            (confirmed, unverified) => Some(format!(
                "{confirmed} confirmed / {unverified} not verifiable: {reason}"
            )),
        }
    }

    /// The `Link graph` block of the text or Markdown report.
    pub fn summary_lines(&self, markdown: bool) -> Vec<String> {
        let counts = LinkFindingCounts {
            confirmed: self
                .conclusions
                .values()
                .filter(|conclusion| **conclusion == LegacyLinkConclusion::Confirmed)
                .count(),
            not_verifiable: self
                .conclusions
                .values()
                .filter(|conclusion| !conclusion.proves_finding_false())
                .filter(|conclusion| **conclusion != LegacyLinkConclusion::Confirmed)
                .count(),
        };
        let mut coverage = format!(
            "Crawl complete: {}",
            if self.crawl_complete { "yes" } else { "no" }
        );
        if !self.issues.is_empty() {
            let shown = self.issues[..self.issues.len().min(SHOWN_ISSUES)].join(", ");
            coverage.push_str(&format!("; issues: {shown}"));
            if self.issues.len() > SHOWN_ISSUES {
                coverage.push_str(&format!(" (+{} more)", self.issues.len() - SHOWN_ISSUES));
            }
        }
        let findings = format!(
            "LNK002/LNK004 findings: {} confirmed, {} not verifiable, {} suppressed as disproven",
            counts.confirmed, counts.not_verifiable, self.suppressed
        );
        let graph = self
            .graph_path
            .as_ref()
            .map(|path| format!("Graph report: {path}"));
        let items = [Some(coverage), Some(findings), graph]
            .into_iter()
            .flatten();
        let mut lines = Vec::new();
        if markdown {
            lines.push("## Link graph".to_string());
            lines.push(String::new());
            lines.extend(items.map(|item| format!("- {item}")));
        } else {
            lines.push("Link graph:".to_string());
            lines.extend(items.map(|item| format!("  {item}")));
        }
        lines.push(String::new());
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reporting::{
        prioritize_findings, prioritize_findings_with_links, render_markdown_artifact,
        render_markdown_artifact_with_links, render_text_artifact, render_text_artifact_with_links,
    };
    use aexeo_contracts::{AuditArtifact, AuditStatus, Finding, FindingScope};

    fn orphan(route: &str) -> Finding {
        Finding {
            rule_id: "LNK002".into(),
            message: format!("no inbound links observed in the analyzed pages: /{route}"),
            path: format!("crawl/{route}/index.html"),
            line: 1,
            column: 1,
            severity: "warning".into(),
            suggestion: None,
            scope: FindingScope::Page,
            subject: None,
        }
    }

    /// `alpha` sorts before `zulu`, so only the qualification can rank `zulu`
    /// first.
    fn artifact() -> AuditArtifact {
        crate::reporting::build_audit_artifact(
            "crawl",
            &[orphan("alpha"), orphan("zulu")],
            AuditStatus::Complete,
            None,
            None,
        )
    }

    fn legacy(route: &str, conclusion: &str) -> serde_json::Value {
        serde_json::json!({"rule_id": "LNK002", "path": format!("crawl/{route}/index.html"),
            "url": format!("https://example.org/{route}"), "conclusion": conclusion,
            "source_pages_by_context": {}, "via_redirect_source_pages": 0, "navigation_only": false})
    }

    fn notes(complete: bool, inventory: bool) -> LinkFindingNotes {
        let reconciliation = inventory.then(|| {
            serde_json::json!({"comparable": true, "issues": [], "inventory_only": [],
                "crawl_only": [], "eligible_pages": ["https://example.org/zulu"]})
        });
        let issues = if complete {
            vec![]
        } else {
            vec![
                "audit_crawl_partial",
                "audit_failed_responses",
                "crawl_incomplete",
                "inventory_missing",
            ]
        };
        let report: LinkReport = serde_json::from_value(serde_json::json!({
            "version": 1, "identity": null, "snapshot_date": null, "crawl_complete": complete,
            "issues": issues, "reconciliation": reconciliation, "pages": [],
            "links": {"destinations": [], "same_document": [], "external": [], "unsupported": [], "indeterminate": []},
            "legacy_findings": [legacy("zulu", "confirmed"), legacy("alpha", "not_eligible")],
            "suppressed_legacy_findings": [legacy("old", "inbound_observed")],
        }))
        .unwrap();
        LinkFindingNotes::from_report(&report, Some("graph.json"))
    }

    #[test]
    fn the_text_report_qualifies_each_finding_and_summarizes_the_link_graph() {
        let text =
            render_text_artifact_with_links(&artifact(), "ok", None, Some(&notes(true, true)));
        assert!(text.contains(
            "Start here (2 of 2 distinct problems):\n   \
             1. [warning] LNK002 no inbound links observed in the analyzed pages: /zulu (confirmed orphan) (on crawl/zulu/index.html)\n   \
             2. [warning] LNK002 no inbound links observed in the analyzed pages: /alpha (not verifiable: page not eligible) (on crawl/alpha/index.html)\n"
        ), "{text}");
        assert!(
            text.contains(
                "Link graph:\n  Crawl complete: yes\n  \
             LNK002/LNK004 findings: 1 confirmed, 1 not verifiable, 1 suppressed as disproven\n  \
             Graph report: graph.json\n"
            ),
            "{text}"
        );
        assert!(text.contains("pages: /zulu (confirmed orphan)\n"), "{text}");
        assert!(
            text.contains("pages: /alpha (not verifiable: page not eligible)\n"),
            "{text}"
        );
    }

    #[test]
    fn an_incomplete_crawl_names_its_reasons_and_top_issues() {
        let notes = notes(false, false);
        let markdown = render_markdown_artifact_with_links(&artifact(), None, Some(&notes));
        assert!(markdown.contains(
            "## Link graph\n\n\
             - Crawl complete: no; issues: audit_crawl_partial, audit_failed_responses, crawl_incomplete (+1 more)\n\
             - LNK002/LNK004 findings: 1 confirmed, 1 not verifiable, 1 suppressed as disproven\n\
             - Graph report: graph.json\n"
        ), "{markdown}");
        assert!(
            markdown.contains("`LNK002` no inbound links observed in the analyzed pages: /alpha (not verifiable: crawl incomplete / no inventory)\n"),
            "{markdown}"
        );
        assert!(
            markdown.contains("pages: /zulu (confirmed orphan)\n"),
            "{markdown}"
        );
    }

    #[test]
    fn a_group_counts_confirmed_and_unverifiable_findings() {
        let notes = notes(true, true);
        let counts = notes.count([
            ("LNK002", "crawl/zulu/index.html"),
            ("LNK002", "crawl/alpha/index.html"),
            ("LNK002", "crawl/other/index.html"),
        ]);
        assert_eq!(
            counts,
            LinkFindingCounts {
                confirmed: 1,
                not_verifiable: 1
            }
        );
        assert_eq!(
            notes.describe("LNK004", counts).as_deref(),
            Some("1 confirmed / 1 not verifiable: page not eligible")
        );
        assert_eq!(notes.describe("LNK004", LinkFindingCounts::default()), None);
    }

    #[test]
    fn a_confirmed_finding_outranks_an_unverifiable_one_of_the_same_rule() {
        let findings = artifact().findings;
        let plain = prioritize_findings(&findings, 10);
        assert_eq!(plain[0].examples, ["crawl/alpha/index.html"]);
        let notes = notes(true, true);
        let ranked = prioritize_findings_with_links(&findings, 10, Some(&notes));
        assert_eq!(ranked[0].examples, ["crawl/zulu/index.html"]);
        assert_eq!(
            ranked[0].link_graph,
            Some(LinkFindingCounts {
                confirmed: 1,
                not_verifiable: 0
            })
        );
        // Additive in JSON: the field appears only with a report.
        let json = serde_json::to_value(&ranked).unwrap();
        assert_eq!(
            json[1]["link_graph"],
            serde_json::json!({"confirmed": 0, "not_verifiable": 1})
        );
        assert!(
            serde_json::to_value(&plain).unwrap()[0]
                .get("link_graph")
                .is_none()
        );
    }

    #[test]
    fn without_a_report_every_output_is_unchanged() {
        let artifact = artifact();
        let text = render_text_artifact(&artifact, "ok", None);
        assert_eq!(
            text,
            render_text_artifact_with_links(&artifact, "ok", None, None)
        );
        let markdown = render_markdown_artifact(&artifact, None);
        assert_eq!(
            markdown,
            render_markdown_artifact_with_links(&artifact, None, None)
        );
        for output in [&text, &markdown] {
            assert!(
                !output.contains("Link graph") && !output.contains("orphan)"),
                "{output}"
            );
        }
        assert_eq!(
            prioritize_findings(&artifact.findings, 10),
            prioritize_findings_with_links(&artifact.findings, 10, None)
        );
        // A clean run with a report still shows the summary; without, nothing.
        let clean =
            crate::reporting::build_audit_artifact("crawl", &[], AuditStatus::Complete, None, None);
        assert_eq!(render_text_artifact(&clean, "ok", None), "ok");
        assert_eq!(
            render_text_artifact_with_links(&clean, "ok", None, Some(&notes(true, true))),
            "ok\n\nLink graph:\n  Crawl complete: yes\n  \
             LNK002/LNK004 findings: 1 confirmed, 1 not verifiable, 1 suppressed as disproven\n  \
             Graph report: graph.json"
        );
    }
}

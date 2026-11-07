//! Qualify the legacy inbound-link findings of an audit with the coverage-aware
//! link report built from the same audit. Findings themselves never change:
//! the qualification is a separate observation, so rule IDs, severities,
//! messages and fingerprints stay as the legacy rules produced them.
use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use url::Url;

use aexeo_contracts::{AuditPageSnapshot, Finding};

use crate::link_evidence::LinkContext;
use crate::link_report::{AliasEvidence, LinkReport, ObservedPageLinks};

/// Legacy rules whose claim is about a page's incoming links.
const INBOUND_RULES: [&str; 2] = ["LNK002", "LNK004"];

/// What the link report says about a legacy inbound-link finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LegacyLinkConclusion {
    /// The page is eligible: complete, comparable coverage stands behind the
    /// finding. For `LNK002`, the report also observed no incoming source.
    Confirmed,
    /// `LNK002` only: the report credits incoming sources the legacy rule did
    /// not, such as links to a URL that redirects to the page.
    InboundObserved,
    /// The page answered as a verified inventory alias of another URL, so its
    /// own incoming links say nothing about the content it stands for.
    InventoryAlias,
    /// Coverage does not support a conclusion about this page's incoming links.
    NotEligible,
    /// The report holds no page for the finding's path, such as a page without
    /// response provenance.
    NotInReport,
}

/// One legacy inbound-link finding and the report's observation of its page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyLinkFinding {
    pub rule_id: String,
    /// The finding's path, as in its fingerprint.
    pub path: String,
    pub url: Option<String>,
    pub conclusion: LegacyLinkConclusion,
    /// Distinct other source pages per context, excluding the page itself;
    /// contexts without sources are omitted.
    pub source_pages_by_context: BTreeMap<LinkContext, usize>,
    /// Distinct source pages credited through a recorded redirect chain.
    pub via_redirect_source_pages: usize,
    /// Every incoming source links only from navigation, footer, breadcrumb or
    /// pagination components.
    pub navigation_only: bool,
}

impl LegacyLinkConclusion {
    pub const ALL: [Self; 5] = [
        Self::Confirmed,
        Self::InboundObserved,
        Self::InventoryAlias,
        Self::NotEligible,
        Self::NotInReport,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Confirmed => "confirmed",
            Self::InboundObserved => "inbound_observed",
            Self::InventoryAlias => "inventory_alias",
            Self::NotEligible => "not_eligible",
            Self::NotInReport => "not_in_report",
        }
    }
}

/// How many qualified findings reached each conclusion, every conclusion
/// included, keyed by its serialized name.
pub fn count_conclusions(findings: &[LegacyLinkFinding]) -> BTreeMap<&'static str, usize> {
    LegacyLinkConclusion::ALL
        .into_iter()
        .map(|conclusion| {
            let count = findings
                .iter()
                .filter(|finding| finding.conclusion == conclusion)
                .count();
            (conclusion.as_str(), count)
        })
        .collect()
}

/// Qualify each `LNK002` and `LNK004` finding of `findings` against `report`.
/// `pages` and `static_urls` give each finding path the URL the report used.
pub fn qualify_legacy_link_findings(
    findings: &[Finding],
    pages: &[AuditPageSnapshot],
    static_urls: Option<&BTreeMap<String, String>>,
    report: &LinkReport,
) -> Vec<LegacyLinkFinding> {
    let eligible = report
        .reconciliation
        .as_ref()
        .filter(|reconciliation| report.crawl_complete && reconciliation.comparable)
        .map(|reconciliation| {
            reconciliation
                .eligible_pages
                .iter()
                .map(|url| normalized(url))
                .collect::<BTreeSet<_>>()
        })
        .unwrap_or_default();
    findings
        .iter()
        .filter(|finding| INBOUND_RULES.contains(&finding.rule_id.as_str()))
        .map(|finding| {
            let page = pages
                .iter()
                .find(|page| page.path == finding.path)
                .and_then(|page| report_url(page, static_urls))
                .and_then(|url| report.pages.iter().find(|observed| observed.url == url));
            qualify(finding, page, report, &eligible)
        })
        .collect()
}

fn qualify(
    finding: &Finding,
    page: Option<&ObservedPageLinks>,
    report: &LinkReport,
    eligible: &BTreeSet<String>,
) -> LegacyLinkFinding {
    let mut result = LegacyLinkFinding {
        rule_id: finding.rule_id.clone(),
        path: finding.path.clone(),
        url: page.map(|page| page.url.clone()),
        conclusion: LegacyLinkConclusion::NotInReport,
        source_pages_by_context: BTreeMap::new(),
        via_redirect_source_pages: 0,
        navigation_only: false,
    };
    let Some(page) = page else {
        return result;
    };
    result.source_pages_by_context = other_sources_by_context(page, report);
    result.via_redirect_source_pages = page
        .via_redirect
        .as_ref()
        .map_or(0, |credit| credit.distinct_source_pages);
    result.navigation_only = !result.source_pages_by_context.is_empty()
        && result
            .source_pages_by_context
            .keys()
            .all(|context| is_structural(*context));
    let inbound =
        !result.source_pages_by_context.is_empty() || result.via_redirect_source_pages > 0;
    result.conclusion = if page
        .inventory_aliases
        .iter()
        .any(|alias| alias.verified_by == Some(AliasEvidence::Canonical))
    {
        LegacyLinkConclusion::InventoryAlias
    } else if finding.rule_id == "LNK002" && inbound {
        LegacyLinkConclusion::InboundObserved
    } else if eligible.contains(&normalized(&page.url)) {
        LegacyLinkConclusion::Confirmed
    } else {
        LegacyLinkConclusion::NotEligible
    };
    result
}

/// The URL the report gives an audit page, as the audit adapter derives it.
fn report_url(
    page: &AuditPageSnapshot,
    static_urls: Option<&BTreeMap<String, String>>,
) -> Option<String> {
    match static_urls {
        Some(urls) => urls.get(&page.relative_path).cloned(),
        None => page
            .crawl_response
            .as_ref()?
            .observed_url
            .clone()
            .filter(|url| !url.is_empty()),
    }
}

/// Distinct sources per context, without the page's links to itself, which
/// the legacy rules do not count either.
fn other_sources_by_context(
    page: &ObservedPageLinks,
    report: &LinkReport,
) -> BTreeMap<LinkContext, usize> {
    let url = normalized(&page.url);
    let mut sources = BTreeMap::<LinkContext, BTreeSet<String>>::new();
    for destination in report
        .links
        .destinations
        .iter()
        .filter(|destination| destination.destination == url)
    {
        for link in &destination.evidence {
            let source = normalized(&link.source);
            if source != url {
                sources.entry(link.context).or_default().insert(source);
            }
        }
    }
    sources
        .into_iter()
        .map(|(context, sources)| (context, sources.len()))
        .collect()
}

fn is_structural(context: LinkContext) -> bool {
    matches!(
        context,
        LinkContext::Navigation
            | LinkContext::Footer
            | LinkContext::Breadcrumb
            | LinkContext::Pagination
    )
}

fn normalized(url: &str) -> String {
    Url::parse(url).map_or_else(|_| url.to_string(), String::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::link_report::{
        AuditLinkContext, build_link_report_from_audit, build_link_report_from_audit_with_context,
    };
    use crate::link_rules::run_link_rules;
    use crate::verification::{diff_finding_sets, site_from_audit_artifact};
    use aexeo_contracts::AuditArtifact;

    fn identity() -> serde_json::Value {
        serde_json::json!({"origin":"https://example.org", "release":"r1", "surface":"public", "snapshot":"s1"})
    }

    fn page(route: &str, html: &str) -> serde_json::Value {
        let url = format!("https://example.org/{route}");
        let relative = if route.is_empty() {
            "index.html".to_string()
        } else {
            format!("{route}/index.html")
        };
        serde_json::json!({"path":format!("crawl/{relative}"), "relative_path":relative, "route":route, "raw_html":html, "response_headers":{},
            "crawl_response":{"requested_url":url, "observed_url":url, "status_code":200, "headers_observed":true}})
    }

    /// A complete HTTP crawl seeded from an inventory of `pages`' routes plus
    /// `aliases`, with the legacy link findings of `config` in it.
    fn audit(pages: Vec<serde_json::Value>, config: &Config) -> AuditArtifact {
        let count = pages.len();
        let mut artifact: AuditArtifact = serde_json::from_value(serde_json::json!({"command":"crawl", "status":"complete",
            "crawl":{"engine":"http", "visited_pages":count, "discovered_internal_routes":count, "queued_routes_remaining":0, "max_pages":10, "fetch_failures":0, "fetch_retries":0, "skipped_non_html":0, "truncated":false, "elapsed_ms":0, "pages_per_minute":0, "checkpoints_written":0, "partial_artifacts_written":0, "total_fetch_ms":0, "average_fetch_ms":0, "total_page_process_ms":0, "average_page_process_ms":0, "total_partial_audit_ms":0, "average_partial_audit_ms":0, "slowest_paths":[]},
            "site":{"site_url":"https://example.org/", "root":"crawl", "deployment_model":"runtime_snapshot",
                "crawl_meta":{"visited_pages":count, "max_pages":10, "discovered_internal_routes":count, "truncated":false},
                "pages":pages},
            "crawl_attestation":{"seeded_from":{"inventory_identity":identity(), "url_count":count}, "engine":"http"}}))
        .unwrap();
        artifact.findings = run_link_rules(&site_from_audit_artifact(&artifact).unwrap(), config);
        artifact
    }

    fn context(entries: &[(&str, &str, Option<&str>)]) -> AuditLinkContext {
        let pages = entries
            .iter()
            .map(|(route, disposition, target)| {
                serde_json::json!({"url":format!("https://example.org/{route}"), "disposition":disposition,
                    "target":target.map(|route| format!("https://example.org/{route}")), "locale":null})
            })
            .collect::<Vec<_>>();
        serde_json::from_value(
            serde_json::json!({"identity":identity(), "snapshot_date":"2026-11-07",
            "inventory":{"identity":identity(), "complete":true, "pages":pages}}),
        )
        .unwrap()
    }

    fn published(
        routes: &[&'static str],
    ) -> Vec<(&'static str, &'static str, Option<&'static str>)> {
        routes
            .iter()
            .map(|route| (*route, "published", None))
            .collect()
    }

    fn qualified<'a>(report: &'a LinkReport, rule_id: &str, route: &str) -> &'a LegacyLinkFinding {
        let path = if route.is_empty() {
            "crawl/index.html".to_string()
        } else {
            format!("crawl/{route}/index.html")
        };
        report
            .legacy_findings
            .iter()
            .find(|finding| finding.rule_id == rule_id && finding.path == path)
            .unwrap_or_else(|| panic!("{rule_id} {path}: {:?}", report.legacy_findings))
    }

    fn orphan_site() -> Vec<serde_json::Value> {
        vec![
            page("", "<main><a href='/a'>A</a></main>"),
            page("a", "<main><a href='/'>Home</a></main>"),
            page("orphan", "<main><a href='/orphan'>Self</a></main>"),
        ]
    }

    #[test]
    fn an_eligible_page_without_inbound_links_confirms_the_orphan_finding() {
        let artifact = audit(orphan_site(), &Config::default());
        let context = context(&published(&["", "a", "orphan"]));
        let report = build_link_report_from_audit_with_context(&artifact, Some(&context)).unwrap();
        assert!(report.crawl_complete, "{:?}", report.issues);
        let orphan = qualified(&report, "LNK002", "orphan");
        assert_eq!(orphan.conclusion, LegacyLinkConclusion::Confirmed);
        assert_eq!(orphan.url.as_deref(), Some("https://example.org/orphan"));
        // The page's link to itself is not an incoming source.
        assert!(orphan.source_pages_by_context.is_empty());
        assert_eq!(report.legacy_findings.len(), 1);
    }

    #[test]
    fn an_unattested_crawl_leaves_the_orphan_finding_not_eligible() {
        let artifact = audit(orphan_site(), &Config::default());
        let before = artifact.findings.clone();
        let report = build_link_report_from_audit(&artifact).unwrap();
        assert!(!report.crawl_complete);
        let orphan = qualified(&report, "LNK002", "orphan");
        assert_eq!(orphan.conclusion, LegacyLinkConclusion::NotEligible);
        // The finding itself, and so every baseline built from it, is untouched.
        assert_eq!(artifact.findings, before);
        let diff = diff_finding_sets(&before, &artifact.findings);
        assert!(diff.new_findings.is_empty() && diff.resolved_findings.is_empty());
    }

    #[test]
    fn navigation_only_inbound_links_are_reported_as_such() {
        let config = Config {
            min_inbound_links: 2,
            ..Config::default()
        };
        let artifact = audit(
            vec![
                page(
                    "",
                    "<nav><a href='/a'>A</a></nav><main><a href='/b'>B</a></main>",
                ),
                page("a", "<main><a href='/'>Home</a></main>"),
                page("b", "<footer><a href='/'>Home</a></footer>"),
            ],
            &config,
        );
        let context = context(&published(&["", "a", "b"]));
        let report = build_link_report_from_audit_with_context(&artifact, Some(&context)).unwrap();
        let nav = qualified(&report, "LNK004", "a");
        assert_eq!(nav.conclusion, LegacyLinkConclusion::Confirmed);
        assert!(nav.navigation_only);
        assert_eq!(
            nav.source_pages_by_context,
            BTreeMap::from([(LinkContext::Navigation, 1)])
        );
        let body = qualified(&report, "LNK004", "b");
        assert!(!body.navigation_only);
        // The legacy rule counts the navigation link as connectivity.
        assert!(
            !artifact
                .findings
                .iter()
                .any(|finding| finding.rule_id == "LNK002")
        );
    }

    #[test]
    fn a_page_reached_through_a_redirect_is_credited_its_redirected_links() {
        let mut new = page("new", "<main>New</main>");
        new["crawl_response"]["requested_url"] = "https://example.org/old".into();
        new["crawl_response"]["redirect_chain"] =
            serde_json::json!([{"url":"https://example.org/old", "status_code":301}]);
        let artifact = audit(
            vec![page("", "<main><a href='/old'>Old</a></main>"), new],
            &Config::default(),
        );
        assert!(
            artifact.findings.iter().any(
                |finding| finding.rule_id == "LNK002" && finding.path == "crawl/new/index.html"
            )
        );
        let report = build_link_report_from_audit(&artifact).unwrap();
        let moved = qualified(&report, "LNK002", "new");
        assert_eq!(moved.conclusion, LegacyLinkConclusion::InboundObserved);
        assert_eq!(moved.via_redirect_source_pages, 1);
    }

    #[test]
    fn a_verified_canonical_alias_is_not_treated_as_an_orphan() {
        let artifact = audit(
            vec![
                page("", "<main><a href='/a'>A</a></main>"),
                page("a", "<main><a href='/'>Home</a></main>"),
                page(
                    "fr-a",
                    "<head><link rel='canonical' href='https://example.org/a'></head><main>A</main>",
                ),
            ],
            &Config::default(),
        );
        let mut entries = published(&["", "a"]);
        entries.push(("fr-a", "canonical_alias", Some("a")));
        let report =
            build_link_report_from_audit_with_context(&artifact, Some(&context(&entries))).unwrap();
        let alias = qualified(&report, "LNK002", "fr-a");
        assert_eq!(alias.conclusion, LegacyLinkConclusion::InventoryAlias);
    }

    #[test]
    fn a_page_without_provenance_is_not_in_the_report() {
        let mut pages = orphan_site();
        pages[2].as_object_mut().unwrap().remove("crawl_response");
        let artifact = audit(pages, &Config::default());
        let report = build_link_report_from_audit(&artifact).unwrap();
        let orphan = qualified(&report, "LNK002", "orphan");
        assert_eq!(orphan.conclusion, LegacyLinkConclusion::NotInReport);
        assert_eq!(orphan.url, None);
    }

    #[test]
    fn reports_without_inbound_findings_keep_their_shape() {
        let artifact = audit(
            vec![
                page("", "<main><a href='/a'>A</a></main>"),
                page("a", "<main><a href='/'>Home</a></main>"),
            ],
            &Config::default(),
        );
        assert!(
            artifact
                .findings
                .iter()
                .all(|finding| !INBOUND_RULES.contains(&finding.rule_id.as_str()))
        );
        let report = build_link_report_from_audit(&artifact).unwrap();
        let json = serde_json::to_value(&report).unwrap();
        assert!(json.get("legacy_findings").is_none());
    }
}

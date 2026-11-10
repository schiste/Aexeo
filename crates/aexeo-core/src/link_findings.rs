//! Qualify the legacy inbound-link findings of an audit with the coverage-aware
//! link report built from the same audit. A finding the report proves false
//! (`inbound_observed`, `inventory_alias`) is suppressed: listed in the
//! report's `suppressed_legacy_findings` and removed from a run that built the
//! report. Every other finding stays as the legacy rules produced it, its rule
//! ID, severity, message and fingerprint unchanged.
use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use url::Url;

use aexeo_contracts::{AuditPageSnapshot, Finding};

use crate::link_counts::DestinationLinks;
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
    /// Conditional or historical unclassified inbound markup makes the result uncertain.
    ConditionalInbound,
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
    pub const ALL: [Self; 6] = [
        Self::Confirmed,
        Self::InboundObserved,
        Self::InventoryAlias,
        Self::ConditionalInbound,
        Self::NotEligible,
        Self::NotInReport,
    ];

    /// The report shows the finding's claim is false, so a run holding both
    /// removes it rather than reporting it.
    pub fn proves_finding_false(self) -> bool {
        matches!(self, Self::InboundObserved | Self::InventoryAlias)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Confirmed => "confirmed",
            Self::InboundObserved => "inbound_observed",
            Self::InventoryAlias => "inventory_alias",
            Self::ConditionalInbound => "conditional_inbound",
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

/// Remove from `findings` each legacy finding `report` suppressed, matched by
/// rule ID and path. Without suppressed findings nothing changes.
pub fn remove_suppressed_findings(findings: &mut Vec<Finding>, report: &LinkReport) {
    if report.suppressed_legacy_findings.is_empty() {
        return;
    }
    let suppressed = report
        .suppressed_legacy_findings
        .iter()
        .map(|finding| (finding.rule_id.as_str(), finding.path.as_str()))
        .collect::<BTreeSet<_>>();
    findings
        .retain(|finding| !suppressed.contains(&(finding.rule_id.as_str(), finding.path.as_str())));
}

/// Qualify each `LNK002` and `LNK004` finding of `findings` against `report`.
/// `pages` and `static_urls` give each finding path the URL the report used.
pub fn qualify_legacy_link_findings(
    findings: &[Finding],
    pages: &[AuditPageSnapshot],
    static_urls: Option<&BTreeMap<String, String>>,
    report: &LinkReport,
) -> Vec<LegacyLinkFinding> {
    if !findings
        .iter()
        .any(|finding| INBOUND_RULES.contains(&finding.rule_id.as_str()))
    {
        return Vec::new();
    }
    // Preserve the original first-match behavior for duplicate paths and URLs.
    let mut pages_by_path = BTreeMap::new();
    for page in pages {
        pages_by_path.entry(page.path.as_str()).or_insert(page);
    }
    let mut observed_by_url = BTreeMap::new();
    for page in &report.pages {
        observed_by_url.entry(page.url.as_str()).or_insert(page);
    }
    let destinations = destinations_by_url(report);
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
            let page = pages_by_path
                .get(finding.path.as_str())
                .and_then(|page| report_url(page, static_urls))
                .and_then(|url| observed_by_url.get(url.as_str()).copied());
            qualify(finding, page, &destinations, &eligible)
        })
        .collect()
}

fn qualify(
    finding: &Finding,
    page: Option<&ObservedPageLinks>,
    destinations: &BTreeMap<&str, Vec<&DestinationLinks>>,
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
    let (contexts, mut conditional, mut nonconditional) =
        other_sources_by_context(page, destinations);
    result.source_pages_by_context = contexts;
    result.via_redirect_source_pages = page
        .via_redirect
        .as_ref()
        .map_or(0, |credit| credit.distinct_source_pages);
    result.navigation_only = !result.source_pages_by_context.is_empty()
        && result
            .source_pages_by_context
            .keys()
            .all(|context| is_structural(*context));
    if let Some(credit) = &page.via_redirect {
        conditional |= credit
            .conditional_source_pages
            .unwrap_or(credit.distinct_source_pages)
            > 0;
        nonconditional |= credit.nonconditional_source_pages.unwrap_or(0) > 0;
    }
    result.conclusion = if page
        .inventory_aliases
        .iter()
        .any(|alias| alias.verified_by == Some(AliasEvidence::Canonical))
    {
        LegacyLinkConclusion::InventoryAlias
    } else if finding.rule_id == "LNK002" && nonconditional {
        LegacyLinkConclusion::InboundObserved
    } else if conditional {
        LegacyLinkConclusion::ConditionalInbound
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
fn destinations_by_url(report: &LinkReport) -> BTreeMap<&str, Vec<&DestinationLinks>> {
    let mut destinations = BTreeMap::<&str, Vec<&DestinationLinks>>::new();
    for destination in &report.links.destinations {
        destinations
            .entry(&destination.destination)
            .or_default()
            .push(destination);
    }
    destinations
}

fn other_sources_by_context(
    page: &ObservedPageLinks,
    destinations: &BTreeMap<&str, Vec<&DestinationLinks>>,
) -> (BTreeMap<LinkContext, usize>, bool, bool) {
    let url = normalized(&page.url);
    let mut sources = BTreeMap::<LinkContext, BTreeSet<String>>::new();
    let mut conditional = false;
    let mut nonconditional = false;
    for destination in destinations.get(url.as_str()).into_iter().flatten() {
        for link in &destination.evidence {
            let source = normalized(&link.source);
            if source != url {
                sources.entry(link.context).or_default().insert(source);
                if link.conditional_markup == Some(false) {
                    nonconditional = true;
                } else {
                    conditional = true;
                }
            }
        }
    }
    (
        sources
            .into_iter()
            .map(|(context, sources)| (context, sources.len()))
            .collect(),
        conditional,
        nonconditional,
    )
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

    fn suppressed<'a>(report: &'a LinkReport, rule_id: &str, route: &str) -> &'a LegacyLinkFinding {
        let path = format!("crawl/{route}/index.html");
        assert!(
            !report
                .legacy_findings
                .iter()
                .any(|finding| finding.path == path),
            "{:?}",
            report.legacy_findings
        );
        report
            .suppressed_legacy_findings
            .iter()
            .find(|finding| finding.rule_id == rule_id && finding.path == path)
            .unwrap_or_else(|| panic!("{rule_id} {path}: {:?}", report.suppressed_legacy_findings))
    }

    /// The rule IDs and paths of `artifact`'s findings once `report` has
    /// removed those it suppressed.
    fn kept(artifact: &AuditArtifact, report: &LinkReport) -> Vec<(String, String)> {
        let mut findings = artifact.findings.clone();
        remove_suppressed_findings(&mut findings, report);
        findings
            .into_iter()
            .map(|finding| (finding.rule_id, finding.path))
            .collect()
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
        assert!(report.suppressed_legacy_findings.is_empty());
        let findings = artifact.findings.clone();
        let mut kept = findings.clone();
        remove_suppressed_findings(&mut kept, &report);
        assert_eq!(kept, findings);
    }

    #[test]
    fn an_unattested_crawl_leaves_the_orphan_finding_not_eligible() {
        let artifact = audit(orphan_site(), &Config::default());
        let before = artifact.findings.clone();
        let report = build_link_report_from_audit(&artifact).unwrap();
        assert!(!report.crawl_complete);
        let orphan = qualified(&report, "LNK002", "orphan");
        assert_eq!(orphan.conclusion, LegacyLinkConclusion::NotEligible);
        // A finding the report cannot prove false is kept, and so is every
        // baseline built from it.
        let mut findings = artifact.findings.clone();
        remove_suppressed_findings(&mut findings, &report);
        assert_eq!(findings, before);
        let diff = diff_finding_sets(&before, &findings);
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
    fn qualification_preserves_first_page_and_duplicate_destination_semantics() {
        let artifact = audit(
            vec![
                page("", "<main><a href='/a'>A</a></main>"),
                page("a", "<main>Destination</main>"),
                page("b", "<main>Other</main>"),
            ],
            &Config {
                min_inbound_links: 3,
                ..Config::default()
            },
        );
        let mut report = build_link_report_from_audit(&artifact).unwrap();
        let destination = report
            .links
            .destinations
            .iter()
            .find(|destination| destination.destination.ends_with("/a"))
            .unwrap()
            .clone();
        report.links.destinations.push(destination);
        let mut pages = artifact.site.as_ref().unwrap().pages.clone();
        let mut duplicate = pages
            .iter()
            .find(|page| page.path == "crawl/a/index.html")
            .unwrap()
            .clone();
        duplicate.crawl_response.as_mut().unwrap().observed_url =
            Some("https://example.org/b".into());
        pages.push(duplicate);
        let results = qualify_legacy_link_findings(&artifact.findings, &pages, None, &report);
        let a = results
            .iter()
            .find(|finding| finding.rule_id == "LNK004" && finding.path == "crawl/a/index.html")
            .unwrap();
        assert_eq!(a.url.as_deref(), Some("https://example.org/a"));
        assert_eq!(a.source_pages_by_context.get(&LinkContext::Body), Some(&1));
    }

    #[test]
    fn conditional_only_inbound_is_reported_even_when_legacy_rule_is_silent() {
        let artifact = audit(
            vec![
                page("", "<main><template><a href='/a'>A</a></template></main>"),
                page("a", "<main><a href='/a'>Self</a></main>"),
                page("b", "<main>Unlinked</main>"),
            ],
            &Config::default(),
        );
        let report = build_link_report_from_audit(&artifact).unwrap();
        assert!(
            !artifact
                .findings
                .iter()
                .any(|finding| finding.rule_id == "LNK002" && finding.path == "crawl/a/index.html")
        );
        let a = report
            .pages
            .iter()
            .find(|page| page.url.ends_with("/a"))
            .unwrap();
        assert_eq!(a.conditional_only_inbound, Some(true));
        let b = report
            .pages
            .iter()
            .find(|page| page.url.ends_with("/b"))
            .unwrap();
        assert_eq!(b.conditional_only_inbound, Some(false));
        let mut json = serde_json::to_value(&report).unwrap();
        for page in json["pages"].as_array_mut().unwrap() {
            page.as_object_mut()
                .unwrap()
                .remove("conditional_only_inbound");
        }
        let older: crate::link_report::LinkReport = serde_json::from_value(json).unwrap();
        assert!(
            older
                .pages
                .iter()
                .all(|page| page.conditional_only_inbound.is_none())
        );
    }

    #[test]
    fn conditional_inbound_withholds_low_link_count_confirmation() {
        let config = Config {
            min_inbound_links: 3,
            ..Config::default()
        };
        let artifact = audit(
            vec![
                page(
                    "",
                    "<main><a href='/a'>A</a><noscript><a href='/a'>fallback</a></noscript></main>",
                ),
                page("a", "<main>Destination</main>"),
            ],
            &config,
        );
        let report = build_link_report_from_audit_with_context(
            &artifact,
            Some(&context(&published(&["", "a"]))),
        )
        .unwrap();
        assert_eq!(
            qualified(&report, "LNK004", "a").conclusion,
            LegacyLinkConclusion::ConditionalInbound
        );
        let target = report
            .pages
            .iter()
            .find(|page| page.url.ends_with("/a"))
            .unwrap();
        assert_eq!(target.conditional_source_pages, Some(1));
        assert_eq!(target.nonconditional_source_pages, Some(1));
        assert_eq!(target.conditional_only_inbound, Some(false));
        let rendered = crate::link_notes::LinkFindingNotes::from_report(&report, None);
        let counts = rendered.count([("LNK004", "crawl/a/index.html")]);
        assert_eq!(counts.not_verifiable, 1);
        assert!(
            rendered
                .describe("LNK004", counts)
                .unwrap()
                .contains("conditional link visibility unknown")
        );
    }

    #[test]
    fn conditional_redirect_does_not_suppress_an_orphan_finding() {
        let mut new = page("new", "<main>New</main>");
        new["crawl_response"]["requested_url"] = "https://example.org/old".into();
        new["crawl_response"]["redirect_chain"] =
            serde_json::json!([{"url":"https://example.org/old", "status_code":301}]);
        let artifact = audit(
            vec![
                page(
                    "",
                    "<main><template><a href='/old'>Old</a></template></main>",
                ),
                new.clone(),
            ],
            &Config::default(),
        );
        let report = build_link_report_from_audit(&artifact).unwrap();
        let finding = qualified(&report, "LNK002", "new");
        assert_eq!(finding.conclusion, LegacyLinkConclusion::ConditionalInbound);
        assert!(
            kept(&artifact, &report).contains(&("LNK002".into(), "crawl/new/index.html".into()))
        );
        let credit = report
            .pages
            .iter()
            .find(|page| page.url.ends_with("/new"))
            .unwrap()
            .via_redirect
            .as_ref()
            .unwrap();
        assert_eq!(credit.conditional_source_pages, Some(1));
        assert_eq!(credit.nonconditional_source_pages, Some(0));
        assert_eq!(
            report
                .pages
                .iter()
                .find(|page| page.url.ends_with("/new"))
                .unwrap()
                .conditional_only_inbound,
            Some(true)
        );
        // Older serialized reports do not have redirect visibility counts.
        // Unknown provenance must not become proof of a nonconditional link.
        let mut old_report = report.clone();
        let old_page = old_report
            .pages
            .iter_mut()
            .find(|page| page.url.ends_with("/new"))
            .unwrap();
        let old_credit = old_page.via_redirect.as_mut().unwrap();
        old_credit.conditional_source_pages = None;
        old_credit.nonconditional_source_pages = None;
        let old_page = old_report
            .pages
            .iter()
            .find(|page| page.url.ends_with("/new"))
            .unwrap();
        let original_finding = artifact
            .findings
            .iter()
            .find(|finding| finding.rule_id == "LNK002" && finding.path == "crawl/new/index.html")
            .unwrap();
        assert_eq!(
            qualify(
                original_finding,
                Some(old_page),
                &destinations_by_url(&old_report),
                &BTreeSet::new()
            )
            .conclusion,
            LegacyLinkConclusion::ConditionalInbound
        );
        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(
            json["links"]["destinations"][0]["evidence"][0]["conditional_markup"],
            true
        );

        let mixed = audit(
            vec![
                page(
                    "",
                    "<main><template><a href='/old'>Old</a></template><a href='/old'>Old</a></main>",
                ),
                new,
            ],
            &Config::default(),
        );
        let mixed_report = build_link_report_from_audit(&mixed).unwrap();
        assert_eq!(
            suppressed(&mixed_report, "LNK002", "new").conclusion,
            LegacyLinkConclusion::InboundObserved
        );
        let credit = mixed_report
            .pages
            .iter()
            .find(|page| page.url.ends_with("/new"))
            .unwrap()
            .via_redirect
            .as_ref()
            .unwrap();
        assert_eq!(credit.conditional_source_pages, Some(1));
        assert_eq!(credit.nonconditional_source_pages, Some(1));
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
        let moved = suppressed(&report, "LNK002", "new");
        assert_eq!(moved.conclusion, LegacyLinkConclusion::InboundObserved);
        assert_eq!(moved.via_redirect_source_pages, 1);
        assert!(
            !kept(&artifact, &report).contains(&("LNK002".into(), "crawl/new/index.html".into()))
        );
        // Baselines see the removed finding as resolved.
        let mut after = artifact.findings.clone();
        remove_suppressed_findings(&mut after, &report);
        let diff = diff_finding_sets(&artifact.findings, &after);
        assert_eq!(diff.resolved_findings.len(), 1);
        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(
            json["suppressed_legacy_findings"],
            serde_json::json!([{"rule_id":"LNK002", "path":"crawl/new/index.html", "url":"https://example.org/new",
                "conclusion":"inbound_observed", "source_pages_by_context":{}, "via_redirect_source_pages":1,
                "navigation_only":false}])
        );
        assert!(json.get("legacy_findings").is_none());
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
        let alias = suppressed(&report, "LNK002", "fr-a");
        assert_eq!(alias.conclusion, LegacyLinkConclusion::InventoryAlias);
        assert!(
            kept(&artifact, &report)
                .iter()
                .all(|(rule_id, _)| rule_id != "LNK002")
        );
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
        assert_eq!(kept(&artifact, &report).len(), artifact.findings.len());
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
        assert!(json.get("suppressed_legacy_findings").is_none());
    }
}

//! Produce an evidence report from explicitly identified rendered pages.
use serde::{Deserialize, Serialize};
use url::Url;

use crate::link_counts::{LinkCounts, count_links};
use crate::link_evidence::{HtmlLinkMetadata, LinkContext, extract_document};
use crate::link_graph::{LinkResolution, resolve_link};
use crate::link_inventory::{
    CrawlSurface, InventoryDisposition, InventoryReconciliation, InventorySnapshot,
    SurfaceIdentity, reconcile,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderedPage {
    /// The observed absolute URL, including a query when it identifies the page.
    pub url: String,
    pub html: String,
    pub status_code: Option<u16>,
    /// None means response headers were not captured; an empty map means they were.
    pub response_headers: Option<std::collections::BTreeMap<String, String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkReportInput {
    pub version: u32,
    /// Origin, release, surface and snapshot identity supplied by the producer.
    pub identity: Option<SurfaceIdentity>,
    /// Actual capture date if known. Never inferred from the snapshot identifier.
    pub snapshot_date: Option<String>,
    /// Attestation that the crawl had no truncation or missing fetches.
    #[serde(default)]
    pub crawl_complete: bool,
    pub pages: Vec<RenderedPage>,
    pub inventory: Option<InventorySnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObservedPageLinks {
    pub url: String,
    pub status_code: Option<u16>,
    pub response_headers_captured: bool,
    pub noindex_observed: bool,
    pub canonical_hrefs: Vec<String>,
    pub redirect_target: Option<String>,
    /// Observed counts only; zero does not establish orphanhood.
    pub distinct_source_pages: usize,
    pub source_pages_by_context: std::collections::BTreeMap<LinkContext, usize>,
}

fn has_noindex(value: &str) -> bool {
    value
        .split(|ch: char| ch.is_ascii_whitespace() || matches!(ch, ',' | ';' | ':'))
        .any(|part| part.eq_ignore_ascii_case("noindex") || part.eq_ignore_ascii_case("none"))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkReport {
    pub version: u32,
    pub identity: Option<SurfaceIdentity>,
    pub snapshot_date: Option<String>,
    pub crawl_complete: bool,
    pub issues: Vec<String>,
    pub reconciliation: Option<InventoryReconciliation>,
    pub pages: Vec<ObservedPageLinks>,
    pub links: LinkCounts,
}

/// Caller-supplied identity and inventory for comparison with an audit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditLinkContext {
    pub identity: Option<SurfaceIdentity>,
    pub snapshot_date: Option<String>,
    pub inventory: Option<InventorySnapshot>,
}

/// Analyze only pages whose effective URL was actually captured in an audit.
/// Audit artifacts do not attest the publication surface or CMS inventory, so
/// this adapter deliberately never claims complete graph coverage.
pub fn build_link_report_from_audit(
    artifact: &aexeo_contracts::AuditArtifact,
) -> anyhow::Result<LinkReport> {
    build_link_report_from_audit_with_context(artifact, None)
}

/// Compare an audit with an explicitly supplied inventory. The caller's
/// identity does not prove a complete crawl or a coherent snapshot.
pub fn build_link_report_from_audit_with_context(
    artifact: &aexeo_contracts::AuditArtifact,
    context: Option<&AuditLinkContext>,
) -> anyhow::Result<LinkReport> {
    let site = artifact
        .site
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("audit artifact has no site snapshot"))?;
    let origin = site
        .site_url
        .as_deref()
        .and_then(|value| Url::parse(value).ok())
        .filter(|url| {
            matches!(url.scheme(), "http" | "https")
                && url.username().is_empty()
                && url.password().is_none()
        })
        .map(|url| url.origin().ascii_serialization())
        .unwrap_or_default();
    let mut issues = std::collections::BTreeSet::new();
    let pages = site
        .pages
        .iter()
        .filter_map(|page| {
            let Some(response) = &page.crawl_response else {
                issues.insert("audit_page_response_provenance_missing");
                return None;
            };
            let Some(url) = response.observed_url.as_ref().filter(|url| !url.is_empty()) else {
                issues.insert("audit_page_observed_url_missing");
                return None;
            };
            if response.requested_url != *url {
                issues.insert("audit_redirect_chain_unavailable");
            }
            Some(RenderedPage {
                url: url.clone(),
                html: page.raw_html.clone(),
                status_code: response.status_code,
                response_headers: response
                    .headers_observed
                    .then(|| page.response_headers.clone()),
            })
        })
        .collect();
    let input = LinkReportInput {
        version: 1,
        identity: context
            .and_then(|value| value.identity.clone())
            .or(Some(SurfaceIdentity {
                origin,
                release: String::new(),
                surface: String::new(),
                snapshot: String::new(),
            })),
        snapshot_date: context.and_then(|value| value.snapshot_date.clone()),
        crawl_complete: false,
        pages,
        inventory: context.and_then(|value| value.inventory.clone()),
    };
    let mut report = build_link_report(&input)?;
    report.issues.extend(issues.into_iter().map(str::to_string));
    Ok(report)
}

/// This report lists observations and coverage issues. It never declares an
/// orphan: completeness and semantic classification must be assessed first.
pub fn build_link_report(input: &LinkReportInput) -> anyhow::Result<LinkReport> {
    anyhow::ensure!(
        input.version == 1,
        "unsupported link report input version: {}",
        input.version
    );
    let mut issues = Vec::new();
    let origin = input
        .identity
        .as_ref()
        .map(|identity| identity.origin.as_str())
        .unwrap_or("");
    let origin_url = Url::parse(origin).ok().filter(|url| {
        matches!(url.scheme(), "http" | "https")
            && url.username().is_empty()
            && url.password().is_none()
            && url.path() == "/"
            && url.query().is_none()
            && url.fragment().is_none()
    });
    if origin_url.is_none() {
        issues.push("origin_missing_or_invalid".to_string());
    }
    if input.identity.as_ref().is_none_or(|identity| {
        [&identity.release, &identity.surface, &identity.snapshot]
            .iter()
            .any(|value| value.trim().is_empty())
    }) {
        issues.push("surface_identity_missing".to_string());
    }
    if !input.crawl_complete {
        issues.push("crawl_incomplete".to_string());
    }
    if input
        .snapshot_date
        .as_deref()
        .is_none_or(|date| date.trim().is_empty())
    {
        issues.push("snapshot_date_missing".to_string());
    }
    let mut seen = std::collections::BTreeSet::new();
    let mut evidence = Vec::new();
    let mut observations = Vec::<(HtmlLinkMetadata, bool, Option<String>)>::new();
    let mut coverage_issues = std::collections::BTreeSet::new();
    if input
        .snapshot_date
        .as_deref()
        .is_none_or(|date| date.trim().is_empty())
    {
        coverage_issues.insert("snapshot_date_missing");
    }
    for page in &input.pages {
        if !seen.insert(&page.url) {
            issues.push("duplicate_crawl_url".to_string());
        }
        let valid = Url::parse(&page.url)
            .ok()
            .zip(origin_url.as_ref())
            .is_some_and(|(url, origin)| {
                matches!(url.scheme(), "http" | "https")
                    && url.username().is_empty()
                    && url.password().is_none()
                    && url.fragment().is_none()
                    && url.origin() == origin.origin()
            });
        if !valid {
            issues.push("invalid_or_foreign_crawl_url".to_string());
        }
        let (links, metadata) = extract_document(
            &page.html,
            &page.url,
            origin,
            input.snapshot_date.as_deref(),
        );
        evidence.extend(links);
        let headers = page.response_headers.as_ref();
        if headers.is_none() {
            coverage_issues.insert("response_headers_missing");
        }
        match page.status_code {
            None => {
                coverage_issues.insert("response_status_missing");
            }
            Some(status) if (300..400).contains(&status) => {
                coverage_issues.insert("redirect_observed");
            }
            Some(status) if !(200..300).contains(&status) => {
                coverage_issues.insert("non_success_response");
            }
            _ => {}
        }
        let header_noindex = headers.is_some_and(|headers| {
            headers.iter().any(|(name, value)| {
                name.eq_ignore_ascii_case("x-robots-tag") && has_noindex(value)
            })
        });
        let redirect_target = headers.and_then(|headers| {
            headers
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case("location"))
                .map(|(_, value)| value.clone())
        });
        let noindex = metadata
            .robots_directives
            .iter()
            .any(|value| has_noindex(value))
            || header_noindex;
        if noindex {
            issues.push("noindex_observed".to_string());
            if input.inventory.as_ref().is_some_and(|inventory| {
                inventory.pages.iter().any(|entry| {
                    entry.url == page.url && entry.disposition == InventoryDisposition::Published
                })
            }) {
                coverage_issues.insert("inventory_indexability_mismatch");
            }
        }
        if metadata.canonical_hrefs.len() > 1
            || metadata.canonical_hrefs.iter().any(|href| {
                !matches!(
                    resolve_link(href, &page.url, origin, metadata.base_href.as_deref()),
                    LinkResolution::SameDocument { .. }
                )
            })
        {
            coverage_issues.insert("canonical_target_unresolved");
        }
        observations.push((metadata, noindex, redirect_target));
    }
    if evidence
        .iter()
        .any(|link| link.context == LinkContext::Unknown)
    {
        issues.push("unclassified_link_context".to_string());
    }
    let links = count_links(evidence);
    if !links.indeterminate.is_empty() {
        issues.push("indeterminate_links".to_string());
    }
    let mut reconciliation = input.inventory.as_ref().map(|inventory| {
        reconcile(
            inventory,
            &CrawlSurface {
                identity: input.identity.clone(),
                complete: input.crawl_complete,
                pages: input.pages.iter().map(|page| page.url.clone()).collect(),
            },
        )
    });
    issues.extend(coverage_issues.iter().map(|issue| (*issue).to_string()));
    match reconciliation.as_mut() {
        Some(result) if !coverage_issues.is_empty() => {
            result.comparable = false;
            result.eligible_pages.clear();
            result
                .issues
                .extend(coverage_issues.iter().map(|issue| (*issue).to_string()));
            result.issues.sort();
            result.issues.dedup();
        }
        _ => {}
    }
    match &reconciliation {
        None => issues.push("inventory_missing".to_string()),
        Some(result) if !result.comparable => issues.push("inventory_not_comparable".to_string()),
        _ => {}
    }
    issues.sort();
    issues.dedup();
    let by_destination = links
        .destinations
        .iter()
        .map(|destination| (destination.destination.as_str(), destination))
        .collect::<std::collections::BTreeMap<_, _>>();
    let pages = input
        .pages
        .iter()
        .zip(observations)
        .map(|(page, (metadata, noindex, redirect_target))| {
            let inbound = by_destination.get(page.url.as_str());
            let mut contexts = [
                LinkContext::Body,
                LinkContext::RelatedContent,
                LinkContext::Navigation,
                LinkContext::Footer,
                LinkContext::Breadcrumb,
                LinkContext::Pagination,
                LinkContext::Unknown,
            ]
            .into_iter()
            .map(|context| (context, 0))
            .collect::<std::collections::BTreeMap<_, _>>();
            if let Some(destination) = inbound {
                contexts.extend(
                    destination
                        .source_pages_by_context
                        .iter()
                        .map(|(context, count)| (*context, *count)),
                );
            }
            ObservedPageLinks {
                url: page.url.clone(),
                status_code: page.status_code,
                response_headers_captured: page.response_headers.is_some(),
                noindex_observed: noindex,
                canonical_hrefs: metadata.canonical_hrefs,
                redirect_target,
                distinct_source_pages: inbound
                    .map_or(0, |destination| destination.distinct_source_pages),
                source_pages_by_context: contexts,
            }
        })
        .collect();
    Ok(LinkReport {
        version: 1,
        identity: input.identity.clone(),
        snapshot_date: input.snapshot_date.clone(),
        crawl_complete: input.crawl_complete,
        issues,
        reconciliation,
        pages,
        links,
    })
}

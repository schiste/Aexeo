//! Produce an evidence report from explicitly identified rendered pages.
use serde::{Deserialize, Serialize};
use url::Url;

pub use aexeo_contracts::RedirectHop;

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
    /// The URL fetched when it differs from `url`, if the producer captured it.
    #[serde(default)]
    pub requested_url: Option<String>,
    /// Redirect responses between `requested_url` and `url`, in request order,
    /// if the producer recorded them. Empty means none recorded, not none seen.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub redirect_chain: Vec<RedirectHop>,
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
    /// Sources with a link free of `nofollow`, `ugc` and `sponsored`. Zero while
    /// `distinct_source_pages` is positive means every observed link is qualified.
    #[serde(default)]
    pub followed_source_pages: usize,
    #[serde(default)]
    pub source_pages_by_rel: std::collections::BTreeMap<String, usize>,
    /// Observed destinations that differ from this page's URL only by a trailing
    /// slash, percent-encoding spelling or an empty query. They are not counted
    /// above: the URLs are not proven equivalent.
    #[serde(default)]
    pub near_variant_destinations: Vec<String>,
    /// The URL requested when the response came from this different URL.
    #[serde(default)]
    pub redirected_from: Option<String>,
    /// Other pages linking to `redirected_from`; not included in the counts above.
    #[serde(default)]
    pub redirected_source_pages: usize,
    /// The recorded redirect hops from `redirected_from` to this page, in
    /// order. Links to a hop are credited only in `via_redirect`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub redirect_chain: Vec<RedirectHop>,
    /// Other pages linking to the requested URL or to any hop of a recorded,
    /// consistent `redirect_chain`. Never added to the counts above and never
    /// a basis for eligibility. Omitted when nothing is credited.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub via_redirect: Option<RedirectCredit>,
}

/// Distinct source pages credited to a page through its recorded redirects.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RedirectCredit {
    pub distinct_source_pages: usize,
    pub source_pages_by_context: std::collections::BTreeMap<LinkContext, usize>,
}

fn has_noindex(value: &str) -> bool {
    value
        .split(|ch: char| ch.is_ascii_whitespace() || matches!(ch, ',' | ';' | ':'))
        .any(|part| part.eq_ignore_ascii_case("noindex") || part.eq_ignore_ascii_case("none"))
}

fn http_origin(value: &str) -> Option<String> {
    Url::parse(value)
        .ok()
        .filter(|url| {
            matches!(url.scheme(), "http" | "https")
                && url.username().is_empty()
                && url.password().is_none()
        })
        .map(|url| url.origin().ascii_serialization())
}

/// Spell percent escapes canonically: decode unreserved bytes, uppercase the rest.
fn normalize_percent_encoding(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut result = String::with_capacity(value.len());
    let mut index = 0;
    while index < bytes.len() {
        let escape = (bytes[index] == b'%')
            .then(|| value.get(index + 1..index + 3))
            .flatten()
            .and_then(|hex| u8::from_str_radix(hex, 16).ok().map(|byte| (hex, byte)));
        if let Some((hex, byte)) = escape {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
                result.push(byte as char);
            } else {
                result.push('%');
                result.push_str(&hex.to_ascii_uppercase());
            }
            index += 3;
        } else {
            let ch = value[index..].chars().next().unwrap_or_default();
            result.push(ch);
            index += ch.len_utf8();
        }
    }
    result
}

/// A comparison key shared by URLs differing only by a trailing slash, percent
/// spelling or an empty query. It detects likely mismatches and is never used
/// to credit links, since a server may treat those URLs as different pages.
fn near_variant_key(value: &str) -> Option<String> {
    let url = Url::parse(value).ok()?;
    let path = normalize_percent_encoding(url.path());
    let path = path.strip_suffix('/').unwrap_or(&path);
    let query = url
        .query()
        .filter(|query| !query.is_empty())
        .map(normalize_percent_encoding)
        .unwrap_or_default();
    Some(format!(
        "{}{path}?{query}",
        url.origin().ascii_serialization()
    ))
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
/// Crawl coverage is claimed only as `crawl_audit_completeness_issues` allows:
/// without a supplied inventory matching the crawl's seed, never.
pub fn build_link_report_from_audit(
    artifact: &aexeo_contracts::AuditArtifact,
) -> anyhow::Result<LinkReport> {
    build_link_report_from_audit_with_context(artifact, None)
}

/// Compare an audit with an explicitly supplied inventory. The caller's
/// identity does not prove a complete crawl or a coherent snapshot, and the
/// context never makes the crawl complete on its own.
pub fn build_link_report_from_audit_with_context(
    artifact: &aexeo_contracts::AuditArtifact,
    context: Option<&AuditLinkContext>,
) -> anyhow::Result<LinkReport> {
    build_link_report_from_audit_with_static_urls(artifact, context, None)
}

/// Analyze a static export using caller-provided relative-path to public-URL mappings.
/// The mappings are not observed HTTP responses and never establish coverage.
pub fn build_link_report_from_audit_with_static_urls(
    artifact: &aexeo_contracts::AuditArtifact,
    context: Option<&AuditLinkContext>,
    static_urls: Option<&std::collections::BTreeMap<String, String>>,
) -> anyhow::Result<LinkReport> {
    let site = artifact
        .site
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("audit artifact has no site snapshot"))?;
    anyhow::ensure!(
        static_urls.is_none() || site.deployment_model == "static_export",
        "static URL mapping requires a static_export audit artifact"
    );
    let mut origin = site
        .site_url
        .as_deref()
        .and_then(http_origin)
        .unwrap_or_default();
    let mut issues = std::collections::BTreeSet::new();
    // The crawl target can redirect to another origin, such as apex to www.
    // Observed URLs decide then, because links resolve against those pages.
    if static_urls.is_none() && !origin.is_empty() {
        let observed = site
            .pages
            .iter()
            .filter_map(|page| page.crawl_response.as_ref()?.observed_url.as_deref())
            .filter_map(http_origin)
            .collect::<std::collections::BTreeSet<_>>();
        let mut observed = observed.into_iter();
        match (observed.next(), observed.next()) {
            (Some(_), Some(_)) => {
                issues.insert("audit_observed_origins_ambiguous");
            }
            (Some(single), None) if single != origin => {
                issues.insert("audit_observed_origin_differs_from_target");
                origin = single;
            }
            _ => {}
        }
    }
    if let Some(identity) = context.and_then(|value| value.identity.as_ref()) {
        if origin.is_empty() {
            issues.insert("audit_origin_missing_or_invalid");
        } else if Url::parse(&identity.origin)
            .ok()
            .is_some_and(|declared| declared.origin().ascii_serialization() != origin)
        {
            issues.insert("audit_origin_mismatch");
        }
    }
    let mut pages: Vec<_> = site
        .pages
        .iter()
        .filter_map(|page| {
            if let Some(urls) = static_urls {
                let Some(url) = urls.get(&page.relative_path) else {
                    issues.insert("static_url_mapping_missing");
                    return None;
                };
                issues.insert("static_url_caller_supplied");
                return Some(RenderedPage {
                    url: url.clone(),
                    html: page.raw_html.clone(),
                    status_code: None,
                    response_headers: None,
                    requested_url: None,
                    redirect_chain: Vec::new(),
                });
            }
            let Some(response) = &page.crawl_response else {
                issues.insert("audit_page_response_provenance_missing");
                return None;
            };
            let Some(url) = response.observed_url.as_ref().filter(|url| !url.is_empty()) else {
                issues.insert("audit_page_observed_url_missing");
                return None;
            };
            let redirect_chain = recorded_redirect_chain(response, &mut issues);
            if response.requested_url != *url && redirect_chain.is_empty() {
                issues.insert("audit_redirect_chain_unavailable");
            }
            Some(RenderedPage {
                url: url.clone(),
                html: page.raw_html.clone(),
                status_code: response.status_code,
                response_headers: response
                    .headers_observed
                    .then(|| page.response_headers.clone()),
                requested_url: Some(response.requested_url.clone()),
                redirect_chain,
            })
        })
        .collect();
    // Non-2xx responses are not site pages but remain observations of the crawl.
    pages.extend(
        site.crawl_meta
            .iter()
            .flat_map(|meta| &meta.failed_responses)
            .map(|response| RenderedPage {
                url: response
                    .observed_url
                    .clone()
                    .unwrap_or_else(|| response.requested_url.clone()),
                html: String::new(),
                status_code: response.status_code,
                response_headers: None,
                requested_url: Some(response.requested_url.clone()),
                redirect_chain: recorded_redirect_chain(response, &mut issues),
            }),
    );
    // A static export is not a crawl: neither its coverage nor its release
    // is ever attested.
    let attestation = artifact
        .crawl_attestation
        .as_ref()
        .filter(|_| static_urls.is_none());
    let declared_release = context
        .and_then(|value| value.identity.as_ref())
        .map(|identity| identity.release.trim())
        .filter(|release| !release.is_empty());
    let (observed_release, release_issues) = crawl_release(
        attestation.and_then(|value| value.release.as_ref()),
        declared_release,
    );
    issues.extend(release_issues.iter().copied());
    let crawl_complete = static_urls.is_none() && {
        let incomplete = crawl_audit_completeness_issues(
            artifact,
            context.and_then(|value| value.inventory.as_ref()),
        );
        let complete = incomplete.is_empty();
        issues.extend(incomplete);
        complete
    };
    let mut identity =
        context
            .and_then(|value| value.identity.clone())
            .unwrap_or(SurfaceIdentity {
                origin,
                release: String::new(),
                surface: String::new(),
                snapshot: String::new(),
            });
    // The release the pages served outranks the one the caller declared, so
    // reconciliation compares the inventory's release with what was crawled.
    if let Some(release) = observed_release {
        identity.release = release;
    }
    let input = LinkReportInput {
        version: 1,
        identity: Some(identity),
        snapshot_date: context.and_then(|value| value.snapshot_date.clone()),
        crawl_complete,
        pages,
        inventory: context.and_then(|value| value.inventory.clone()),
    };
    let mut report = build_link_report(&input)?;
    // An unconfirmed release, like a coverage gap, keeps every page ineligible.
    if let Some(result) = report
        .reconciliation
        .as_mut()
        .filter(|_| !release_issues.is_empty())
    {
        result.comparable = false;
        result.eligible_pages.clear();
        result
            .issues
            .extend(release_issues.iter().map(|issue| (*issue).to_string()));
        result.issues.sort();
        result.issues.dedup();
        issues.insert("inventory_not_comparable");
    }
    report.issues.extend(issues.into_iter().map(str::to_string));
    report.issues.sort();
    report.issues.dedup();
    Ok(report)
}

/// The release the crawl attests, and why it does not when one was asked for.
/// A crawl attests a release only when every fetched HTML page served the
/// same marker; a declared release that differs from it is a mismatch.
/// Without a requested marker there is nothing to report.
fn crawl_release(
    release: Option<&aexeo_contracts::ReleaseAttestation>,
    declared: Option<&str>,
) -> (Option<String>, Vec<&'static str>) {
    let Some(release) = release else {
        return (None, Vec::new());
    };
    let mut issues = Vec::new();
    let observed = release
        .value
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    match observed {
        Some(value) if declared.is_some_and(|declared| declared != value) => {
            issues.push("audit_release_mismatch");
        }
        Some(_) => {}
        None => {
            let inconsistent = release
                .distinct_value_count
                .unwrap_or(release.distinct_values.len())
                > 1;
            if inconsistent {
                issues.push("audit_release_inconsistent");
            }
            // Anything else that leaves no value, such as a page without a
            // marker, means the release is not attested.
            if !inconsistent || release.pages_without_marker > 0 {
                issues.push("audit_release_unattested");
            }
        }
    }
    (observed.map(str::to_string), issues)
}

/// Why a runtime audit does not attest complete crawl coverage, one issue per
/// unmet condition; empty only when every condition holds. The crawl must
/// have finished complete and untruncated with the HTTP engine (which records
/// redirect chains), with no failed response and no URL skipped by robots.txt.
/// It must have been seeded from an inventory, every seeded URL must have a
/// page or response, and `inventory`, the one used for reconciliation, must
/// carry the seed inventory's identity.
fn crawl_audit_completeness_issues(
    artifact: &aexeo_contracts::AuditArtifact,
    inventory: Option<&InventorySnapshot>,
) -> Vec<&'static str> {
    let mut issues = Vec::new();
    let crawl = artifact.crawl.as_ref();
    let meta = artifact
        .site
        .as_ref()
        .and_then(|site| site.crawl_meta.as_ref());
    let attestation = artifact.crawl_attestation.as_ref();
    if artifact.status != aexeo_contracts::AuditStatus::Complete
        || artifact.truncation_reason.is_some()
        || crawl.is_none_or(|crawl| crawl.truncated)
        || meta.is_some_and(|meta| meta.truncated)
    {
        issues.push("audit_crawl_partial");
    }
    let drafts = inventory
        .iter()
        .flat_map(|inventory| &inventory.pages)
        .filter(|page| page.disposition == InventoryDisposition::Draft)
        .filter_map(|page| Url::parse(&page.url).ok().map(String::from))
        .collect::<std::collections::BTreeSet<_>>();
    if meta.is_some_and(|meta| {
        meta.failed_responses
            .iter()
            .any(|response| !unserved_draft(response, &drafts))
    }) {
        issues.push("audit_failed_responses");
    }
    if crawl.is_some_and(|crawl| crawl.skipped_by_robots > 0)
        || meta.is_some_and(|meta| !meta.robots_skipped_urls.is_empty())
        || attestation.is_some_and(|value| !value.robots_skipped_urls.is_empty())
    {
        issues.push("audit_robots_skipped");
    }
    if crawl.is_some_and(|crawl| crawl.engine != "http")
        || attestation.is_some_and(|value| value.engine != "http")
    {
        issues.push("audit_browser_engine");
    }
    match attestation.filter(|value| value.seeded_from.url_count > 0) {
        None => issues.push("audit_not_inventory_seeded"),
        Some(attestation) => {
            if !attestation.unfetched_urls.is_empty() {
                issues.push("audit_seed_unfetched");
            }
            let seeded = attestation.seeded_from.inventory_identity.as_ref();
            let reconciled = inventory.and_then(|inventory| inventory.identity.as_ref());
            if seeded.is_none() || seeded != reconciled {
                issues.push("audit_seed_inventory_mismatch");
            }
        }
    }
    issues
}

/// A draft is not published, so a 404 or 410 for it is the expected answer
/// rather than a failure. Any other status keeps its usual treatment.
fn draft_absence_expected(disposition: Option<InventoryDisposition>, status: u16) -> bool {
    disposition == Some(InventoryDisposition::Draft) && matches!(status, 404 | 410)
}

/// A failed response that is a draft answering 404 or 410 directly, with no
/// redirect, at the exact URL the inventory lists.
fn unserved_draft(
    response: &aexeo_contracts::CrawlResponseProvenance,
    drafts: &std::collections::BTreeSet<String>,
) -> bool {
    let parse = |url: &str| Url::parse(url).ok().map(String::from);
    let requested = parse(&response.requested_url);
    response.redirect_chain.is_empty()
        && response
            .observed_url
            .as_deref()
            .is_none_or(|observed| parse(observed) == requested)
        && requested.is_some_and(|url| drafts.contains(&url))
        && response
            .status_code
            .is_some_and(|status| draft_absence_expected(Some(InventoryDisposition::Draft), status))
}

/// The crawl's recorded hops, when they start at the requested URL and every
/// hop is a redirect status. A chain failing that is reported and dropped
/// rather than shown as evidence.
fn recorded_redirect_chain(
    response: &aexeo_contracts::CrawlResponseProvenance,
    issues: &mut std::collections::BTreeSet<&'static str>,
) -> Vec<RedirectHop> {
    let chain = &response.redirect_chain;
    if chain.is_empty() {
        return Vec::new();
    }
    if redirect_chain_consistent(&response.requested_url, chain) {
        chain.clone()
    } else {
        issues.insert("audit_redirect_chain_inconsistent");
        Vec::new()
    }
}

/// A chain is evidence only when it starts at the requested URL and every hop
/// is a redirect status.
fn redirect_chain_consistent(requested_url: &str, chain: &[RedirectHop]) -> bool {
    chain.first().is_some_and(|hop| hop.url == requested_url)
        && chain
            .iter()
            .all(|hop| (300..400).contains(&hop.status_code))
}

/// Sources linking to any URL of the page's recorded, consistent redirect
/// chain, the requested URL included. Without such a chain nothing is credited.
fn redirect_credit(
    page: &RenderedPage,
    by_destination: &std::collections::BTreeMap<&str, &crate::link_counts::DestinationLinks>,
) -> Option<RedirectCredit> {
    let requested = page.requested_url.as_deref()?;
    if !redirect_chain_consistent(requested, &page.redirect_chain) {
        return None;
    }
    let own = Url::parse(&page.url).ok().map(String::from);
    let chain_urls = page
        .redirect_chain
        .iter()
        .filter_map(|hop| Url::parse(&hop.url).ok().map(String::from))
        .filter(|url| own.as_ref() != Some(url))
        .collect::<std::collections::BTreeSet<_>>();
    let mut sources = std::collections::BTreeSet::new();
    let mut contexts =
        std::collections::BTreeMap::<LinkContext, std::collections::BTreeSet<&str>>::new();
    for link in chain_urls
        .iter()
        .filter_map(|url| by_destination.get(url.as_str()))
        .flat_map(|destination| &destination.evidence)
        .filter(|link| link.source != page.url)
    {
        sources.insert(link.source.as_str());
        contexts
            .entry(link.context)
            .or_default()
            .insert(link.source.as_str());
    }
    (!sources.is_empty()).then(|| RedirectCredit {
        distinct_source_pages: sources.len(),
        source_pages_by_context: contexts
            .into_iter()
            .map(|(context, sources)| (context, sources.len()))
            .collect(),
    })
}

/// The serialized requested URL, when it differs from the observed page URL.
fn redirected_from(page: &RenderedPage) -> Option<String> {
    let requested = String::from(Url::parse(page.requested_url.as_deref()?).ok()?);
    (Url::parse(&page.url).ok().map(String::from).as_ref() != Some(&requested)).then_some(requested)
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
    let inventory_dispositions = input
        .inventory
        .iter()
        .flat_map(|inventory| &inventory.pages)
        .map(|entry| (entry.url.as_str(), entry.disposition))
        .collect::<std::collections::BTreeMap<_, _>>();
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
        // An unserved draft is the expected outcome; its headers cannot make
        // it indexable, so neither its status nor missing headers is a gap.
        let unserved_draft = page.status_code.is_some_and(|status| {
            draft_absence_expected(
                inventory_dispositions.get(page.url.as_str()).copied(),
                status,
            )
        });
        if headers.is_none() && !unserved_draft {
            coverage_issues.insert("response_headers_missing");
        }
        match page.status_code {
            None => {
                coverage_issues.insert("response_status_missing");
            }
            Some(status) if (300..400).contains(&status) => {
                coverage_issues.insert("redirect_observed");
            }
            Some(status) if !(200..300).contains(&status) && !unserved_draft => {
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
        }
        // Without captured headers, an X-Robots-Tag noindex cannot be ruled out.
        let served_indexable = !noindex
            && headers.is_some()
            && page
                .status_code
                .is_some_and(|status| (200..300).contains(&status));
        match inventory_dispositions.get(page.url.as_str()) {
            Some(InventoryDisposition::Published) if noindex => {
                coverage_issues.insert("inventory_indexability_mismatch");
            }
            Some(InventoryDisposition::Draft | InventoryDisposition::Noindex)
                if served_indexable =>
            {
                coverage_issues.insert("inventory_indexability_mismatch");
            }
            _ => {}
        }
        if redirected_from(page).is_some() || !page.redirect_chain.is_empty() {
            coverage_issues.insert("redirect_observed");
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
    let observed_urls = input
        .pages
        .iter()
        .filter_map(|page| Url::parse(&page.url).ok().map(String::from))
        .collect::<std::collections::BTreeSet<_>>();
    let observed_keys = observed_urls
        .iter()
        .filter_map(|url| near_variant_key(url))
        .collect::<std::collections::BTreeSet<_>>();
    let mut near_variants = std::collections::BTreeMap::<String, Vec<String>>::new();
    for destination in &links.destinations {
        if observed_urls.contains(&destination.destination) {
            continue;
        }
        if let Some(key) =
            near_variant_key(&destination.destination).filter(|key| observed_keys.contains(key))
        {
            near_variants
                .entry(key)
                .or_default()
                .push(destination.destination.clone());
            coverage_issues.insert("destination_unmatched_near_variant");
        }
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
            // Link resolution serializes parsed URLs (normalizing host case and
            // default ports), while page observations preserve the input spelling.
            let normalized_url = Url::parse(&page.url).ok().map(String::from);
            let inbound = normalized_url
                .as_deref()
                .and_then(|url| by_destination.get(url));
            let redirected = redirected_from(page);
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
                followed_source_pages: inbound
                    .map_or(0, |destination| destination.followed_source_pages),
                source_pages_by_rel: inbound
                    .map(|destination| destination.source_pages_by_rel.clone())
                    .unwrap_or_default(),
                near_variant_destinations: near_variant_key(&page.url)
                    .and_then(|key| near_variants.get(&key).cloned())
                    .unwrap_or_default(),
                redirected_source_pages: redirected
                    .as_deref()
                    .and_then(|url| by_destination.get(url))
                    .map_or(0, |destination| {
                        destination
                            .evidence
                            .iter()
                            .map(|link| link.source.as_str())
                            .filter(|source| *source != page.url)
                            .collect::<std::collections::BTreeSet<_>>()
                            .len()
                    }),
                redirected_from: redirected,
                redirect_chain: page.redirect_chain.clone(),
                via_redirect: redirect_credit(page, &by_destination),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_observed_pages_using_resolved_url_serialization() {
        let report = build_link_report(&LinkReportInput {
            version: 1,
            identity: Some(SurfaceIdentity {
                origin: "https://example.org".into(),
                release: "r1".into(),
                surface: "public".into(),
                snapshot: "s1".into(),
            }),
            snapshot_date: Some("2026-11-06".into()),
            crawl_complete: false,
            pages: vec![
                RenderedPage {
                    url: "https://example.org/source".into(),
                    html: "<main><a href='/target?lang=fr'>link</a></main>".into(),
                    status_code: Some(200),
                    response_headers: Some(Default::default()),
                    requested_url: None,
                    redirect_chain: Vec::new(),
                },
                RenderedPage {
                    url: "https://EXAMPLE.org:443/target?lang=fr".into(),
                    html: String::new(),
                    status_code: Some(200),
                    response_headers: Some(Default::default()),
                    requested_url: None,
                    redirect_chain: Vec::new(),
                },
                RenderedPage {
                    url: "https://example.org/target?lang=en".into(),
                    html: String::new(),
                    status_code: Some(200),
                    response_headers: Some(Default::default()),
                    requested_url: None,
                    redirect_chain: Vec::new(),
                },
            ],
            inventory: None,
        })
        .unwrap();
        assert_eq!(
            report.pages[1].url,
            "https://EXAMPLE.org:443/target?lang=fr"
        );
        assert_eq!(report.pages[1].distinct_source_pages, 1);
        assert_eq!(
            report.pages[1].source_pages_by_context[&LinkContext::Body],
            1
        );
        assert_eq!(report.pages[2].distinct_source_pages, 0);
    }

    fn redirected_audit(chain: Option<serde_json::Value>) -> aexeo_contracts::AuditArtifact {
        let mut response = serde_json::json!({"requested_url":"https://example.org/old", "observed_url":"https://example.org/new", "status_code":200, "headers_observed":true});
        if let Some(chain) = chain {
            response["redirect_chain"] = chain;
        }
        serde_json::from_value(serde_json::json!({"command":"crawl", "site":{"site_url":"https://example.org/", "root":"crawl", "deployment_model":"runtime_snapshot", "pages":[
            {"path":"crawl/index.html", "relative_path":"index.html", "route":"", "raw_html":"<main><a href='/old'>Old</a><a href='/moved'>Moved</a></main>", "response_headers":{}, "crawl_response":{"requested_url":"https://example.org/", "observed_url":"https://example.org/", "status_code":200, "headers_observed":true}},
            {"path":"crawl/new/index.html", "relative_path":"new/index.html", "route":"new", "raw_html":"<main>New</main>", "response_headers":{}, "crawl_response":response},
        ]}}))
        .unwrap()
    }

    #[test]
    fn a_recorded_redirect_chain_replaces_the_unavailable_issue() {
        let report = build_link_report_from_audit(&redirected_audit(Some(serde_json::json!([
            {"url":"https://example.org/old", "status_code":301},
            {"url":"https://example.org/moved", "status_code":302},
        ]))))
        .unwrap();
        let page = &report.pages[1];
        assert_eq!(
            page.redirect_chain,
            [
                RedirectHop {
                    url: "https://example.org/old".into(),
                    status_code: 301
                },
                RedirectHop {
                    url: "https://example.org/moved".into(),
                    status_code: 302
                },
            ]
        );
        // Links to the requested URL and to an intermediate hop stay outside
        // the page's own counts and are credited only through the redirect.
        assert_eq!(page.distinct_source_pages, 0);
        assert_eq!(
            page.via_redirect
                .as_ref()
                .map(|credit| credit.distinct_source_pages),
            Some(1)
        );
        assert_eq!(
            page.redirected_from.as_deref(),
            Some("https://example.org/old")
        );
        assert_eq!(page.redirected_source_pages, 1);
        assert!(report.pages[0].redirect_chain.is_empty());
        assert!(
            !report
                .issues
                .contains(&"audit_redirect_chain_unavailable".to_string())
        );
        assert!(report.issues.contains(&"redirect_observed".to_string()));
    }

    #[test]
    fn an_artifact_without_recorded_hops_keeps_the_unavailable_issue() {
        let report = build_link_report_from_audit(&redirected_audit(None)).unwrap();
        assert!(report.pages[1].redirect_chain.is_empty());
        assert!(report.pages[1].via_redirect.is_none());
        assert!(
            report
                .issues
                .contains(&"audit_redirect_chain_unavailable".to_string())
        );
        let page = serde_json::to_value(&report.pages[1]).unwrap();
        assert!(page.get("redirect_chain").is_none(), "{page}");
    }

    #[test]
    fn a_chain_not_starting_at_the_requested_url_is_not_evidence() {
        let report = build_link_report_from_audit(&redirected_audit(Some(serde_json::json!([
            {"url":"https://example.org/elsewhere", "status_code":301},
        ]))))
        .unwrap();
        assert!(report.pages[1].redirect_chain.is_empty());
        assert!(report.pages[1].via_redirect.is_none());
        for issue in [
            "audit_redirect_chain_inconsistent",
            "audit_redirect_chain_unavailable",
        ] {
            assert!(report.issues.contains(&issue.to_string()), "{issue}");
        }
    }

    const ATTESTED_ISSUES: [&str; 7] = [
        "audit_browser_engine",
        "audit_crawl_partial",
        "audit_failed_responses",
        "audit_not_inventory_seeded",
        "audit_robots_skipped",
        "audit_seed_inventory_mismatch",
        "audit_seed_unfetched",
    ];

    fn attested_identity() -> serde_json::Value {
        serde_json::json!({"origin":"https://example.org", "release":"r1", "surface":"public", "snapshot":"s1"})
    }

    /// A complete HTTP crawl seeded from the context inventory; `/orphan` was
    /// reached only through the seed.
    fn attested_audit() -> (serde_json::Value, AuditLinkContext) {
        let page = |route: &str, html: &str| {
            let url = format!("https://example.org/{route}");
            serde_json::json!({"path":"", "relative_path":"", "route":route, "raw_html":html, "response_headers":{},
                "crawl_response":{"requested_url":url, "observed_url":url, "status_code":200, "headers_observed":true}})
        };
        let artifact = serde_json::json!({"command":"crawl", "status":"complete",
            "crawl":{"engine":"http", "visited_pages":3, "discovered_internal_routes":3, "queued_routes_remaining":0, "max_pages":10, "fetch_failures":0, "fetch_retries":0, "skipped_non_html":0, "truncated":false, "elapsed_ms":0, "pages_per_minute":0, "checkpoints_written":0, "partial_artifacts_written":0, "total_fetch_ms":0, "average_fetch_ms":0, "total_page_process_ms":0, "average_page_process_ms":0, "total_partial_audit_ms":0, "average_partial_audit_ms":0, "slowest_paths":[]},
            "site":{"site_url":"https://example.org/", "root":"crawl", "deployment_model":"runtime_snapshot",
                "crawl_meta":{"visited_pages":3, "max_pages":10, "discovered_internal_routes":3, "truncated":false},
                "pages":[page("", "<main><a href='/a'>A</a></main>"), page("a", "<main>A</main>"), page("orphan", "<main>Orphan</main>")]},
            "crawl_attestation":{"seeded_from":{"inventory_identity":attested_identity(), "url_count":3}, "engine":"http"}});
        let inventory_pages = ["", "a", "orphan"].map(|route| {
            serde_json::json!({"url":format!("https://example.org/{route}"), "disposition":"published", "target":null, "locale":null})
        });
        let context = serde_json::from_value(serde_json::json!({"identity":attested_identity(), "snapshot_date":"2026-11-07",
            "inventory":{"identity":attested_identity(), "complete":true, "pages":inventory_pages}}))
        .unwrap();
        (artifact, context)
    }

    /// Breaks one completeness condition of `attested_audit`.
    type Mutation = fn(&mut serde_json::Value, &mut AuditLinkContext);

    fn attested_report(artifact: &serde_json::Value, context: &AuditLinkContext) -> LinkReport {
        build_link_report_from_audit_with_context(
            &serde_json::from_value(artifact.clone()).unwrap(),
            Some(context),
        )
        .unwrap()
    }

    #[test]
    fn a_seeded_complete_http_crawl_attests_coverage_and_reaches_eligibility() {
        let (artifact, context) = attested_audit();
        let report = attested_report(&artifact, &context);
        assert!(report.crawl_complete, "{:?}", report.issues);
        for issue in ATTESTED_ISSUES.iter().chain(&["crawl_incomplete"]) {
            assert!(!report.issues.iter().any(|value| value == issue), "{issue}");
        }
        let reconciliation = report.reconciliation.as_ref().unwrap();
        assert!(reconciliation.comparable, "{:?}", reconciliation.issues);
        assert!(
            reconciliation
                .eligible_pages
                .contains(&"https://example.org/orphan".to_string())
        );
        let orphan = &report.pages[2];
        assert_eq!(orphan.url, "https://example.org/orphan");
        assert_eq!(orphan.distinct_source_pages, 0);
    }

    #[test]
    fn each_unmet_condition_raises_exactly_its_own_issue() {
        let cases: [(&str, Mutation); 9] = [
            ("audit_crawl_partial", |artifact, _| {
                artifact["status"] = "partial".into()
            }),
            ("audit_crawl_partial", |artifact, _| {
                artifact["crawl"]["truncated"] = true.into()
            }),
            ("audit_failed_responses", |artifact, _| {
                artifact["site"]["crawl_meta"]["failed_responses"] = serde_json::json!([{"requested_url":"https://example.org/gone", "status_code":404}]);
            }),
            ("audit_robots_skipped", |artifact, _| {
                artifact["site"]["crawl_meta"]["robots_skipped_urls"] =
                    serde_json::json!(["https://example.org/private"]);
            }),
            ("audit_browser_engine", |artifact, _| {
                artifact["crawl"]["engine"] = "playwright".into();
                artifact["crawl_attestation"]["engine"] = "playwright".into();
            }),
            ("audit_not_inventory_seeded", |artifact, _| {
                artifact
                    .as_object_mut()
                    .unwrap()
                    .remove("crawl_attestation");
            }),
            ("audit_not_inventory_seeded", |artifact, _| {
                artifact["crawl_attestation"]["seeded_from"]["url_count"] = 0.into();
            }),
            ("audit_seed_unfetched", |artifact, _| {
                artifact["crawl_attestation"]["unfetched_urls"] =
                    serde_json::json!(["https://example.org/b"]);
            }),
            ("audit_seed_inventory_mismatch", |_, context| {
                context
                    .inventory
                    .as_mut()
                    .unwrap()
                    .identity
                    .as_mut()
                    .unwrap()
                    .snapshot = "s2".into();
            }),
        ];
        for (expected, mutate) in cases {
            let (mut artifact, mut context) = attested_audit();
            mutate(&mut artifact, &mut context);
            let report = attested_report(&artifact, &context);
            assert!(!report.crawl_complete, "{expected}");
            assert!(report.issues.contains(&"crawl_incomplete".to_string()));
            let raised = ATTESTED_ISSUES
                .iter()
                .filter(|issue| report.issues.iter().any(|value| value == *issue))
                .copied()
                .collect::<Vec<_>>();
            assert_eq!(raised, [expected], "{:?}", report.issues);
            assert!(
                report
                    .reconciliation
                    .as_ref()
                    .unwrap()
                    .eligible_pages
                    .is_empty()
            );
        }
    }

    /// `attested_audit` with a seeded draft at `/draft` answering `status`.
    fn audit_with_draft(status: u16) -> (serde_json::Value, AuditLinkContext) {
        let (mut artifact, mut context) = attested_audit();
        artifact["site"]["crawl_meta"]["failed_responses"] = serde_json::json!([
            {"requested_url":"https://example.org/draft", "observed_url":"https://example.org/draft", "status_code":status, "headers_observed":true}
        ]);
        artifact["crawl_attestation"]["seeded_from"]["url_count"] = 4.into();
        context
            .inventory
            .as_mut()
            .unwrap()
            .pages
            .push(crate::link_inventory::InventoryPage {
                url: "https://example.org/draft".into(),
                disposition: InventoryDisposition::Draft,
                target: None,
                locale: None,
            });
        (artifact, context)
    }

    #[test]
    fn a_draft_answering_404_or_410_keeps_the_crawl_complete_and_eligible() {
        for status in [404, 410] {
            let (artifact, context) = audit_with_draft(status);
            let report = attested_report(&artifact, &context);
            assert!(report.crawl_complete, "{status}: {:?}", report.issues);
            assert!(
                !report
                    .issues
                    .iter()
                    .any(|issue| issue.starts_with("audit_"))
            );
            assert!(!report.issues.contains(&"non_success_response".to_string()));
            let reconciliation = report.reconciliation.as_ref().unwrap();
            assert!(reconciliation.comparable, "{:?}", reconciliation.issues);
            assert!(
                reconciliation
                    .eligible_pages
                    .contains(&"https://example.org/orphan".to_string())
            );
        }
    }

    #[test]
    fn a_draft_answering_another_error_or_a_published_page_answering_404_fails() {
        for status in [500, 403] {
            let (artifact, context) = audit_with_draft(status);
            let report = attested_report(&artifact, &context);
            assert!(!report.crawl_complete, "{status}");
            assert!(
                report
                    .issues
                    .contains(&"audit_failed_responses".to_string())
            );
        }
        // The exemption follows the inventory disposition, not the status alone.
        let (artifact, mut context) = audit_with_draft(404);
        context.inventory.as_mut().unwrap().pages[3].disposition = InventoryDisposition::Published;
        let report = attested_report(&artifact, &context);
        assert!(!report.crawl_complete);
        assert!(
            report
                .issues
                .contains(&"audit_failed_responses".to_string())
        );
        // So does a draft reached only through a redirect.
        let (mut artifact, context) = audit_with_draft(404);
        artifact["site"]["crawl_meta"]["failed_responses"][0]["observed_url"] =
            "https://example.org/elsewhere".into();
        let report = attested_report(&artifact, &context);
        assert!(
            report
                .issues
                .contains(&"audit_failed_responses".to_string())
        );
    }

    #[test]
    fn a_context_never_makes_an_unseeded_or_mismatched_crawl_complete() {
        let (mut artifact, context) = attested_audit();
        // A context identical to the seed inventory, but no seed recorded.
        artifact
            .as_object_mut()
            .unwrap()
            .remove("crawl_attestation");
        assert!(!attested_report(&artifact, &context).crawl_complete);
        // A seed recorded, but no inventory to reconcile against.
        let (artifact, mut context) = attested_audit();
        context.inventory = None;
        let report = attested_report(&artifact, &context);
        assert!(!report.crawl_complete);
        assert!(
            report
                .issues
                .contains(&"audit_seed_inventory_mismatch".to_string())
        );
        // A seed inventory without identity matches nothing, even another one without.
        let (mut artifact, mut context) = attested_audit();
        artifact["crawl_attestation"]["seeded_from"]["inventory_identity"] =
            serde_json::Value::Null;
        context.inventory.as_mut().unwrap().identity = None;
        let report = attested_report(&artifact, &context);
        assert!(!report.crawl_complete);
        assert!(
            report
                .issues
                .contains(&"audit_seed_inventory_mismatch".to_string())
        );
        // Without any context the crawl stays incomplete too.
        let (artifact, _) = attested_audit();
        let report =
            build_link_report_from_audit(&serde_json::from_value(artifact).unwrap()).unwrap();
        assert!(!report.crawl_complete);
    }

    /// `attested_audit` whose crawl recorded `release`.
    fn audit_with_release(release: serde_json::Value) -> (serde_json::Value, AuditLinkContext) {
        let (mut artifact, context) = attested_audit();
        artifact["crawl_attestation"]["release"] = release;
        (artifact, context)
    }

    fn release(
        value: Option<&str>,
        with: usize,
        without: usize,
        distinct: &[&str],
    ) -> serde_json::Value {
        serde_json::json!({"source":"header", "name":"x-release", "value":value,
            "pages_with_marker":with, "pages_without_marker":without, "distinct_values":distinct})
    }

    const RELEASE_ISSUES: [&str; 3] = [
        "audit_release_inconsistent",
        "audit_release_mismatch",
        "audit_release_unattested",
    ];

    #[test]
    fn an_attested_release_fills_the_identity_and_keeps_eligibility() {
        let (artifact, mut context) = audit_with_release(release(Some("r1"), 3, 0, &["r1"]));
        context.identity.as_mut().unwrap().release = String::new();
        let report = attested_report(&artifact, &context);
        assert_eq!(report.identity.as_ref().unwrap().release, "r1");
        assert!(report.crawl_complete, "{:?}", report.issues);
        assert!(
            !report
                .issues
                .iter()
                .any(|issue| RELEASE_ISSUES.contains(&issue.as_str()))
        );
        let reconciliation = report.reconciliation.as_ref().unwrap();
        assert!(reconciliation.comparable, "{:?}", reconciliation.issues);
        assert!(!reconciliation.eligible_pages.is_empty());
        // An attested release never makes an unseeded crawl complete.
        let (mut artifact, context) = audit_with_release(release(Some("r1"), 3, 0, &["r1"]));
        artifact["crawl_attestation"]["seeded_from"]["url_count"] = 0.into();
        let report = attested_report(&artifact, &context);
        assert!(!report.crawl_complete);
        assert!(
            report
                .issues
                .contains(&"audit_not_inventory_seeded".to_string())
        );
    }

    #[test]
    fn an_unconfirmed_release_raises_its_issue_and_keeps_pages_ineligible() {
        let cases: [(&str, Mutation); 4] = [
            ("audit_release_inconsistent", |artifact, _| {
                artifact["crawl_attestation"]["release"] = release(None, 3, 0, &["r1", "r2"]);
            }),
            ("audit_release_unattested", |artifact, _| {
                artifact["crawl_attestation"]["release"] = release(None, 2, 1, &["r1"]);
            }),
            ("audit_release_unattested", |artifact, _| {
                artifact["crawl_attestation"]["release"] = release(None, 0, 3, &[]);
            }),
            ("audit_release_mismatch", |_, context| {
                context.identity.as_mut().unwrap().release = "r0".into();
            }),
        ];
        for (expected, mutate) in cases {
            let (mut artifact, mut context) =
                audit_with_release(release(Some("r1"), 3, 0, &["r1"]));
            mutate(&mut artifact, &mut context);
            let report = attested_report(&artifact, &context);
            let raised = RELEASE_ISSUES
                .iter()
                .filter(|issue| report.issues.iter().any(|value| value == *issue))
                .copied()
                .collect::<Vec<_>>();
            assert_eq!(raised, [expected], "{:?}", report.issues);
            // Coverage is a separate question; the release only withholds eligibility.
            assert!(report.crawl_complete, "{expected}: {:?}", report.issues);
            let reconciliation = report.reconciliation.as_ref().unwrap();
            assert!(!reconciliation.comparable, "{expected}");
            assert!(reconciliation.eligible_pages.is_empty());
            assert!(reconciliation.issues.contains(&expected.to_string()));
            assert!(
                report
                    .issues
                    .contains(&"inventory_not_comparable".to_string())
            );
        }
    }

    #[test]
    fn the_crawled_release_wins_and_is_compared_with_the_inventory() {
        // The caller declares r0, the pages serve r1: r1 is the report's release.
        let (artifact, mut context) = audit_with_release(release(Some("r1"), 3, 0, &["r1"]));
        context.identity.as_mut().unwrap().release = "r0".into();
        let report = attested_report(&artifact, &context);
        assert_eq!(report.identity.as_ref().unwrap().release, "r1");
        // An inventory of another release does not reconcile with the crawl,
        // even when the caller declares that release too.
        let (artifact, mut context) = audit_with_release(release(Some("r1"), 3, 0, &["r1"]));
        context
            .inventory
            .as_mut()
            .unwrap()
            .identity
            .as_mut()
            .unwrap()
            .release = "r0".into();
        context.identity.as_mut().unwrap().release = "r0".into();
        let report = attested_report(&artifact, &context);
        let reconciliation = report.reconciliation.as_ref().unwrap();
        assert!(
            reconciliation
                .issues
                .contains(&"surface_identity_mismatch".to_string())
        );
        assert!(reconciliation.eligible_pages.is_empty());
        // Without a crawled release, the declared one is compared as before.
        let (mut artifact, mut context) = attested_audit();
        artifact["crawl_attestation"]["seeded_from"]["inventory_identity"]["release"] = "r0".into();
        context
            .inventory
            .as_mut()
            .unwrap()
            .identity
            .as_mut()
            .unwrap()
            .release = "r0".into();
        let report = attested_report(&artifact, &context);
        let reconciliation = report.reconciliation.as_ref().unwrap();
        assert!(
            reconciliation
                .issues
                .contains(&"surface_identity_mismatch".to_string())
        );
        assert!(
            !report
                .issues
                .iter()
                .any(|issue| RELEASE_ISSUES.contains(&issue.as_str()))
        );
    }

    fn redirect_report(chain: Vec<RedirectHop>) -> LinkReport {
        let page = |url: &str, html: &str| RenderedPage {
            url: url.into(),
            html: html.into(),
            status_code: Some(200),
            response_headers: Some(Default::default()),
            requested_url: Some(url.into()),
            redirect_chain: Vec::new(),
        };
        let mut redirected = page(
            "https://example.org/new",
            "<main><a href='/old'>Self through redirect</a></main>",
        );
        redirected.requested_url = Some("https://example.org/old".into());
        redirected.redirect_chain = chain;
        build_link_report(&LinkReportInput {
            version: 1,
            identity: Some(SurfaceIdentity {
                origin: "https://example.org".into(),
                release: "r1".into(),
                surface: "public".into(),
                snapshot: "s1".into(),
            }),
            snapshot_date: Some("2026-11-07".into()),
            crawl_complete: true,
            pages: vec![
                page(
                    "https://example.org/a",
                    "<main><a href='/old'>Requested</a></main>",
                ),
                page(
                    "https://example.org/b",
                    "<nav><a href='/middle'>Hop</a></nav><main><a href='/old'>Requested</a></main>",
                ),
                page(
                    "https://example.org/c",
                    "<nav><a href='/middle'>Hop</a></nav>",
                ),
                redirected,
            ],
            // An otherwise matching, complete inventory: only the redirect
            // withholds eligibility.
            inventory: Some(InventorySnapshot {
                identity: Some(SurfaceIdentity {
                    origin: "https://example.org".into(),
                    release: "r1".into(),
                    surface: "public".into(),
                    snapshot: "s1".into(),
                }),
                complete: true,
                pages: ["a", "b", "c", "new"]
                    .map(|path| crate::link_inventory::InventoryPage {
                        url: format!("https://example.org/{path}"),
                        disposition: InventoryDisposition::Published,
                        target: None,
                        locale: None,
                    })
                    .to_vec(),
            }),
        })
        .unwrap()
    }

    fn hop(url: &str, status_code: u16) -> RedirectHop {
        RedirectHop {
            url: url.into(),
            status_code,
        }
    }

    #[test]
    fn links_to_a_consistent_redirect_chain_are_credited_separately() {
        let report = redirect_report(vec![
            hop("https://example.org/old", 301),
            hop("https://example.org/middle", 302),
        ]);
        let page = &report.pages[3];
        let credit = page.via_redirect.as_ref().expect("redirect credit");
        // a and b link to the requested URL, b and c to the intermediate hop;
        // the page's own link through the redirect is not credited.
        assert_eq!(credit.distinct_source_pages, 3);
        assert_eq!(credit.source_pages_by_context[&LinkContext::Body], 2);
        assert_eq!(credit.source_pages_by_context[&LinkContext::Navigation], 2);
        // The page's own counts and the requested-URL count are unchanged.
        assert_eq!(page.distinct_source_pages, 0);
        assert!(
            page.source_pages_by_context
                .values()
                .all(|count| *count == 0)
        );
        assert_eq!(page.redirected_source_pages, 2);
        // Redirects still withhold eligibility.
        assert!(report.issues.contains(&"redirect_observed".to_string()));
        let reconciliation = report.reconciliation.as_ref().unwrap();
        assert!(!reconciliation.comparable);
        assert!(reconciliation.eligible_pages.is_empty());
        assert_eq!(reconciliation.issues, ["redirect_observed"]);
        let json = serde_json::to_value(page).unwrap();
        assert_eq!(
            json["via_redirect"],
            serde_json::json!({"distinct_source_pages":3, "source_pages_by_context":{"body":2, "navigation":2}})
        );
        assert!(report.pages[0].via_redirect.is_none());
        let unredirected = serde_json::to_value(&report.pages[0]).unwrap();
        assert!(unredirected.get("via_redirect").is_none(), "{unredirected}");
    }

    #[test]
    fn missing_or_inconsistent_redirect_chains_credit_nothing() {
        for chain in [
            Vec::new(),
            vec![hop("https://example.org/elsewhere", 301)],
            vec![
                hop("https://example.org/old", 301),
                hop("https://example.org/middle", 200),
            ],
        ] {
            let report = redirect_report(chain.clone());
            let page = &report.pages[3];
            assert!(page.via_redirect.is_none(), "{chain:?}");
            assert_eq!(page.distinct_source_pages, 0);
            assert_eq!(page.redirected_source_pages, 2);
            assert!(report.issues.contains(&"redirect_observed".to_string()));
        }
    }
}

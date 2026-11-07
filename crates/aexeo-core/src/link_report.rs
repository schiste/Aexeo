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
    /// order. Reported only; links to a hop are never credited to this page.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub redirect_chain: Vec<RedirectHop>,
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
    report.issues.sort();
    report.issues.dedup();
    Ok(report)
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
    let consistent = chain[0].url == response.requested_url
        && chain
            .iter()
            .all(|hop| (300..400).contains(&hop.status_code));
    if consistent {
        chain.clone()
    } else {
        issues.insert("audit_redirect_chain_inconsistent");
        Vec::new()
    }
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
        // the page's own counts.
        assert_eq!(page.distinct_source_pages, 0);
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
        for issue in [
            "audit_redirect_chain_inconsistent",
            "audit_redirect_chain_unavailable",
        ] {
            assert!(report.issues.contains(&issue.to_string()), "{issue}");
        }
    }
}

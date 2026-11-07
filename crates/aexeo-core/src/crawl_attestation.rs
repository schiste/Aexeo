//! Seed a runtime crawl from an inventory and record what it then covered.
//!
//! The crawl requests each seeded URL; the attestation is recomputed from the
//! finished artifact, so it reflects what was observed, not what was queued.

use std::collections::BTreeSet;

use aexeo_contracts::{
    AuditArtifact, CrawlAttestation, CrawlSeedSource, CrawlSnapshotWindow, SurfaceIdentity,
};
use url::Url;

use crate::intelligence::presence::iso_from_unix_seconds;
use crate::link_graph::redact_url;
use crate::link_inventory::InventorySnapshot;
use crate::release_marker::{ReleaseMarkerSpec, attest_release};

/// When a runtime crawl ran and which release markers its segments saw.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CrawlRun {
    /// Start of the first segment, in Unix seconds; `None` when resumed from
    /// a checkpoint that did not record it.
    pub started_at: Option<i64>,
    pub finished_at: i64,
    pub resumed: bool,
    /// Markers of the pages a resumed checkpoint already held.
    pub earlier_release_values: BTreeSet<String>,
    /// Markers of the pages fetched by this segment.
    pub segment_release_values: BTreeSet<String>,
}

impl CrawlRun {
    /// A resumed segment saw a release value the earlier segments did not.
    pub fn spans_releases(&self) -> bool {
        !self.earlier_release_values.is_empty()
            && !self
                .segment_release_values
                .is_subset(&self.earlier_release_values)
    }

    pub fn snapshot_window(&self) -> CrawlSnapshotWindow {
        CrawlSnapshotWindow {
            started_at: self.started_at.map(iso_from_unix_seconds),
            finished_at: iso_from_unix_seconds(self.finished_at),
            resumed: self.resumed,
        }
    }
}

/// The attestation of a runtime crawl, when it was seeded from an inventory
/// or asked for a release marker; `None` otherwise, so an artifact made
/// without those options is unchanged. Without a seed, `seeded_from` stays
/// empty, which never counts as seeded.
pub fn attest_runtime_crawl(
    seed: Option<&InventorySeed>,
    release: Option<&ReleaseMarkerSpec>,
    run: &CrawlRun,
    artifact: &AuditArtifact,
) -> Option<CrawlAttestation> {
    if seed.is_none() && release.is_none() {
        return None;
    }
    let mut attestation = seed.map_or_else(
        || CrawlAttestation {
            engine: crawl_engine(artifact),
            ..CrawlAttestation::default()
        },
        |seed| attest_crawl(seed, artifact),
    );
    attestation.release = release.map(|spec| attest_release(spec, artifact, run.spans_releases()));
    attestation.snapshot = Some(run.snapshot_window());
    Some(attestation)
}

fn crawl_engine(artifact: &AuditArtifact) -> String {
    artifact
        .crawl
        .as_ref()
        .map(|crawl| crawl.engine.clone())
        .unwrap_or_default()
}

/// The same-origin URLs of an inventory, ready to seed a crawl.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InventorySeed {
    pub inventory_identity: Option<SurfaceIdentity>,
    /// Distinct same-origin page URLs, serialized after parsing.
    pub urls: Vec<String>,
    /// Entries left out, redacted: another origin, or not an HTTP(S) page URL.
    pub out_of_origin_urls: Vec<String>,
}

/// Split an inventory into URLs on the crawl target's exact origin and the rest.
/// Only exact origins qualify: the crawler maps a seed to a path on the target,
/// so a foreign URL would otherwise be requested from the wrong host.
pub fn inventory_seed(inventory: &InventorySnapshot, target_url: &str) -> InventorySeed {
    let target = Url::parse(target_url).ok().map(|url| url.origin());
    let mut urls = BTreeSet::new();
    let mut out_of_origin = BTreeSet::new();
    for page in &inventory.pages {
        let same_origin = Url::parse(&page.url).ok().filter(|url| {
            matches!(url.scheme(), "http" | "https")
                && url.username().is_empty()
                && url.password().is_none()
                && target.as_ref() == Some(&url.origin())
        });
        match same_origin {
            Some(url) => {
                urls.insert(String::from(url));
            }
            None => {
                out_of_origin.insert(redact_url(&page.url));
            }
        }
    }
    InventorySeed {
        inventory_identity: inventory.identity.clone(),
        urls: urls.into_iter().collect(),
        out_of_origin_urls: out_of_origin.into_iter().collect(),
    }
}

/// Record which seeded URLs the artifact holds a page or response for. A URL
/// counts as fetched when it was requested, answered as the final response or
/// answered as a redirect hop. A seed with a query or fragment never counts:
/// the crawler requests only its path, and persisted URLs drop the query.
pub fn attest_crawl(seed: &InventorySeed, artifact: &AuditArtifact) -> CrawlAttestation {
    let site = artifact.site.as_ref();
    let responses = site
        .into_iter()
        .flat_map(|site| {
            site.pages
                .iter()
                .filter_map(|page| page.crawl_response.as_ref())
        })
        .chain(
            site.and_then(|site| site.crawl_meta.as_ref())
                .into_iter()
                .flat_map(|meta| &meta.failed_responses),
        );
    let observed = responses
        .flat_map(|response| {
            std::iter::once(response.requested_url.as_str())
                .chain(response.observed_url.as_deref())
                .chain(response.redirect_chain.iter().map(|hop| hop.url.as_str()))
        })
        .filter_map(|url| Url::parse(url).ok().map(String::from))
        .collect::<BTreeSet<_>>();
    let unfetched_urls = seed
        .urls
        .iter()
        .filter(|url| {
            Url::parse(url).map_or(true, |parsed| {
                parsed.query().is_some() || parsed.fragment().is_some()
            }) || !observed.contains(url.as_str())
        })
        .map(|url| redact_url(url))
        .collect();
    CrawlAttestation {
        seeded_from: CrawlSeedSource {
            inventory_identity: seed.inventory_identity.clone(),
            url_count: seed.urls.len(),
        },
        engine: crawl_engine(artifact),
        out_of_origin_urls: seed.out_of_origin_urls.clone(),
        unfetched_urls,
        robots_skipped_urls: site
            .and_then(|site| site.crawl_meta.as_ref())
            .map(|meta| meta.robots_skipped_urls.clone())
            .unwrap_or_default(),
        release: None,
        snapshot: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::link_inventory::{InventoryDisposition, InventoryPage};

    fn inventory(urls: &[&str]) -> InventorySnapshot {
        InventorySnapshot {
            identity: None,
            complete: true,
            pages: urls
                .iter()
                .map(|url| InventoryPage {
                    url: (*url).into(),
                    disposition: InventoryDisposition::Published,
                    target: None,
                    locale: None,
                })
                .collect(),
        }
    }

    fn run(earlier: &[&str], segment: &[&str]) -> CrawlRun {
        CrawlRun {
            started_at: Some(0),
            finished_at: 61,
            resumed: !earlier.is_empty(),
            earlier_release_values: earlier.iter().map(|value| value.to_string()).collect(),
            segment_release_values: segment.iter().map(|value| value.to_string()).collect(),
        }
    }

    #[test]
    fn only_a_new_value_after_a_resume_spans_releases() {
        assert!(!run(&[], &["r1", "r2"]).spans_releases());
        assert!(!run(&["r1"], &["r1"]).spans_releases());
        assert!(!run(&["r1", "r2"], &["r2"]).spans_releases());
        assert!(!run(&["r1"], &[]).spans_releases());
        assert!(run(&["r1"], &["r1", "r2"]).spans_releases());
    }

    #[test]
    fn no_seed_and_no_marker_means_no_attestation() {
        let artifact = AuditArtifact {
            command: "crawl".into(),
            ..AuditArtifact::default()
        };
        assert_eq!(
            attest_runtime_crawl(None, None, &run(&[], &[]), &artifact),
            None
        );
        let spec = ReleaseMarkerSpec::new(Some("x-release"), None)
            .unwrap()
            .unwrap();
        let attestation =
            attest_runtime_crawl(None, Some(&spec), &run(&[], &[]), &artifact).unwrap();
        assert_eq!(attestation.seeded_from.url_count, 0);
        assert_eq!(
            attestation.snapshot,
            Some(CrawlSnapshotWindow {
                started_at: Some("1970-01-01T00:00:00Z".into()),
                finished_at: "1970-01-01T00:01:01Z".into(),
                resumed: false,
            })
        );
        assert_eq!(attestation.release.unwrap().value, None);
    }

    #[test]
    fn only_exact_origin_page_urls_are_seeded() {
        let seed = inventory_seed(
            &inventory(&[
                "https://example.org/a",
                "https://EXAMPLE.org:443/a",
                "https://example.org/b",
                "https://www.example.org/c",
                "http://example.org/d",
                "https://other.example/e?token=secret",
                "https://user:pw@example.org/f",
                "mailto:someone@example.org",
            ]),
            "https://example.org",
        );
        assert_eq!(
            seed.urls,
            ["https://example.org/a", "https://example.org/b"]
        );
        assert_eq!(
            seed.out_of_origin_urls,
            [
                "[invalid URL]",
                "http://example.org/d",
                "https://example.org/f",
                "https://other.example/e",
                "https://www.example.org/c",
            ]
        );
    }

    #[test]
    fn requested_final_and_hop_urls_count_as_fetched_but_queries_never_do() {
        let seed = inventory_seed(
            &inventory(&[
                "https://example.org/",
                "https://example.org/old",
                "https://example.org/middle",
                "https://example.org/gone",
                "https://example.org/q?x=1",
                "https://example.org/missing",
            ]),
            "https://example.org/",
        );
        let artifact: AuditArtifact = serde_json::from_value(serde_json::json!({
            "command": "crawl",
            "crawl": {"engine": "http", "visited_pages": 3, "discovered_internal_routes": 3, "queued_routes_remaining": 0, "max_pages": 10, "fetch_failures": 0, "fetch_retries": 0, "skipped_non_html": 0, "truncated": false, "elapsed_ms": 0, "pages_per_minute": 0, "checkpoints_written": 0, "partial_artifacts_written": 0, "total_fetch_ms": 0, "average_fetch_ms": 0, "total_page_process_ms": 0, "average_page_process_ms": 0, "total_partial_audit_ms": 0, "average_partial_audit_ms": 0, "slowest_paths": []},
            "site": {"site_url": "https://example.org/", "root": "crawl", "deployment_model": "runtime_snapshot",
                "crawl_meta": {"visited_pages": 3, "max_pages": 10, "discovered_internal_routes": 3, "truncated": false,
                    "failed_responses": [{"requested_url": "https://example.org/gone", "status_code": 404}],
                    "robots_skipped_urls": ["https://example.org/private"]},
                "pages": [
                    {"path": "", "relative_path": "index.html", "route": "", "crawl_response": {"requested_url": "https://example.org/", "observed_url": "https://example.org/"}},
                    {"path": "", "relative_path": "new/index.html", "route": "new", "crawl_response": {"requested_url": "https://example.org/old", "observed_url": "https://example.org/new",
                        "redirect_chain": [{"url": "https://example.org/old", "status_code": 301}, {"url": "https://example.org/middle", "status_code": 302}]}},
                    {"path": "", "relative_path": "q/index.html", "route": "q", "crawl_response": {"requested_url": "https://example.org/q", "observed_url": "https://example.org/q"}}
                ]}
        }))
        .unwrap();
        let attestation = attest_crawl(&seed, &artifact);
        assert_eq!(attestation.seeded_from.url_count, 6);
        assert_eq!(attestation.engine, "http");
        assert_eq!(
            attestation.unfetched_urls,
            ["https://example.org/missing", "https://example.org/q"]
        );
        assert_eq!(
            attestation.robots_skipped_urls,
            ["https://example.org/private"]
        );
    }
}

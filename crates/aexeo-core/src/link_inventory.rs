//! Conservative reconciliation of crawl and CMS inventory surfaces.
//!
//! A missing page or link is not evidence of an orphan unless the caller can
//! establish that both inputs describe the same complete snapshot. This module
//! does not infer completeness, canonical equivalence, or redirect targets.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use url::Url;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SurfaceIdentity {
    pub origin: String,
    pub release: String,
    pub surface: String,
    pub snapshot: String,
}

impl SurfaceIdentity {
    fn is_complete(&self) -> bool {
        [&self.origin, &self.release, &self.surface, &self.snapshot]
            .iter()
            .all(|value| !value.trim().is_empty())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InventoryDisposition {
    Published,
    Draft,
    Noindex,
    Redirect,
    CanonicalAlias,
    LocaleFallback,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InventoryPage {
    /// An absolute URL supplied by the inventory producer; no aliases are guessed.
    pub url: String,
    pub disposition: InventoryDisposition,
    pub target: Option<String>,
    pub locale: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InventorySnapshot {
    pub identity: Option<SurfaceIdentity>,
    /// True only if the inventory producer attests it covers the whole surface.
    pub complete: bool,
    pub pages: Vec<InventoryPage>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrawlSurface {
    pub identity: Option<SurfaceIdentity>,
    /// True only if the caller verified no truncation or missing fetches.
    pub complete: bool,
    pub pages: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InventoryReconciliation {
    pub comparable: bool,
    pub issues: Vec<String>,
    pub inventory_only: Vec<String>,
    pub crawl_only: Vec<String>,
    /// Eligible URLs only when all coverage conditions are satisfied.
    pub eligible_pages: Vec<String>,
}

fn valid_page_url(value: &str, origin: &Url) -> bool {
    Url::parse(value).is_ok_and(|url| {
        matches!(url.scheme(), "http" | "https")
            && url.username().is_empty()
            && url.password().is_none()
            && url.fragment().is_none()
            && url.origin() == origin.origin()
    })
}

/// Compare exact URL identities. Normalization and alias resolution are not
/// inferred; aliases prevent eligibility until the graph can account for them.
pub fn reconcile(inventory: &InventorySnapshot, crawl: &CrawlSurface) -> InventoryReconciliation {
    let mut issues = Vec::new();
    if !inventory.complete {
        issues.push("inventory_incomplete".to_string());
    }
    if !crawl.complete {
        issues.push("crawl_incomplete".to_string());
    }
    match (&inventory.identity, &crawl.identity) {
        (Some(left), Some(right)) if left.is_complete() && right.is_complete() => {
            if left != right {
                issues.push("surface_identity_mismatch".to_string());
            }
        }
        _ => issues.push("surface_identity_missing".to_string()),
    }
    let origin = inventory.identity.as_ref().and_then(|identity| {
        Url::parse(&identity.origin).ok().filter(|url| {
            matches!(url.scheme(), "http" | "https")
                && url.username().is_empty()
                && url.password().is_none()
                && url.path() == "/"
                && url.query().is_none()
                && url.fragment().is_none()
        })
    });
    if origin.is_none() {
        issues.push("invalid_origin".to_string());
    }
    let mut records = BTreeMap::new();
    for page in &inventory.pages {
        if origin
            .as_ref()
            .is_none_or(|origin| !valid_page_url(&page.url, origin))
            || records.insert(page.url.as_str(), page).is_some()
        {
            issues.push("invalid_or_duplicate_inventory_url".to_string());
        }
        if page.disposition == InventoryDisposition::Unknown {
            issues.push("unknown_inventory_disposition".to_string());
        }
        if matches!(
            page.disposition,
            InventoryDisposition::Redirect
                | InventoryDisposition::CanonicalAlias
                | InventoryDisposition::LocaleFallback
        ) {
            issues.push("alias_resolution_required".to_string());
            if page
                .target
                .as_deref()
                .is_none_or(|target| target.trim().is_empty())
            {
                issues.push("alias_target_missing".to_string());
            }
        }
    }
    let crawled: BTreeSet<_> = crawl.pages.iter().map(String::as_str).collect();
    if crawled.len() != crawl.pages.len()
        || crawl.pages.iter().any(|page| {
            origin
                .as_ref()
                .is_none_or(|origin| !valid_page_url(page, origin))
        })
    {
        issues.push("invalid_or_duplicate_crawl_url".to_string());
    }
    let expected: BTreeSet<_> = inventory
        .pages
        .iter()
        .filter(|page| page.disposition == InventoryDisposition::Published)
        .map(|page| page.url.as_str())
        .collect();
    let inventory_only = expected
        .difference(&crawled)
        .map(|url| (*url).to_string())
        .collect::<Vec<_>>();
    // Non-published records remain known to the inventory; do not label them
    // unexpected merely because a crawl happens to visit them.
    let known: BTreeSet<_> = records.keys().copied().collect();
    let crawl_only = crawled
        .difference(&known)
        .map(|url| (*url).to_string())
        .collect::<Vec<_>>();
    if !inventory_only.is_empty() || !crawl_only.is_empty() {
        issues.push("inventory_crawl_mismatch".to_string());
    }
    issues.sort();
    issues.dedup();
    let comparable = issues.is_empty();
    let eligible_pages = if comparable {
        expected.into_iter().map(str::to_string).collect()
    } else {
        Vec::new()
    };
    InventoryReconciliation {
        comparable,
        issues,
        inventory_only,
        crawl_only,
        eligible_pages,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity() -> SurfaceIdentity {
        SurfaceIdentity {
            origin: "https://example.org".into(),
            release: "release-1".into(),
            surface: "public".into(),
            snapshot: "2026-11-05T00:00:00Z".into(),
        }
    }
    fn inputs() -> (InventorySnapshot, CrawlSurface) {
        (
            InventorySnapshot {
                identity: Some(identity()),
                complete: true,
                pages: vec![InventoryPage {
                    url: "https://example.org/a".into(),
                    disposition: InventoryDisposition::Published,
                    target: None,
                    locale: Some("en".into()),
                }],
            },
            CrawlSurface {
                identity: Some(identity()),
                complete: true,
                pages: vec!["https://example.org/a".into()],
            },
        )
    }
    #[test]
    fn accepts_only_matching_complete_inventory_and_crawl() {
        let (inventory, mut crawl) = inputs();
        assert_eq!(
            reconcile(&inventory, &crawl).eligible_pages,
            vec!["https://example.org/a"]
        );
        crawl.complete = false;
        let result = reconcile(&inventory, &crawl);
        assert!(!result.comparable);
        assert!(result.eligible_pages.is_empty());
        crawl.complete = true;
        crawl.identity.as_mut().unwrap().snapshot = "other".into();
        assert!(
            reconcile(&inventory, &crawl)
                .issues
                .contains(&"surface_identity_mismatch".into())
        );
    }
    #[test]
    fn mismatch_and_unknown_alias_fail_closed() {
        let (mut inventory, mut crawl) = inputs();
        crawl.pages.clear();
        assert_eq!(
            reconcile(&inventory, &crawl).inventory_only,
            vec!["https://example.org/a"]
        );
        crawl.pages.push("https://example.org/other".into());
        assert_eq!(
            reconcile(&inventory, &crawl).crawl_only,
            vec!["https://example.org/other"]
        );
        inventory.pages[0].disposition = InventoryDisposition::Redirect;
        assert!(
            reconcile(&inventory, &crawl)
                .issues
                .contains(&"alias_target_missing".into())
        );
        inventory.pages[0].disposition = InventoryDisposition::Unknown;
        assert!(
            reconcile(&inventory, &crawl)
                .issues
                .contains(&"unknown_inventory_disposition".into())
        );
    }

    #[test]
    fn foreign_urls_and_unresolved_aliases_are_not_eligible() {
        let (mut inventory, mut crawl) = inputs();
        inventory.pages[0].url = "https://other.example/a".into();
        crawl.pages[0] = inventory.pages[0].url.clone();
        let result = reconcile(&inventory, &crawl);
        assert!(!result.comparable);
        assert!(result.eligible_pages.is_empty());

        let (mut inventory, crawl) = inputs();
        inventory.pages.push(InventoryPage {
            url: "https://example.org/old".into(),
            disposition: InventoryDisposition::Redirect,
            target: Some("https://example.org/a".into()),
            locale: None,
        });
        let result = reconcile(&inventory, &crawl);
        assert!(result.issues.contains(&"alias_resolution_required".into()));
        assert!(result.eligible_pages.is_empty());
    }
}

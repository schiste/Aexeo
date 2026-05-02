//! Computed page identity: the canonical "what is this page" answer Aexeo
//! derives from the page's existing signals.
//!
//! The drift-detection rules (GEO009, SCH005) tell hosts when their title /
//! H1 / Open Graph / JSON-LD don't agree. They don't tell hosts what the
//! computed canonical answer should be. This module exposes that canonical
//! answer so a host can build templates *from* it instead of writing four
//! parallel strings and waiting for findings.
//!
//! Source priority: title is the anchor. If absent, fall back to the first
//! H1, then to the first JSON-LD `name` or `headline`. We never invent — if
//! none of the four is present the canonical title is None and the host gets
//! a clear "no signal" answer.

use serde::{Deserialize, Serialize};

use crate::schema_rules::iter_schema_field_values;
use crate::site::{Page, Site};

/// All identity-bearing signals collected from a single page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PageIdentitySources {
    pub title: Option<String>,
    pub first_h1: Option<String>,
    pub og_title: Option<String>,
    pub schema_names: Vec<String>,
    pub schema_headlines: Vec<String>,
}

/// One dimension of drift between the canonical title and another signal.
/// Surfaced verbatim so hosts can render a UI like "your H1 says X but
/// title says Y" without re-deriving the comparison.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdentityDrift {
    pub dimension: String,
    pub canonical: String,
    pub observed: String,
}

/// The full identity blob for a route. `canonical_title` is what the host
/// should treat as the page's identity; everything else is diagnostic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PageIdentity {
    pub route: String,
    pub canonical_title: Option<String>,
    pub canonical_source: &'static str,
    pub sources: PageIdentitySources,
    pub drift: Vec<IdentityDrift>,
    pub agrees: bool,
}

/// Compute the page identity for a given route, or `None` if the route
/// doesn't exist on the site.
pub fn compute_page_identity(site: &Site, route: &str) -> Option<PageIdentity> {
    let page = site.page(route)?;
    Some(compute_for_page(page))
}

fn compute_for_page(page: &Page) -> PageIdentity {
    let sources = collect_sources(page);
    let (canonical_title, canonical_source) = pick_canonical(&sources);
    let drift = match canonical_title.as_deref() {
        Some(canonical) => detect_drift(canonical, &sources, canonical_source),
        None => Vec::new(),
    };
    PageIdentity {
        route: page.route.clone(),
        canonical_title,
        canonical_source,
        sources,
        agrees: drift.is_empty(),
        drift,
    }
}

fn collect_sources(page: &Page) -> PageIdentitySources {
    let mut schema_names = Vec::new();
    let mut schema_headlines = Vec::new();
    for block in &page.json_ld_blocks {
        let Ok(payload) = serde_json::from_str::<serde_json::Value>(&block.raw) else {
            continue;
        };
        for name in iter_schema_field_values(&payload, "name") {
            if !schema_names.contains(&name) {
                schema_names.push(name);
            }
        }
        for headline in iter_schema_field_values(&payload, "headline") {
            if !schema_headlines.contains(&headline) {
                schema_headlines.push(headline);
            }
        }
    }
    PageIdentitySources {
        title: page.title.clone(),
        first_h1: page.h1_texts.first().cloned(),
        og_title: page.metadata("og:title").map(str::to_string),
        schema_names,
        schema_headlines,
    }
}

/// Pick the canonical title in priority order: title → first H1 → first
/// schema name → first schema headline. Returns the value plus a label
/// identifying which source it came from so hosts can show "Aexeo derived
/// this from your H1 because the page has no <title>" rather than guessing.
fn pick_canonical(sources: &PageIdentitySources) -> (Option<String>, &'static str) {
    if let Some(title) = &sources.title
        && !title.trim().is_empty()
    {
        return (Some(title.clone()), "title");
    }
    if let Some(h1) = &sources.first_h1
        && !h1.trim().is_empty()
    {
        return (Some(h1.clone()), "h1");
    }
    if let Some(name) = sources.schema_names.first()
        && !name.trim().is_empty()
    {
        return (Some(name.clone()), "schema_name");
    }
    if let Some(headline) = sources.schema_headlines.first()
        && !headline.trim().is_empty()
    {
        return (Some(headline.clone()), "schema_headline");
    }
    (None, "none")
}

/// Surface every signal that disagrees with the canonical title. Comparisons
/// are normalized (case-insensitive, whitespace-collapsed) but the surfaced
/// strings are the raw originals — hosts may want to render the actual text.
/// We intentionally do NOT surface the canonical-source signal itself in
/// the drift list (it always agrees with itself).
fn detect_drift(
    canonical: &str,
    sources: &PageIdentitySources,
    canonical_source: &str,
) -> Vec<IdentityDrift> {
    let canonical_norm = normalize(canonical);
    let mut drift = Vec::new();

    if canonical_source != "title"
        && let Some(title) = &sources.title
        && normalize(title) != canonical_norm
    {
        drift.push(IdentityDrift {
            dimension: "title".to_string(),
            canonical: canonical.to_string(),
            observed: title.clone(),
        });
    }
    if canonical_source != "h1"
        && let Some(h1) = &sources.first_h1
        && normalize(h1) != canonical_norm
    {
        drift.push(IdentityDrift {
            dimension: "h1".to_string(),
            canonical: canonical.to_string(),
            observed: h1.clone(),
        });
    }
    if let Some(og) = &sources.og_title
        && normalize(og) != canonical_norm
    {
        drift.push(IdentityDrift {
            dimension: "og_title".to_string(),
            canonical: canonical.to_string(),
            observed: og.clone(),
        });
    }
    for name in &sources.schema_names {
        if normalize(name) != canonical_norm {
            drift.push(IdentityDrift {
                dimension: "schema_name".to_string(),
                canonical: canonical.to_string(),
                observed: name.clone(),
            });
        }
    }
    for headline in &sources.schema_headlines {
        if normalize(headline) != canonical_norm {
            drift.push(IdentityDrift {
                dimension: "schema_headline".to_string(),
                canonical: canonical.to_string(),
                observed: headline.clone(),
            });
        }
    }
    drift
}

fn normalize(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::site::load_site;
    use anyhow::Result;
    use std::fs;

    fn write(path: &std::path::Path, text: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, text).unwrap();
    }

    #[test]
    fn agreeing_signals_yield_empty_drift() -> Result<()> {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        write(
            &root.join("index.html"),
            r#"<html lang="en"><head>
                <title>Aexeo</title>
                <meta name="description" content="x">
                <meta property="og:title" content="Aexeo">
                <link rel="canonical" href="https://example.com/">
                <script type="application/ld+json">{"@type":"Organization","name":"Aexeo"}</script>
            </head><body><h1>Aexeo</h1></body></html>"#,
        );
        let site = load_site(root)?;
        let identity = compute_page_identity(&site, "").expect("home identity");
        assert_eq!(identity.canonical_title.as_deref(), Some("Aexeo"));
        assert_eq!(identity.canonical_source, "title");
        assert!(identity.agrees);
        assert!(identity.drift.is_empty());
        Ok(())
    }

    #[test]
    fn disagreeing_signals_surface_drift_dimensions() -> Result<()> {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        write(
            &root.join("index.html"),
            r#"<html lang="en"><head>
                <title>Aexeo</title>
                <meta name="description" content="x">
                <meta property="og:title" content="Aexeo Documentation">
                <link rel="canonical" href="https://example.com/">
                <script type="application/ld+json">{"@type":"Organization","name":"Aexeo Inc."}</script>
            </head><body><h1>Aexeo Docs</h1></body></html>"#,
        );
        let site = load_site(root)?;
        let identity = compute_page_identity(&site, "").expect("home identity");
        assert_eq!(identity.canonical_title.as_deref(), Some("Aexeo"));
        assert_eq!(identity.canonical_source, "title");
        assert!(!identity.agrees);
        let dims: Vec<_> = identity
            .drift
            .iter()
            .map(|d| d.dimension.as_str())
            .collect();
        assert!(dims.contains(&"h1"));
        assert!(dims.contains(&"og_title"));
        assert!(dims.contains(&"schema_name"));
        Ok(())
    }

    #[test]
    fn fallback_canonical_when_title_missing() -> Result<()> {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        // No <title> tag — canonical should fall back to H1.
        write(
            &root.join("index.html"),
            r#"<html lang="en"><head>
                <meta name="description" content="x">
                <link rel="canonical" href="https://example.com/">
            </head><body><h1>Fallback Title</h1></body></html>"#,
        );
        let site = load_site(root)?;
        let identity = compute_page_identity(&site, "").expect("home identity");
        assert_eq!(identity.canonical_source, "h1");
        assert_eq!(identity.canonical_title.as_deref(), Some("Fallback Title"));
        Ok(())
    }

    #[test]
    fn unknown_route_returns_none() -> Result<()> {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        write(
            &root.join("index.html"),
            r#"<html lang="en"><head><title>x</title><meta name="description" content="x"><link rel="canonical" href="https://example.com/"></head><body><h1>x</h1></body></html>"#,
        );
        let site = load_site(root)?;
        assert!(compute_page_identity(&site, "nope").is_none());
        Ok(())
    }
}

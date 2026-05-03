//! Schema generation for common route families.
//!
//! Aexeo's existing schema rules tell hosts when their pages are missing
//! the JSON-LD types they'd benefit from (SCH011 home, SCH012 listing,
//! SCH014 docs, SCH015 search). This module *emits* those types so hosts
//! can stop hand-writing them per page.
//!
//! Output shape: one `SchemaSuggestion` per route, each carrying a map of
//! schema-type-name to JSON-LD body. Consumers (the public-bundle artifact
//! emitter; the `--inject-schema` fix) apply per-type augmentation —
//! never replace, never duplicate. A page that already has `Article` keeps
//! its hand-written `Article`; if the page is missing `BreadcrumbList`,
//! Aexeo adds one alongside.
//!
//! Defaults are opinionated, but every type emitted is reversible by
//! either (a) the host removing the schema-suggestions artifact before
//! deploy, or (b) hand-writing an alternative schema of the same type
//! before the next inject. The system never fights the host.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::site::{Page, PageKind, Site};

/// Per-route bundle of schema types Aexeo would suggest emitting.
///
/// `types` is keyed by schema-type name (`"WebSite"`, `"BreadcrumbList"`,
/// `"ItemList"`, `"Article"`, ...) so per-type augmentation logic can ask
/// "does the host already have this type?" and skip if yes. The values
/// are the full JSON-LD bodies, ready to drop into a
/// `<script type="application/ld+json">` tag.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchemaSuggestion {
    pub route: String,
    pub types: BTreeMap<String, Value>,
}

/// Deterministic sweep over every page; emit suggestions for the four
/// supported families. Returned vector is sorted by route so the bundle
/// output is byte-identical across runs given the same input.
///
/// Family detection uses two signals together:
///
///   - the existing `PageKind` classifier for Home / Docs / Search /
///     skip categories (Admin, Legal, Feed, NotFound), and
///   - a **structural** check for "has direct children?" to identify
///     listing-shaped routes the classifier doesn't hardcode.
///
/// The structural signal is what makes a generic `/blog` route get an
/// ItemList — the existing classifier only marks `skills`, `features`,
/// `category/...` etc. as `PageKind::Listing`, which would miss most
/// real-world listings.
pub fn generate_schema_suggestions(site: &Site, site_url: Option<&str>) -> Vec<SchemaSuggestion> {
    let Some(url) = site_url else {
        // Without a site_url every type would have a half-broken URL
        // field. Emitting nothing is the right default.
        return Vec::new();
    };

    let mut out = Vec::new();
    for (route, page) in site.route_page_pairs() {
        if route.as_str() == "404" {
            continue;
        }
        if matches!(
            page.page_kind,
            PageKind::Admin
                | PageKind::Legal
                | PageKind::Feed
                | PageKind::NotFound
                | PageKind::Search
                | PageKind::Utility
        ) {
            continue;
        }

        let mut types: BTreeMap<String, Value> = BTreeMap::new();

        // Home gets WebSite; nothing else.
        if matches!(page.page_kind, PageKind::Home) {
            types.insert("WebSite".to_string(), synthesize_website(url, site, page));
            if !types.is_empty() {
                out.push(SchemaSuggestion {
                    route: route.clone(),
                    types,
                });
            }
            continue;
        }

        // Every non-home, non-skipped route gets a BreadcrumbList — the
        // navigation context is universally useful and per-type
        // augmentation skips it when the host already authored one.
        types.insert(
            "BreadcrumbList".to_string(),
            synthesize_breadcrumb_list(url, route, site),
        );

        // Listing-shaped: has direct children. Emit ItemList enumerating
        // those children. This catches both classifier-marked listings
        // (Listing PageKind) and structural ones (e.g. a generic /blog).
        if let Some(item_list) = synthesize_item_list(url, route, site) {
            types.insert("ItemList".to_string(), item_list);
        }

        // Docs-shaped: emit TechArticle in addition to the breadcrumb.
        if matches!(page.page_kind, PageKind::Docs) {
            types.insert(
                "TechArticle".to_string(),
                synthesize_article(url, route, page, /* tech */ true),
            );
        }

        out.push(SchemaSuggestion {
            route: route.clone(),
            types,
        });
    }
    out.sort_by(|a, b| a.route.cmp(&b.route));
    out
}

/// WebSite schema for the home page. Includes a SearchAction sub-resource
/// only when the site has a `Search` page (so SearchAction's existence is
/// grounded in the actual routes the host ships).
fn synthesize_website(site_url: &str, site: &Site, page: &Page) -> Value {
    let normalized = site_url.trim_end_matches('/');
    let name = page
        .title
        .clone()
        .or_else(|| page.h1_texts.first().cloned())
        .unwrap_or_else(|| normalized.to_string());

    let has_search = site
        .route_page_pairs()
        .any(|(_, p)| matches!(p.page_kind, PageKind::Search));

    let mut value = json!({
        "@context": "https://schema.org",
        "@type": "WebSite",
        "url": format!("{normalized}/"),
        "name": name,
    });

    if has_search {
        value["potentialAction"] = json!({
            "@type": "SearchAction",
            "target": format!("{normalized}/search?q={{search_term_string}}"),
            "query-input": "required name=search_term_string",
        });
    }

    value
}

/// BreadcrumbList from the path segments of a route. Each segment becomes
/// one ListItem; the segment's name is taken from the matching page's
/// title when one exists, or title-cased from the URL slug as a fallback.
/// Segment URLs always include the trailing path through that level — a
/// breadcrumb mid-tree links to a page that may or may not exist on the
/// site, but that's the host's call (it's the path users would type).
fn synthesize_breadcrumb_list(site_url: &str, route: &str, site: &Site) -> Value {
    let normalized = site_url.trim_end_matches('/');
    let mut items = Vec::new();
    items.push(json!({
        "@type": "ListItem",
        "position": 1,
        "name": "Home",
        "item": format!("{normalized}/"),
    }));

    let mut accumulated = String::new();
    for (index, segment) in route.split('/').enumerate() {
        if segment.is_empty() {
            continue;
        }
        if !accumulated.is_empty() {
            accumulated.push('/');
        }
        accumulated.push_str(segment);
        let name = site
            .page(&accumulated)
            .and_then(|p| p.title.clone())
            .unwrap_or_else(|| title_case(segment));
        items.push(json!({
            "@type": "ListItem",
            "position": index + 2,
            "name": name,
            "item": format!("{normalized}/{accumulated}"),
        }));
    }

    json!({
        "@context": "https://schema.org",
        "@type": "BreadcrumbList",
        "itemListElement": items,
    })
}

/// ItemList enumerating direct children of a listing route. Returns None
/// when the listing has zero children — emitting an empty ItemList would
/// be misleading and the SCH012 rule already covers the "listing exists
/// but has no enumeration" case via a different angle.
fn synthesize_item_list(site_url: &str, listing_route: &str, site: &Site) -> Option<Value> {
    let normalized = site_url.trim_end_matches('/');
    let prefix = if listing_route.is_empty() {
        "".to_string()
    } else {
        format!("{listing_route}/")
    };
    // A child is any route that starts with the prefix and has a page.
    // We only enumerate DIRECT children (one extra path segment beyond
    // the listing) so an `/blog/year/post` doesn't appear under `/blog`'s
    // ItemList; that nesting belongs in the year's own ItemList.
    let mut children: Vec<(String, &Page)> = Vec::new();
    for (route, page) in site.route_page_pairs() {
        if route == listing_route {
            continue;
        }
        let Some(rest) = route.strip_prefix(&prefix) else {
            continue;
        };
        if rest.is_empty() || rest.contains('/') {
            continue;
        }
        children.push((route.clone(), page));
    }
    if children.is_empty() {
        return None;
    }
    children.sort_by(|a, b| a.0.cmp(&b.0));

    let items: Vec<Value> = children
        .iter()
        .enumerate()
        .map(|(index, (route, page))| {
            let name = page
                .title
                .clone()
                .or_else(|| page.h1_texts.first().cloned())
                .unwrap_or_else(|| title_case(route.split('/').next_back().unwrap_or(route)));
            json!({
                "@type": "ListItem",
                "position": index + 1,
                "url": format!("{normalized}/{route}"),
                "name": name,
            })
        })
        .collect();

    Some(json!({
        "@context": "https://schema.org",
        "@type": "ItemList",
        "itemListElement": items,
    }))
}

/// Minimal Article (or TechArticle) for docs-like pages. Author and date
/// are deliberately omitted — without reliable signals for those, emitting
/// fabricated values would be worse than emitting nothing. Hosts who want
/// the full Article shape can hand-write the fields; per-type augmentation
/// will skip Aexeo's bare version when a host-authored one exists.
fn synthesize_article(site_url: &str, route: &str, page: &Page, tech: bool) -> Value {
    let normalized = site_url.trim_end_matches('/');
    let url = if route.is_empty() {
        format!("{normalized}/")
    } else {
        format!("{normalized}/{route}")
    };
    let headline = page
        .title
        .clone()
        .or_else(|| page.h1_texts.first().cloned())
        .unwrap_or_else(|| title_case(route));

    let mut value = json!({
        "@context": "https://schema.org",
        "@type": if tech { "TechArticle" } else { "Article" },
        "headline": headline,
        "url": url,
    });

    if let Some(description) = page.meta_description() {
        value["description"] = json!(description);
    }
    value
}

fn title_case(slug: &str) -> String {
    slug.split('-')
        .filter(|s| !s.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
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

    fn page_with_title(route: &str, title: &str) -> String {
        let canonical = if route.is_empty() {
            "https://example.com/".to_string()
        } else {
            format!("https://example.com/{route}")
        };
        format!(
            "<html lang=\"en\"><head><title>{title}</title>\
             <meta name=\"description\" content=\"d for {route}\">\
             <link rel=\"canonical\" href=\"{canonical}\"></head>\
             <body><h1>{title}</h1></body></html>"
        )
    }

    #[test]
    fn home_gets_website_schema() -> Result<()> {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        write(&root.join("index.html"), &page_with_title("", "Aexeo"));
        let site = load_site(root)?;
        let suggestions = generate_schema_suggestions(&site, Some("https://example.com"));
        let home = suggestions
            .iter()
            .find(|s| s.route.is_empty())
            .expect("home suggestion");
        let website = home.types.get("WebSite").expect("WebSite");
        assert_eq!(website["@type"], "WebSite");
        assert_eq!(website["url"], "https://example.com/");
        assert_eq!(website["name"], "Aexeo");
        // No search page, so SearchAction is not emitted.
        assert!(website.get("potentialAction").is_none());
        Ok(())
    }

    #[test]
    fn listing_gets_breadcrumb_and_item_list() -> Result<()> {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        write(&root.join("index.html"), &page_with_title("", "Aexeo"));
        // Astro-style nested listing: blog/index.html is the listing,
        // direct children at blog/foo/index.html.
        write(
            &root.join("blog/index.html"),
            &page_with_title("blog", "Blog"),
        );
        write(
            &root.join("blog/foo/index.html"),
            &page_with_title("blog/foo", "Foo Post"),
        );
        write(
            &root.join("blog/bar/index.html"),
            &page_with_title("blog/bar", "Bar Post"),
        );
        let site = load_site(root)?;
        let suggestions = generate_schema_suggestions(&site, Some("https://example.com"));
        let blog = suggestions
            .iter()
            .find(|s| s.route == "blog")
            .expect("blog suggestion");

        let breadcrumb = blog.types.get("BreadcrumbList").expect("BreadcrumbList");
        let crumbs = breadcrumb["itemListElement"].as_array().unwrap();
        assert_eq!(crumbs.len(), 2);
        assert_eq!(crumbs[0]["name"], "Home");
        assert_eq!(crumbs[1]["name"], "Blog");

        let item_list = blog.types.get("ItemList").expect("ItemList");
        let items = item_list["itemListElement"].as_array().unwrap();
        assert_eq!(items.len(), 2, "should enumerate both child posts");
        assert_eq!(items[0]["name"], "Bar Post");
        assert_eq!(items[1]["name"], "Foo Post");
        Ok(())
    }

    #[test]
    fn deeper_descendants_do_not_pollute_listing_item_list() -> Result<()> {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        write(&root.join("index.html"), &page_with_title("", "Aexeo"));
        write(
            &root.join("blog/index.html"),
            &page_with_title("blog", "Blog"),
        );
        write(
            &root.join("blog/2024/index.html"),
            &page_with_title("blog/2024", "2024"),
        );
        write(
            &root.join("blog/2024/post-a/index.html"),
            &page_with_title("blog/2024/post-a", "Post A"),
        );
        let site = load_site(root)?;
        let suggestions = generate_schema_suggestions(&site, Some("https://example.com"));
        let blog = suggestions
            .iter()
            .find(|s| s.route == "blog")
            .expect("blog suggestion");
        let item_list = blog.types.get("ItemList").expect("ItemList");
        let items = item_list["itemListElement"].as_array().unwrap();
        // Only the year is a direct child of /blog. Post A is two levels down
        // and belongs in the year's own ItemList.
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["name"], "2024");
        Ok(())
    }

    #[test]
    fn omits_search_action_when_no_search_route() -> Result<()> {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        write(&root.join("index.html"), &page_with_title("", "Aexeo"));
        let site = load_site(root)?;
        let suggestions = generate_schema_suggestions(&site, Some("https://example.com"));
        let home = suggestions.iter().find(|s| s.route.is_empty()).unwrap();
        let website = home.types.get("WebSite").unwrap();
        assert!(website.get("potentialAction").is_none());
        Ok(())
    }

    #[test]
    fn includes_search_action_when_search_route_exists() -> Result<()> {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        write(&root.join("index.html"), &page_with_title("", "Aexeo"));
        write(
            &root.join("search/index.html"),
            &page_with_title("search", "Search"),
        );
        let site = load_site(root)?;
        let suggestions = generate_schema_suggestions(&site, Some("https://example.com"));
        let home = suggestions.iter().find(|s| s.route.is_empty()).unwrap();
        let website = home.types.get("WebSite").unwrap();
        let action = website
            .get("potentialAction")
            .expect("SearchAction should be present");
        assert_eq!(action["@type"], "SearchAction");
        assert!(
            action["target"]
                .as_str()
                .unwrap()
                .contains("search?q={search_term_string}")
        );
        Ok(())
    }

    #[test]
    fn site_url_required_for_schema_emission() -> Result<()> {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        write(&root.join("index.html"), &page_with_title("", "Aexeo"));
        let site = load_site(root)?;
        let suggestions = generate_schema_suggestions(&site, None);
        // Without a site_url we can't construct absolute URLs, so the
        // generator emits nothing rather than half-broken schema.
        assert!(suggestions.is_empty());
        Ok(())
    }
}

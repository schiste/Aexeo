use anyhow::Result;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use crate::config::Config;
use crate::generate::{build_link_suggestions, render_llms_txt, render_robots_txt};
use crate::site::{Page, Site, load_site};

fn escape_html_attr(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn canonical_url_for_page(route: &str, site_url: &str) -> String {
    let normalized_site = site_url.trim_end_matches('/');
    if route.is_empty() {
        format!("{}/", normalized_site)
    } else {
        format!("{}/{}", normalized_site, route)
    }
}

fn render_missing_head_tags(page: &Page, config: &Config) -> Vec<String> {
    let mut tags = Vec::new();
    if let Some(site_url) = config.site_url.as_deref() {
        if page.canonical.is_none() {
            tags.push(format!(
                "<link rel=\"canonical\" href=\"{}\">",
                escape_html_attr(&canonical_url_for_page(&page.route, site_url))
            ));
        }
        if page.metadata("og:url").is_none() {
            tags.push(format!(
                "<meta property=\"og:url\" content=\"{}\">",
                escape_html_attr(&canonical_url_for_page(&page.route, site_url))
            ));
        }
    }
    if page.metadata("og:title").is_none()
        && let Some(title) = &page.title
    {
        tags.push(format!(
            "<meta property=\"og:title\" content=\"{}\">",
            escape_html_attr(title)
        ));
    }
    if page.metadata("og:description").is_none()
        && let Some(description) = page.meta_description()
    {
        tags.push(format!(
            "<meta property=\"og:description\" content=\"{}\">",
            escape_html_attr(description)
        ));
    }
    if page.metadata("og:type").is_none() {
        tags.push("<meta property=\"og:type\" content=\"website\">".to_string());
    }
    if page.metadata("twitter:card").is_none() && !config.default_twitter_card.is_empty() {
        tags.push(format!(
            "<meta name=\"twitter:card\" content=\"{}\">",
            escape_html_attr(&config.default_twitter_card)
        ));
    }
    tags
}

fn inject_head_tags(raw_text: &str, tags: &[String]) -> String {
    if tags.is_empty() {
        return raw_text.to_string();
    }
    let insertion = format!("  {}\n", tags.join("\n  "));
    if let Some(index) = raw_text.to_ascii_lowercase().find("</head>") {
        let mut updated = String::with_capacity(raw_text.len() + insertion.len());
        updated.push_str(&raw_text[..index]);
        updated.push_str(&insertion);
        updated.push_str(&raw_text[index..]);
        return updated;
    }
    raw_text.to_string()
}

fn apply_html_metadata_fixes(site: &Site, config: &Config) -> Result<Vec<PathBuf>> {
    let mut changed = Vec::new();
    for page in site.route_pages() {
        let raw_text = fs::read_to_string(&page.path)?;
        let updated = inject_head_tags(&raw_text, &render_missing_head_tags(page, config));
        if updated != raw_text {
            fs::write(&page.path, updated)?;
            changed.push(page.path.clone());
        }
    }
    Ok(changed)
}

fn render_related_links_section(
    source_route: &str,
    target_routes: &[String],
    heading: &str,
) -> String {
    let items = target_routes
        .iter()
        .map(|target| {
            let label = target
                .replace('-', " ")
                .replace('/', " / ")
                .split_whitespace()
                .map(|part| {
                    let mut chars = part.chars();
                    match chars.next() {
                        Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
                        None => String::new(),
                    }
                })
                .collect::<Vec<_>>()
                .join(" ");
            format!("      <li><a href=\"/{}\">{}</a></li>", target, label)
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "\n  <section data-ui=\"related-links\" data-source-route=\"{}\">\n    <h2>{}</h2>\n    <ul>\n{}\n    </ul>\n  </section>\n",
        escape_html_attr(if source_route.is_empty() {
            "/"
        } else {
            source_route
        }),
        escape_html_attr(heading),
        items,
    )
}

fn apply_related_link_insertions(site: &Site, config: &Config) -> Result<Vec<PathBuf>> {
    if !config.enable_link_autofix {
        return Ok(Vec::new());
    }
    let suggestions = build_link_suggestions(site, config.link_suggestion_count);
    let mut changed = Vec::new();
    for (source_route, target_routes) in suggestions {
        let Some(source_page) = site.page(&source_route) else {
            continue;
        };
        let raw_text = fs::read_to_string(&source_page.path)?;
        if raw_text.contains("data-ui=\"related-links\"")
            || raw_text.contains(&config.related_links_heading)
        {
            continue;
        }
        let section = render_related_links_section(
            &source_route,
            &target_routes,
            &config.related_links_heading,
        );
        let updated = if let Some(index) = raw_text.to_ascii_lowercase().find("</body>") {
            let mut text = String::with_capacity(raw_text.len() + section.len());
            text.push_str(&raw_text[..index]);
            text.push_str(&section);
            text.push_str(&raw_text[index..]);
            text
        } else {
            format!("{}{}", raw_text, section)
        };
        if updated != raw_text {
            fs::write(&source_page.path, updated)?;
            changed.push(source_page.path.clone());
        }
    }
    Ok(changed)
}

/// Inject schema-suggestions JSON-LD blocks into HTML pages, applying
/// per-type augmentation. Reads schema-suggestions.json from the same
/// root; for each route in the suggestions, finds the matching HTML
/// page (index.html / route/index.html / route.html) and adds any
/// suggested types the page does not already have. Existing
/// `<script type="application/ld+json">` blocks are inspected for
/// their `@type` and respected — Aexeo never overwrites or duplicates.
///
/// Silent no-op when the suggestions file is missing. Opt-in fix: only
/// runs when the caller explicitly enables it (the default `fix`
/// invocation does not write JSON-LD into pages).
fn apply_schema_injection_fixes(root: &Path) -> Result<Vec<PathBuf>> {
    let suggestions_path = root.join("schema-suggestions.json");
    let Ok(suggestions_text) = fs::read_to_string(&suggestions_path) else {
        return Ok(Vec::new());
    };
    let Ok(payload) = serde_json::from_str::<serde_json::Value>(&suggestions_text) else {
        return Ok(Vec::new());
    };
    let Some(suggestions) = payload.get("suggestions").and_then(|v| v.as_array()) else {
        return Ok(Vec::new());
    };

    let mut changed = Vec::new();
    for entry in suggestions {
        let Some(route) = entry.get("route").and_then(|v| v.as_str()) else {
            continue;
        };
        let Some(types) = entry.get("types").and_then(|v| v.as_object()) else {
            continue;
        };
        let candidates: Vec<PathBuf> = if route.is_empty() {
            vec![root.join("index.html")]
        } else {
            vec![
                root.join(route).join("index.html"),
                root.join(format!("{route}.html")),
            ]
        };
        let Some(html_path) = candidates.into_iter().find(|p| p.exists()) else {
            continue;
        };
        let raw_text = fs::read_to_string(&html_path)?;
        let existing_types = collect_existing_schema_types(&raw_text);
        let mut to_inject = Vec::new();
        for (type_name, body) in types {
            if existing_types.contains(type_name.as_str()) {
                continue;
            }
            let body_text = serde_json::to_string(body).unwrap_or_else(|_| "{}".to_string());
            to_inject.push(format!(
                "<script type=\"application/ld+json\">{body_text}</script>"
            ));
        }
        if to_inject.is_empty() {
            continue;
        }
        let updated = inject_head_tags(&raw_text, &to_inject);
        if updated != raw_text {
            fs::write(&html_path, updated)?;
            changed.push(html_path);
        }
    }
    Ok(changed)
}

/// Scan an HTML string for the `@type` values of every existing
/// `<script type="application/ld+json">` block. We intentionally don't
/// fully parse the page or the JSON; we only need to know which types
/// are present so per-type augmentation can skip them. Both string and
/// array forms of `@type` are recognized (JSON-LD permits either).
fn collect_existing_schema_types(html: &str) -> std::collections::BTreeSet<String> {
    let mut found = std::collections::BTreeSet::new();
    let mut cursor = 0;
    let lower = html.to_ascii_lowercase();
    while let Some(start) = lower[cursor..].find("<script") {
        let absolute_start = cursor + start;
        let Some(open_end) = lower[absolute_start..].find('>') else {
            break;
        };
        let tag_end = absolute_start + open_end;
        let tag_open = &lower[absolute_start..=tag_end];
        if !tag_open.contains("application/ld+json") {
            cursor = tag_end + 1;
            continue;
        }
        let body_start = tag_end + 1;
        let Some(close_offset) = lower[body_start..].find("</script>") else {
            break;
        };
        let body_end = body_start + close_offset;
        let body = &html[body_start..body_end];
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(body.trim()) {
            collect_at_types(&value, &mut found);
        }
        cursor = body_end;
    }
    found
}

fn collect_at_types(value: &serde_json::Value, out: &mut std::collections::BTreeSet<String>) {
    match value {
        serde_json::Value::Object(map) => {
            if let Some(t) = map.get("@type") {
                match t {
                    serde_json::Value::String(s) => {
                        out.insert(s.clone());
                    }
                    serde_json::Value::Array(items) => {
                        for item in items {
                            if let serde_json::Value::String(s) = item {
                                out.insert(s.clone());
                            }
                        }
                    }
                    _ => {}
                }
            }
            for nested in map.values() {
                collect_at_types(nested, out);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                collect_at_types(item, out);
            }
        }
        _ => {}
    }
}

/// Inject `<link rel="alternate" type="text/markdown" href="...">` tags into
/// every HTML page that has a markdown mirror declared in the bundle's
/// manifest.json. Idempotent: re-runs are no-ops once the tag is present.
///
/// Reads manifest.json from the same root the fix is run on. When no
/// manifest exists (i.e. the host hasn't generated a public-bundle there),
/// the fix is a silent no-op — the safer behavior than guessing at mirror
/// paths from filename conventions.
fn apply_discovery_link_fixes(root: &Path) -> Result<Vec<PathBuf>> {
    let manifest_path = root.join("manifest.json");
    let Ok(manifest_text) = fs::read_to_string(&manifest_path) else {
        return Ok(Vec::new());
    };
    let Ok(manifest) = serde_json::from_str::<serde_json::Value>(&manifest_text) else {
        return Ok(Vec::new());
    };
    let Some(artifacts) = manifest.get("artifacts").and_then(|v| v.as_array()) else {
        return Ok(Vec::new());
    };

    // Build a route → mirror-path map from the manifest. The manifest's
    // source_route is the leading "/" form ("/about", "/"); the matching
    // HTML file lives at root/about.html, root/about/index.html, or
    // root/index.html depending on the host's flat-vs-nested layout.
    let mut mirror_by_route: BTreeMap<String, String> = BTreeMap::new();
    for entry in artifacts {
        let Some(kind) = entry.get("kind").and_then(|v| v.as_str()) else {
            continue;
        };
        if kind != "markdown_mirror" {
            continue;
        }
        let Some(path) = entry.get("path").and_then(|v| v.as_str()) else {
            continue;
        };
        let Some(route) = entry.get("source_route").and_then(|v| v.as_str()) else {
            continue;
        };
        mirror_by_route.insert(route.to_string(), path.to_string());
    }
    if mirror_by_route.is_empty() {
        return Ok(Vec::new());
    }

    let mut changed = Vec::new();
    for (route, mirror_path) in &mirror_by_route {
        // Match the route to one of the conventional HTML layouts.
        // Order matters: index.html is the canonical home, then nested
        // <route>/index.html (Astro/Hugo default), then flat <route>.html.
        let candidates: Vec<PathBuf> = if route == "/" {
            vec![root.join("index.html")]
        } else {
            let stripped = route.trim_start_matches('/');
            vec![
                root.join(stripped).join("index.html"),
                root.join(format!("{stripped}.html")),
            ]
        };
        let Some(html_path) = candidates.into_iter().find(|p| p.exists()) else {
            continue;
        };
        let raw_text = fs::read_to_string(&html_path)?;
        // Idempotency: skip if the discovery link is already present.
        // Match on the href specifically rather than just the rel/type
        // string so a different mirror path doesn't get silently
        // duplicated. The mirror path is the canonical signal.
        let href = format!("/{mirror_path}");
        let needle = format!("href=\"{href}\"");
        if raw_text.contains(&needle) {
            continue;
        }
        let tag = format!(r#"<link rel="alternate" type="text/markdown" href="{href}">"#);
        let updated = inject_head_tags(&raw_text, &[tag]);
        if updated != raw_text {
            fs::write(&html_path, updated)?;
            changed.push(html_path);
        }
    }
    Ok(changed)
}

/// Caller-controlled toggles for fixes that mutate the host's HTML beyond
/// the minimum-viable metadata path. Each flag opts into one specific
/// content-bearing change, so hosts can gate them per release rather than
/// inheriting them all when the package version bumps.
#[derive(Debug, Clone, Copy, Default)]
pub struct FixOptions {
    /// When true, inject schema-suggestions.json contents as JSON-LD
    /// blocks into matching HTML pages, applying per-type augmentation.
    /// Off by default because schema injection is content-bearing — the
    /// host is making claims about its own content; that should be
    /// deliberate.
    pub inject_schema: bool,
}

pub fn apply_safe_fixes(root: &Path, config: &Config) -> Result<Vec<PathBuf>> {
    apply_safe_fixes_with_options(root, config, FixOptions::default())
}

pub fn apply_safe_fixes_with_options(
    root: &Path,
    config: &Config,
    options: FixOptions,
) -> Result<Vec<PathBuf>> {
    let mut changed = BTreeSet::new();
    let site = load_site(root)?;

    let llms_path = root.join("llms.txt");
    if site.llms_text.is_some() {
        let generated = render_llms_txt(&site, config.site_url.as_deref());
        if site.llms_text.as_deref() != Some(generated.as_str()) {
            fs::write(&llms_path, generated)?;
            changed.insert(llms_path);
        }
    }

    if let Some(site_url) = config.site_url.as_deref() {
        let robots_path = root.join("robots.txt");
        match site.robots_text.as_deref() {
            None => {
                fs::write(&robots_path, render_robots_txt(site_url))?;
                changed.insert(robots_path);
            }
            Some(text) if !text.to_ascii_lowercase().contains("sitemap:") => {
                let updated = format!(
                    "{}\nSitemap: {}/sitemap.xml\n",
                    text.trim_end(),
                    site_url.trim_end_matches('/')
                );
                fs::write(&robots_path, updated)?;
                changed.insert(robots_path);
            }
            _ => {}
        }
    }

    let refreshed_site = load_site(root)?;
    for path in apply_html_metadata_fixes(&refreshed_site, config)? {
        changed.insert(path);
    }
    let refreshed_site = load_site(root)?;
    for path in apply_related_link_insertions(&refreshed_site, config)? {
        changed.insert(path);
    }
    // Discovery-link injection runs against the manifest (if present) so
    // hosts who've generated a public-bundle into the same root get their
    // <link rel="alternate"> tags wired up automatically. Silent no-op
    // when no manifest is found — the safer behavior than guessing at
    // mirror paths from filename conventions.
    for path in apply_discovery_link_fixes(root)? {
        changed.insert(path);
    }
    if options.inject_schema {
        for path in apply_schema_injection_fixes(root)? {
            changed.insert(path);
        }
    }

    Ok(changed.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use super::apply_safe_fixes;
    use crate::config::Config;
    use anyhow::Result;
    use std::fs;

    fn write(path: &std::path::Path, text: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, text).unwrap();
    }

    fn make_html_page(route: &str, body: &str) -> String {
        let canonical = if route.is_empty() {
            "https://example.com/".to_string()
        } else {
            format!("https://example.com/{}", route)
        };
        format!(
            "<html lang=\"en\"><head><title>x</title><meta name=\"description\" content=\"y\"><link rel=\"canonical\" href=\"{}\"></head><body><h1>x</h1>{}</body></html>",
            canonical, body,
        )
    }

    #[test]
    fn updates_llms_and_creates_robots_and_head_tags() -> Result<()> {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        write(
            &root.join("index.html"),
            "<html><head><title>x</title><meta name=\"description\" content=\"y\"></head><body><h1>x</h1></body></html>",
        );
        write(
            &root.join("features/alpha/index.html"),
            &make_html_page("features/alpha", ""),
        );
        write(
            &root.join("feature-data.json"),
            r#"{"categories":[{"id":"x","name":"X","features":[{"slug":"alpha"}]}]}"#,
        );
        write(
            &root.join("llms.txt"),
            "# Site\n\n## Key Facts\n- 9 features across 3 categories\n\n## Pages\n- [Home](index.html)\n",
        );
        let config = Config {
            site_url: Some("https://example.com".to_string()),
            ..Config::default()
        };
        let changed = apply_safe_fixes(root, &config)?;
        assert!(!changed.is_empty());
        assert!((root.join("robots.txt")).exists());
        let index_html = fs::read_to_string(root.join("index.html"))?;
        assert!(index_html.contains("rel=\"canonical\""));
        assert!(index_html.contains("property=\"og:title\""));
        assert!(index_html.contains("name=\"twitter:card\""));
        assert!(fs::read_to_string(root.join("llms.txt"))?.contains("features/alpha)"));
        Ok(())
    }

    #[test]
    fn can_insert_related_links_when_enabled() -> Result<()> {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        write(
            &root.join("index.html"),
            &make_html_page("", "<p>Guide to alpha workflows and alpha setup.</p>"),
        );
        write(
            &root.join("guide/index.html"),
            &make_html_page("guide", "<p>Guide body alpha workflows.</p>"),
        );
        write(
            &root.join("alpha/index.html"),
            &make_html_page("alpha", "<p>Alpha workflows reference.</p>"),
        );
        let config = Config {
            site_url: Some("https://example.com".to_string()),
            enable_link_autofix: true,
            related_links_heading: "Related pages".to_string(),
            ..Config::default()
        };
        apply_safe_fixes(root, &config)?;
        let updated = fs::read_to_string(root.join("index.html"))?;
        assert!(updated.contains("/alpha") || updated.contains("data-ui=\"related-links\""));
        Ok(())
    }

    #[test]
    fn injects_schema_with_per_type_augmentation_and_idempotency() -> Result<()> {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        // Page already has Article schema — that one type should be
        // skipped during injection. BreadcrumbList should still land.
        write(
            &root.join("guide.html"),
            r#"<html lang="en"><head>
                <title>Guide</title>
                <meta name="description" content="g">
                <link rel="canonical" href="https://example.com/guide">
                <script type="application/ld+json">{"@context":"https://schema.org","@type":"Article","headline":"Hand-authored"}</script>
            </head><body><h1>Guide</h1></body></html>"#,
        );
        write(
            &root.join("schema-suggestions.json"),
            r#"{
              "version": 1,
              "site_url": "https://example.com",
              "suggestions": [
                {
                  "route": "guide",
                  "types": {
                    "BreadcrumbList": {"@context": "https://schema.org", "@type": "BreadcrumbList", "itemListElement": []},
                    "Article": {"@context": "https://schema.org", "@type": "Article", "headline": "Aexeo-generated"}
                  }
                }
              ]
            }"#,
        );

        let config = Config::default();
        let options = super::FixOptions {
            inject_schema: true,
        };
        super::apply_safe_fixes_with_options(root, &config, options)?;

        let updated = fs::read_to_string(root.join("guide.html"))?;
        // Per-type augmentation: BreadcrumbList added, Article NOT
        // duplicated.
        assert!(updated.contains("\"BreadcrumbList\""));
        assert_eq!(
            updated.matches("\"Article\"").count(),
            1,
            "Article type must not be duplicated when host already authored one"
        );
        // The hand-authored Article must survive verbatim.
        assert!(updated.contains("Hand-authored"));
        assert!(!updated.contains("Aexeo-generated"));

        // Idempotency: a second run does not re-add the BreadcrumbList.
        super::apply_safe_fixes_with_options(root, &config, options)?;
        let twice = fs::read_to_string(root.join("guide.html"))?;
        assert_eq!(
            twice.matches("\"BreadcrumbList\"").count(),
            1,
            "schema injection must not duplicate types on second run"
        );
        Ok(())
    }

    #[test]
    fn schema_injection_off_by_default() -> Result<()> {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        write(
            &root.join("page.html"),
            r#"<html lang="en"><head><title>p</title><meta name="description" content="p"><link rel="canonical" href="https://example.com/page"></head><body><h1>p</h1></body></html>"#,
        );
        write(
            &root.join("schema-suggestions.json"),
            r#"{"version": 1, "site_url": "https://example.com", "suggestions": [
              {"route": "page", "types": {"BreadcrumbList": {"@context": "https://schema.org", "@type": "BreadcrumbList", "itemListElement": []}}}
            ]}"#,
        );

        let config = Config::default();
        // Default options (inject_schema=false) — schema-suggestions.json
        // is on disk but the fix should not write any schema into the
        // page. Content-bearing changes are opt-in.
        super::apply_safe_fixes(root, &config)?;
        let unchanged = fs::read_to_string(root.join("page.html"))?;
        assert!(!unchanged.contains("BreadcrumbList"));
        Ok(())
    }

    #[test]
    fn injects_discovery_links_for_manifest_listed_mirrors() -> Result<()> {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        write(&root.join("index.html"), &make_html_page("", ""));
        write(&root.join("about.html"), &make_html_page("about", ""));
        // Hand-craft a manifest matching the public-bundle output shape.
        write(
            &root.join("manifest.json"),
            r#"{
              "version": 1,
              "site_url": "https://example.com",
              "artifacts": [
                {"kind": "markdown_mirror", "path": "index.md.txt", "source_route": "/"},
                {"kind": "markdown_mirror", "path": "about.md.txt", "source_route": "/about"},
                {"kind": "sitemap", "path": "sitemap.xml"}
              ]
            }"#,
        );
        // Empty mirrors so the fix can find HTML and inject without
        // depending on real mirror generation; we're testing the
        // injection, not the bundle.
        write(&root.join("index.md.txt"), "");
        write(&root.join("about.md.txt"), "");

        let config = Config::default();
        apply_safe_fixes(root, &config)?;

        let home = fs::read_to_string(root.join("index.html"))?;
        assert!(
            home.contains(r#"href="/index.md.txt""#),
            "home should have the index mirror link"
        );
        let about = fs::read_to_string(root.join("about.html"))?;
        assert!(
            about.contains(r#"href="/about.md.txt""#),
            "about should have the about mirror link"
        );

        // Idempotency: a second run should not duplicate the link.
        apply_safe_fixes(root, &config)?;
        let home2 = fs::read_to_string(root.join("index.html"))?;
        assert_eq!(
            home2.matches(r#"href="/index.md.txt""#).count(),
            1,
            "discovery link must not be duplicated on second run"
        );
        Ok(())
    }
}

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use crate::site::{Page, Site, collapse_whitespace, strip_tags};

fn tokenize_text(text: &str) -> BTreeSet<String> {
    let mut current = String::new();
    let mut tokens = BTreeSet::new();
    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() {
            current.push(ch.to_ascii_lowercase());
        } else if !current.is_empty() {
            tokens.insert(current.clone());
            current.clear();
        }
    }
    if !current.is_empty() {
        tokens.insert(current);
    }
    tokens
}

fn categorize_routes(site: &Site) -> (Vec<String>, Vec<String>) {
    let mut pages = Vec::new();
    let mut features = Vec::new();
    for route in site.route_keys().cloned().collect::<Vec<_>>() {
        if route.is_empty() || route == "404" {
            continue;
        }
        if route.starts_with("features/") {
            features.push(route);
        } else {
            pages.push(route);
        }
    }
    pages.sort();
    features.sort();
    (pages, features)
}

fn derive_feature_counts(root: &Path, feature_routes: &[String]) -> (usize, Option<usize>) {
    let feature_data = root.join("feature-data.json");
    let Ok(text) = fs::read_to_string(&feature_data) else {
        return (feature_routes.len(), None);
    };
    let Ok(payload) = serde_json::from_str::<Value>(&text) else {
        return (feature_routes.len(), None);
    };
    let categories = match &payload {
        Value::Object(map) => map.get("categories"),
        _ => Some(&payload),
    };
    let Some(Value::Array(items)) = categories else {
        return (feature_routes.len(), None);
    };
    (feature_routes.len(), Some(items.len()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum PageKind {
    Home,
    Feature,
    Skill,
    Category,
    Maintainer,
    Listing,
    Utility,
    Legal,
    Docs,
    Detail,
    Other,
}

fn classify_page_kind(route: &str) -> PageKind {
    if route.is_empty() {
        return PageKind::Home;
    }
    if route.starts_with("features/") {
        return PageKind::Feature;
    }
    if route.starts_with("skill/") {
        return PageKind::Skill;
    }
    if route.starts_with("category/") {
        return PageKind::Category;
    }
    if route.starts_with("maintainer/") {
        return PageKind::Maintainer;
    }
    if matches!(route, "skills" | "maintainers") {
        return PageKind::Listing;
    }
    if matches!(route, "search" | "submit" | "why" | "ops") {
        return PageKind::Utility;
    }
    if matches!(route, "legal" | "privacy") || route.contains("legal") {
        return PageKind::Legal;
    }
    if route.starts_with("docs/") || route.starts_with("guide") {
        return PageKind::Docs;
    }
    if route.contains('/') {
        return PageKind::Detail;
    }
    PageKind::Other
}

fn page_kind_label(kind: PageKind) -> &'static str {
    match kind {
        PageKind::Home => "Home",
        PageKind::Feature => "Feature",
        PageKind::Skill => "Skill",
        PageKind::Category => "Category",
        PageKind::Maintainer => "Maintainer",
        PageKind::Listing => "Listing",
        PageKind::Utility => "Utility",
        PageKind::Legal => "Legal",
        PageKind::Docs => "Docs",
        PageKind::Detail => "Detail",
        PageKind::Other => "Other",
    }
}

fn page_kind_heading(kind: PageKind) -> &'static str {
    match kind {
        PageKind::Home => "Home",
        PageKind::Feature => "Feature Pages",
        PageKind::Skill => "Skill Pages",
        PageKind::Category => "Category Pages",
        PageKind::Maintainer => "Maintainer Pages",
        PageKind::Listing => "Listing Pages",
        PageKind::Utility => "Utility Pages",
        PageKind::Legal => "Legal Pages",
        PageKind::Docs => "Docs Pages",
        PageKind::Detail => "Detail Pages",
        PageKind::Other => "Other Pages",
    }
}

fn schema_types_for_page(page: &Page) -> Vec<String> {
    let mut found = BTreeSet::new();
    for block in &page.json_ld_blocks {
        let Ok(payload) = serde_json::from_str::<Value>(&block.raw) else {
            continue;
        };
        collect_schema_types(&payload, &mut found);
    }
    found.into_iter().collect()
}

fn collect_schema_types(payload: &Value, out: &mut BTreeSet<String>) {
    match payload {
        Value::Object(map) => {
            if let Some(value) = map.get("@type") {
                match value {
                    Value::String(text) => {
                        out.insert(text.clone());
                    }
                    Value::Array(items) => {
                        for item in items {
                            if let Value::String(text) = item {
                                out.insert(text.clone());
                            }
                        }
                    }
                    _ => {}
                }
            }
            for nested in map.values() {
                collect_schema_types(nested, out);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_schema_types(item, out);
            }
        }
        _ => {}
    }
}

fn page_summary(page: &Page) -> String {
    let mut fragments = Vec::new();
    if let Some(title) = &page.title {
        fragments.push(title.clone());
    }
    if let Some(description) = page.meta_description() {
        fragments.push(description.to_string());
    }
    if let Some(h1) = page.h1_texts.first() {
        fragments.push(h1.clone());
    }
    let text = if fragments.is_empty() {
        visible_text(&page.raw_text)
    } else {
        fragments.join(". ")
    };
    text.chars().take(320).collect()
}

fn page_related_links(page: &Page, limit: usize) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut links = Vec::new();
    for link in &page.internal_links {
        if seen.insert(link) {
            links.push(link.clone());
        }
        if links.len() >= limit {
            break;
        }
    }
    links
}

fn page_outline(page: &Page, limit: usize) -> Vec<String> {
    let mut outline = Vec::new();
    for block in &page.blocks {
        let label = block.data_ui.clone().or_else(|| {
            if block.has_heading {
                Some(block.tag.clone())
            } else {
                None
            }
        });
        if let Some(label) = label {
            let text = collapse_whitespace(&block.text);
            let snippet: String = text.chars().take(120).collect();
            if snippet.is_empty() {
                outline.push(label);
            } else {
                outline.push(format!("{}: {}", label, snippet));
            }
        }
        if outline.len() >= limit {
            break;
        }
    }
    outline
}

fn image_alt_summary(page: &Page) -> String {
    let total = page.images.len();
    if total == 0 {
        return "0/0 images with alt text".to_string();
    }
    let with_alt = page
        .images
        .iter()
        .filter(|image| {
            image
                .alt
                .as_deref()
                .is_some_and(|alt| !alt.trim().is_empty())
        })
        .count();
    format!("{}/{} images with alt text", with_alt, total)
}

fn render_page_metadata(page: &Page, route: &str) -> Vec<String> {
    let kind = classify_page_kind(route);
    let schema_types = schema_types_for_page(page);
    let related_links = page_related_links(page, 5);
    let mut lines = vec![
        format!("- Kind: {}", page_kind_label(kind)),
        format!(
            "- URL: {}",
            if route.is_empty() {
                "/".to_string()
            } else {
                format!("/{}", route)
            }
        ),
        format!(
            "- Canonical: {}",
            page.canonical.as_deref().unwrap_or("(none)")
        ),
        format!(
            "- HTML lang: {}",
            page.html_lang.as_deref().unwrap_or("(none)")
        ),
        format!(
            "- H1: {}",
            page.h1_texts
                .first()
                .map(String::as_str)
                .unwrap_or("(none)")
        ),
        format!(
            "- Description: {}",
            page.meta_description().unwrap_or("(none)")
        ),
        format!(
            "- Structured data: {}",
            if schema_types.is_empty() {
                "(none)".to_string()
            } else {
                schema_types.join(", ")
            }
        ),
        format!("- Internal links: {}", page.internal_links.len()),
        format!("- Image coverage: {}", image_alt_summary(page)),
        format!("- Summary: {}", page_summary(page)),
    ];
    if !related_links.is_empty() {
        lines.push(format!("- Related links: {}", related_links.join(", ")));
    }
    let outline = page_outline(page, 3);
    if !outline.is_empty() {
        lines.push(format!("- Outline: {}", outline.join(" | ")));
    }
    lines
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GeneratedMachineArtifact {
    pub path: String,
    pub kind: String,
    pub bytes: usize,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MachineArtifactBundle {
    pub artifacts: Vec<GeneratedMachineArtifact>,
    pub route_count: usize,
    pub markdown_pages: usize,
    pub deploy_notes: Vec<String>,
}

pub fn render_llms_txt(site: &Site, _site_url: Option<&str>) -> String {
    let (pages, feature_routes) = categorize_routes(site);
    let (feature_count, category_count) = derive_feature_counts(&site.root, &feature_routes);
    let mut lines = vec!["# Site".to_string(), String::new()];
    if let Some(category_count) = category_count {
        lines.push("## Key Facts".to_string());
        lines.push(format!(
            "- {} features across {} categories",
            feature_count, category_count
        ));
        lines.push(String::new());
    }
    lines.push("## Pages".to_string());
    let mut grouped: BTreeMap<PageKind, Vec<String>> = BTreeMap::new();
    for route in std::iter::once(String::new()).chain(pages) {
        grouped
            .entry(classify_page_kind(&route))
            .or_default()
            .push(route);
    }
    for (kind, routes) in grouped {
        lines.push(format!("### {}", page_kind_heading(kind)));
        for route in routes {
            let label = if route.is_empty() {
                "Home".to_string()
            } else {
                route
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
                    .join(" ")
            };
            let href = if route.is_empty() {
                "/".to_string()
            } else {
                format!("/{}", route)
            };
            lines.push(format!("- [{}]({})", label, href));
        }
        lines.push(String::new());
    }
    if !feature_routes.is_empty() {
        lines.push(String::new());
        lines.push(format!(
            "## Feature Pages ({} individual feature deep-dives)",
            feature_routes.len()
        ));
        for route in feature_routes {
            let slug = route.trim_start_matches("features/");
            let label = slug
                .split('-')
                .map(|part| {
                    let mut chars = part.chars();
                    match chars.next() {
                        Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
                        None => String::new(),
                    }
                })
                .collect::<Vec<_>>()
                .join(" ");
            lines.push(format!("- [{}](/{} )", label, route).replace(" )", ")"));
        }
    }
    lines.push(String::new());
    lines.join("\n")
}

fn visible_text(raw_text: &str) -> String {
    collapse_whitespace(&strip_tags(raw_text))
}

fn markdown_escape_heading(text: &str) -> String {
    text.replace('\n', " ").trim().to_string()
}

fn markdown_path_for_route(route: &str) -> String {
    if route.is_empty() {
        "index.md.txt".to_string()
    } else {
        format!("{route}.md.txt")
    }
}

fn render_page_markdown_mirror(route: &str, page: &Page) -> String {
    let title = page.title.clone().unwrap_or_else(|| {
        if route.is_empty() {
            "Home".to_string()
        } else {
            route.replace('/', " / ")
        }
    });
    let mut lines = vec![
        format!("# {}", markdown_escape_heading(&title)),
        String::new(),
    ];
    lines.extend(render_page_metadata(page, route));
    lines.push(String::new());

    if page.blocks.is_empty() {
        let text = visible_text(&page.raw_text);
        lines.push(if text.is_empty() {
            "_No visible text._".to_string()
        } else {
            text
        });
        return lines.join("\n").trim_end().to_string() + "\n";
    }

    for block in &page.blocks {
        let heading = block
            .data_ui
            .clone()
            .unwrap_or_else(|| page_kind_heading(classify_page_kind(route)).to_string());
        lines.push(format!("## {}", markdown_escape_heading(&heading)));
        lines.push(String::new());
        let text = collapse_whitespace(&block.text);
        lines.push(if text.is_empty() {
            "_No visible text._".to_string()
        } else {
            text
        });
        lines.push(String::new());
    }
    lines.join("\n").trim_end().to_string() + "\n"
}

pub fn render_markdown_mirror_pages(site: &Site) -> Vec<GeneratedMachineArtifact> {
    site.route_page_pairs()
        .filter(|(route, _)| route.as_str() != "404")
        .map(|(route, page)| {
            let content = render_page_markdown_mirror(route, page);
            GeneratedMachineArtifact {
                path: markdown_path_for_route(route),
                kind: "markdown_mirror".to_string(),
                bytes: content.len(),
                content,
            }
        })
        .collect()
}

fn render_bundle_llms_txt(site: &Site, site_url: Option<&str>) -> String {
    let mut text = render_llms_txt(site, site_url);
    let mirrors = render_markdown_mirror_pages(site);
    if !mirrors.is_empty() {
        if !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str("\n## Markdown Mirrors\n");
        for artifact in mirrors {
            let label = artifact
                .path
                .strip_suffix(".md.txt")
                .unwrap_or(&artifact.path)
                .replace('-', " ")
                .replace('/', " / ");
            text.push_str(&format!(
                "- [{}](/{}): LLM-readable Markdown mirror\n",
                label, artifact.path
            ));
        }
    }
    text
}

fn machine_artifact(path: &str, kind: &str, content: String) -> GeneratedMachineArtifact {
    GeneratedMachineArtifact {
        path: path.to_string(),
        kind: kind.to_string(),
        bytes: content.len(),
        content,
    }
}

pub fn build_machine_artifact_bundle(site: &Site, site_url: Option<&str>) -> MachineArtifactBundle {
    let mut artifacts = vec![
        machine_artifact(
            "llms.txt",
            "llms_index",
            render_bundle_llms_txt(site, site_url),
        ),
        machine_artifact(
            "llms-full.txt",
            "llms_full_context",
            render_llms_full_txt(site, site_url),
        ),
    ];
    if let Some(url) = site_url {
        artifacts.push(machine_artifact(
            "sitemap.xml",
            "sitemap",
            render_sitemap_xml(site, url),
        ));
        artifacts.push(machine_artifact(
            "robots.txt",
            "robots",
            render_robots_txt(url),
        ));
    }
    let facts = crate::intelligence::generate_truth_manifest_with_options(site, false);
    let facts_content =
        serde_json::to_string_pretty(&facts.manifest).unwrap_or_else(|_| "{}".to_string());
    artifacts.push(machine_artifact(
        "facts.json",
        "facts_manifest",
        facts_content,
    ));

    // Schema suggestions: emitted only when a site_url is configured
    // (every synthesized type needs absolute URLs). The bundle ships
    // them in one JSON file; the inject-schema fix reads it to apply
    // per-type augmentation into HTML <head> blocks. Suggestions are
    // additive — never replacements — so the artifact is safe to ship
    // even when consumers don't run the inject-fix.
    if let Some(url) = site_url {
        let suggestions = crate::intelligence::generate_schema_suggestions(site, Some(url));
        if !suggestions.is_empty() {
            let payload = serde_json::json!({
                "version": 1,
                "site_url": url,
                "suggestions": suggestions,
            });
            let content =
                serde_json::to_string_pretty(&payload).unwrap_or_else(|_| "{}".to_string());
            artifacts.push(machine_artifact(
                "schema-suggestions.json",
                "schema_suggestions",
                content,
            ));
        }
    }
    let markdown_pages = render_markdown_mirror_pages(site);
    let markdown_count = markdown_pages.len();
    artifacts.extend(markdown_pages);
    artifacts.sort_by(|left, right| left.path.cmp(&right.path));

    // The discovery manifest lists every artifact in the bundle with its
    // bundle-relative path, kind, byte size, and optional public URL. It
    // exists so an LLM crawler (or a `<link rel="alternate">` injector,
    // see `aexeo-cli fix add-discovery-links`) can find every machine
    // surface in one fetch instead of probing per-route. Computed AFTER
    // the other artifacts and inserted last so it self-references its
    // own entry consistently — and we sort once more so the final
    // artifact ordering on disk stays alphabetical.
    let manifest_content = render_discovery_manifest(&artifacts, site_url);
    artifacts.push(machine_artifact(
        "manifest.json",
        "discovery_manifest",
        manifest_content,
    ));
    artifacts.sort_by(|left, right| left.path.cmp(&right.path));

    MachineArtifactBundle {
        artifacts,
        route_count: site.route_page_pairs().count(),
        markdown_pages: markdown_count,
        deploy_notes: vec![
            "Deploy generated files at the same relative paths from the public site root.".to_string(),
            "Review facts.json before publishing; generated facts are a deterministic first draft, not a legal source of truth.".to_string(),
            "Run `aexeo-cli fix dist` to inject manifest-driven `<link rel=\"alternate\">` discovery tags into HTML <head>; pass `--inject-schema` if you also want the synthesized JSON-LD blocks injected (per-type augmentation, off by default).".to_string(),
        ],
    }
}

fn render_discovery_manifest(
    artifacts: &[GeneratedMachineArtifact],
    site_url: Option<&str>,
) -> String {
    let entries: Vec<serde_json::Value> = artifacts
        .iter()
        .map(|artifact| {
            let url = site_url.map(|base| {
                let normalized = base.trim_end_matches('/');
                format!("{}/{}", normalized, artifact.path)
            });
            // Markdown mirrors carry their source-route name as a stem
            // (e.g. about.md.txt → /about). Surface that explicitly so
            // the link-injector can match mirrors back to their HTML
            // source pages without re-deriving the convention.
            let source_route = artifact.path.strip_suffix(".md.txt").map(|stem| {
                if stem == "index" {
                    "/".to_string()
                } else {
                    format!("/{}", stem)
                }
            });
            let mut entry = serde_json::Map::new();
            entry.insert(
                "kind".to_string(),
                serde_json::Value::String(artifact.kind.clone()),
            );
            entry.insert(
                "path".to_string(),
                serde_json::Value::String(artifact.path.clone()),
            );
            entry.insert(
                "bytes".to_string(),
                serde_json::Value::Number(serde_json::Number::from(artifact.bytes)),
            );
            if let Some(url) = url {
                entry.insert("url".to_string(), serde_json::Value::String(url));
            }
            if let Some(route) = source_route {
                entry.insert("source_route".to_string(), serde_json::Value::String(route));
            }
            serde_json::Value::Object(entry)
        })
        .collect();
    let manifest = serde_json::json!({
        "version": 1,
        "site_url": site_url,
        "artifacts": entries,
    });
    serde_json::to_string_pretty(&manifest).unwrap_or_else(|_| "{}".to_string())
}

pub fn render_llms_full_txt(site: &Site, _site_url: Option<&str>) -> String {
    let mut lines = vec!["# Site Full Context".to_string(), String::new()];
    for (route, page) in site.route_page_pairs() {
        let label = page.title.clone().unwrap_or_else(|| {
            if route.is_empty() {
                "Home".to_string()
            } else {
                route.clone()
            }
        });
        lines.push(format!("## {}", label));
        lines.extend(render_page_metadata(page, route));
        lines.push(String::new());
    }
    lines.join("\n").trim_end().to_string() + "\n"
}

pub fn render_markdown_mirror(site: &Site) -> String {
    let mut lines = vec!["# Site Mirror".to_string(), String::new()];
    for (route, page) in site.route_page_pairs() {
        let title = page.title.clone().unwrap_or_else(|| {
            if route.is_empty() {
                "Home".to_string()
            } else {
                route.clone()
            }
        });
        lines.push(format!("## {}", title));
        lines.push(String::new());
        lines.extend(render_page_metadata(page, route));
        lines.push(String::new());
        if page.blocks.is_empty() {
            let text = visible_text(&page.raw_text);
            lines.push(if text.is_empty() {
                "_No visible text._".to_string()
            } else {
                text
            });
            lines.push(String::new());
            continue;
        }
        for block in &page.blocks {
            let heading = block.data_ui.clone().unwrap_or_else(|| block.tag.clone());
            lines.push(format!("### {}", heading));
            lines.push(String::new());
            let text = collapse_whitespace(&block.text);
            lines.push(if text.is_empty() {
                "_No visible text._".to_string()
            } else {
                text
            });
            lines.push(String::new());
        }
    }
    lines.join("\n").trim_end().to_string() + "\n"
}

pub fn render_robots_txt(site_url: &str) -> String {
    let normalized = site_url.trim_end_matches('/');
    format!(
        "User-agent: *\nAllow: /\nSitemap: {}/sitemap.xml\n",
        normalized
    )
}

fn page_is_indexable(page: &Page) -> bool {
    let robots_meta = page.metadata("robots").unwrap_or("").to_ascii_lowercase();
    if robots_meta.contains("noindex") {
        return false;
    }
    let robots_header = page
        .response_headers
        .get("x-robots-tag")
        .map(String::as_str)
        .unwrap_or("")
        .to_ascii_lowercase();
    if robots_header.contains("noindex") {
        return false;
    }
    true
}

fn route_to_url(site_url: &str, route: &str) -> String {
    let base = site_url.trim_end_matches('/');
    if route.is_empty() {
        format!("{}/", base)
    } else {
        format!("{}/{}", base, route)
    }
}

fn xml_escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\'' => out.push_str("&apos;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(ch),
        }
    }
    out
}

pub fn render_sitemap_xml(site: &Site, site_url: &str) -> String {
    let mut routes: Vec<&str> = site
        .route_page_pairs()
        .filter(|(route, page)| {
            if route.as_str() == "404" {
                return false;
            }
            page_is_indexable(page)
        })
        .map(|(route, _)| route.as_str())
        .collect();
    routes.sort();

    let mut out = String::new();
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    out.push_str("<urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">\n");
    for route in routes {
        let url = route_to_url(site_url, route);
        out.push_str("  <url>\n");
        out.push_str(&format!("    <loc>{}</loc>\n", xml_escape(&url)));
        out.push_str("  </url>\n");
    }
    out.push_str("</urlset>\n");
    out
}

fn tokenize_route(route: &str) -> BTreeSet<String> {
    tokenize_text(route)
}

fn collect_page_tokens(page: &Page) -> BTreeSet<String> {
    let mut tokens = tokenize_route(&page.route);
    if let Some(title) = &page.title {
        tokens.extend(tokenize_text(title));
    }
    if let Some(description) = page.meta_description() {
        tokens.extend(tokenize_text(description));
    }
    for h1 in &page.h1_texts {
        tokens.extend(tokenize_text(h1));
    }
    for block in &page.blocks {
        if let Some(data_ui) = &block.data_ui {
            tokens.extend(tokenize_text(data_ui));
        }
        if block.has_heading || block.text.len() > 80 {
            let limited = block
                .text
                .split_whitespace()
                .take(80)
                .collect::<Vec<_>>()
                .join(" ");
            tokens.extend(tokenize_text(&limited));
        }
    }
    tokens
}

fn score_link_candidate(
    site: &Site,
    route_tokens: &BTreeMap<String, BTreeSet<String>>,
    source: &str,
    target: &str,
) -> usize {
    let source_tokens = route_tokens.get(source).cloned().unwrap_or_default();
    let target_tokens = route_tokens.get(target).cloned().unwrap_or_default();
    let shared_score = source_tokens.intersection(&target_tokens).count();
    let source_prefix = source.split('/').next().unwrap_or("");
    let target_prefix = target.split('/').next().unwrap_or("");
    let prefix_score = if source_prefix == target_prefix { 3 } else { 0 };
    let target_inbound = site
        .inbound_links
        .get(target)
        .map(|set| set.len())
        .unwrap_or(0);
    let weakness_bonus = if target_inbound < 2 {
        3
    } else if target_inbound < 4 {
        1
    } else {
        0
    };
    shared_score + prefix_score + weakness_bonus
}

fn collect_link_candidate_scores(
    site: &Site,
    route_tokens: &BTreeMap<String, BTreeSet<String>>,
    source: &str,
    routes: &[String],
) -> Vec<(usize, String)> {
    let Some(source_page) = site.page(source) else {
        return Vec::new();
    };
    let mut scored = Vec::new();
    for target in routes {
        if target == source || source_page.internal_links.iter().any(|link| link == target) {
            continue;
        }
        let score = score_link_candidate(site, route_tokens, source, target);
        if score > 2 {
            scored.push((score, target.clone()));
        }
    }
    scored.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(&right.1)));
    scored
}

pub fn build_link_suggestions(site: &Site, top_n: usize) -> BTreeMap<String, Vec<String>> {
    let route_tokens: BTreeMap<String, BTreeSet<String>> = site
        .route_keys()
        .filter(|route| route.as_str() != "404")
        .filter_map(|route| {
            site.page(route)
                .map(|page| (route.clone(), collect_page_tokens(page)))
        })
        .collect();
    let routes: Vec<String> = site
        .route_keys()
        .filter(|route| route.as_str() != "404")
        .cloned()
        .collect();
    let mut candidates = BTreeMap::new();
    for source in &routes {
        let scored = collect_link_candidate_scores(site, &route_tokens, source, &routes);
        if !scored.is_empty() {
            candidates.insert(
                source.clone(),
                scored
                    .into_iter()
                    .take(top_n)
                    .map(|(_, route)| route)
                    .collect(),
            );
        }
    }
    candidates
}

pub fn suggest_internal_links(site: &Site, top_n: usize) -> String {
    let candidates = build_link_suggestions(site, top_n);
    if candidates.is_empty() {
        return "No internal-link suggestions.".to_string();
    }
    let mut lines = vec!["# Internal Link Suggestions".to_string(), String::new()];
    for (route, suggestions) in candidates {
        lines.push(format!("## /{}", route));
        for suggestion in suggestions {
            lines.push(format!("- add link to `/{}`", suggestion));
        }
        lines.push(String::new());
    }
    lines.join("\n").trim_end().to_string() + "\n"
}

#[cfg(test)]
mod tests {
    use super::{
        build_link_suggestions, build_machine_artifact_bundle, render_llms_full_txt,
        render_llms_txt, render_markdown_mirror, render_markdown_mirror_pages, render_sitemap_xml,
        suggest_internal_links,
    };
    use crate::site::load_site;
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
    fn generates_llm_artifacts_and_link_suggestions() -> Result<()> {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        write(
            &root.join("index.html"),
            &make_html_page("", "<a href=\"/guide\">Guide</a>"),
        );
        write(&root.join("guide.html"), &make_html_page("guide", ""));
        write(
            &root.join("features/alpha/index.html"),
            &make_html_page("features/alpha", ""),
        );
        write(
            &root.join("feature-data.json"),
            r#"{"categories":[{"id":"x","name":"X","features":[{"slug":"alpha"}]}]}"#,
        );

        let site = load_site(root)?;
        let llms_txt = render_llms_txt(&site, None);
        assert!(llms_txt.contains("## Pages"));
        assert!(llms_txt.contains("## Feature Pages"));
        assert!(llms_txt.contains("### Home"));

        let llms_full = render_llms_full_txt(&site, None);
        assert!(llms_full.contains("# Site Full Context"));
        assert!(llms_full.contains("- Kind: Home"));
        assert!(llms_full.contains("- Structured data:"));
        assert!(llms_full.contains("- Related links:"));

        let mirror = render_markdown_mirror(&site);
        assert!(mirror.contains("# Site Mirror"));
        assert!(mirror.contains("- Canonical:"));
        assert!(mirror.contains("- Image coverage:"));
        assert!(suggest_internal_links(&site, 3).contains("/features/alpha"));
        let suggestion_map = build_link_suggestions(&site, 3);
        assert!(!suggestion_map.get("").unwrap_or(&Vec::new()).is_empty());
        Ok(())
    }

    #[test]
    fn sitemap_emits_indexable_routes_and_excludes_noindex_and_404() -> Result<()> {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        write(&root.join("index.html"), &make_html_page("", ""));
        write(&root.join("about.html"), &make_html_page("about", ""));
        // 404 is excluded by route, regardless of indexability.
        write(&root.join("404.html"), &make_html_page("404", ""));
        // noindex via meta robots — must be excluded.
        write(
            &root.join("hidden.html"),
            "<html lang=\"en\"><head><title>x</title><meta name=\"description\" content=\"y\"><meta name=\"robots\" content=\"noindex\"><link rel=\"canonical\" href=\"https://example.com/hidden\"></head><body><h1>x</h1></body></html>",
        );
        let site = load_site(root)?;
        let xml = render_sitemap_xml(&site, "https://example.com");
        assert!(xml.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>"));
        assert!(xml.contains("<loc>https://example.com/</loc>"));
        assert!(xml.contains("<loc>https://example.com/about</loc>"));
        assert!(!xml.contains("/hidden"));
        assert!(!xml.contains("/404"));
        // Trailing slash on site_url should not produce double-slash on root URL.
        let xml_trailing = render_sitemap_xml(&site, "https://example.com/");
        assert!(xml_trailing.contains("<loc>https://example.com/</loc>"));
        assert!(!xml_trailing.contains("https://example.com//"));
        Ok(())
    }

    #[test]
    fn machine_bundle_includes_sitemap_and_robots_when_site_url_set() -> Result<()> {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        write(&root.join("index.html"), &make_html_page("", ""));
        let site = load_site(root)?;
        let with_url = build_machine_artifact_bundle(&site, Some("https://example.com"));
        let names: Vec<_> = with_url.artifacts.iter().map(|a| a.path.as_str()).collect();
        assert!(names.contains(&"sitemap.xml"));
        assert!(names.contains(&"robots.txt"));
        let without = build_machine_artifact_bundle(&site, None);
        let names: Vec<_> = without.artifacts.iter().map(|a| a.path.as_str()).collect();
        assert!(!names.contains(&"sitemap.xml"));
        assert!(!names.contains(&"robots.txt"));
        Ok(())
    }

    #[test]
    fn public_bundle_emits_discovery_manifest_listing_every_artifact() -> Result<()> {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        write(&root.join("index.html"), &make_html_page("", ""));
        write(&root.join("about.html"), &make_html_page("about", ""));
        let site = load_site(root)?;
        let bundle = build_machine_artifact_bundle(&site, Some("https://example.com"));

        // Manifest must be present.
        let manifest = bundle
            .artifacts
            .iter()
            .find(|a| a.path == "manifest.json")
            .expect("public-bundle should emit manifest.json");
        assert_eq!(manifest.kind, "discovery_manifest");
        let payload: serde_json::Value = serde_json::from_str(&manifest.content)?;
        assert_eq!(payload["version"], serde_json::json!(1));
        assert_eq!(
            payload["site_url"],
            serde_json::json!("https://example.com")
        );
        let entries = payload["artifacts"]
            .as_array()
            .expect("artifacts should be an array");
        // Every artifact except the manifest itself is referenced. The
        // manifest doesn't self-reference (a crawler that fetched it
        // already knows what file it just got).
        for artifact in &bundle.artifacts {
            if artifact.path == "manifest.json" {
                continue;
            }
            assert!(
                entries
                    .iter()
                    .any(|entry| entry["path"] == serde_json::Value::String(artifact.path.clone())),
                "manifest missing entry for {}",
                artifact.path
            );
        }
        assert!(
            entries
                .iter()
                .all(|entry| entry["path"] != serde_json::json!("manifest.json")),
            "manifest should not self-reference",
        );
        // Markdown mirrors carry their source-route mapping so the link
        // injector can match mirrors back to their source pages.
        let about_entry = entries
            .iter()
            .find(|e| e["path"] == serde_json::json!("about.md.txt"))
            .expect("manifest should reference about.md.txt");
        assert_eq!(about_entry["source_route"], serde_json::json!("/about"));
        let index_entry = entries
            .iter()
            .find(|e| e["path"] == serde_json::json!("index.md.txt"))
            .expect("manifest should reference index.md.txt");
        assert_eq!(index_entry["source_route"], serde_json::json!("/"));
        Ok(())
    }

    #[test]
    fn generates_machine_artifact_bundle_with_page_markdown_and_facts() -> Result<()> {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        write(
            &root.join("index.html"),
            &make_html_page(
                "",
                "<section data-ui=\"hero\"><h2>Hero</h2><p>Official product overview.</p></section>",
            ),
        );
        write(
            &root.join("features/search/index.html"),
            &make_html_page(
                "features/search",
                "<section data-ui=\"feature\"><h2>Feature</h2><p>Search feature details.</p></section>",
            ),
        );

        let site = load_site(root)?;
        let markdown_pages = render_markdown_mirror_pages(&site);
        assert_eq!(markdown_pages.len(), 2);
        assert!(
            markdown_pages
                .iter()
                .any(|artifact| artifact.path == "features/search.md.txt")
        );
        assert!(
            markdown_pages
                .iter()
                .any(|artifact| artifact.content.contains("Official product overview"))
        );

        let bundle = build_machine_artifact_bundle(&site, Some("https://example.com"));
        assert!(
            bundle
                .artifacts
                .iter()
                .any(|artifact| artifact.path == "facts.json")
        );
        assert!(
            bundle
                .artifacts
                .iter()
                .any(|artifact| artifact.path == "llms.txt"
                    && artifact.content.contains("## Markdown Mirrors"))
        );
        assert_eq!(bundle.markdown_pages, 2);
        Ok(())
    }
}

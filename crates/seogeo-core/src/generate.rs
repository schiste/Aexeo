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
    for route in std::iter::once(String::new()).chain(pages.into_iter()) {
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
        build_link_suggestions, render_llms_full_txt, render_llms_txt, render_markdown_mirror,
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
}

use std::path::{Path, PathBuf};

use crate::config::RuntimeConfig;
use crate::site::normalize_internal_href;

const ASSET_EXTENSIONS: &[&str] = &[
    ".css", ".gif", ".html", ".ico", ".jpeg", ".jpg", ".js", ".json", ".mjs", ".png", ".svg",
    ".txt", ".webp", ".xml",
];

pub(crate) fn response_report_path(route: &str) -> String {
    if route.is_empty() {
        "crawl/index.html".to_string()
    } else {
        format!("crawl/{}/index.html", route)
    }
}

pub(crate) fn snapshot_path_for_route(root: &Path, route: &str) -> PathBuf {
    if route.is_empty() {
        root.join("index.html")
    } else {
        root.join(route).join("index.html")
    }
}

fn attr_value(snippet: &str, attr: &str) -> Option<String> {
    let lower = snippet.to_ascii_lowercase();
    let marker = format!("{}=", attr.to_ascii_lowercase());
    let index = lower.find(&marker)?;
    let after = &snippet[index + marker.len()..];
    let mut chars = after.chars();
    let quote = chars.next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let rest = &after[quote.len_utf8()..];
    let end = rest.find(quote)?;
    Some(rest[..end].to_string())
}

pub(crate) fn extract_internal_links(raw: &str) -> Vec<String> {
    let mut links = Vec::new();
    let mut offset = 0;
    while let Some(index) = raw[offset..].find("<a") {
        let start = offset + index;
        let Some(open_end_rel) = raw[start..].find('>') else {
            break;
        };
        let open_end = start + open_end_rel;
        let snippet = &raw[start..=open_end];
        if let Some(href) = attr_value(snippet, "href")
            && let Some(target) = normalize_internal_href(&href)
        {
            links.push(target);
        }
        offset = open_end + 1;
    }
    links
}

pub(crate) fn should_enqueue_link(target: &str) -> bool {
    let suffix = Path::new(target)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| format!(".{}", ext.to_ascii_lowercase()))
        .unwrap_or_default();
    suffix.is_empty() || !ASSET_EXTENSIONS.contains(&suffix.as_str()) || suffix == ".html"
}

pub(crate) fn read_loc_values(text: &str) -> Vec<String> {
    let mut values = Vec::new();
    let mut offset = 0;
    while let Some(start_rel) = text[offset..].find("<loc>") {
        let start = offset + start_rel + 5;
        let Some(end_rel) = text[start..].find("</loc>") else {
            break;
        };
        let end = start + end_rel;
        values.push(text[start..end].trim().to_string());
        offset = end + 6;
    }
    values
}

fn route_matches_patterns(route: &str, patterns: &[String]) -> bool {
    patterns.is_empty()
        || patterns
            .iter()
            .any(|pattern| route.contains(pattern.trim_matches('/')))
}

fn route_is_excluded(route: &str, patterns: &[String]) -> bool {
    patterns
        .iter()
        .any(|pattern| route.contains(pattern.trim_matches('/')))
}

pub(crate) fn route_is_allowed(route: &str, runtime: &RuntimeConfig<'_>) -> bool {
    !route_is_excluded(route, runtime.crawl_exclude_patterns)
        && route_matches_patterns(route, runtime.crawl_include_patterns)
}

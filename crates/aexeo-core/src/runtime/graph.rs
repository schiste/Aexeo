use std::path::Path;

use crate::config::RuntimeConfig;
use crate::link_evidence::crawl_links;
use crate::site::audited_link_targets;

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

pub(crate) fn should_enqueue_link(target: &str) -> bool {
    let suffix = Path::new(target)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| format!(".{}", ext.to_ascii_lowercase()))
        .unwrap_or_default();
    suffix.is_empty() || !ASSET_EXTENSIONS.contains(&suffix.as_str()) || suffix == ".html"
}

/// Routes a fetched page offers the planner: the links a browser would resolve
/// from the page URL, and every root-relative link the audit checks. The audit
/// also reads anchors inert in the DOM, such as `<template>` and `<noscript>`
/// contents; leaving them out let a complete crawl report a link broken that it
/// never requested.
pub(crate) fn page_link_routes(body: &str, page_url: &str) -> Vec<String> {
    crawl_links(body, page_url)
        .into_iter()
        .map(|link| link.route)
        .chain(audited_link_targets(body))
        .filter(|route| should_enqueue_link(route))
        .collect()
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
    !route.starts_with("cdn-cgi/")
        && !route_is_excluded(route, runtime.crawl_exclude_patterns)
        && route_matches_patterns(route, runtime.crawl_include_patterns)
}

#[cfg(test)]
mod tests {
    use super::{page_link_routes, route_is_allowed};
    use crate::config::Config;

    #[test]
    fn rejects_cloudflare_infrastructure_routes() {
        let config = Config::default();
        let runtime = config.runtime();
        assert!(!route_is_allowed("cdn-cgi/l/email-protection", &runtime));
        assert!(route_is_allowed("features/terminal-search", &runtime));
    }

    #[test]
    fn page_links_include_every_route_the_audit_checks() {
        let html = r#"<html><body><a href="docs">Docs</a>
            <template><a href="/legal-notice">Legal</a></template>
            <noscript><a href="/privacy-policy">Privacy</a></noscript>
            <script>var a = '<a href="/scripted">x</a>';</script>
            <a href="/logo.png">logo</a></body></html>"#;
        let routes = page_link_routes(html, "https://www.example.com/en/article");
        for route in ["en/docs", "legal-notice", "privacy-policy"] {
            assert!(
                routes.iter().any(|seen| seen == route),
                "{route}: {routes:?}"
            );
        }
        assert!(!routes.iter().any(|seen| seen == "scripted"), "{routes:?}");
        assert!(!routes.iter().any(|seen| seen == "logo.png"), "{routes:?}");
    }
}

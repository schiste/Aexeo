use std::path::Path;

use super::http::{host_for_url, origin_for_url, same_site_host};
use crate::config::RuntimeConfig;
use crate::link_evidence::extract_links;
use crate::link_graph::LinkResolution;
use crate::site::route_from_urlish;

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

/// One `<a href>` of a crawled document, resolved against its observed URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CrawlLink {
    pub(crate) href: String,
    pub(crate) anchor: String,
    pub(crate) route: String,
}

/// Same-site links of a document fetched from `page_url`, in DOM order.
///
/// Parsing and resolution reuse the link graph path, so relative references,
/// unquoted attributes and `<base href>` behave as in a browser. Bare and `www.`
/// hosts count as one site, as they do for the crawl itself.
///
/// The query string is deliberately not part of a route: `?page=2` resolves to
/// the route of its path, so query variants, including query pagination, are
/// never requested separately. This keeps faceted URLs from becoming a crawl
/// trap, at the cost of not reaching pages linked only through a query variant.
pub(crate) fn crawl_links(html: &str, page_url: &str) -> Vec<CrawlLink> {
    let site_host = host_for_url(page_url);
    extract_links(html, page_url, &origin_for_url(page_url), None)
        .into_iter()
        .filter_map(|link| {
            let url = match &link.resolution {
                LinkResolution::Internal { url } => url,
                LinkResolution::External { url }
                    if same_site_host(&site_host, &host_for_url(url)) =>
                {
                    url
                }
                _ => return None,
            };
            let route = route_from_urlish(url)?;
            Some(CrawlLink {
                href: link.href,
                anchor: link.anchor,
                route,
            })
        })
        .collect()
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
    !route.starts_with("cdn-cgi/")
        && !route_is_excluded(route, runtime.crawl_exclude_patterns)
        && route_matches_patterns(route, runtime.crawl_include_patterns)
}

#[cfg(test)]
mod tests {
    use super::{crawl_links, route_is_allowed};
    use crate::config::Config;

    fn routes(html: &str, page_url: &str) -> Vec<String> {
        crawl_links(html, page_url)
            .into_iter()
            .map(|link| link.route)
            .collect()
    }

    #[test]
    fn extracts_relative_and_absolute_same_site_links() {
        let html = r#"
            <a href="/about">About</a>
            <a href="https://www.example.com/contact">Contact</a>
            <a href="https://example.com/legal/">Legal</a>
            <a href="https://docs.example.com/guide">Guide</a>
        "#;
        assert_eq!(
            routes(html, "https://www.example.com/"),
            ["about", "contact", "legal"]
        );
    }

    #[test]
    fn resolves_links_the_way_a_browser_does() {
        let html = r##"
            <a href="sub/page">relative</a>
            <a href="../up">parent</a>
            <a href=/unquoted>unquoted</a>
            <a data-href="/wrong" href="/right">data attribute first</a>
            <A HREF="/upper">uppercase</A>
            <a href="#top">fragment</a>
            <a href="mailto:a@example.org">mail</a>
            <script>"<a href='/in-script'>x</a>"</script>
        "##;
        assert_eq!(
            routes(html, "https://example.org/docs/guide/"),
            [
                "docs/guide/sub/page",
                "docs/up",
                "unquoted",
                "right",
                "upper"
            ]
        );
    }

    #[test]
    fn honours_base_href() {
        let html = r#"<head><base href="/en/"></head><a href="intro">Intro</a>"#;
        assert_eq!(routes(html, "https://example.org/fr/page"), ["en/intro"]);
        let foreign = r#"<head><base href="https://cdn.example.net/"></head><a href="x">X</a>"#;
        assert!(routes(foreign, "https://example.org/").is_empty());
    }

    #[test]
    fn query_variants_collapse_onto_the_route_of_their_path() {
        let html = r#"<a href="?page=2">Next</a><a href="/list?page=3">Last</a>"#;
        assert_eq!(routes(html, "https://example.org/list"), ["list", "list"]);
    }

    #[test]
    fn rejects_cloudflare_infrastructure_routes() {
        let config = Config::default();
        let runtime = config.runtime();
        assert!(!route_is_allowed("cdn-cgi/l/email-protection", &runtime));
        assert!(route_is_allowed("features/terminal-search", &runtime));
    }
}

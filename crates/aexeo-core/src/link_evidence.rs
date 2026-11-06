//! Extract attributable links from rendered HTML without guessing unknown contexts.
use html5ever::{parse_document, tendril::TendrilSink};
use markup5ever_rcdom::{Handle, NodeData, RcDom};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::link_graph::{LinkResolution, resolve_link, same_site_host};
use crate::site::route_from_urlish;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkContext {
    Body,
    RelatedContent,
    Navigation,
    Footer,
    Breadcrumb,
    Pagination,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkEvidence {
    pub source: String,
    pub href: String,
    pub anchor: String,
    pub locale: Option<String>,
    pub snapshot_date: Option<String>,
    pub context: LinkContext,
    pub component: Option<String>,
    pub paragraph: Option<String>,
    /// DOM parsing does not reliably preserve original byte positions.
    pub line: Option<usize>,
    pub column: Option<usize>,
    pub resolution: LinkResolution,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HtmlLinkMetadata {
    pub base_href: Option<String>,
    pub robots_directives: Vec<String>,
    pub canonical_hrefs: Vec<String>,
}

#[derive(Clone)]
struct Scope {
    context: LinkContext,
    component: Option<String>,
    paragraph: Option<String>,
    locale: Option<String>,
    explicit: bool,
}

fn attribute(handle: &Handle, key: &str) -> Option<String> {
    if let NodeData::Element { attrs, .. } = &handle.data {
        attrs
            .borrow()
            .iter()
            .find(|attr| attr.name.local.as_ref() == key)
            .map(|attr| attr.value.to_string())
    } else {
        None
    }
}

fn text_content(root: &Handle) -> String {
    let mut result = String::new();
    let mut stack = vec![root.clone()];
    while let Some(node) = stack.pop() {
        if matches!(&node.data, NodeData::Element { name, .. } if matches!(name.local.as_ref(), "script" | "style"))
        {
            continue;
        }
        if let NodeData::Text { contents } = &node.data {
            result.push_str(&contents.borrow());
        }
        stack.extend(node.children.borrow().iter().rev().cloned());
    }
    result.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn explicit_context(value: &str) -> LinkContext {
    match value.trim().to_ascii_lowercase().as_str() {
        "body" => LinkContext::Body,
        "related" | "related_content" => LinkContext::RelatedContent,
        "navigation" => LinkContext::Navigation,
        "footer" => LinkContext::Footer,
        "breadcrumb" => LinkContext::Breadcrumb,
        "pagination" => LinkContext::Pagination,
        _ => LinkContext::Unknown,
    }
}

/// Extract evidence in DOM order. Contexts can be explicitly marked using
/// `data-aexeo-link-context`; otherwise only unambiguous semantic elements are
/// classified. Unmarked related-content components remain unknown.
pub fn extract_links(
    html: &str,
    source_url: &str,
    site_origin: &str,
    snapshot_date: Option<&str>,
) -> Vec<LinkEvidence> {
    extract_document(html, source_url, site_origin, snapshot_date).0
}

/// Extract link evidence and document-level metadata in the same DOM parse.
pub fn extract_document(
    html: &str,
    source_url: &str,
    site_origin: &str,
    snapshot_date: Option<&str>,
) -> (Vec<LinkEvidence>, HtmlLinkMetadata) {
    let dom = parse_document(RcDom::default(), Default::default()).one(html);
    let mut base = None;
    let mut metadata = HtmlLinkMetadata::default();
    let mut nodes = vec![dom.document.clone()];
    while let Some(node) = nodes.pop() {
        if base.is_none()
            && matches!(&node.data, NodeData::Element { name, .. } if name.local.as_ref() == "base")
        {
            base = attribute(&node, "href");
        }
        let robots_content = if matches!(&node.data, NodeData::Element { name, .. } if name.local.as_ref() == "meta")
            && attribute(&node, "name").is_some_and(|name| name.eq_ignore_ascii_case("robots"))
        {
            attribute(&node, "content")
        } else {
            None
        };
        if let Some(content) = robots_content {
            metadata.robots_directives.push(content);
        }
        let canonical = if matches!(&node.data, NodeData::Element { name, .. } if name.local.as_ref() == "link")
            && attribute(&node, "rel").is_some_and(|rel| {
                rel.split_ascii_whitespace()
                    .any(|part| part.eq_ignore_ascii_case("canonical"))
            }) {
            attribute(&node, "href")
        } else {
            None
        };
        if let Some(href) = canonical {
            metadata.canonical_hrefs.push(href);
        }
        nodes.extend(node.children.borrow().iter().rev().cloned());
    }
    let initial = Scope {
        context: LinkContext::Unknown,
        component: None,
        paragraph: None,
        locale: None,
        explicit: false,
    };
    // Keep the document alive while traversing: RcDom drains descendants when
    // the last handle to a node is dropped, even if descendants were cloned.
    let mut stack = vec![(dom.document.clone(), initial)];
    let mut links = Vec::new();
    while let Some((node, mut scope)) = stack.pop() {
        if let NodeData::Element { name, .. } = &node.data {
            let tag = name.local.as_ref();
            if let Some(lang) = attribute(&node, "lang").filter(|lang| !lang.trim().is_empty()) {
                scope.locale = Some(lang);
            }
            let explicit = attribute(&node, "data-aexeo-link-context");
            let semantic = match tag {
                "article" | "main" if !scope.explicit && scope.context == LinkContext::Unknown => {
                    Some(LinkContext::Body)
                }
                "nav" if !scope.explicit => Some(
                    match attribute(&node, "aria-label")
                        .as_deref()
                        .map(str::trim)
                        .map(str::to_ascii_lowercase)
                        .as_deref()
                    {
                        Some("breadcrumb" | "breadcrumbs") => LinkContext::Breadcrumb,
                        Some("pagination") => LinkContext::Pagination,
                        _ => LinkContext::Navigation,
                    },
                ),
                "footer" if !scope.explicit => Some(LinkContext::Footer),
                _ => None,
            };
            if explicit.is_some() {
                scope.explicit = true;
            }
            if let Some(context) = explicit.as_deref().map(explicit_context).or(semantic) {
                scope.context = context;
                scope.component = Some(match attribute(&node, "id") {
                    Some(id) if !id.is_empty() => format!("{tag}#{id}"),
                    _ => tag.to_string(),
                });
            }
            if tag == "p" {
                scope.paragraph = Some(text_content(&node));
            }
            let href = if tag == "a" {
                attribute(&node, "href")
            } else {
                None
            };
            if let Some(href) = href {
                let resolution = resolve_link(&href, source_url, site_origin, base.as_deref());
                links.push(LinkEvidence {
                    source: source_url.to_string(),
                    href,
                    anchor: text_content(&node),
                    locale: scope.locale.clone(),
                    snapshot_date: snapshot_date.map(str::to_string),
                    context: scope.context,
                    component: scope.component.clone(),
                    paragraph: scope.paragraph.clone(),
                    line: None,
                    column: None,
                    resolution,
                });
            }
        }
        stack.extend(
            node.children
                .borrow()
                .iter()
                .rev()
                .cloned()
                .map(|child| (child, scope.clone())),
        );
    }
    metadata.base_href = base;
    (links, metadata)
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
    let Ok(page) = Url::parse(page_url) else {
        return Vec::new();
    };
    let site_host = page.host_str().unwrap_or_default();
    extract_links(html, page_url, &page.origin().ascii_serialization(), None)
        .into_iter()
        .filter_map(|link| {
            let url = match &link.resolution {
                LinkResolution::Internal { url } => url,
                LinkResolution::External { url }
                    if Url::parse(url).ok().is_some_and(|target| {
                        same_site_host(site_host, target.host_str().unwrap_or_default())
                    }) =>
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

#[cfg(test)]
mod tests {
    use super::*;

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
    fn attributes_nested_components_and_excludes_script_markup() {
        let html = r##"<html lang="fr"><head><base href="/guide/"><script>"<a href='/bad'>bad</a>"</script></head><body><nav><a href="/menu">menu</a></nav><main><p>Read <a href="next">next</a>.</p><div data-aexeo-link-context="related"><a href="#part">related</a></div></main><footer><a href="https://example.org/legal">legal</a></footer></body></html>"##;
        let links = extract_links(
            html,
            "https://example.org/guide/",
            "https://example.org",
            Some("2026-11-05"),
        );
        assert_eq!(links.len(), 4);
        assert_eq!(
            links.iter().map(|link| link.context).collect::<Vec<_>>(),
            vec![
                LinkContext::Navigation,
                LinkContext::Body,
                LinkContext::RelatedContent,
                LinkContext::Footer
            ]
        );
        assert_eq!(links[1].paragraph.as_deref(), Some("Read next."));
        assert_eq!(links[1].locale.as_deref(), Some("fr"));
        assert_eq!(
            links[1].resolution,
            LinkResolution::Internal {
                url: "https://example.org/guide/next".into()
            }
        );
        assert!(matches!(
            links[2].resolution,
            LinkResolution::SameDocument { .. }
        ));
    }

    #[test]
    fn unknown_explicit_context_is_not_guessed() {
        let links = extract_links(
            "<main><div data-aexeo-link-context='custom'><nav><a href='/x'>x</a></nav></div></main>",
            "https://example.org/",
            "https://example.org",
            None,
        );
        assert_eq!(links[0].context, LinkContext::Unknown);
        assert_eq!(links[0].line, None);
    }

    #[test]
    fn semantic_navigation_labels_are_distinguished() {
        let links = extract_links(
            "<nav aria-label='Breadcrumb'><a href='/a'>A</a></nav><nav aria-label='Pagination'><a href='/b'>B</a></nav>",
            "https://example.org/",
            "https://example.org",
            None,
        );
        assert_eq!(links[0].context, LinkContext::Breadcrumb);
        assert_eq!(links[1].context, LinkContext::Pagination);
    }
}

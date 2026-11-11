//! Extract attributable links from rendered HTML without guessing unknown contexts.
use crate::site::markup::tag_breaks_text;
use html5ever::{ParseOpts, parse_document, tendril::TendrilSink, tree_builder::TreeBuilderOpts};
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
    /// Whether the link appeared inside template or noscript source markup.
    /// Absent in older reports, where its status cannot be established.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conditional_markup: Option<bool>,
    /// Whether this link or an ancestor has an HTML `hidden` attribute.
    /// Absent in older reports; CSS and scripts can alter actual visibility.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hidden_attribute: Option<bool>,
    /// Whether the link is outside the first summary of a closed details element.
    /// Absent in older reports; scripts may change the element state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collapsed_details: Option<bool>,
    /// `nofollow`, `ugc` and `sponsored` tokens of the anchor's `rel`, lowercase
    /// and sorted. Other tokens, such as `noopener`, do not qualify a link.
    #[serde(default)]
    pub rel: Vec<String>,
    /// DOM parsing does not reliably preserve original byte positions.
    pub line: Option<usize>,
    pub column: Option<usize>,
    pub resolution: LinkResolution,
}

impl LinkEvidence {
    /// True only when all observed conditional states are absent.
    pub fn is_nonconditional(&self) -> bool {
        self.conditional_markup == Some(false)
            && self.hidden_attribute == Some(false)
            && self.collapsed_details == Some(false)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HtmlLinkMetadata {
    /// Language declared on the document root, not a verified CMS locale.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document_language: Option<String>,
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
    /// Inside an `aside` or `header`, which `article` and `main` cannot promote to body.
    non_body_landmark: bool,
    /// Source-only markup may not be displayed in the evaluated page.
    conditional_markup: bool,
    hidden_attribute: bool,
    collapsed_details: bool,
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

pub(crate) fn text_content(root: &Handle) -> String {
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
        if matches!(&node.data, NodeData::Element { name, .. } if tag_breaks_text(&name.local)) {
            result.push(' ');
        }
        stack.extend(node.children.borrow().iter().rev().cloned());
    }
    result.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Report a literal label; this does not implement the browser's
/// accessible-name algorithm or infer rendered visibility.
fn anchor_label(node: &Handle) -> String {
    let text = text_content(node);
    if !text.is_empty() {
        return text;
    }
    if let Some(label) = attribute(node, "aria-label") {
        let label = label.split_whitespace().collect::<Vec<_>>().join(" ");
        if !label.is_empty() {
            return label;
        }
    }
    let mut alternatives = Vec::new();
    let mut stack = node
        .children
        .borrow()
        .iter()
        .rev()
        .cloned()
        .collect::<Vec<_>>();
    while let Some(child) = stack.pop() {
        if is_element(&child, "img")
            && let Some(alt) = attribute(&child, "alt")
        {
            let alt = alt.split_whitespace().collect::<Vec<_>>().join(" ");
            if !alt.is_empty() {
                alternatives.push(alt);
            }
        }
        stack.extend(child.children.borrow().iter().rev().cloned());
    }
    alternatives.join(" ")
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

fn crawl_rel_tokens(rel: Option<String>) -> Vec<String> {
    let tokens = rel
        .iter()
        .flat_map(|value| value.split_ascii_whitespace())
        .map(str::to_ascii_lowercase)
        .filter(|token| matches!(token.as_str(), "nofollow" | "ugc" | "sponsored"))
        .collect::<std::collections::BTreeSet<_>>();
    tokens.into_iter().collect()
}

/// Extract evidence in DOM order. Contexts can be explicitly marked using
/// `data-aexeo-link-context`; otherwise only unambiguous semantic elements are
/// classified. An `aside` or `header` inside body content is unknown rather
/// than body. Generic containers inherit their surrounding context, so a related
/// block built from an unmarked `div` inside `main` needs an explicit marker.
pub fn extract_links(
    html: &str,
    source_url: &str,
    site_origin: &str,
    snapshot_date: Option<&str>,
) -> Vec<LinkEvidence> {
    extract_document(html, source_url, site_origin, snapshot_date).0
}

/// Children of `node` in document order, followed by the contents of a
/// `<template>`, which the DOM keeps apart from its children.
fn children_with_template_contents(node: &Handle) -> Vec<Handle> {
    let mut children = node.children.borrow().clone();
    if let NodeData::Element {
        template_contents, ..
    } = &node.data
        && let Some(contents) = template_contents.borrow().as_ref()
    {
        children.push(contents.clone());
    }
    children
}

fn is_element(node: &Handle, tag: &str) -> bool {
    matches!(&node.data, NodeData::Element { name, .. } if name.local.as_ref() == tag)
}

/// Extract link evidence and document-level metadata in the same DOM parse.
///
/// Links inside `<template>` and `<noscript>` count, as they do for the audit
/// and the crawl: the document is parsed with scripting off, so `<noscript>`
/// content is markup rather than text, and template contents are walked.
/// Their context is unknown, even under explicit markers, because the HTML alone
/// does not establish that a browser displays them. Document metadata ignores
/// both, since a scripting browser applies neither.
pub fn extract_document(
    html: &str,
    source_url: &str,
    site_origin: &str,
    snapshot_date: Option<&str>,
) -> (Vec<LinkEvidence>, HtmlLinkMetadata) {
    let opts = ParseOpts {
        tree_builder: TreeBuilderOpts {
            scripting_enabled: false,
            ..Default::default()
        },
        ..Default::default()
    };
    let dom = parse_document(RcDom::default(), opts).one(html);
    let mut base = None;
    let mut metadata = HtmlLinkMetadata::default();
    let mut nodes = vec![dom.document.clone()];
    while let Some(node) = nodes.pop() {
        if is_element(&node, "html") {
            metadata.document_language =
                attribute(&node, "lang").filter(|lang| !lang.trim().is_empty());
        }
        if base.is_none()
            && matches!(&node.data, NodeData::Element { name, .. } if name.local.as_ref() == "base")
        {
            base = attribute(&node, "href");
        }
        let robots_content = if matches!(&node.data, NodeData::Element { name, .. } if name.local.as_ref() == "meta")
            && attribute(&node, "name").is_some_and(|name| {
                name.eq_ignore_ascii_case("robots") || name.eq_ignore_ascii_case("googlebot")
            }) {
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
        if !is_element(&node, "noscript") {
            nodes.extend(node.children.borrow().iter().rev().cloned());
        }
    }
    let initial = Scope {
        context: LinkContext::Unknown,
        component: None,
        paragraph: None,
        locale: None,
        explicit: false,
        non_body_landmark: false,
        conditional_markup: false,
        hidden_attribute: false,
        collapsed_details: false,
    };
    // Keep the document alive while traversing: RcDom drains descendants when
    // the last handle to a node is dropped, even if descendants were cloned.
    let mut stack = vec![(dom.document.clone(), initial)];
    let mut links = Vec::new();
    while let Some((node, mut scope)) = stack.pop() {
        if let NodeData::Element { name, .. } = &node.data {
            let tag = name.local.as_ref();
            if attribute(&node, "hidden").is_some() {
                scope.hidden_attribute = true;
            }
            if matches!(tag, "template" | "noscript") && !scope.conditional_markup {
                scope.conditional_markup = true;
                scope.context = LinkContext::Unknown;
                scope.component = Some(tag.to_string());
                scope.paragraph = None;
            }
            if let Some(lang) = attribute(&node, "lang") {
                // An explicitly empty language means unknown, not inherited.
                scope.locale = (!lang.trim().is_empty()).then_some(lang);
            }
            let explicit = (!scope.conditional_markup)
                .then(|| attribute(&node, "data-aexeo-link-context"))
                .flatten();
            let semantic = match tag {
                _ if scope.conditional_markup => None,
                "article" | "main"
                    if !scope.explicit
                        && !scope.non_body_landmark
                        && scope.context == LinkContext::Unknown =>
                {
                    Some(LinkContext::Body)
                }
                "aside" | "header" if !scope.explicit && scope.context == LinkContext::Body => {
                    Some(LinkContext::Unknown)
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
            if matches!(tag, "aside" | "header") {
                scope.non_body_landmark = true;
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
                let anchor = anchor_label(&node);
                links.push(LinkEvidence {
                    source: source_url.to_string(),
                    href,
                    anchor,
                    locale: scope.locale.clone(),
                    snapshot_date: snapshot_date.map(str::to_string),
                    context: scope.context,
                    component: scope.component.clone(),
                    paragraph: scope.paragraph.clone(),
                    conditional_markup: Some(scope.conditional_markup),
                    hidden_attribute: Some(scope.hidden_attribute),
                    collapsed_details: Some(scope.collapsed_details),
                    rel: crawl_rel_tokens(attribute(&node, "rel")),
                    line: None,
                    column: None,
                    resolution,
                });
            }
        }
        let children = children_with_template_contents(&node);
        // Only the first direct summary of a closed details element is shown.
        // An ancestor may already be collapsed, including around this summary.
        let closed_details = is_element(&node, "details") && attribute(&node, "open").is_none();
        let first_summary = closed_details
            .then(|| {
                children
                    .iter()
                    .position(|child| is_element(child, "summary"))
            })
            .flatten();
        stack.extend(
            children
                .into_iter()
                .enumerate()
                .rev()
                .map(|(index, child)| {
                    let mut child_scope = scope.clone();
                    if closed_details && Some(index) != first_summary {
                        child_scope.collapsed_details = true;
                    }
                    (child, child_scope)
                }),
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
    fn labels_text_image_and_aria_links_without_changing_report_shape() {
        let html = r#"<main><p>Before <a href='/text' aria-label='Other'>Visible <img alt='Icon'></a>
          <a href='/aria' aria-label='  Read   more  '><img alt='Ignored'></a>
          <a href='/image'><img alt='First'><span><img alt=' Deux '></span></a>
          <a href='/empty'><img alt=''></a></p></main>"#;
        let links = extract_links(html, "https://example.org/a", "https://example.org", None);
        assert_eq!(
            links
                .iter()
                .map(|link| link.anchor.as_str())
                .collect::<Vec<_>>(),
            vec!["Visible", "Read more", "First Deux", ""]
        );
        assert!(links.iter().all(|link| link.context == LinkContext::Body));
        assert!(
            serde_json::to_value(&links[0])
                .unwrap()
                .get("anchor_source")
                .is_none()
        );
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
    fn closed_details_mark_only_content_outside_the_first_summary() {
        let html = "<main><details><summary><a href='/summary'>summary</a></summary><p><a href='/closed'>closed</a></p><summary><a href='/second'>second</a></summary><details open><summary><a href='/nested'>nested</a></summary></details></details><details open><summary><a href='/open-summary'>summary</a></summary><a href='/open'>open</a></details></main>";
        let links = extract_links(html, "https://example.org/", "https://example.org", None);
        let states: Vec<_> = links
            .iter()
            .map(|link| {
                (
                    link.href.as_str(),
                    link.collapsed_details,
                    link.is_nonconditional(),
                )
            })
            .collect();
        assert_eq!(
            states,
            vec![
                ("/summary", Some(false), true),
                ("/closed", Some(true), false),
                ("/second", Some(true), false),
                ("/nested", Some(true), false),
                ("/open-summary", Some(false), true),
                ("/open", Some(false), true)
            ]
        );
        let mut old = serde_json::to_value(&links[0]).unwrap();
        old.as_object_mut().unwrap().remove("collapsed_details");
        let old: LinkEvidence = serde_json::from_value(old).unwrap();
        assert_eq!(old.collapsed_details, None);
        assert!(!old.is_nonconditional());
    }

    #[test]
    fn hidden_attribute_is_inherited_without_claiming_css_visibility() {
        let links = extract_links(
            "<main><div hidden><a href='/one'>one</a></div><a hidden='until-found' href='/two'>two</a><a href='/three'>three</a></main>",
            "https://example.org/",
            "https://example.org",
            None,
        );
        assert_eq!(
            links
                .iter()
                .map(|link| link.hidden_attribute)
                .collect::<Vec<_>>(),
            vec![Some(true), Some(true), Some(false)]
        );
        assert_eq!(
            links
                .iter()
                .map(LinkEvidence::is_nonconditional)
                .collect::<Vec<_>>(),
            vec![false, false, true]
        );
        let mut older = serde_json::to_value(&links[2]).unwrap();
        older.as_object_mut().unwrap().remove("hidden_attribute");
        let older: LinkEvidence = serde_json::from_value(older).unwrap();
        assert_eq!(older.hidden_attribute, None);
        assert!(!older.is_nonconditional());
    }

    #[test]
    fn conditional_markup_links_remain_discoverable_but_have_unknown_context() {
        let html = r#"<html><head><noscript><meta name="robots" content="noindex"></noscript></head><body><main><noscript><p><a href="/plain">plain</a></p></noscript></main><footer><template><a href="/legal-notice">Legal</a></template></footer></body></html>"#;
        let (links, metadata) =
            extract_document(html, "https://example.org/", "https://example.org", None);
        assert_eq!(
            links
                .iter()
                .map(|link| (link.href.as_str(), link.context, link.anchor.as_str()))
                .collect::<Vec<_>>(),
            [
                ("/plain", LinkContext::Unknown, "plain"),
                ("/legal-notice", LinkContext::Unknown, "Legal")
            ]
        );
        assert_eq!(links[0].component.as_deref(), Some("noscript"));
        assert_eq!(links[1].component.as_deref(), Some("template"));
        assert_eq!(
            routes(html, "https://example.org/"),
            ["plain", "legal-notice"]
        );
        let nested = extract_links(
            "<main><template><div data-aexeo-link-context='body'><nav><a href='/nested'>x</a></nav></div></template><p><a href='/real'>y</a></p></main>",
            "https://example.org/",
            "https://example.org",
            None,
        );
        assert_eq!(nested[0].context, LinkContext::Unknown);
        assert_eq!(nested[0].component.as_deref(), Some("template"));
        assert_eq!(nested[1].context, LinkContext::Body);
        assert!(
            metadata.robots_directives.is_empty(),
            "a scripting browser ignores <noscript> metadata"
        );
    }

    #[test]
    fn query_variants_collapse_onto_the_route_of_their_path() {
        let html = r#"<a href="?page=2">Next</a><a href="/list?page=3">Last</a>"#;
        assert_eq!(routes(html, "https://example.org/list"), ["list", "list"]);
    }

    #[test]
    fn empty_language_resets_inheritance_until_a_nested_override() {
        let links = extract_links(
            r#"<html lang="fr"><main><a href="/fr">fr</a><div lang=""><a href="/unknown">unknown</a><span lang="en"><a href="/en">en</a></span><a lang=" " href="/empty">empty</a></div><a href="/fr-again">fr</a></main></html>"#,
            "https://example.org/",
            "https://example.org",
            None,
        );
        assert_eq!(
            links
                .iter()
                .map(|link| link.locale.as_deref())
                .collect::<Vec<_>>(),
            vec![Some("fr"), None, Some("en"), None, Some("fr")]
        );
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

    fn contexts(html: &str) -> Vec<LinkContext> {
        extract_links(html, "https://example.org/", "https://example.org", None)
            .iter()
            .map(|link| link.context)
            .collect()
    }

    #[test]
    fn non_body_landmarks_inside_main_do_not_inherit_body() {
        assert_eq!(
            contexts(
                "<main><aside><a href='/a'>a</a></aside><header><a href='/b'>b</a></header><nav><a href='/c'>c</a></nav><footer><a href='/d'>d</a></footer><p><a href='/e'>e</a></p></main>"
            ),
            vec![
                LinkContext::Unknown,
                LinkContext::Unknown,
                LinkContext::Navigation,
                LinkContext::Footer,
                LinkContext::Body
            ]
        );
    }

    #[test]
    fn articles_nested_in_non_body_landmarks_are_not_promoted_to_body() {
        assert_eq!(
            contexts(
                "<main><aside><article><a href='/a'>a</a></article></aside></main><aside><article><a href='/b'>b</a></article></aside><header><main><a href='/c'>c</a></main></header>"
            ),
            vec![
                LinkContext::Unknown,
                LinkContext::Unknown,
                LinkContext::Unknown
            ]
        );
    }

    #[test]
    fn landmarks_refine_or_keep_the_surrounding_non_body_context() {
        assert_eq!(
            contexts(
                "<main><aside><nav><a href='/a'>a</a></nav></aside></main><footer><aside><a href='/b'>b</a></aside></footer><nav><header><a href='/c'>c</a></header></nav>"
            ),
            vec![
                LinkContext::Navigation,
                LinkContext::Footer,
                LinkContext::Navigation
            ]
        );
    }

    #[test]
    fn explicit_markers_override_non_body_landmarks() {
        assert_eq!(
            contexts(
                "<main><aside data-aexeo-link-context='related'><a href='/a'>a</a></aside><aside><div data-aexeo-link-context='body'><a href='/b'>b</a></div></aside></main>"
            ),
            vec![LinkContext::RelatedContent, LinkContext::Body]
        );
    }

    #[test]
    fn unmarked_generic_containers_inside_main_remain_body() {
        // Class names are not semantics: an unmarked related block needs
        // `data-aexeo-link-context` to be attributed as related content.
        assert_eq!(
            contexts("<main><div class='related-posts'><a href='/a'>a</a></div></main>"),
            vec![LinkContext::Body]
        );
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

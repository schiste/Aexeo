//! Extract attributable links from rendered HTML without guessing unknown contexts.
use html5ever::{parse_document, tendril::TendrilSink};
use markup5ever_rcdom::{Handle, NodeData, RcDom};
use serde::{Deserialize, Serialize};

use crate::link_graph::{LinkResolution, resolve_link};

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
    /// `nofollow`, `ugc` and `sponsored` tokens of the anchor's `rel`, lowercase
    /// and sorted. Other tokens, such as `noopener`, do not qualify a link.
    #[serde(default)]
    pub rel: Vec<String>,
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
    /// Inside an `aside` or `header`, which `article` and `main` cannot promote to body.
    non_body_landmark: bool,
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
        nodes.extend(node.children.borrow().iter().rev().cloned());
    }
    let initial = Scope {
        context: LinkContext::Unknown,
        component: None,
        paragraph: None,
        locale: None,
        explicit: false,
        non_body_landmark: false,
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
                links.push(LinkEvidence {
                    source: source_url.to_string(),
                    href,
                    anchor: text_content(&node),
                    locale: scope.locale.clone(),
                    snapshot_date: snapshot_date.map(str::to_string),
                    context: scope.context,
                    component: scope.component.clone(),
                    paragraph: scope.paragraph.clone(),
                    rel: crawl_rel_tokens(attribute(&node, "rel")),
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

#[cfg(test)]
mod tests {
    use super::*;

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

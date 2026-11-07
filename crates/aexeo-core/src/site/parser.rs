use anyhow::Result;
use html5ever::driver::ParseOpts;
use html5ever::tendril::TendrilSink;
use html5ever::tree_builder::TreeBuilderOpts;
use html5ever::{namespace_url, ns, parse_document};
use markup5ever_rcdom::{NodeData, RcDom};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::link_evidence::text_content;

use super::markup::{
    LineIndex, attr_value, mask_non_markup, next_close_tag, next_open_tag, open_tag_end,
};

use super::{
    AlternateLink, Block, DetailsBlock, ImageReference, JsonLdBlock, Link, Page, PreBlock,
    capture_route_for_relative_path, classify_page_kind, normalize_internal_href, strip_tags,
};

pub(super) fn parse_page_from_file(path: &Path, root: &Path) -> Result<Page> {
    let relative = path
        .strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/");
    // A single page in a legacy encoding must not abort the whole site load.
    let raw = String::from_utf8_lossy(&fs::read(path)?).into_owned();
    Ok(build_page_from_source(
        path.to_path_buf(),
        relative,
        raw,
        BTreeMap::new(),
    ))
}

pub(super) fn build_page_from_source(
    path: PathBuf,
    relative_path: String,
    raw: String,
    response_headers: BTreeMap<String, String>,
) -> Page {
    // Everything but the JSON-LD and DOM text is read from `markup`, so text in a
    // script, style or comment is never mistaken for an element.
    let markup = mask_non_markup(&raw);
    let lines = LineIndex::new(&raw);
    let (meta_by_name, meta_by_property) = capture_meta_maps(&markup);
    let links = capture_links(&markup, &lines);
    let internal_links = links
        .iter()
        .filter_map(|link| link.target.clone())
        .collect();
    let (title, h1_texts) = capture_title_and_h1s(&raw);
    let route = capture_route_for_relative_path(&relative_path);
    let canonical = capture_canonical_href(&markup);
    let html_lang = capture_html_lang(&markup);
    let has_breadcrumb_nav = capture_breadcrumb_nav(&markup);
    let alternate_links = capture_alternate_links(&markup);
    let images = capture_images(&markup, &lines);
    let blocks = capture_semantic_blocks(&markup, &lines);
    let details_blocks = capture_details_blocks(&markup, &lines);
    let pre_blocks = capture_pre_blocks(&markup, &lines);
    let json_ld_blocks = capture_json_ld_blocks(&raw, &markup, &lines);
    Page {
        path,
        relative_path: relative_path.clone(),
        route: route.clone(),
        page_kind: classify_page_kind(&relative_path, &route),
        raw_text: raw,
        title,
        meta_by_name,
        meta_by_property,
        canonical,
        html_lang,
        h1_count: h1_texts.len(),
        h1_texts,
        has_breadcrumb_nav,
        response_headers,
        crawl_response: None,
        links,
        internal_links,
        alternate_links,
        images,
        blocks,
        details_blocks,
        pre_blocks,
        json_ld_blocks,
    }
}

/// First non-empty `<title>` and every non-empty `<h1>` of the parsed document,
/// so tag case, comments, scripts and `<template>` contents are handled the way
/// a browser handles them. Scripting is off to match a non-rendering crawler:
/// `<noscript>` content counts as markup. SVG `<title>` elements do not count.
fn capture_title_and_h1s(raw: &str) -> (Option<String>, Vec<String>) {
    let opts = ParseOpts {
        tree_builder: TreeBuilderOpts {
            scripting_enabled: false,
            ..Default::default()
        },
        ..Default::default()
    };
    let dom = parse_document(RcDom::default(), opts).one(raw);
    let mut title = None;
    let mut h1_texts = Vec::new();
    let mut nodes = vec![dom.document.clone()];
    while let Some(node) = nodes.pop() {
        if let NodeData::Element { name, .. } = &node.data
            && name.ns == ns!(html)
        {
            match name.local.as_ref() {
                "title" if title.is_none() => {
                    title = Some(text_content(&node)).filter(|text| !text.is_empty());
                    continue;
                }
                "h1" => {
                    let text = text_content(&node);
                    if !text.is_empty() {
                        h1_texts.push(text);
                    }
                    continue;
                }
                _ => {}
            }
        }
        nodes.extend(node.children.borrow().iter().rev().cloned());
    }
    (title, h1_texts)
}

fn capture_meta_maps(raw: &str) -> (BTreeMap<String, String>, BTreeMap<String, String>) {
    let mut by_name = BTreeMap::new();
    let mut by_property = BTreeMap::new();
    for snippet in tag_snippets(raw, "meta") {
        let content = attr_value(snippet, "content").unwrap_or_default();
        if content.is_empty() {
            continue;
        }
        if let Some(name) = attr_value(snippet, "name") {
            by_name.insert(name.to_ascii_lowercase(), content.clone());
        }
        if let Some(property) = attr_value(snippet, "property") {
            by_property.insert(property.to_ascii_lowercase(), content.clone());
        }
    }
    (by_name, by_property)
}

fn capture_canonical_href(raw: &str) -> Option<String> {
    for snippet in tag_snippets(raw, "link") {
        let rel = attr_value(snippet, "rel")
            .unwrap_or_default()
            .to_ascii_lowercase();
        if rel.split_whitespace().any(|part| part == "canonical") {
            return attr_value(snippet, "href");
        }
    }
    None
}

fn capture_alternate_links(raw: &str) -> Vec<AlternateLink> {
    let mut links = Vec::new();
    for snippet in tag_snippets(raw, "link") {
        let rel = attr_value(snippet, "rel")
            .unwrap_or_default()
            .to_ascii_lowercase();
        if !rel.split_whitespace().any(|part| part == "alternate") {
            continue;
        }
        let Some(href) = attr_value(snippet, "href") else {
            continue;
        };
        let hreflang = attr_value(snippet, "hreflang").filter(|value| !value.is_empty());
        let type_attr = attr_value(snippet, "type").filter(|value| !value.is_empty());
        links.push(AlternateLink {
            href,
            hreflang,
            type_attr,
        });
    }
    links
}

fn capture_links(raw: &str, lines: &LineIndex) -> Vec<Link> {
    let mut links = Vec::new();
    let mut offset = 0;
    while let Some(start) = next_open_tag(raw, "a", offset) {
        let Some(open_end) = open_tag_end(raw, start) else {
            break;
        };
        let snippet = &raw[start..=open_end];
        let Some(href) = attr_value(snippet, "href") else {
            offset = open_end + 1;
            continue;
        };
        let Some((close_start, close_end)) = next_close_tag(raw, "a", open_end + 1) else {
            offset = open_end + 1;
            continue;
        };
        let text = strip_tags(&raw[open_end + 1..close_start]);
        let (line, column) = lines.line_column(start);
        links.push(Link {
            href: href.clone(),
            target: normalize_internal_href(&href),
            text,
            line,
            column,
        });
        offset = close_end;
    }
    links
}

fn capture_html_lang(raw: &str) -> Option<String> {
    let start = next_open_tag(raw, "html", 0)?;
    let end = open_tag_end(raw, start)?;
    attr_value(&raw[start..=end], "lang").filter(|value| !value.is_empty())
}

fn capture_breadcrumb_nav(raw: &str) -> bool {
    raw.to_ascii_lowercase().contains("breadcrumb")
}

fn capture_images(raw: &str, lines: &LineIndex) -> Vec<ImageReference> {
    let mut images = Vec::new();
    let mut offset = 0;
    while let Some(start) = next_open_tag(raw, "img", offset) {
        let Some(end) = open_tag_end(raw, start) else {
            break;
        };
        let snippet = &raw[start..=end];
        offset = end + 1;
        let Some(src) = attr_value(snippet, "src") else {
            continue;
        };
        let alt = attr_value(snippet, "alt");
        let role = attr_value(snippet, "role");
        let aria_hidden = attr_value(snippet, "aria-hidden")
            .map(|value| value.trim().eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        let (line, column) = lines.line_column(start);
        images.push(ImageReference {
            src,
            alt,
            line,
            column,
            role,
            aria_hidden,
        });
    }
    images
}

fn capture_semantic_blocks(raw: &str, lines: &LineIndex) -> Vec<Block> {
    let mut blocks = Vec::new();
    blocks.extend(capture_named_blocks(raw, "section", lines));
    blocks.extend(capture_named_blocks(raw, "article", lines));
    blocks.sort_by_key(|block| (block.line, block.column));
    blocks
}

fn capture_named_blocks(raw: &str, tag: &str, lines: &LineIndex) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut offset = 0;
    while let Some(start) = next_open_tag(raw, tag, offset) {
        let Some(open_end) = open_tag_end(raw, start) else {
            break;
        };
        let snippet = &raw[start..=open_end];
        let Some((close_start, close_end)) = next_close_tag(raw, tag, open_end + 1) else {
            offset = open_end + 1;
            continue;
        };
        let inner = &raw[open_end + 1..close_start];
        let (line, column) = lines.line_column(start);
        blocks.push(Block {
            tag: tag.to_string(),
            data_ui: attr_value(snippet, "data-ui"),
            line,
            column,
            has_heading: has_heading_tag(inner),
            text: strip_tags(inner),
        });
        offset = close_end;
    }
    blocks
}

fn has_heading_tag(raw: &str) -> bool {
    ["h1", "h2", "h3", "h4", "h5", "h6"]
        .into_iter()
        .any(|tag| next_open_tag(raw, tag, 0).is_some())
}

/// Start, inner range and end of each `<tag>...</tag>` in the document.
fn element_spans(raw: &str, tag: &str) -> Vec<(usize, usize, usize, usize)> {
    let mut spans = Vec::new();
    let mut offset = 0;
    while let Some(start) = next_open_tag(raw, tag, offset) {
        let Some(open_end) = open_tag_end(raw, start) else {
            break;
        };
        let Some((close_start, close_end)) = next_close_tag(raw, tag, open_end + 1) else {
            offset = open_end + 1;
            continue;
        };
        spans.push((start, open_end, close_start, close_end));
        offset = close_end;
    }
    spans
}

fn capture_details_blocks(raw: &str, lines: &LineIndex) -> Vec<DetailsBlock> {
    element_spans(raw, "details")
        .into_iter()
        .map(|(start, open_end, close_start, _)| {
            let (line, column) = lines.line_column(start);
            DetailsBlock {
                line,
                column,
                has_summary: next_open_tag(&raw[open_end + 1..close_start], "summary", 0).is_some(),
            }
        })
        .collect()
}

fn capture_pre_blocks(raw: &str, lines: &LineIndex) -> Vec<PreBlock> {
    element_spans(raw, "pre")
        .into_iter()
        .map(|(start, open_end, close_start, _)| {
            let (line, column) = lines.line_column(start);
            PreBlock {
                line,
                column,
                has_code: next_open_tag(&raw[open_end + 1..close_start], "code", 0).is_some(),
            }
        })
        .collect()
}

/// Finds scripts in `markup`, so one inside a comment is skipped, and reads their
/// contents from `raw`, where they were not masked.
fn capture_json_ld_blocks(raw: &str, markup: &str, lines: &LineIndex) -> Vec<JsonLdBlock> {
    element_spans(markup, "script")
        .into_iter()
        .filter(|(start, open_end, _, _)| {
            attr_value(&markup[*start..=*open_end], "type")
                .is_some_and(|value| value.eq_ignore_ascii_case("application/ld+json"))
        })
        .map(|(start, open_end, close_start, _)| {
            let (line, column) = lines.line_column(start);
            JsonLdBlock {
                raw: raw[open_end + 1..close_start].trim().to_string(),
                line,
                column,
            }
        })
        .collect()
}

fn tag_snippets<'a>(raw: &'a str, tag: &str) -> Vec<&'a str> {
    let mut snippets = Vec::new();
    let mut offset = 0;
    while let Some(start) = next_open_tag(raw, tag, offset) {
        let Some(end) = open_tag_end(raw, start) else {
            break;
        };
        snippets.push(&raw[start..=end]);
        offset = end + 1;
    }
    snippets
}

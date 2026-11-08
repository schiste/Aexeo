use anyhow::Result;
use html5ever::driver::ParseOpts;
use html5ever::tendril::TendrilSink;
use html5ever::tree_builder::TreeBuilderOpts;
use html5ever::{ns, parse_document};
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
    capture_route_for_relative_path, classify_page_kind, decode_url_path, normalize_internal_href,
    strip_tags,
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

/// One `<picture>`, `<figure>`, `<source>` or `<img>` tag boundary, in document
/// order.
enum ImageMarkup {
    Open(&'static str, usize, usize),
    Close(&'static str),
}

fn image_markup_events(raw: &str) -> Vec<(usize, ImageMarkup)> {
    let mut events = Vec::new();
    for tag in ["picture", "figure", "source", "img"] {
        let mut offset = 0;
        while let Some(start) = next_open_tag(raw, tag, offset) {
            let Some(end) = open_tag_end(raw, start) else {
                break;
            };
            events.push((start, ImageMarkup::Open(tag, start, end)));
            offset = end + 1;
        }
    }
    for tag in ["picture", "figure"] {
        let mut offset = 0;
        while let Some((start, end)) = next_close_tag(raw, tag, offset) {
            events.push((start, ImageMarkup::Close(tag)));
            offset = end;
        }
    }
    events.sort_by_key(|(offset, _)| *offset);
    // A match inside an earlier tag, such as `<figure title="</figure>">`, is
    // attribute text, not markup.
    let mut tag_end = None;
    events.retain(|(offset, event)| {
        if tag_end.is_some_and(|end| *offset <= end) {
            return false;
        }
        if let ImageMarkup::Open(_, _, end) = event {
            tag_end = Some(*end);
        }
        true
    });
    events
}

/// Content range of each `<figure>`, keyed by its opening offset. Nested figures
/// pair with their own closing tags.
fn figure_content_ranges(events: &[(usize, ImageMarkup)]) -> BTreeMap<usize, (usize, usize)> {
    let mut ranges = BTreeMap::new();
    let mut open = Vec::new();
    for (offset, event) in events {
        match event {
            ImageMarkup::Open("figure", start, end) => open.push((*start, *end + 1)),
            ImageMarkup::Close("figure") => {
                if let Some((start, content_start)) = open.pop() {
                    ranges.insert(start, (content_start, *offset));
                }
            }
            _ => {}
        }
    }
    ranges
}

fn is_hidden_element(snippet: &str) -> bool {
    attr_value(snippet, "hidden").is_some() || has_hidden_inline_style(snippet)
}

fn has_hidden_inline_style(snippet: &str) -> bool {
    let Some(style) = attr_value(snippet, "style") else {
        return false;
    };
    style.split(';').any(|declaration| {
        let Some((property, value)) = declaration.split_once(':') else {
            return false;
        };
        let value = value
            .split_once('!')
            .map_or(value, |(value, _)| value)
            .trim();
        match property.trim().to_ascii_lowercase().as_str() {
            "display" => value.eq_ignore_ascii_case("none"),
            "visibility" => {
                value.eq_ignore_ascii_case("hidden") || value.eq_ignore_ascii_case("collapse")
            }
            "content-visibility" => value.eq_ignore_ascii_case("hidden"),
            _ => false,
        }
    })
}

/// Text of a fragment without the elements marked hidden. A hidden element with
/// no closing tag hides only itself.
fn visible_text(fragment: &str) -> String {
    let mut visible = String::new();
    let mut offset = 0;
    while let Some(relative) = fragment[offset..].find('<') {
        let start = offset + relative;
        visible.push_str(&fragment[offset..start]);
        let name_len = fragment[start + 1..]
            .bytes()
            .take_while(u8::is_ascii_alphanumeric)
            .count();
        let Some(end) = (name_len > 0)
            .then(|| open_tag_end(fragment, start))
            .flatten()
        else {
            visible.push('<');
            offset = start + 1;
            continue;
        };
        offset = end + 1;
        if is_hidden_element(&fragment[start..=end]) {
            let name = &fragment[start + 1..start + 1 + name_len];
            if let Some((_, close_end)) = next_close_tag(fragment, name, end + 1) {
                offset = close_end;
            }
            continue;
        }
        visible.push_str(&fragment[start..=end]);
    }
    visible.push_str(&fragment[offset..]);
    strip_tags(&visible)
}

/// First non-empty, visible `<figcaption>` text between `start` and `end`.
fn figure_caption(raw: &str, start: usize, end: usize) -> Option<String> {
    let content = raw.get(start..end)?;
    let mut offset = 0;
    while let Some(open) = next_open_tag(content, "figcaption", offset) {
        let open_end = open_tag_end(content, open)?;
        let (close_start, close_end) = next_close_tag(content, "figcaption", open_end + 1)?;
        offset = close_end;
        if is_hidden_element(&content[open..=open_end]) {
            continue;
        }
        let caption = visible_text(content.get(open_end + 1..close_start)?);
        if !caption.trim().is_empty() {
            return Some(caption);
        }
    }
    None
}

fn parse_srcset_urls(srcset: &str) -> Vec<String> {
    let mut candidates = Vec::new();
    let mut remaining = srcset.trim();

    while !remaining.is_empty() {
        remaining = remaining.trim_start_matches(|character: char| {
            character.is_ascii_whitespace() || character == ','
        });
        if remaining.is_empty() {
            break;
        }

        let url_end = remaining
            .find(|character: char| character.is_ascii_whitespace())
            .unwrap_or(remaining.len());
        let token = &remaining[..url_end];
        let url = token.trim_end_matches(',');
        if !url.is_empty() {
            candidates.push(url.to_string());
        }
        let has_trailing_separator = url.len() < token.len();
        remaining = &remaining[url_end..];
        if has_trailing_separator {
            continue;
        }

        let mut parentheses_depth = 0usize;
        let mut next_candidate = None;
        for (index, character) in remaining.char_indices() {
            match character {
                '(' => parentheses_depth += 1,
                ')' => parentheses_depth = parentheses_depth.saturating_sub(1),
                ',' if parentheses_depth == 0 => {
                    next_candidate = Some(index);
                    break;
                }
                _ => {}
            }
        }
        let Some(next_candidate) = next_candidate else {
            break;
        };
        remaining = &remaining[next_candidate + 1..];
    }

    candidates
}

fn is_modern_image_candidate(candidate: &str) -> bool {
    let path = candidate.split(['?', '#']).next().unwrap_or_default();
    let Some(path) = decode_url_path(path) else {
        return false;
    };
    let filename = path.rsplit('/').next().unwrap_or_default();
    filename.rsplit_once('.').is_some_and(|(_, extension)| {
        matches!(extension.to_ascii_lowercase().as_str(), "avif" | "webp")
    })
}

/// The candidates of a `<source>` that are WebP or AVIF, by declared `type` when
/// there is one and by file extension otherwise.
fn modern_source_candidates(snippet: &str, candidates: &[String]) -> Vec<String> {
    let declared_type = attr_value(snippet, "type")
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty());
    match declared_type {
        Some(content_type) => {
            let media_type = content_type.split(';').next().unwrap_or_default().trim();
            if matches!(media_type, "image/avif" | "image/webp") {
                candidates.to_vec()
            } else {
                Vec::new()
            }
        }
        None => candidates
            .iter()
            .filter(|candidate| is_modern_image_candidate(candidate))
            .cloned()
            .collect(),
    }
}

#[derive(Default)]
struct PictureSources {
    asset_sources: Vec<String>,
    modern_sources: Vec<String>,
}

fn capture_images(raw: &str, lines: &LineIndex) -> Vec<ImageReference> {
    let events = image_markup_events(raw);
    let figure_ranges = figure_content_ranges(&events);
    let mut images = Vec::new();
    let mut pictures: Vec<PictureSources> = Vec::new();
    let mut figures: Vec<usize> = Vec::new();
    for (_, event) in &events {
        let (tag, start, end) = match event {
            ImageMarkup::Close("picture") => {
                pictures.pop();
                continue;
            }
            ImageMarkup::Close(_) => {
                figures.pop();
                continue;
            }
            ImageMarkup::Open(tag, start, end) => (*tag, *start, *end),
        };
        let snippet = &raw[start..=end];
        match tag {
            "picture" => pictures.push(PictureSources::default()),
            "figure" => figures.push(start),
            "source" => {
                if let Some(picture) = pictures.last_mut() {
                    let candidates = attr_value(snippet, "srcset")
                        .map(|srcset| parse_srcset_urls(&srcset))
                        .unwrap_or_default();
                    picture
                        .modern_sources
                        .extend(modern_source_candidates(snippet, &candidates));
                    picture.asset_sources.extend(candidates);
                }
            }
            _ => {
                let picture = pictures.last();
                let mut asset_sources = attr_value(snippet, "srcset")
                    .map(|srcset| parse_srcset_urls(&srcset))
                    .unwrap_or_default();
                let src = attr_value(snippet, "src")
                    .or_else(|| asset_sources.first().cloned())
                    .or_else(|| picture.and_then(|picture| picture.asset_sources.first().cloned()));
                let Some(src) = src else {
                    continue;
                };
                let modern_sources = picture
                    .map(|picture| picture.modern_sources.clone())
                    .unwrap_or_default();
                if let Some(picture) = picture {
                    asset_sources.extend(picture.asset_sources.iter().cloned());
                }
                let figure = figures.last().map(|figure_start| {
                    let figure_open = &raw[*figure_start..];
                    let hidden = open_tag_end(figure_open, 0)
                        .is_some_and(|open_end| is_hidden_element(&figure_open[..=open_end]));
                    figure_ranges
                        .get(figure_start)
                        .filter(|_| !hidden)
                        .and_then(|(content_start, content_end)| {
                            figure_caption(raw, *content_start, *content_end)
                        })
                });
                let (line, column) = lines.line_column(start);
                images.push(ImageReference {
                    src,
                    has_modern_source: !modern_sources.is_empty(),
                    asset_sources,
                    modern_sources,
                    alt: attr_value(snippet, "alt"),
                    line,
                    column,
                    role: attr_value(snippet, "role"),
                    aria_hidden: attr_value(snippet, "aria-hidden")
                        .is_some_and(|value| value.trim().eq_ignore_ascii_case("true")),
                    width: attr_value(snippet, "width"),
                    height: attr_value(snippet, "height"),
                    inside_figure: figure.is_some(),
                    figure_caption: figure.flatten(),
                });
            }
        }
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

#[cfg(test)]
mod tests {
    use super::{figure_content_ranges, image_markup_events};

    #[test]
    fn a_close_tag_inside_an_attribute_gives_no_inverted_figure_range() {
        let raw = r#"<figure title="</figure>"><img src="/a.jpg"></figure><figure"#;
        let ranges = figure_content_ranges(&image_markup_events(raw));
        assert!(
            ranges.values().all(|(start, end)| start <= end),
            "{ranges:?}"
        );
        assert_eq!(ranges.len(), 1);
    }
}

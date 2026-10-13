use anyhow::Result;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use super::{
    AlternateLink, Block, DetailsBlock, ImageReference, JsonLdBlock, Link, Page, PreBlock,
    capture_route_for_relative_path, classify_page_kind, line_column_for, normalize_internal_href,
    strip_tags,
};

pub(super) fn parse_page_from_file(path: &Path, root: &Path) -> Result<Page> {
    let relative = path
        .strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/");
    let raw = fs::read_to_string(path)?;
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
    let (meta_by_name, meta_by_property) = capture_meta_maps(&raw);
    let links = capture_links(&raw);
    let internal_links = links
        .iter()
        .filter_map(|link| link.target.clone())
        .collect();
    let h1_texts = capture_paired_tag_texts(&raw, "h1");
    let route = capture_route_for_relative_path(&relative_path);
    let title = capture_tag_text(&raw, "title");
    let canonical = capture_canonical_href(&raw);
    let html_lang = capture_html_lang(&raw);
    let has_breadcrumb_nav = capture_breadcrumb_nav(&raw);
    let alternate_links = capture_alternate_links(&raw);
    let images = capture_images(&raw);
    let blocks = capture_semantic_blocks(&raw);
    let details_blocks = capture_details_blocks(&raw);
    let pre_blocks = capture_pre_blocks(&raw);
    let json_ld_blocks = capture_json_ld_blocks(&raw);
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

fn capture_tag_text(raw: &str, tag: &str) -> Option<String> {
    capture_paired_tag_texts(raw, tag).into_iter().next()
}

fn capture_paired_tag_texts(raw: &str, tag: &str) -> Vec<String> {
    let mut texts = Vec::new();
    let start_marker = format!("<{}", tag);
    let end_marker = format!("</{}>", tag);
    let mut offset = 0;
    while let Some(index) = raw[offset..].find(&start_marker) {
        let start = offset + index;
        let Some(open_end_rel) = raw[start..].find('>') else {
            break;
        };
        let open_end = start + open_end_rel;
        let Some(close_start_rel) = raw[open_end + 1..].find(&end_marker) else {
            break;
        };
        let close_start = open_end + 1 + close_start_rel;
        let text = strip_tags(&raw[open_end + 1..close_start]);
        if !text.is_empty() {
            texts.push(text);
        }
        offset = close_start + end_marker.len();
    }
    texts
}

fn capture_meta_maps(raw: &str) -> (BTreeMap<String, String>, BTreeMap<String, String>) {
    let mut by_name = BTreeMap::new();
    let mut by_property = BTreeMap::new();
    for snippet in tag_snippets(raw, "meta") {
        let content = attr_value(&snippet, "content").unwrap_or_default();
        if content.is_empty() {
            continue;
        }
        if let Some(name) = attr_value(&snippet, "name") {
            by_name.insert(name.to_ascii_lowercase(), content.clone());
        }
        if let Some(property) = attr_value(&snippet, "property") {
            by_property.insert(property.to_ascii_lowercase(), content.clone());
        }
    }
    (by_name, by_property)
}

fn capture_canonical_href(raw: &str) -> Option<String> {
    for snippet in tag_snippets(raw, "link") {
        let rel = attr_value(&snippet, "rel")
            .unwrap_or_default()
            .to_ascii_lowercase();
        if rel.split_whitespace().any(|part| part == "canonical") {
            return attr_value(&snippet, "href");
        }
    }
    None
}

fn capture_alternate_links(raw: &str) -> Vec<AlternateLink> {
    let mut links = Vec::new();
    for snippet in tag_snippets(raw, "link") {
        let rel = attr_value(&snippet, "rel")
            .unwrap_or_default()
            .to_ascii_lowercase();
        if !rel.split_whitespace().any(|part| part == "alternate") {
            continue;
        }
        let Some(href) = attr_value(&snippet, "href") else {
            continue;
        };
        let hreflang = attr_value(&snippet, "hreflang").filter(|value| !value.is_empty());
        let type_attr = attr_value(&snippet, "type").filter(|value| !value.is_empty());
        links.push(AlternateLink {
            href,
            hreflang,
            type_attr,
        });
    }
    links
}

fn capture_links(raw: &str) -> Vec<Link> {
    let mut links = Vec::new();
    let mut offset = 0;
    while let Some(index) = raw[offset..].find("<a") {
        let start = offset + index;
        let Some(open_end_rel) = raw[start..].find('>') else {
            break;
        };
        let open_end = start + open_end_rel;
        let snippet = &raw[start..=open_end];
        let Some(href) = attr_value(snippet, "href") else {
            offset = open_end + 1;
            continue;
        };
        let Some(close_start_rel) = raw[open_end + 1..].find("</a>") else {
            offset = open_end + 1;
            continue;
        };
        let close_start = open_end + 1 + close_start_rel;
        let text = strip_tags(&raw[open_end + 1..close_start]);
        let (line, column) = line_column_for(raw, start);
        links.push(Link {
            href: href.clone(),
            target: normalize_internal_href(&href),
            text,
            line,
            column,
        });
        offset = close_start + 4;
    }
    links
}

fn capture_html_lang(raw: &str) -> Option<String> {
    let start = raw.find("<html")?;
    let end = raw[start..].find('>')? + start;
    attr_value(&raw[start..=end], "lang").filter(|value| !value.is_empty())
}

fn capture_breadcrumb_nav(raw: &str) -> bool {
    raw.to_ascii_lowercase().contains("breadcrumb")
}

#[derive(Debug, Clone, Copy)]
struct HtmlTag {
    start: usize,
    end: usize,
    name_start: usize,
    name_end: usize,
    closing: bool,
}

impl HtmlTag {
    fn name<'a>(&self, raw_lowercase: &'a str) -> &'a str {
        &raw_lowercase[self.name_start..self.name_end]
    }
}

fn find_raw_text_closing_tag(raw_lowercase: &str, from: usize, tag_name: &str) -> Option<usize> {
    let needle = format!("</{tag_name}");
    let mut offset = from;
    while let Some(relative) = raw_lowercase[offset..].find(&needle) {
        let start = offset + relative;
        let boundary = raw_lowercase
            .as_bytes()
            .get(start + needle.len())
            .copied()?;
        if boundary.is_ascii_whitespace() || matches!(boundary, b'/' | b'>') {
            return Some(start);
        }
        offset = start + needle.len();
    }
    None
}

fn scan_html_tags(raw_lowercase: &str) -> Vec<HtmlTag> {
    let mut tags = Vec::new();
    let mut offset = 0;
    let mut template_depth = 0usize;
    while let Some(relative) = raw_lowercase[offset..].find('<') {
        let start = offset + relative;
        if raw_lowercase[start..].starts_with("<!--") {
            let Some(close) = raw_lowercase[start + 4..].find("-->") else {
                break;
            };
            offset = start + 4 + close + 3;
            continue;
        }
        if raw_lowercase[start..].starts_with("<![cdata[") {
            let Some(close) = raw_lowercase[start + 9..].find("]]>") else {
                break;
            };
            offset = start + 9 + close + 3;
            continue;
        }
        if raw_lowercase
            .as_bytes()
            .get(start + 1)
            .is_some_and(|byte| matches!(byte, b'!' | b'?'))
        {
            let Some(end) = html_tag_end(raw_lowercase, start) else {
                break;
            };
            offset = end + 1;
            continue;
        }
        let candidate_name_start = if raw_lowercase.as_bytes().get(start + 1) == Some(&b'/') {
            start + 2
        } else {
            start + 1
        };
        if !raw_lowercase
            .as_bytes()
            .get(candidate_name_start)
            .is_some_and(u8::is_ascii_alphabetic)
        {
            offset = start + 1;
            continue;
        }
        let Some(end) = html_tag_end(raw_lowercase, start) else {
            break;
        };
        let mut name_start = start + 1;
        let closing = raw_lowercase.as_bytes().get(name_start) == Some(&b'/');
        if closing {
            name_start += 1;
        }
        let mut name_end = name_start;
        while raw_lowercase
            .as_bytes()
            .get(name_end)
            .is_some_and(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b':'))
        {
            name_end += 1;
        }
        if name_end == name_start {
            offset = end + 1;
            continue;
        }
        let tag_name = &raw_lowercase[name_start..name_end];
        if closing {
            if tag_name == "template" {
                template_depth = template_depth.saturating_sub(1);
            } else if template_depth == 0 {
                tags.push(HtmlTag {
                    start,
                    end,
                    name_start,
                    name_end,
                    closing: true,
                });
            }
            offset = end + 1;
            continue;
        }
        if tag_name == "template" {
            template_depth += 1;
            offset = end + 1;
            continue;
        }
        if matches!(
            tag_name,
            "script"
                | "style"
                | "textarea"
                | "title"
                | "xmp"
                | "iframe"
                | "noembed"
                | "noframes"
                | "noscript"
        ) {
            let Some(close_start) = find_raw_text_closing_tag(raw_lowercase, end + 1, tag_name)
            else {
                break;
            };
            let Some(close_end) = html_tag_end(raw_lowercase, close_start) else {
                break;
            };
            offset = close_end + 1;
            continue;
        }
        if tag_name == "plaintext" {
            break;
        }
        if template_depth > 0 {
            offset = end + 1;
            continue;
        }
        tags.push(HtmlTag {
            start,
            end,
            name_start,
            name_end,
            closing: false,
        });
        offset = end + 1;
    }
    tags
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

fn is_modern_picture_source(raw: &str, tag: &HtmlTag) -> bool {
    let snippet = &raw[tag.start..=tag.end];
    let has_srcset = attr_value(snippet, "srcset").is_some_and(|value| !value.trim().is_empty());
    let modern_type = attr_value(snippet, "type").is_some_and(|value| {
        matches!(
            value
                .split(';')
                .next()
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase()
                .as_str(),
            "image/avif" | "image/webp"
        )
    });
    has_srcset && modern_type
}

fn is_void_html_element(name: &str) -> bool {
    matches!(
        name,
        "area"
            | "base"
            | "br"
            | "col"
            | "embed"
            | "hr"
            | "img"
            | "input"
            | "link"
            | "meta"
            | "param"
            | "source"
            | "track"
            | "wbr"
    )
}

fn visible_html_text(raw: &str) -> String {
    let raw_lowercase = raw.to_ascii_lowercase();
    let mut result = String::new();
    let mut open_elements: Vec<(String, bool)> = Vec::new();
    let mut hidden_depth = 0usize;
    let mut template_depth = 0usize;
    let mut offset = 0usize;
    while offset < raw.len() {
        let Some(character) = raw[offset..].chars().next() else {
            break;
        };
        if character != '<' {
            if hidden_depth == 0 && template_depth == 0 {
                result.push(character);
            }
            offset += character.len_utf8();
            continue;
        }
        if raw_lowercase[offset..].starts_with("<!--") {
            let Some(close) = raw_lowercase[offset + 4..].find("-->") else {
                break;
            };
            offset += 4 + close + 3;
            continue;
        }
        if raw_lowercase[offset..].starts_with("<![cdata[") {
            let Some(close) = raw_lowercase[offset + 9..].find("]]>") else {
                break;
            };
            offset += 9 + close + 3;
            continue;
        }
        if raw_lowercase
            .as_bytes()
            .get(offset + 1)
            .is_some_and(|byte| matches!(byte, b'!' | b'?'))
        {
            let Some(end) = html_tag_end(&raw_lowercase, offset) else {
                break;
            };
            offset = end + 1;
            continue;
        }
        let candidate_name_start = if raw_lowercase.as_bytes().get(offset + 1) == Some(&b'/') {
            offset + 2
        } else {
            offset + 1
        };
        if !raw_lowercase
            .as_bytes()
            .get(candidate_name_start)
            .is_some_and(u8::is_ascii_alphabetic)
        {
            result.push('<');
            offset += 1;
            continue;
        }
        let Some(end) = html_tag_end(&raw_lowercase, offset) else {
            break;
        };
        let closing = raw_lowercase.as_bytes().get(offset + 1) == Some(&b'/');
        let name_start = candidate_name_start;
        let mut name_end = name_start;
        while raw_lowercase
            .as_bytes()
            .get(name_end)
            .is_some_and(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b':'))
        {
            name_end += 1;
        }
        let tag_name = &raw_lowercase[name_start..name_end];
        if closing {
            let matching_open_element = if template_depth == 0 {
                open_elements
                    .iter()
                    .rposition(|(open_name, _)| open_name == tag_name)
            } else {
                None
            };
            if tag_name == "template" {
                template_depth = template_depth.saturating_sub(1);
            } else if let Some(open_index) = matching_open_element {
                let removed_hidden = open_elements[open_index..]
                    .iter()
                    .filter(|(_, hidden)| *hidden)
                    .count();
                open_elements.truncate(open_index);
                hidden_depth = hidden_depth.saturating_sub(removed_hidden);
            }
            offset = end + 1;
            continue;
        }
        if tag_name == "template" {
            template_depth += 1;
            offset = end + 1;
            continue;
        }
        if matches!(
            tag_name,
            "script"
                | "style"
                | "textarea"
                | "title"
                | "xmp"
                | "iframe"
                | "noembed"
                | "noframes"
                | "noscript"
        ) {
            let Some(close_start) = find_raw_text_closing_tag(&raw_lowercase, end + 1, tag_name)
            else {
                break;
            };
            let Some(close_end) = html_tag_end(&raw_lowercase, close_start) else {
                break;
            };
            offset = close_end + 1;
            continue;
        }
        if tag_name == "plaintext" {
            break;
        }
        if template_depth == 0 && !is_void_html_element(tag_name) {
            let snippet = &raw[offset..=end];
            let hidden = has_html_attribute(snippet, "hidden") || has_hidden_inline_style(snippet);
            if hidden {
                hidden_depth += 1;
            }
            open_elements.push((tag_name.to_string(), hidden));
        }
        offset = end + 1;
    }
    super::collapse_whitespace(&result)
}

fn figure_caption_for_image(
    raw: &str,
    raw_lowercase: &str,
    tags: &[HtmlTag],
    figure_start: Option<usize>,
) -> (bool, Option<String>) {
    let Some(figure_start) = figure_start else {
        return (false, None);
    };
    let figure_tag = &tags[figure_start];
    let figure_snippet = &raw[figure_tag.start..=figure_tag.end];
    if has_html_attribute(figure_snippet, "hidden") || has_hidden_inline_style(figure_snippet) {
        return (true, None);
    }

    let mut nested_figures = 0usize;
    let mut figure_end = tags.len();
    for (index, tag) in tags.iter().enumerate().skip(figure_start + 1) {
        if tag.name(raw_lowercase) != "figure" {
            continue;
        }
        if tag.closing {
            if nested_figures == 0 {
                figure_end = index;
                break;
            }
            nested_figures -= 1;
        } else {
            nested_figures += 1;
        }
    }

    for (caption_index, tag) in tags
        .iter()
        .enumerate()
        .take(figure_end)
        .skip(figure_start + 1)
    {
        if tag.closing || tag.name(raw_lowercase) != "figcaption" {
            continue;
        }
        let caption_snippet = &raw[tag.start..=tag.end];
        if has_html_attribute(caption_snippet, "hidden") || has_hidden_inline_style(caption_snippet)
        {
            continue;
        }
        let mut nested_captions = 0usize;
        let mut closing_tag = None;
        for (close_index, candidate) in tags
            .iter()
            .enumerate()
            .skip(caption_index + 1)
            .take(figure_end.saturating_sub(caption_index + 1))
        {
            if candidate.name(raw_lowercase) != "figcaption" {
                continue;
            }
            if candidate.closing {
                if nested_captions == 0 {
                    closing_tag = Some((close_index, candidate));
                    break;
                }
                nested_captions -= 1;
            } else {
                nested_captions += 1;
            }
        }
        let Some((_, closing_tag)) = closing_tag else {
            continue;
        };
        let text_start = tag.end + 1;
        let caption = visible_html_text(&raw[text_start..closing_tag.start]);
        if !caption.trim().is_empty() {
            return (true, Some(caption));
        }
    }
    (true, None)
}

#[derive(Default)]
struct PictureSources {
    asset_sources: Vec<String>,
    modern_sources: Vec<String>,
}

fn capture_images(raw: &str) -> Vec<ImageReference> {
    let raw_lowercase = raw.to_ascii_lowercase();
    let tags = scan_html_tags(&raw_lowercase);
    let mut images = Vec::new();
    let mut picture_stack: Vec<PictureSources> = Vec::new();
    let mut figure_stack = Vec::new();
    for (tag_index, tag) in tags.iter().enumerate() {
        let tag_name = tag.name(&raw_lowercase);
        if tag.closing {
            if tag_name == "picture" {
                picture_stack.pop();
            } else if tag_name == "figure" {
                figure_stack.pop();
            }
            continue;
        }
        if tag_name == "picture" {
            picture_stack.push(PictureSources::default());
            continue;
        }
        if tag_name == "figure" {
            figure_stack.push(tag_index);
            continue;
        }
        if tag_name == "source" {
            if let Some(picture) = picture_stack.last_mut() {
                let snippet = &raw[tag.start..=tag.end];
                let candidates = attr_value(snippet, "srcset")
                    .map(|srcset| parse_srcset_urls(&srcset))
                    .unwrap_or_default();
                if is_modern_picture_source(raw, tag) {
                    picture.modern_sources.extend(candidates.iter().cloned());
                }
                picture.asset_sources.extend(candidates);
            }
            continue;
        }
        if tag_name != "img" {
            continue;
        }
        let snippet = &raw[tag.start..=tag.end];
        let mut asset_sources = attr_value(snippet, "srcset")
            .map(|srcset| parse_srcset_urls(&srcset))
            .unwrap_or_default();
        let src = attr_value(snippet, "src")
            .or_else(|| asset_sources.first().cloned())
            .or_else(|| {
                picture_stack
                    .last()
                    .and_then(|picture| picture.asset_sources.first().cloned())
            });
        let Some(src) = src else {
            continue;
        };
        let alt = attr_value(snippet, "alt");
        let role = attr_value(snippet, "role");
        let aria_hidden = attr_value(snippet, "aria-hidden")
            .map(|value| value.trim().eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        let width = attr_value(snippet, "width");
        let height = attr_value(snippet, "height");
        let modern_sources = picture_stack
            .last()
            .map(|picture| picture.modern_sources.clone())
            .unwrap_or_default();
        let has_modern_source = !modern_sources.is_empty();
        if let Some(picture) = picture_stack.last() {
            asset_sources.extend(picture.asset_sources.iter().cloned());
        }
        let (inside_figure, figure_caption) =
            figure_caption_for_image(raw, &raw_lowercase, &tags, figure_stack.last().copied());
        let (line, column) = line_column_for(raw, tag.start);
        images.push(ImageReference {
            src,
            asset_sources,
            modern_sources,
            alt,
            line,
            column,
            role,
            aria_hidden,
            width,
            height,
            has_modern_source,
            inside_figure,
            figure_caption,
        });
    }
    images
}

fn capture_semantic_blocks(raw: &str) -> Vec<Block> {
    let mut blocks = Vec::new();
    blocks.extend(capture_named_blocks(raw, "section"));
    blocks.extend(capture_named_blocks(raw, "article"));
    blocks.sort_by_key(|block| (block.line, block.column));
    blocks
}

fn capture_named_blocks(raw: &str, tag: &str) -> Vec<Block> {
    let mut blocks = Vec::new();
    let start_marker = format!("<{}", tag);
    let end_marker = format!("</{}>", tag);
    let mut offset = 0;
    while let Some(index) = raw[offset..].find(&start_marker) {
        let start = offset + index;
        let Some(open_end_rel) = raw[start..].find('>') else {
            break;
        };
        let open_end = start + open_end_rel;
        let snippet = &raw[start..=open_end];
        let Some(close_start_rel) = raw[open_end + 1..].find(&end_marker) else {
            offset = open_end + 1;
            continue;
        };
        let close_start = open_end + 1 + close_start_rel;
        let inner = &raw[open_end + 1..close_start];
        let (line, column) = line_column_for(raw, start);
        blocks.push(Block {
            tag: tag.to_string(),
            data_ui: attr_value(snippet, "data-ui"),
            line,
            column,
            has_heading: has_heading_tag(inner),
            text: strip_tags(inner),
        });
        offset = close_start + end_marker.len();
    }
    blocks
}

fn has_heading_tag(raw: &str) -> bool {
    (1..=6).any(|level| raw.contains(&format!("<h{}", level)))
}

fn capture_details_blocks(raw: &str) -> Vec<DetailsBlock> {
    let mut blocks = Vec::new();
    let mut offset = 0;
    while let Some(index) = raw[offset..].find("<details") {
        let start = offset + index;
        let Some(open_end_rel) = raw[start..].find('>') else {
            break;
        };
        let open_end = start + open_end_rel;
        let Some(close_start_rel) = raw[open_end + 1..].find("</details>") else {
            offset = open_end + 1;
            continue;
        };
        let close_start = open_end + 1 + close_start_rel;
        let inner = &raw[open_end + 1..close_start];
        let (line, column) = line_column_for(raw, start);
        blocks.push(DetailsBlock {
            line,
            column,
            has_summary: inner.contains("<summary"),
        });
        offset = close_start + 10;
    }
    blocks
}

fn capture_pre_blocks(raw: &str) -> Vec<PreBlock> {
    let mut blocks = Vec::new();
    let mut offset = 0;
    while let Some(index) = raw[offset..].find("<pre") {
        let start = offset + index;
        let Some(open_end_rel) = raw[start..].find('>') else {
            break;
        };
        let open_end = start + open_end_rel;
        let Some(close_start_rel) = raw[open_end + 1..].find("</pre>") else {
            offset = open_end + 1;
            continue;
        };
        let close_start = open_end + 1 + close_start_rel;
        let inner = &raw[open_end + 1..close_start];
        let (line, column) = line_column_for(raw, start);
        blocks.push(PreBlock {
            line,
            column,
            has_code: inner.contains("<code"),
        });
        offset = close_start + 6;
    }
    blocks
}

fn capture_json_ld_blocks(raw: &str) -> Vec<JsonLdBlock> {
    let mut blocks = Vec::new();
    let mut offset = 0;
    while let Some(index) = raw[offset..].find("<script") {
        let start = offset + index;
        let Some(open_end_rel) = raw[start..].find('>') else {
            break;
        };
        let open_end = start + open_end_rel;
        let snippet = &raw[start..=open_end];
        let script_type = attr_value(snippet, "type")
            .unwrap_or_default()
            .to_ascii_lowercase();
        let Some(close_start_rel) = raw[open_end + 1..].find("</script>") else {
            offset = open_end + 1;
            continue;
        };
        let close_start = open_end + 1 + close_start_rel;
        if script_type == "application/ld+json" {
            let (line, column) = line_column_for(raw, start);
            blocks.push(JsonLdBlock {
                raw: raw[open_end + 1..close_start].trim().to_string(),
                line,
                column,
            });
        }
        offset = close_start + 9;
    }
    blocks
}

fn tag_snippets(raw: &str, tag: &str) -> Vec<String> {
    let mut snippets = Vec::new();
    let needle = format!("<{}", tag);
    let mut offset = 0;
    while let Some(index) = raw[offset..].find(&needle) {
        let start = offset + index;
        let Some(end_rel) = raw[start..].find('>') else {
            break;
        };
        let end = start + end_rel;
        snippets.push(raw[start..=end].to_string());
        offset = end + 1;
    }
    snippets
}

fn attribute_value_or_presence(snippet: &str, attr: &str) -> Option<Option<String>> {
    let bytes = snippet.as_bytes();
    let mut cursor = snippet.find('<')? + 1;
    while cursor < bytes.len()
        && !bytes[cursor].is_ascii_whitespace()
        && !matches!(bytes[cursor], b'/' | b'>')
    {
        cursor += 1;
    }

    loop {
        while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        if cursor >= bytes.len() || bytes[cursor] == b'>' {
            return None;
        }
        if bytes[cursor] == b'/' {
            cursor += 1;
            continue;
        }

        let name_start = cursor;
        while cursor < bytes.len()
            && !bytes[cursor].is_ascii_whitespace()
            && !matches!(bytes[cursor], b'=' | b'/' | b'>')
        {
            cursor += 1;
        }
        if cursor == name_start {
            cursor += 1;
            continue;
        }
        let name = &snippet[name_start..cursor];
        while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }

        if cursor >= bytes.len() || bytes[cursor] != b'=' {
            if name.eq_ignore_ascii_case(attr) {
                return Some(None);
            }
            continue;
        }
        cursor += 1;
        while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }

        let (value_start, value_end) =
            if cursor < bytes.len() && matches!(bytes[cursor], b'"' | b'\'') {
                let quote = bytes[cursor];
                cursor += 1;
                let value_start = cursor;
                while cursor < bytes.len() && bytes[cursor] != quote {
                    cursor += 1;
                }
                if cursor >= bytes.len() {
                    return None;
                }
                let value_end = cursor;
                cursor += 1;
                (value_start, value_end)
            } else {
                let value_start = cursor;
                while cursor < bytes.len()
                    && !bytes[cursor].is_ascii_whitespace()
                    && bytes[cursor] != b'>'
                {
                    cursor += 1;
                }
                (value_start, cursor)
            };
        if name.eq_ignore_ascii_case(attr) {
            return Some(Some(snippet[value_start..value_end].trim().to_string()));
        }
    }
}

fn attr_value(snippet: &str, attr: &str) -> Option<String> {
    attribute_value_or_presence(snippet, attr).flatten()
}

fn has_html_attribute(snippet: &str, attr: &str) -> bool {
    attribute_value_or_presence(snippet, attr).is_some()
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

fn html_tag_end(raw: &str, start: usize) -> Option<usize> {
    let mut quote = None;
    for (offset, character) in raw.get(start..)?.char_indices() {
        if let Some(quote_character) = quote {
            if character == quote_character {
                quote = None;
            }
        } else if matches!(character, '\'' | '"') {
            quote = Some(character);
        } else if character == '>' {
            return Some(start + offset);
        }
    }
    None
}

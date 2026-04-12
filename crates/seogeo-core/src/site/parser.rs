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
        links.push(AlternateLink { href, hreflang });
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

fn capture_images(raw: &str) -> Vec<ImageReference> {
    let mut images = Vec::new();
    let mut offset = 0;
    while let Some(index) = raw[offset..].find("<img") {
        let start = offset + index;
        let Some(end_rel) = raw[start..].find('>') else {
            break;
        };
        let end = start + end_rel;
        let snippet = &raw[start..=end];
        let Some(src) = attr_value(snippet, "src") else {
            offset = end + 1;
            continue;
        };
        let alt = attr_value(snippet, "alt");
        let (line, column) = line_column_for(raw, start);
        images.push(ImageReference {
            src,
            alt,
            line,
            column,
        });
        offset = end + 1;
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

fn attr_value(snippet: &str, attr: &str) -> Option<String> {
    for quote in ['"', '\''] {
        let needle = format!("{}={}", attr, quote);
        if let Some(start) = snippet.find(&needle) {
            let value_start = start + needle.len();
            let end = snippet[value_start..].find(quote)? + value_start;
            return Some(snippet[value_start..end].trim().to_string());
        }
    }
    None
}

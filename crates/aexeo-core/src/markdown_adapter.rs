//! CommonMark adapter. Locations in reports refer to the extracted text.
use crate::content_gate::{ContentContext, ContentInput, ContentParagraph};
use anyhow::{Result, anyhow};
use markdown::{ParseOptions, mdast::Node};

/// A conservative mapping from extracted text back to exact Markdown text spans.
/// Transformed or synthetic text has no mapping.
#[derive(Debug, Default)]
pub struct MarkdownSourceMap {
    enabled: bool,
    spans: Vec<(usize, usize, usize)>,
    source_lines: Vec<usize>,
    projected_lines: Vec<usize>,
}

impl MarkdownSourceMap {
    /// Resolve one-based Unicode-character coordinates. Returns `None` when
    /// the projection does not correspond exactly to a source text span. Pass
    /// the same source and extracted text used to create this map, unchanged.
    pub fn source_position(
        &self,
        source: &str,
        projected: &str,
        line: usize,
        column: usize,
    ) -> Option<(usize, usize)> {
        let line_start = *self.projected_lines.get(line.checked_sub(1)?)?;
        let line_text = projected.get(line_start..)?.split('\n').next()?;
        let offset = line_start + line_text.char_indices().nth(column.checked_sub(1)?)?.0;
        let span_index = self
            .spans
            .partition_point(|&(start, _, _)| start <= offset)
            .checked_sub(1)?;
        let &(start, end, source_start) = self.spans.get(span_index)?;
        if offset >= end {
            return None;
        }
        let source_offset = source_start + offset - start;
        let source_line = self
            .source_lines
            .partition_point(|&start| start <= source_offset);
        let source_line_start = *self.source_lines.get(source_line.checked_sub(1)?)?;
        let source_column = source
            .get(source_line_start..source_offset)?
            .chars()
            .count()
            + 1;
        Some((source_line, source_column))
    }
}

/// Extract complete Markdown link destinations without fetching them. Returns
/// `None` when a reference cannot be resolved conservatively.
pub fn markdown_link_destinations(source: &str) -> Result<Option<Vec<String>>> {
    Ok(link_destinations_from_root(&parse_markdown(source)?))
}

fn parse_markdown(source: &str) -> Result<Node> {
    let mut options = ParseOptions::gfm();
    options.constructs.frontmatter = true;
    markdown::to_mdast(source, &options).map_err(|err| anyhow!("Markdown parse error: {err}"))
}

fn link_destinations_from_root(root: &Node) -> Option<Vec<String>> {
    fn normalize(value: &str) -> Option<String> {
        value.is_ascii().then(|| {
            value
                .split_ascii_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .to_ascii_lowercase()
        })
    }
    fn collect_definitions(node: &Node, defs: &mut std::collections::BTreeMap<String, String>) {
        if let Node::Definition(def) = node
            && let Some(key) = normalize(&def.identifier)
        {
            defs.entry(key).or_insert_with(|| def.url.clone());
        }
        if let Some(children) = node.children() {
            for child in children {
                collect_definitions(child, defs);
            }
        }
    }
    fn collect_links(
        node: &Node,
        defs: &std::collections::BTreeMap<String, String>,
        links: &mut Vec<String>,
    ) -> Option<()> {
        match node {
            Node::Link(link) => links.push(link.url.clone()),
            Node::LinkReference(link) => {
                links.push(defs.get(&normalize(&link.identifier)?)?.clone())
            }
            _ => {}
        }
        if let Some(children) = node.children() {
            for child in children {
                collect_links(child, defs, links)?;
            }
        }
        Some(())
    }
    let mut definitions = std::collections::BTreeMap::new();
    collect_definitions(root, &mut definitions);
    let mut links = Vec::new();
    collect_links(root, &definitions, &mut links).map(|_| links)
}

/// Extract prose and headings from Markdown, excluding code, HTML, and front matter.
pub fn markdown_input(id: impl Into<String>, source: &str) -> Result<ContentInput> {
    Ok(markdown_input_with_context(id, source)?.0)
}

/// Extract text and optional structural capabilities for content evaluation.
pub fn markdown_input_with_context(
    id: impl Into<String>,
    source: &str,
) -> Result<(ContentInput, ContentContext)> {
    let (input, context, _) = markdown_input_with_links(id, source)?;
    Ok((input, context))
}

/// Extract structural context and the number of hyperlinks without network access.
pub fn markdown_input_with_links(
    id: impl Into<String>,
    source: &str,
) -> Result<(ContentInput, ContentContext, usize)> {
    let (input, context, links, _) = extract_markdown(id, source, false)?;
    Ok((input, context, links))
}

/// Extract content and a conservative mapping for exact source text spans.
pub fn markdown_input_with_source_map(
    id: impl Into<String>,
    source: &str,
) -> Result<(ContentInput, ContentContext, usize, MarkdownSourceMap)> {
    extract_markdown(id, source, true)
}

fn extract_markdown(
    id: impl Into<String>,
    source: &str,
    map_source: bool,
) -> Result<(ContentInput, ContentContext, usize, MarkdownSourceMap)> {
    let root = parse_markdown(source)?;
    extract_markdown_from_root(id, source, map_source, &root)
}

/// The result of adapting Markdown and extracting link destinations together.
pub struct MarkdownContentWithDestinations {
    pub input: ContentInput,
    pub context: ContentContext,
    pub link_count: usize,
    pub source_map: MarkdownSourceMap,
    pub destinations: Option<Vec<String>>,
}

/// Extract source-mapped content and link destinations from one Markdown parse.
pub fn markdown_input_with_source_map_and_destinations(
    id: impl Into<String>,
    source: &str,
) -> Result<MarkdownContentWithDestinations> {
    let root = parse_markdown(source)?;
    let destinations = link_destinations_from_root(&root);
    let (input, context, count, map) = extract_markdown_from_root(id, source, true, &root)?;
    Ok(MarkdownContentWithDestinations {
        input,
        context,
        link_count: count,
        source_map: map,
        destinations,
    })
}

fn extract_markdown_from_root(
    id: impl Into<String>,
    source: &str,
    map_source: bool,
    root: &Node,
) -> Result<(ContentInput, ContentContext, usize, MarkdownSourceMap)> {
    let mut text = String::new();
    let mut headings = Vec::new();
    let mut ranges = Vec::new();
    let mut link_count = 0;
    let mut source_map = MarkdownSourceMap {
        enabled: map_source,
        ..Default::default()
    };
    fn visit(
        node: &Node,
        text: &mut String,
        headings: &mut Vec<String>,
        ranges: &mut Vec<(usize, usize)>,
        link_count: &mut usize,
        source: &str,
        source_map: &mut MarkdownSourceMap,
    ) {
        match node {
            Node::Text(item) => {
                let start = text.len();
                text.push_str(&item.value);
                if let Some(pos) = item.position.as_ref().filter(|_| source_map.enabled) {
                    let (a, b) = (pos.start.offset, pos.end.offset);
                    if source.get(a..b) == Some(item.value.as_str()) {
                        source_map.spans.push((start, text.len(), a));
                    }
                }
            }
            Node::Image(item) => text.push_str(&item.alt),
            Node::ImageReference(item) => text.push_str(&item.alt),
            Node::Break(_) => text.push('\n'),
            Node::Heading(_) => {
                let start = text.len();
                if let Some(children) = node.children() {
                    for child in children {
                        visit(
                            child, text, headings, ranges, link_count, source, source_map,
                        );
                    }
                }
                headings.push(text[start..].to_owned());
                text.push('\n');
            }
            Node::Paragraph(_) | Node::TableCell(_) => {
                let start = text.len();
                if let Some(children) = node.children() {
                    for child in children {
                        visit(
                            child, text, headings, ranges, link_count, source, source_map,
                        );
                    }
                }
                if matches!(node, Node::Paragraph(_)) {
                    ranges.push((start, text.len()));
                }
                text.push('\n');
            }
            _ => {
                if matches!(node, Node::Link(_) | Node::LinkReference(_)) {
                    *link_count += 1;
                }
                if let Some(children) = node.children() {
                    for child in children {
                        visit(
                            child, text, headings, ranges, link_count, source, source_map,
                        );
                    }
                }
            }
        }
    }
    visit(
        root,
        &mut text,
        &mut headings,
        &mut ranges,
        &mut link_count,
        source,
        &mut source_map,
    );
    if map_source {
        source_map.source_lines.push(0);
        source_map
            .source_lines
            .extend(source.match_indices('\n').map(|(i, _)| i + 1));
        source_map.projected_lines.push(0);
        source_map
            .projected_lines
            .extend(text.match_indices('\n').map(|(i, _)| i + 1));
    }
    let (mut cursor, mut line, mut column) = (0, 1, 1);
    let mut paragraphs = Vec::with_capacity(ranges.len());
    for (start, end) in ranges {
        for ch in text[cursor..start].chars() {
            if ch == '\n' {
                line += 1;
                column = 1;
            } else {
                column += 1;
            }
        }
        paragraphs.push(ContentParagraph {
            text: text[start..end].to_owned(),
            line,
            column,
        });
        cursor = start;
    }
    Ok((
        ContentInput {
            id: id.into(),
            text: Some(text),
            source_text: Some(source.into()),
            headings: Some(headings),
            metadata: None,
            evidence_sources: None,
        },
        ContentContext {
            paragraphs: Some(paragraphs),
        },
        link_count,
        source_map,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn combined_adapter_matches_independent_extraction() {
        for source in [
            "# Heading\n\n[direct](https://example.org) and [ref][ref].\n\n[ref]: /relative",
            "[réf][réf]\n\n[réf]: https://example.org",
        ] {
            let MarkdownContentWithDestinations {
                input,
                link_count: count,
                source_map: map,
                destinations,
                ..
            } = markdown_input_with_source_map_and_destinations("test.md", source).unwrap();
            let (baseline, _, baseline_count, baseline_map) =
                markdown_input_with_source_map("test.md", source).unwrap();
            assert_eq!(input.text, baseline.text);
            assert_eq!(input.headings, baseline.headings);
            assert_eq!(count, baseline_count);
            assert_eq!(destinations, markdown_link_destinations(source).unwrap());
            if let Some(text) = &input.text {
                assert_eq!(
                    map.source_position(source, text, 1, 1),
                    baseline_map.source_position(source, text, 1, 1)
                );
            }
        }
    }

    #[test]
    fn maps_only_exact_markdown_text_spans() {
        let source = "---\ntitle: value\n---\n# Héllo *world*\n\nA [link](https://example.org).\n\nEscaped \\*star\\*.\n";
        let (input, _, _, map) = markdown_input_with_source_map("test.md", source).unwrap();
        let text = input.text.unwrap();
        assert_eq!(map.source_position(source, &text, 1, 1), Some((4, 3)));
        assert_eq!(map.source_position(source, &text, 1, 7), Some((4, 10)));
        assert_eq!(map.source_position(source, &text, 2, 3), Some((6, 4)));
        assert_eq!(map.source_position(source, &text, 3, 9), None);
        assert_eq!(map.source_position(source, &text, 0, 1), None);
    }

    #[test]
    fn extracts_direct_and_reference_destinations_without_images() {
        let links = markdown_link_destinations(
            "[one](https://example.org) [two][REF] ![image](img.png)\n\n[ref]: /local",
        )
        .unwrap()
        .unwrap();
        assert_eq!(links, ["https://example.org", "/local"]);
        assert!(
            markdown_link_destinations("[two][é]\n\n[é]: /local")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn link_gate_distinguishes_absent_capability_from_zero() {
        use crate::content_gate::{ContentPreset, GateStatus};
        let preset = ContentPreset::from_toml(
            r#"
version = 1
name = "links"
[[gates]]
id = "links"
kind = "link_count"
min = 2
"#,
        )
        .unwrap();
        let (input, context, link_count) = markdown_input_with_links("test.md", "[one](https://example.org) and [two][ref] and ![image](img.png)\n\n[ref]: https://example.com").unwrap();
        assert_eq!(link_count, 2);
        let evaluated = preset
            .evaluate_with_links(&input, None, None, Some(&context), Some(link_count))
            .unwrap();
        assert_eq!(evaluated.gates[0].status, GateStatus::Passed);
        assert_eq!(evaluated.gates[0].measurements["link_count"], 2);
        assert_eq!(
            preset.evaluate(&input).unwrap().gates[0].status,
            GateStatus::NotChecked
        );
        let (plain, empty, zero) = markdown_input_with_links("plain.md", "No hyperlinks.").unwrap();
        let failed = preset
            .evaluate_with_links(&plain, None, None, Some(&empty), Some(zero))
            .unwrap();
        assert_eq!(failed.gates[0].status, GateStatus::Failed);
        assert_eq!(failed.gates[0].measurements["link_count"], 0);
    }

    #[test]
    fn extracts_readable_text_without_frontmatter_code_or_destinations() {
        let input = markdown_input("test.md", "---\ntitle: hidden\n---\n# Hello *world*\n\nSee [link](https://hidden.example) and `code`.\n\n```rust\nsecret()\n```\n").unwrap();
        let text = input.text.unwrap();
        assert!(text.contains("Hello world"));
        assert!(text.contains("See link"));
        assert!(!text.contains("hidden"));
        assert!(!text.contains("secret"));
        assert!(!text.contains("code"));
        assert_eq!(input.headings.unwrap(), ["Hello world"]);
    }
}

//! CommonMark adapter. Locations in reports refer to the extracted text.
use crate::content_gate::{ContentContext, ContentInput, ContentParagraph, collapse_whitespace};
use crate::text_index::LineIndex;
use anyhow::{Result, anyhow, bail};
use markdown::{
    ParseOptions,
    mdast::{Link, Node},
};

/// Nested block containers accepted from one line. Parse time grows
/// superlinearly with nesting, and the resulting tree is walked recursively.
pub const MAX_BLOCK_NESTING: usize = 100;
/// Unclosed link or image labels accepted in one block.
pub const MAX_LABEL_NESTING: usize = 100;
/// Emphasis delimiter runs accepted in one block, which also bounds how deeply
/// emphasis can nest.
pub const MAX_EMPHASIS_RUNS: usize = 1000;
/// Bound extra mappings for a single transformed multiline text node.
const MAX_MAPPED_CONTAINER_LINES: usize = 10_000;

/// A conservative mapping from extracted text back to exact Markdown text spans.
/// Transformed or synthetic text has no mapping.
#[derive(Debug, Default)]
pub struct MarkdownSourceMap {
    enabled: bool,
    spans: Vec<(usize, usize, usize)>,
    source_index: LineIndex,
    projected_index: LineIndex,
    link_starts: Vec<Option<usize>>,
    heading_starts: Vec<Option<usize>>,
}

impl MarkdownSourceMap {
    /// Resolve a zero-based heading index to the start of its Markdown node.
    pub fn heading_source_position(&self, source: &str, index: usize) -> Option<(usize, usize)> {
        self.source_index
            .position(source, (*self.heading_starts.get(index)?)?)
    }

    /// Resolve a zero-based extracted link index to its original Markdown start.
    /// Pass the unchanged source used to create this map.
    pub fn link_source_position(&self, source: &str, index: usize) -> Option<(usize, usize)> {
        self.source_index
            .position(source, (*self.link_starts.get(index)?)?)
    }

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
        let offset = self.projected_index.offset(projected, line, column)?;
        let span_index = self
            .spans
            .partition_point(|&(start, _, _)| start <= offset)
            .checked_sub(1)?;
        let &(start, end, source_start) = self.spans.get(span_index)?;
        if offset >= end {
            return None;
        }
        self.source_index
            .position(source, source_start + offset - start)
    }
}

pub use crate::source_mapping::prose_gate_has_positions;
use crate::source_mapping::{ContentSourceMap, map_gate_source_positions};

impl ContentSourceMap for MarkdownSourceMap {
    fn heading_count(&self) -> usize {
        self.heading_starts.len()
    }
    fn link_count(&self) -> usize {
        self.link_starts.len()
    }
    fn prose_position(
        &self,
        source: &str,
        projected: &str,
        line: usize,
        column: usize,
    ) -> Option<(usize, usize)> {
        self.source_position(source, projected, line, column)
    }
    fn heading_position(&self, source: &str, index: usize) -> Option<(usize, usize)> {
        self.heading_source_position(source, index)
    }
    fn link_position(&self, source: &str, index: usize) -> Option<(usize, usize)> {
        self.link_source_position(source, index)
    }
}

/// Map positional gate findings to exact source positions. `None` means the
/// gate has no positional semantics; missing positions are never guessed.
/// Counts are checked, but callers must still pass the same source, input,
/// destinations and observations used to evaluate the report.
pub fn gate_source_positions(
    map: &MarkdownSourceMap,
    source: &str,
    input: &ContentInput,
    gate: &crate::content_gate::ContentGate,
    result: &crate::content_gate::ContentGateResult,
    destinations: Option<&[String]>,
    observations: Option<&[crate::content_gate::LinkReachabilityObservation]>,
) -> Option<Vec<Option<(usize, usize)>>> {
    map_gate_source_positions(map, source, input, gate, result, destinations, observations)
}

/// Container markers at the start of a line and the text after them. Indentation
/// before a marker counts too, because nesting a list item requires it.
fn container_prefix(line: &str) -> (usize, &str) {
    fn marker_length(text: &str) -> Option<usize> {
        let bytes = text.as_bytes();
        let length = match bytes.first()? {
            b'>' => return Some(1),
            b'-' | b'+' | b'*' => 1,
            b'0'..=b'9' => {
                let digits = bytes.iter().take_while(|b| b.is_ascii_digit()).count();
                matches!(bytes.get(digits), Some(b'.' | b')')).then_some(digits + 1)?
            }
            _ => return None,
        };
        matches!(bytes.get(length), None | Some(b' ' | b'\t')).then_some(length)
    }
    let mut depth = 0usize;
    let mut rest = line;
    loop {
        let text = rest.trim_start_matches([' ', '\t']);
        let Some(length) = marker_length(text) else {
            return (depth, text);
        };
        let indent: usize = rest[..rest.len() - text.len()]
            .bytes()
            .map(|b| if b == b'\t' { 4 } else { 1 })
            .sum();
        depth = depth.saturating_add(1 + indent / 2);
        rest = &text[length..];
    }
}

/// Explain why this Markdown must not be parsed, or return `None` when it is
/// within limits. This conservative scan runs before the parser, whose time and
/// tree depth are what the limits protect, so it can overestimate nesting.
pub fn markdown_limit_reason(source: &str) -> Option<String> {
    let (mut labels, mut runs) = (0usize, 0usize);
    for line in source.split('\n') {
        let (depth, text) = container_prefix(line);
        if depth > MAX_BLOCK_NESTING {
            return Some(format!(
                "Markdown nesting exceeds the limit of {MAX_BLOCK_NESTING} levels; input was not evaluated"
            ));
        }
        if depth > 0 || text.trim().is_empty() || text.starts_with(['#', '|']) {
            (labels, runs) = (0, 0);
        }
        let mut previous = ' ';
        let mut chars = text.chars().peekable();
        while let Some(ch) = chars.next() {
            match ch {
                '\\' => {
                    chars.next();
                }
                '[' => {
                    labels += 1;
                    if labels > MAX_LABEL_NESTING {
                        return Some(format!(
                            "Markdown link or image nesting exceeds the limit of {MAX_LABEL_NESTING} levels; input was not evaluated"
                        ));
                    }
                }
                ']' => labels = labels.saturating_sub(1),
                '*' | '_' | '~' => {
                    while chars.next_if_eq(&ch).is_some() {}
                    let next = chars.peek().copied().unwrap_or(' ');
                    let intraword =
                        ch == '_' && previous.is_alphanumeric() && next.is_alphanumeric();
                    if !intraword && !(previous.is_whitespace() && next.is_whitespace()) {
                        runs += 1;
                        if runs > MAX_EMPHASIS_RUNS {
                            return Some(format!(
                                "Markdown emphasis exceeds the limit of {MAX_EMPHASIS_RUNS} delimiter runs in one block; input was not evaluated"
                            ));
                        }
                    }
                }
                _ => {}
            }
            previous = ch;
        }
    }
    None
}

/// A GFM literal email address in prose, not written as a link by its author.
/// Only its renderer would add the `mailto:` scheme, so a destination policy
/// does not apply. Bare `www.` and scheme-prefixed URLs remain links because
/// they navigate to a web destination.
fn is_literal_email_autolink(link: &Link, source: &str) -> bool {
    let Some(address) = link.url.strip_prefix("mailto:") else {
        return false;
    };
    link.position
        .as_ref()
        .and_then(|position| source.get(position.start.offset..position.end.offset))
        == Some(address)
}

/// Offset of an inline code value within its source, including the backticks.
fn inline_code_value_offset(raw: &str, value: &str) -> Option<usize> {
    let ticks = raw.bytes().take_while(|byte| *byte == b'`').count();
    let inner = raw.get(ticks..raw.len().checked_sub(ticks)?)?;
    if inner == value {
        return Some(ticks);
    }
    (inner.strip_prefix(' ')?.strip_suffix(' ')? == value).then_some(ticks + 1)
}

/// Extract complete Markdown link destinations without fetching them. Returns
/// `None` when a reference cannot be resolved conservatively.
pub fn markdown_link_destinations(source: &str) -> Result<Option<Vec<String>>> {
    Ok(
        link_destinations_from_root(&parse_markdown(source)?, source)
            .map(|links| links.into_iter().map(|link| link.url).collect()),
    )
}

fn parse_markdown(source: &str) -> Result<Node> {
    if let Some(reason) = markdown_limit_reason(source) {
        bail!(reason);
    }
    let mut options = ParseOptions::gfm();
    options.constructs.frontmatter = true;
    markdown::to_mdast(source, &options).map_err(|err| anyhow!("Markdown parse error: {err}"))
}

struct ExtractedLink {
    url: String,
    start: Option<usize>,
}

fn link_destinations_from_root(root: &Node, source: &str) -> Option<Vec<ExtractedLink>> {
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
        links: &mut Vec<ExtractedLink>,
        source: &str,
    ) -> Option<()> {
        match node {
            Node::Link(link) if is_literal_email_autolink(link, source) => {}
            Node::Link(link) => links.push(ExtractedLink {
                url: link.url.clone(),
                start: link.position.as_ref().map(|pos| pos.start.offset),
            }),
            Node::LinkReference(link) => links.push(ExtractedLink {
                url: defs.get(&normalize(&link.identifier)?)?.clone(),
                start: link.position.as_ref().map(|pos| pos.start.offset),
            }),
            _ => {}
        }
        if let Some(children) = node.children() {
            for child in children {
                collect_links(child, defs, links, source)?;
            }
        }
        Some(())
    }
    let mut definitions = std::collections::BTreeMap::new();
    collect_definitions(root, &mut definitions);
    let mut links = Vec::new();
    collect_links(root, &definitions, &mut links, source).map(|_| links)
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
    let links = link_destinations_from_root(&root, source);
    let (input, context, count, mut map) = extract_markdown_from_root(id, source, true, &root)?;
    let destinations = links.map(|links| {
        map.link_starts = links.iter().map(|link| link.start).collect();
        links.into_iter().map(|link| link.url).collect()
    });
    Ok(MarkdownContentWithDestinations {
        input,
        context,
        link_count: count,
        source_map: map,
        destinations,
    })
}

/// Map only byte-identical text at the edges of a parser text node. When a
/// node contains escapes or entities, stop at their delimiters: matching bytes
/// inside an encoded character do not establish an exact source position.
fn map_text_spans(
    raw: &str,
    value: &str,
    source_start: usize,
    projected_start: usize,
    spans: &mut Vec<(usize, usize, usize)>,
) {
    if raw == value {
        if !value.is_empty() {
            spans.push((projected_start, projected_start + value.len(), source_start));
        }
        return;
    }
    // A soft-wrapped node may retain newlines while Markdown container
    // prefixes (`>` and indentation) are removed at the start of each line.
    // Only map this case when every line matches exactly after such a prefix;
    // transformed entities and escapes remain on the conservative path.
    if !raw.contains(['&', '\\']) && raw.contains('\n') {
        let raw_lines = raw.split('\n');
        let value_lines = value.split('\n');
        let line_count = raw_lines.clone().count();
        if line_count <= MAX_MAPPED_CONTAINER_LINES && line_count == value_lines.clone().count() {
            let mut mapped = Vec::new();
            let mut raw_offset = 0;
            let mut value_offset = 0;
            let mut exact = true;
            for (raw_line, value_line) in raw_lines.zip(value_lines) {
                if let Some(container) = raw_line.strip_suffix(value_line)
                    && container
                        .bytes()
                        .all(|byte| matches!(byte, b' ' | b'\t' | b'>'))
                {
                    if !value_line.is_empty() {
                        mapped.push((
                            projected_start + value_offset,
                            projected_start + value_offset + value_line.len(),
                            source_start + raw_offset + container.len(),
                        ));
                    }
                } else {
                    exact = false;
                    break;
                }
                raw_offset += raw_line.len() + 1;
                value_offset += value_line.len() + 1;
            }
            if exact {
                spans.extend(mapped);
                return;
            }
        }
    }
    let mut prefix = 0;
    for (source, projected) in raw.chars().zip(value.chars()) {
        if source != projected || matches!(source, '&' | '\\') {
            break;
        }
        prefix += source.len_utf8();
    }
    if prefix > 0 {
        spans.push((projected_start, projected_start + prefix, source_start));
    }

    // Ignore the last escape or entity and everything before it. In
    // particular, the final character of an entity is not a literal suffix.
    let suffix_floor = raw.rfind(['&', '\\']).map_or(0, |index| {
        let tail = &raw[index + 1..];
        if raw.as_bytes()[index] == b'&' {
            tail.find(';').map_or(raw.len(), |end| index + 2 + end)
        } else {
            tail.chars()
                .next()
                .map_or(raw.len(), |ch| index + 1 + ch.len_utf8())
        }
    });
    let raw_tail = &raw[prefix.max(suffix_floor)..];
    let value_tail = &value[prefix..];
    let mut suffix = 0;
    for (source, projected) in raw_tail.chars().rev().zip(value_tail.chars().rev()) {
        if source != projected {
            break;
        }
        suffix += source.len_utf8();
    }
    if suffix > 0 {
        spans.push((
            projected_start + value.len() - suffix,
            projected_start + value.len(),
            source_start + raw.len() - suffix,
        ));
    }
}

#[derive(Default)]
struct Projection {
    text: String,
    headings: Vec<String>,
    ranges: Vec<(usize, usize)>,
    block_ends: Vec<usize>,
    link_count: usize,
}

impl Projection {
    fn visit_children(&mut self, node: &Node, source: &str, source_map: &mut MarkdownSourceMap) {
        if let Some(children) = node.children() {
            for child in children {
                self.visit(child, source, source_map);
            }
        }
    }

    fn visit(&mut self, node: &Node, source: &str, source_map: &mut MarkdownSourceMap) {
        match node {
            Node::Text(item) => {
                let start = self.text.len();
                self.text.push_str(&item.value);
                if let Some(pos) = item.position.as_ref().filter(|_| source_map.enabled) {
                    let (a, b) = (pos.start.offset, pos.end.offset);
                    if let Some(raw) = source.get(a..b) {
                        map_text_spans(raw, &item.value, a, start, &mut source_map.spans);
                    }
                }
            }
            Node::InlineCode(item) => {
                let start = self.text.len();
                self.text.push_str(&item.value);
                if let Some(pos) = item.position.as_ref().filter(|_| source_map.enabled) {
                    let (a, b) = (pos.start.offset, pos.end.offset);
                    if let Some(within) = source
                        .get(a..b)
                        .and_then(|raw| inline_code_value_offset(raw, &item.value))
                    {
                        source_map.spans.push((start, self.text.len(), a + within));
                    }
                }
            }
            Node::Image(item) => self.text.push_str(&item.alt),
            Node::ImageReference(item) => self.text.push_str(&item.alt),
            Node::Break(_) => self.text.push('\n'),
            Node::Heading(heading) => {
                if source_map.enabled {
                    source_map
                        .heading_starts
                        .push(heading.position.as_ref().map(|pos| pos.start.offset));
                }
                let start = self.text.len();
                self.visit_children(node, source, source_map);
                self.headings.push(collapse_whitespace(&self.text[start..]));
                self.end_block();
            }
            Node::Paragraph(_) | Node::TableCell(_) => {
                let start = self.text.len();
                self.visit_children(node, source, source_map);
                if matches!(node, Node::Paragraph(_)) {
                    self.ranges.push((start, self.text.len()));
                }
                self.end_block();
            }
            _ => {
                let is_link = match node {
                    Node::Link(link) => !is_literal_email_autolink(link, source),
                    Node::LinkReference(_) => true,
                    _ => false,
                };
                if is_link {
                    self.link_count += 1;
                }
                self.visit_children(node, source, source_map);
            }
        }
    }

    fn end_block(&mut self) {
        self.block_ends.push(self.text.len());
        self.text.push('\n');
    }
}

fn extract_markdown_from_root(
    id: impl Into<String>,
    source: &str,
    map_source: bool,
    root: &Node,
) -> Result<(ContentInput, ContentContext, usize, MarkdownSourceMap)> {
    let mut source_map = MarkdownSourceMap {
        enabled: map_source,
        ..Default::default()
    };
    let mut projection = Projection::default();
    projection.visit(root, source, &mut source_map);
    let Projection {
        text,
        headings,
        ranges,
        block_ends,
        link_count,
    } = projection;
    if map_source {
        source_map.source_index = LineIndex::new(source);
        source_map.projected_index = LineIndex::new(&text);
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
            block_ends: Some(block_ends),
            ..Default::default()
        },
        link_count,
        source_map,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn source_mapping_rejects_mismatched_heading_and_link_counts() {
        use crate::content_gate::{
            ContentEvaluationContext, ContentPreset, LinkReachabilityObservation as Observation,
        };
        let source = "# Heading\n\n[first](/a) and [second](/b)";
        let mut adapted = markdown_input_with_source_map_and_destinations("x", source).unwrap();
        let preset = ContentPreset::from_toml("version=1\nname='test'\n[[gates]]\nid='heading'\nkind='heading_length'\nmax_chars=1\n[[gates]]\nid='links'\nkind='link_reachability'\n").unwrap();
        let observations = [Observation::NotFound, Observation::Reachable];
        let context = ContentEvaluationContext {
            link_reachability: Some(&observations),
            ..Default::default()
        };
        let destinations = adapted.destinations.as_deref().unwrap();
        let report = preset
            .evaluate_with_destinations(&adapted.input, &context, Some(destinations))
            .unwrap();
        let map = &adapted.source_map;
        assert!(
            gate_source_positions(
                map,
                source,
                &adapted.input,
                &preset.gates[1],
                &report.gates[1],
                Some(destinations),
                Some(&observations)
            )
            .unwrap()[0]
                .is_some()
        );
        let too_few = &destinations[..1];
        assert!(
            gate_source_positions(
                map,
                source,
                &adapted.input,
                &preset.gates[1],
                &report.gates[1],
                Some(too_few),
                Some(&observations)
            )
            .is_none()
        );
        assert!(
            gate_source_positions(
                map,
                source,
                &adapted.input,
                &preset.gates[1],
                &report.gates[1],
                Some(destinations),
                Some(&observations[..1])
            )
            .is_none()
        );
        let changed = [Observation::NotFound, Observation::NotFound];
        assert_eq!(
            gate_source_positions(
                map,
                source,
                &adapted.input,
                &preset.gates[1],
                &report.gates[1],
                Some(destinations),
                Some(&changed)
            )
            .unwrap(),
            vec![None]
        );
        adapted
            .input
            .headings
            .as_mut()
            .unwrap()
            .push("extra".into());
        assert!(
            gate_source_positions(
                map,
                source,
                &adapted.input,
                &preset.gates[0],
                &report.gates[0],
                Some(destinations),
                None
            )
            .is_none()
        );
    }

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
    fn maps_exact_wrapped_quote_lines_without_guessing_transformed_text() {
        let source = "> Premier élan\n> second terme\n> troisième terme\n";
        let (input, _, _, map) = markdown_input_with_source_map("t.md", source).unwrap();
        let text = input.text.unwrap();
        assert_eq!(text, "Premier élan\nsecond terme\ntroisième terme\n");
        assert_eq!(map.source_position(source, &text, 1, 9), Some((1, 11)));
        assert_eq!(map.source_position(source, &text, 2, 1), Some((2, 3)));
        assert_eq!(map.source_position(source, &text, 3, 1), Some((3, 3)));

        let escaped = "> first\n> second &amp; transformed\n> last\n";
        let (input, _, _, map) = markdown_input_with_source_map("t.md", escaped).unwrap();
        let text = input.text.unwrap();
        assert_eq!(map.source_position(escaped, &text, 2, 8), None);
    }

    #[test]
    fn bounds_extra_mappings_for_very_long_wrapped_nodes() {
        let raw = (0..=MAX_MAPPED_CONTAINER_LINES)
            .map(|_| "> line")
            .collect::<Vec<_>>()
            .join("\n");
        let value = (0..=MAX_MAPPED_CONTAINER_LINES)
            .map(|_| "line")
            .collect::<Vec<_>>()
            .join("\n");
        let mut spans = Vec::new();
        map_text_spans(&raw, &value, 0, 0, &mut spans);
        assert!(spans.len() <= 2);
    }

    #[test]
    fn maps_findings_on_the_middle_line_of_a_wrapped_quote() {
        let source = "> Introduction\n> forbidden phrase\n> conclusion\n";
        let (input, context, links, map) = markdown_input_with_source_map("t.md", source).unwrap();
        let preset = crate::content_gate::ContentPreset::from_toml(
            "version=1\nname='t'\n[[gates]]\nid='g'\nkind='forbidden_terms'\nterms=['forbidden']\n",
        )
        .unwrap();
        let report = preset
            .evaluate_with_links(&input, None, None, Some(&context), Some(links))
            .unwrap();
        let finding = &report.gates[0].findings[0];
        assert_eq!(
            map.source_position(
                source,
                input.text.as_deref().unwrap(),
                finding.line,
                finding.column
            ),
            Some((2, 3))
        );
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
    fn maps_unchanged_edges_around_entities_and_escapes() {
        let source = "Avant &amp; après élan.\n\nDébut \\*fin\\*.\n\nAvant &#59; après.\n";
        let (input, _, _, map) = markdown_input_with_source_map("t.md", source).unwrap();
        let text = input.text.unwrap();
        assert_eq!(text, "Avant & après élan.\nDébut *fin*.\nAvant ; après.\n");
        assert_eq!(map.source_position(source, &text, 1, 1), Some((1, 1)));
        assert_eq!(map.source_position(source, &text, 1, 7), None);
        assert_eq!(map.source_position(source, &text, 1, 9), Some((1, 13)));
        assert_eq!(map.source_position(source, &text, 2, 1), Some((3, 1)));
        assert_eq!(map.source_position(source, &text, 2, 7), None);
        assert_eq!(map.source_position(source, &text, 3, 7), None);
        assert_eq!(map.source_position(source, &text, 3, 9), Some((5, 13)));
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
    fn extracts_readable_text_without_frontmatter_code_blocks_or_destinations() {
        let input = markdown_input("test.md", "---\ntitle: hidden\n---\n# Hello *world*\n\nSee [link](https://hidden.example).\n\n```rust\nsecret()\n```\n").unwrap();
        let text = input.text.unwrap();
        assert!(text.contains("Hello world"));
        assert!(text.contains("See link"));
        assert!(!text.contains("hidden"));
        assert!(!text.contains("secret"));
        assert_eq!(input.headings.unwrap(), ["Hello world"]);
    }

    fn report(source: &str, gate: &str) -> crate::content_gate::ContentGateResult {
        use crate::content_gate::{ContentEvaluationContext, ContentPreset};
        let preset =
            ContentPreset::from_toml(&format!("version=1\nname='t'\n[[gates]]\nid='g'\n{gate}\n"))
                .unwrap();
        let adapted = markdown_input_with_source_map_and_destinations("t.md", source).unwrap();
        preset
            .evaluate_with_destinations(
                &adapted.input,
                &ContentEvaluationContext {
                    adapter: Some(&adapted.context),
                    link_count: Some(adapted.link_count),
                    ..Default::default()
                },
                adapted.destinations.as_deref(),
            )
            .unwrap()
            .gates
            .remove(0)
    }

    #[test]
    fn block_ends_are_sentence_boundaries() {
        use crate::content_gate::GateStatus;
        let gate = "kind='sentence_length'\nmax_words=6\nblocking=false";
        let bullets = "- one two three four\n- five six seven eight\n- nine ten eleven twelve\n";
        let result = report(bullets, gate);
        assert_eq!(result.status, GateStatus::Passed);
        assert_eq!(result.measurements["sentence_count"], 3);
        for source in [
            "# one two three four\n\nfive six seven eight\n",
            "one two three four\n\nfive six seven eight\n",
            "| one two three four | five six seven eight |\n| --- | --- |\n",
        ] {
            let result = report(source, gate);
            assert_eq!(result.status, GateStatus::Passed, "{source}");
            assert_eq!(result.measurements["sentence_count"], 2, "{source}");
        }
        // A soft wrap continues the sentence.
        let wrapped = report("one two three four\nfive six seven eight\n", gate);
        assert_eq!(wrapped.status, GateStatus::Failed);
        assert_eq!(wrapped.measurements["sentence_count"], 1);
    }

    #[test]
    fn paragraph_only_sentences_ignore_headings_and_keep_positions() {
        use crate::content_gate::GateStatus;
        let source = "# A heading that exceeds the limit\n\nShort text.\n\nA longer sentence spans\nmultiple lines here.\n";
        for (kind, limit) in [
            ("sentence_length", "max_words=3"),
            ("sentence_character_length", "max_chars=15"),
        ] {
            let standard = report(source, &format!("kind='{kind}'\n{limit}"));
            let scoped = report(
                source,
                &format!("kind='{kind}'\n{limit}\nparagraphs_only=true"),
            );
            assert_eq!(standard.measurements["sentence_count"], 3);
            assert_eq!(scoped.measurements["sentence_count"], 2);
            assert_eq!(scoped.status, GateStatus::Failed);
            assert_eq!(scoped.findings.len(), 1);
            assert_eq!((scoped.findings[0].line, scoped.findings[0].column), (3, 1));
            assert_eq!(scoped.required_capability, "paragraphs");
        }
    }

    #[test]
    fn sentence_abbreviations_are_explicit_and_preserve_block_boundaries() {
        use crate::content_gate::GateStatus;
        let preset = crate::content_gate::ContentPreset::from_toml(
            "version=1\nname='t'\n[[gates]]\nid='g'\nkind='sentence_length'\nmax_words=4\n",
        )
        .unwrap();
        let gate_json = serde_json::to_value(&preset.gates[0]).unwrap();
        assert!(gate_json.get("abbreviations").is_none());
        let plain = "kind='sentence_length'\nmax_words=4";
        let configured = "kind='sentence_length'\nmax_words=4\nabbreviations=['Dr.', 'M.']";
        let source = "Dr. Martin writes clear notes.\n";
        assert_eq!(report(source, plain).status, GateStatus::Passed);
        let result = report(source, configured);
        assert_eq!(result.status, GateStatus::Failed);
        assert_eq!(result.measurements["sentence_count"], 1);
        assert_eq!(result.measurements["longest_sentence_words"], 5);
        let blocks = report("Dr.\n\nM. Martin writes.\n", configured);
        assert_eq!(blocks.measurements["sentence_count"], 2);
        let exact = report("Mrs. Martin writes.\n", configured);
        assert_eq!(exact.measurements["sentence_count"], 2);
    }

    #[test]
    fn sentence_abbreviation_configuration_rejects_unusable_tokens() {
        use crate::content_gate::ContentPreset;
        for entries in [
            "['']",
            "['Dr']",
            "['.']",
            "['e.g.']",
            "['Dr Smith.']",
            "['Dr.', 'Dr.']",
        ] {
            let preset = format!(
                "version=1\nname='t'\n[[gates]]\nid='g'\nkind='sentence_length'\nmax_words=10\nabbreviations={entries}\n"
            );
            assert!(ContentPreset::from_toml(&preset).is_err(), "{entries}");
        }
    }

    #[test]
    fn paragraph_character_limit_counts_unicode_scalars() {
        use crate::content_gate::GateStatus;
        let gate = "kind='paragraph_character_length'\nmax_chars=4";
        let result = report("中文文章測試\n", gate);
        assert_eq!(result.status, GateStatus::Failed);
        assert_eq!(result.measurements["longest_paragraph_chars"], 6);
        assert_eq!(result.measurements["max_chars"], 4);
        assert_eq!(report("é😀中文\n", gate).status, GateStatus::Passed);
        let preset = crate::content_gate::ContentPreset::from_toml("version=1\nname='t'\n[[gates]]\nid='g'\nkind='paragraph_character_length'\nmax_chars=4\n").unwrap();
        let input = crate::content_gate::ContentInput {
            id: "t".into(),
            text: Some("中文".into()),
            ..Default::default()
        };
        assert_eq!(
            preset.evaluate(&input).unwrap().gates[0].status,
            GateStatus::NotChecked
        );
        assert!(crate::content_gate::ContentPreset::from_toml("version=1\nname='t'\n[[gates]]\nid='g'\nkind='paragraph_character_length'\nmax_chars=0\n").is_err());
    }

    #[test]
    fn sentence_character_gate_counts_unicode_without_punctuation_or_edge_whitespace() {
        use crate::content_gate::{ContentInput, ContentPreset, GateStatus};
        let gate = "kind='sentence_character_length'\nmax_chars=4\ndelimiters='。！？'";
        let result = report("中文文章測試。 好文。\n", gate);
        assert_eq!(result.status, GateStatus::Failed);
        assert_eq!(result.measurements["sentence_count"], 2);
        assert_eq!(result.measurements["longest_sentence_chars"], 6);
        assert_eq!(result.measurements["max_chars"], 4);
        assert_eq!((result.findings[0].line, result.findings[0].column), (1, 1));
        let abbreviations = "kind='sentence_character_length'\nmax_chars=3\nabbreviations=['Dr.']";
        assert_eq!(
            report("Dr. X y.\n", abbreviations).status,
            GateStatus::Failed
        );
        let preset = ContentPreset::from_toml("version=1\nname='t'\n[[gates]]\nid='g'\nkind='sentence_character_length'\nmax_chars=4\n").unwrap();
        assert_eq!(
            preset.evaluate(&ContentInput::default()).unwrap().gates[0].status,
            GateStatus::NotChecked
        );
        assert!(ContentPreset::from_toml("version=1\nname='t'\n[[gates]]\nid='g'\nkind='sentence_character_length'\nmax_chars=0\n").is_err());
    }

    #[test]
    fn inline_code_stays_in_headings_and_prose_and_maps_to_source() {
        use crate::content_gate::GateStatus;
        let source = "## Option `a`\n\n## Option `b`\n\nUse `` x ``.\n";
        let (input, _, _, map) = markdown_input_with_source_map("t.md", source).unwrap();
        assert_eq!(input.headings.as_ref().unwrap(), &["Option a", "Option b"]);
        let text = input.text.unwrap();
        assert_eq!(text, "Option a\nOption b\nUse x.\n");
        assert_eq!(map.source_position(source, &text, 1, 8), Some((1, 12)));
        assert_eq!(map.source_position(source, &text, 3, 5), Some((5, 8)));
        assert_eq!(
            report(source, "kind='duplicate_headings'").status,
            GateStatus::Passed
        );
        assert_eq!(
            report(source, "kind='required_headings'\nheadings=['Option b']").status,
            GateStatus::Passed
        );
    }

    #[test]
    fn soft_wrapped_terms_match_at_their_source_position() {
        let source = "The best\nprice wins.\n";
        let (input, context, links, map) = markdown_input_with_source_map("t.md", source).unwrap();
        let preset = crate::content_gate::ContentPreset::from_toml(
            "version=1\nname='t'\n[[gates]]\nid='g'\nkind='forbidden_terms'\nterms=['best price']\n",
        )
        .unwrap();
        let result = preset
            .evaluate_with_links(&input, None, None, Some(&context), Some(links))
            .unwrap();
        let finding = &result.gates[0].findings[0];
        assert_eq!((finding.line, finding.column), (1, 5));
        let text = input.text.unwrap();
        assert_eq!(
            map.source_position(source, &text, finding.line, finding.column),
            Some((1, 5))
        );
    }

    #[test]
    fn terms_do_not_match_across_block_ends() {
        use crate::content_gate::GateStatus;
        let gate = "kind='forbidden_terms'\nterms=['best price']";
        assert_eq!(report("- best\n- price\n", gate).status, GateStatus::Passed);
        assert_eq!(report("# best\n\nprice\n", gate).status, GateStatus::Passed);
        assert_eq!(report("- best\n  price\n", gate).status, GateStatus::Failed);
    }

    #[test]
    fn literal_email_is_outside_link_policy_but_other_autolinks_are_not() {
        use crate::content_gate::GateStatus;
        let gate = "kind='link_destinations'\nallowed_schemes=['https']";
        let email = report("Write to me@example.org today.\n", gate);
        assert_eq!(email.status, GateStatus::Passed);
        assert_eq!(email.measurements["checked_destinations"], 0);
        let (input, _, links) =
            markdown_input_with_links("t.md", "Write to me@example.org today.\n").unwrap();
        assert_eq!(links, 0);
        assert!(input.text.unwrap().contains("me@example.org"));
        for (source, value, scheme) in [
            ("<me@example.org>\n", "mailto:me@example.org", "mailto"),
            (
                "[mail](mailto:me@example.org)\n",
                "mailto:me@example.org",
                "mailto",
            ),
            (
                "Go to www.example.org now.\n",
                "http://www.example.org",
                "http",
            ),
            (
                "Go to http://example.org now.\n",
                "http://example.org",
                "http",
            ),
        ] {
            let result = report(source, gate);
            assert_eq!(result.status, GateStatus::Failed, "{source}");
            let message = &result.findings[0].message;
            assert!(message.contains(value), "{message}");
            assert!(message.contains(&format!("{scheme:?}")), "{message}");
        }
    }

    #[test]
    fn limits_reject_deep_nesting_before_parsing() {
        assert!(markdown_limit_reason(&format!("{} x", ">".repeat(100_000))).is_some());
        assert!(markdown_limit_reason(&format!("{} x", "- ".repeat(101))).is_some());
        let indented: String = (0..101)
            .map(|n| format!("{}- x\n", "  ".repeat(n)))
            .collect();
        assert!(markdown_limit_reason(&indented).is_some());
        assert!(markdown_limit_reason(&"![".repeat(101)).is_some());
        assert!(markdown_limit_reason(&"*a ".repeat(1001)).is_some());
        assert!(parse_markdown(&">".repeat(101)).is_err());
    }

    #[test]
    fn limits_accept_ordinary_structure_within_them() {
        for source in [
            format!("{} x", ">".repeat(100)),
            format!("```\n{}x\n```", " ".repeat(400)),
            "snake_case_name 2 * 3 ".repeat(2000),
            "[a](b) *c* ".repeat(400),
            "- *a* b\n".repeat(2000),
            "*a* b\n\n".repeat(2000),
        ] {
            assert_eq!(markdown_limit_reason(&source), None, "{source:.40}");
            assert!(parse_markdown(&source).is_ok());
        }
    }
}

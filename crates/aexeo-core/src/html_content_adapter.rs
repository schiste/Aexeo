//! Explicit static HTML projection for portable content gates.
//!
//! This observes the supplied markup only; it cannot establish browser state,
//! CSS visibility, generated content, or the completeness of a rendered page.
use crate::content_gate::{
    ContentContext, ContentInput, ContentParagraph, MAX_CONTENT_INPUT_BYTES, collapse_whitespace,
};
use html5ever::{parse_document, tendril::TendrilSink};
use markup5ever_rcdom::{Handle, NodeData, RcDom};

/// Maximum number of DOM nodes traversed for one projection.
pub const MAX_HTML_CONTENT_NODES: usize = 100_000;

pub struct HtmlContent {
    pub input: ContentInput,
    pub context: ContentContext,
    pub destinations: Vec<String>,
}

enum Event {
    Enter(Handle),
    EndBlock {
        start: usize,
        heading: bool,
        paragraph: bool,
    },
}

/// Project a complete supplied HTML document or fragment, without fetching links.
/// Errors describe limits, never source contents. No source positions are guessed.
pub fn html_content(id: impl Into<String>, source: &str) -> Result<HtmlContent, &'static str> {
    if source.len() > MAX_CONTENT_INPUT_BYTES {
        return Err("HTML exceeds the content input byte limit");
    }
    let dom = parse_document(RcDom::default(), Default::default()).one(source);
    let mut stack = vec![Event::Enter(dom.document.clone())];
    let mut text = String::new();
    let mut headings = Vec::new();
    let mut ranges = Vec::new();
    let mut block_ends = Vec::new();
    let mut destinations = Vec::new();
    let mut visited = 0usize;
    while let Some(event) = stack.pop() {
        match event {
            Event::EndBlock {
                start,
                heading,
                paragraph,
            } => {
                if heading {
                    headings.push(collapse_whitespace(&text[start..]));
                }
                if paragraph {
                    ranges.push((start, text.len()));
                }
                block_ends.push(text.len());
                text.push('\n');
            }
            Event::Enter(node) => {
                visited += 1;
                if visited > MAX_HTML_CONTENT_NODES {
                    return Err("HTML exceeds the node limit");
                }
                match &node.data {
                    NodeData::Text { contents } => text.push_str(&contents.borrow()),
                    NodeData::Element { name, attrs, .. } => {
                        let tag = name.local.as_ref();
                        let attrs = attrs.borrow();
                        if matches!(tag, "head" | "script" | "style" | "template" | "noscript")
                            || attrs
                                .iter()
                                .any(|attr| attr.name.local.as_ref() == "hidden")
                        {
                            continue;
                        }
                        if tag == "a"
                            && let Some(href) =
                                attrs.iter().find(|attr| attr.name.local.as_ref() == "href")
                        {
                            destinations.push(href.value.to_string());
                        }
                        if tag == "img"
                            && let Some(alt) =
                                attrs.iter().find(|attr| attr.name.local.as_ref() == "alt")
                        {
                            text.push_str(&alt.value);
                        }
                        if tag == "br" {
                            text.push('\n');
                        }
                        let heading = matches!(tag, "h1" | "h2" | "h3" | "h4" | "h5" | "h6");
                        let paragraph = tag == "p";
                        if tag != "br" && crate::site::markup::tag_breaks_text(tag) {
                            if !text.is_empty() && !text.ends_with('\n') {
                                text.push('\n');
                            }
                            stack.push(Event::EndBlock {
                                start: text.len(),
                                heading,
                                paragraph,
                            });
                        }
                    }
                    _ => {}
                }
                stack.extend(
                    node.children
                        .borrow()
                        .iter()
                        .rev()
                        .cloned()
                        .map(Event::Enter),
                );
            }
        }
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
    Ok(HtmlContent {
        input: ContentInput {
            id: id.into(),
            text: Some(text),
            source_text: Some(source.into()),
            headings: Some(headings),
            ..Default::default()
        },
        context: ContentContext {
            paragraphs: Some(paragraphs),
            block_ends: Some(block_ends),
            ..Default::default()
        },
        destinations,
    })
}

/// Exact source mapping for a deliberately restricted, well-nested HTML subset.
/// Unsupported markup and transformed text return no map, never guessed positions.
#[derive(Debug)]
pub struct HtmlSourceMap {
    spans: Vec<(usize, usize, usize)>,
    source_index: crate::text_index::LineIndex,
    projected_index: crate::text_index::LineIndex,
}

impl HtmlSourceMap {
    /// Source coordinates for a character in the unchanged projected text.
    pub fn source_position(
        &self,
        source: &str,
        projected: &str,
        line: usize,
        column: usize,
    ) -> Option<(usize, usize)> {
        let offset = self.projected_index.offset(projected, line, column)?;
        let index = self
            .spans
            .partition_point(|(start, _, _)| *start <= offset)
            .checked_sub(1)?;
        let &(start, end, source_start) = self.spans.get(index)?;
        (offset < end)
            .then(|| {
                self.source_index
                    .position(source, source_start + offset - start)
            })
            .flatten()
    }
}

// Accept ordinary HTML attributes while rejecting malformed syntax and the
// hidden attribute, which changes which subtrees the adapter projects.
fn safe_mapping_attributes(mut rest: &str, void: bool) -> bool {
    loop {
        rest = rest.trim_start_matches(|ch: char| ch.is_ascii_whitespace());
        if rest.is_empty() {
            return true;
        }
        if rest == "/" {
            return void;
        }
        let size = rest
            .bytes()
            .take_while(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b':' | b'.'))
            .count();
        if size == 0 || rest[..size].eq_ignore_ascii_case("hidden") {
            return false;
        }
        rest = rest[size..].trim_start_matches(|ch: char| ch.is_ascii_whitespace());
        if let Some(after_equals) = rest.strip_prefix('=') {
            rest = after_equals.trim_start_matches(|ch: char| ch.is_ascii_whitespace());
            let Some(quote) = rest.as_bytes().first().copied() else {
                return false;
            };
            if matches!(quote, b'\'' | b'"') {
                let Some(end) = rest[1..].find(quote as char) else {
                    return false;
                };
                rest = &rest[end + 2..];
            } else {
                let size = rest
                    .bytes()
                    .take_while(|b| {
                        !b.is_ascii_whitespace() && !matches!(b, b'"' | b'\'' | b'<' | b'=' | b'`')
                    })
                    .count();
                if size == 0 {
                    return false;
                }
                rest = &rest[size..];
            }
        }
    }
}

/// Build a mapping only when a conservative lexical reconstruction exactly
/// matches the parser's projection. This does not cover entities, raw-text or
/// hidden elements, malformed HTML, or browser-generated content.
pub fn html_source_map(source: &str, projected: &str) -> Option<HtmlSourceMap> {
    use crate::site::markup::{open_tag_end, tag_breaks_text};
    if source.len() > MAX_CONTENT_INPUT_BYTES || source.contains(['\r', '\0', '\x0b']) {
        return None;
    }
    let mut stack: Vec<String> = Vec::new();
    let mut spans = Vec::new();
    let mut output = String::new();
    let mut cursor = 0;
    while cursor < source.len() {
        if source.as_bytes()[cursor] != b'<' {
            let end = source[cursor..]
                .find('<')
                .map_or(source.len(), |n| cursor + n);
            let literal = &source[cursor..end];
            if literal.contains('&') {
                return None;
            }
            let start = output.len();
            output.push_str(literal);
            spans.push((start, output.len(), cursor));
            cursor = end;
            continue;
        }
        let end = open_tag_end(source, cursor)?;
        let raw = &source[cursor..=end];
        let inner = raw.strip_prefix('<')?.strip_suffix('>')?;
        let (closing, inner) = inner
            .strip_prefix('/')
            .map_or((false, inner), |s| (true, s));
        let name_len = inner.bytes().take_while(u8::is_ascii_alphanumeric).count();
        if name_len == 0 {
            return None;
        }
        let tag = inner[..name_len].to_ascii_lowercase();
        if !matches!(
            tag.as_str(),
            "html"
                | "body"
                | "main"
                | "article"
                | "section"
                | "div"
                | "p"
                | "h1"
                | "h2"
                | "h3"
                | "h4"
                | "h5"
                | "h6"
                | "span"
                | "b"
                | "strong"
                | "i"
                | "em"
                | "a"
                | "br"
        ) {
            return None;
        }
        let rest = &inner[name_len..];
        if closing {
            if !rest.trim().is_empty() || stack.pop().as_deref() != Some(&tag) {
                return None;
            }
            if tag_breaks_text(&tag) {
                output.push('\n');
            }
        } else {
            // Restrict attributes to a form that cannot affect projection or
            // hide a subtree. Other valid HTML is still evaluated without a map.
            if !safe_mapping_attributes(rest, tag == "br") || stack.len() >= 256 {
                return None;
            }
            let block = tag_breaks_text(&tag) && tag != "br";
            if block
                && stack.iter().any(|parent| {
                    matches!(
                        parent.as_str(),
                        "p" | "h1"
                            | "h2"
                            | "h3"
                            | "h4"
                            | "h5"
                            | "h6"
                            | "span"
                            | "b"
                            | "strong"
                            | "i"
                            | "em"
                            | "a"
                    )
                })
            {
                return None;
            }
            if tag == "a" && stack.iter().any(|parent| parent == "a") {
                return None;
            }
            if tag == "br" {
                output.push('\n');
            } else {
                if block && !output.is_empty() && !output.ends_with('\n') {
                    output.push('\n');
                }
                stack.push(tag);
            }
        }
        cursor = end + 1;
    }
    if !stack.is_empty() || output != projected {
        return None;
    }
    Some(HtmlSourceMap {
        spans,
        source_index: crate::text_index::LineIndex::new(source),
        projected_index: crate::text_index::LineIndex::new(projected),
    })
}

/// Map only findings whose coordinates are defined in projected prose.
pub fn gate_source_positions(
    map: &HtmlSourceMap,
    source: &str,
    input: &ContentInput,
    gate: &crate::content_gate::ContentGate,
    result: &crate::content_gate::ContentGateResult,
) -> Option<Vec<Option<(usize, usize)>>> {
    if !crate::markdown_adapter::prose_gate_has_positions(gate) {
        return None;
    }
    let projected = input.text.as_deref()?;
    Some(
        result
            .findings
            .iter()
            .map(|finding| map.source_position(source, projected, finding.line, finding.column))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projects_fragments_without_hidden_or_inert_content() {
        let html = r#"<h1>Élan &amp; action</h1><p>Read <a href='/first'>this</a> and <a href='https://example.org'>that</a>.</p><p hidden>secret <a href='/hidden'>no</a></p><template><p>template</p></template><script>script</script><p>End <img alt='diagram' src='x'></p>"#;
        let result = html_content("example", html).unwrap();
        assert_eq!(result.input.headings.unwrap(), vec!["Élan & action"]);
        let text = result.input.text.unwrap();
        assert_eq!(text, "Élan & action\nRead this and that.\nEnd diagram\n");
        let paragraphs = result.context.paragraphs.unwrap();
        assert_eq!(
            paragraphs
                .iter()
                .map(|p| (p.text.as_str(), p.line, p.column))
                .collect::<Vec<_>>(),
            vec![("Read this and that.", 2, 1), ("End diagram", 3, 1)]
        );
        assert_eq!(result.destinations, ["/first", "https://example.org"]);
    }

    #[test]
    fn maps_literal_prose_and_refuses_ambiguous_html() {
        let source = "<main>\n<p class='x'>Élan <strong>world</strong><br>fin</p></main>";
        let adapted = html_content("x", source).unwrap();
        let projected = adapted.input.text.as_deref().unwrap();
        let map = html_source_map(source, projected).unwrap();
        assert_eq!(map.source_position(source, projected, 2, 6), Some((2, 27)));
        assert_eq!(map.source_position(source, projected, 3, 1), Some((2, 45)));
        for html in [
            "<p>hello &amp; world</p>",
            "<p hidden>hidden</p><p>world</p>",
            "<p><div>world</div></p>",
            "<p>world",
            "<script>world</script>",
            "<p>world</p><!-- comment -->",
        ] {
            let adapted = html_content("x", html).unwrap();
            assert!(
                html_source_map(html, adapted.input.text.as_deref().unwrap()).is_none(),
                "{html}"
            );
        }
    }

    #[test]
    fn maps_repeated_literals_to_their_own_source_occurrences() {
        let html = format!("<p>{}</p>", "<span>same</span>".repeat(2_000));
        let adapted = html_content("x", &html).unwrap();
        let projected = adapted.input.text.as_deref().unwrap();
        let map = html_source_map(&html, projected).unwrap();
        let source_index = crate::text_index::LineIndex::new(&html);
        for (index, (offset, _)) in html.match_indices("same").enumerate() {
            assert_eq!(
                map.source_position(&html, projected, 1, index * 4 + 1),
                source_index.position(&html, offset)
            );
        }
        assert!(map.source_position(&html, projected, 1, 8_001).is_none());
    }

    #[test]
    fn separates_html_blocks_without_splitting_inline_words() {
        let result = html_content("x", "<div>one<b>two</b></div><div>three</div>").unwrap();
        assert_eq!(
            result
                .input
                .text
                .as_deref()
                .unwrap()
                .split_whitespace()
                .collect::<Vec<_>>(),
            ["onetwo", "three"]
        );
    }

    #[test]
    fn rejects_node_and_byte_overflow_without_partial_projection() {
        let many = "<br>".repeat(MAX_HTML_CONTENT_NODES + 1);
        assert!(html_content("x", &many).is_err());
        assert!(html_content("x", &"x".repeat(MAX_CONTENT_INPUT_BYTES + 1)).is_err());
    }
}

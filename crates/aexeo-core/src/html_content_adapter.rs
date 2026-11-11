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

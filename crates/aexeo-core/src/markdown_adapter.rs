//! CommonMark adapter. Locations in reports refer to the extracted text.
use crate::content_gate::{ContentContext, ContentInput, ContentParagraph};
use anyhow::{Result, anyhow};
use markdown::{ParseOptions, mdast::Node};

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
    let mut options = ParseOptions::gfm();
    options.constructs.frontmatter = true;
    let root = markdown::to_mdast(source, &options)
        .map_err(|err| anyhow!("Markdown parse error: {err}"))?;
    let mut text = String::new();
    let mut headings = Vec::new();
    let mut ranges = Vec::new();
    let mut link_count = 0;
    fn visit(
        node: &Node,
        text: &mut String,
        headings: &mut Vec<String>,
        ranges: &mut Vec<(usize, usize)>,
        link_count: &mut usize,
    ) {
        match node {
            Node::Text(item) => text.push_str(&item.value),
            Node::Image(item) => text.push_str(&item.alt),
            Node::ImageReference(item) => text.push_str(&item.alt),
            Node::Break(_) => text.push('\n'),
            Node::Heading(_) => {
                let start = text.len();
                if let Some(children) = node.children() {
                    for child in children {
                        visit(child, text, headings, ranges, link_count);
                    }
                }
                headings.push(text[start..].to_owned());
                text.push('\n');
            }
            Node::Paragraph(_) | Node::TableCell(_) => {
                let start = text.len();
                if let Some(children) = node.children() {
                    for child in children {
                        visit(child, text, headings, ranges, link_count);
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
                        visit(child, text, headings, ranges, link_count);
                    }
                }
            }
        }
    }
    visit(
        &root,
        &mut text,
        &mut headings,
        &mut ranges,
        &mut link_count,
    );
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
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
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

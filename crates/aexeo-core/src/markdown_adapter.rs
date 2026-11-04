//! CommonMark adapter. Locations in reports refer to the extracted text.
use crate::content_gate::ContentInput;
use anyhow::{Result, anyhow};
use markdown::{ParseOptions, mdast::Node};

/// Extract prose and headings from Markdown, excluding code, HTML, and front matter.
pub fn markdown_input(id: impl Into<String>, source: &str) -> Result<ContentInput> {
    let mut options = ParseOptions::gfm();
    options.constructs.frontmatter = true;
    let root = markdown::to_mdast(source, &options)
        .map_err(|err| anyhow!("Markdown parse error: {err}"))?;
    let mut text = String::new();
    let mut headings = Vec::new();
    fn visit(node: &Node, text: &mut String, headings: &mut Vec<String>) {
        match node {
            Node::Text(item) => text.push_str(&item.value),
            Node::Image(item) => text.push_str(&item.alt),
            Node::ImageReference(item) => text.push_str(&item.alt),
            Node::Break(_) => text.push('\n'),
            Node::Heading(_) => {
                let start = text.len();
                if let Some(children) = node.children() {
                    for child in children {
                        visit(child, text, headings);
                    }
                }
                headings.push(text[start..].to_owned());
                text.push('\n');
            }
            Node::Paragraph(_) | Node::TableCell(_) => {
                if let Some(children) = node.children() {
                    for child in children {
                        visit(child, text, headings);
                    }
                }
                text.push('\n');
            }
            _ => {
                if let Some(children) = node.children() {
                    for child in children {
                        visit(child, text, headings);
                    }
                }
            }
        }
    }
    visit(&root, &mut text, &mut headings);
    Ok(ContentInput {
        id: id.into(),
        text: Some(text),
        headings: Some(headings),
        metadata: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
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

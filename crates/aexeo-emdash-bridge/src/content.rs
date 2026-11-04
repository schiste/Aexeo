//! Adapt EmDash portable text to the same content gates used by the CLI.
use crate::{EmdashDocument, PortableTextChild};
use aexeo_core::content_gate::{ContentInput, ContentPreset, ContentReport};
use anyhow::Result;
use std::collections::BTreeMap;

/// Positions in findings refer to this adapter's projected plain text: spans
/// are concatenated within each block, and blocks are separated by newlines.
pub fn content_input(document: &EmdashDocument) -> ContentInput {
    let text = document
        .body
        .iter()
        .map(|block| {
            block
                .children
                .iter()
                .map(|child| match child {
                    PortableTextChild::Span(span) => span.text.as_str(),
                })
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    let mut metadata: BTreeMap<String, String> = document.meta.clone();
    metadata.insert("title".into(), document.title.clone());
    if let Some(value) = &document.description {
        metadata.insert("description".into(), value.clone());
    }
    if let Some(value) = &document.canonical {
        metadata.insert("canonical".into(), value.clone());
    }
    if let Some(value) = &document.lang {
        metadata.insert("lang".into(), value.clone());
    }
    let headings = document
        .body
        .iter()
        .filter(|block| {
            matches!(
                block.style,
                crate::BlockStyle::H1
                    | crate::BlockStyle::H2
                    | crate::BlockStyle::H3
                    | crate::BlockStyle::H4
                    | crate::BlockStyle::H5
                    | crate::BlockStyle::H6
            )
        })
        .map(|block| {
            block
                .children
                .iter()
                .map(|child| match child {
                    PortableTextChild::Span(span) => span.text.as_str(),
                })
                .collect::<String>()
        })
        .collect();
    ContentInput {
        id: document.route.clone(),
        text: Some(text),
        source_text: None,
        headings: Some(headings),
        metadata: Some(metadata),
    }
}

pub fn evaluate_content(
    document: &EmdashDocument,
    preset: &ContentPreset,
) -> Result<ContentReport> {
    preset.evaluate(&content_input(document))
}

#[cfg(test)]
mod tests {
    use super::*;
    use aexeo_core::{
        content_gate::GateStatus,
        content_presets::{builtin_content_presets, resolve_content_preset},
    };
    #[test]
    fn portable_text_and_plain_text_share_gate_results() {
        let document: EmdashDocument = serde_json::from_str(r#"{"route":"/test","title":"Test","body":[{"children":[{"_type":"span","text":"One "},{"_type":"span","text":"two"}]}]}"#).unwrap();
        let preset = resolve_content_preset("article", &builtin_content_presets()).unwrap();
        let projected = content_input(&document);
        assert_eq!(projected.text.as_deref(), Some("One two"));
        let emdash = evaluate_content(&document, &preset).unwrap();
        let plain = preset
            .evaluate(&ContentInput {
                id: "test.md".into(),
                text: Some("One two".into()),
                source_text: None,
                headings: None,
                metadata: None,
            })
            .unwrap();
        assert_eq!(emdash.gates[0].status, GateStatus::Passed);
        assert_eq!(emdash.gates[0].status, plain.gates[0].status);
    }
    #[test]
    fn markdown_and_emdash_share_heading_gate() {
        let document: EmdashDocument = serde_json::from_str(r#"{"route":"/test","title":"Test","body":[{"style":"h2","children":[{"_type":"span","text":"Summary"}]},{"children":[{"_type":"span","text":"A definition."}]}]}"#).unwrap();
        let preset = aexeo_core::content_gate::ContentPreset::from_toml(
            r#"
version = 1
name = "headings"
[[gates]]
id = "required"
kind = "required_headings"
headings = ["Summary", "Examples"]
"#,
        )
        .unwrap();
        let markdown = aexeo_core::markdown_adapter::markdown_input(
            "article.md",
            "## Summary\n\nA definition.",
        )
        .unwrap();
        let from_markdown = preset.evaluate(&markdown).unwrap();
        let from_emdash = evaluate_content(&document, &preset).unwrap();
        assert_eq!(from_markdown.gates[0].status, from_emdash.gates[0].status);
        assert_eq!(
            from_markdown.gates[0].findings[0].message,
            from_emdash.gates[0].findings[0].message
        );
    }
    #[test]
    fn unknown_custom_extension_is_not_a_pass() {
        let document: EmdashDocument =
            serde_json::from_str(r#"{"route":"/test","title":"Test","body":[]}"#).unwrap();
        let preset = aexeo_core::content_gate::ContentPreset::from_toml(
            "version=1\nname='custom'\n[[gates]]\nid='custom'\nkind='custom'\nextension='host.custom'\n"
        ).unwrap();
        let report = evaluate_content(&document, &preset).unwrap();
        assert_eq!(report.gates[0].status, GateStatus::NotChecked);
    }
}

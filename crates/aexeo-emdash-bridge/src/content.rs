//! Adapt EmDash portable text to the same content gates used by the CLI.
use crate::{EmdashDocument, PortableTextChild};
use aexeo_core::content_gate::{
    ContentContext, ContentInput, ContentParagraph, ContentPreset, ContentReport,
};
use anyhow::Result;
use std::collections::BTreeMap;

/// Positions in findings refer to this adapter's projected plain text: spans
/// are concatenated within each block, and blocks are separated by newlines.
pub fn content_input(document: &EmdashDocument) -> ContentInput {
    content_input_with_context(document).0
}

fn content_input_with_context(document: &EmdashDocument) -> (ContentInput, ContentContext, usize) {
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
    let mut line = 1;
    let mut paragraphs = Vec::new();
    let mut link_count = 0;
    for block in &document.body {
        let link_keys: std::collections::BTreeSet<_> = block
            .mark_defs
            .iter()
            .filter(|def| def.mark_type == "link")
            .map(|def| def.key.as_str())
            .collect();
        let mut previous = std::collections::BTreeSet::new();
        for child in &block.children {
            let PortableTextChild::Span(span) = child;
            let current: std::collections::BTreeSet<_> = span
                .marks
                .iter()
                .map(String::as_str)
                .filter(|key| link_keys.contains(key))
                .collect();
            link_count += current.difference(&previous).count();
            previous = current;
        }
        let block_text = block
            .children
            .iter()
            .map(|child| match child {
                PortableTextChild::Span(span) => span.text.as_str(),
            })
            .collect::<String>();
        if matches!(block.style, crate::BlockStyle::Normal) {
            paragraphs.push(ContentParagraph {
                text: block_text.clone(),
                line,
                column: 1,
            });
        }
        line += block_text.chars().filter(|ch| *ch == '\n').count() + 1;
    }
    (
        ContentInput {
            id: document.route.clone(),
            text: Some(text),
            source_text: None,
            headings: Some(headings),
            metadata: Some(metadata),
            evidence_sources: None,
        },
        ContentContext {
            paragraphs: Some(paragraphs),
        },
        link_count,
    )
}

pub fn evaluate_content(
    document: &EmdashDocument,
    preset: &ContentPreset,
) -> Result<ContentReport> {
    evaluate_content_with_evidence(document, preset, None)
}

/// Evaluate with caller-supplied source snapshots; no source is fetched here.
pub fn evaluate_content_with_evidence(
    document: &EmdashDocument,
    preset: &ContentPreset,
    evidence_sources: Option<BTreeMap<String, String>>,
) -> anyhow::Result<ContentReport> {
    let (mut input, context, link_count) = content_input_with_context(document);
    input.evidence_sources = evidence_sources;
    preset.evaluate_with_links(&input, None, None, Some(&context), Some(link_count))
}

#[cfg(test)]
mod tests {
    use super::*;
    use aexeo_core::{
        content_gate::GateStatus,
        content_presets::{builtin_content_presets, resolve_content_preset},
    };
    #[test]
    fn link_count_coalesces_adjacent_spans_and_ignores_unused_marks() {
        let document: EmdashDocument = serde_json::from_str(r#"{"route":"/test","title":"Test","body":[{"markDefs":[{"_key":"a","_type":"link","href":"https://example.org"},{"_key":"unused","_type":"link","href":"https://example.net"}],"children":[{"_type":"span","text":"One ","marks":["a"]},{"_type":"span","text":"link","marks":["a","strong"]},{"_type":"span","text":" then "},{"_type":"span","text":"again","marks":["a"]}]}]}"#).unwrap();
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
        let result = evaluate_content(&document, &preset).unwrap();
        assert_eq!(result.gates[0].status, GateStatus::Passed);
        assert_eq!(result.gates[0].measurements["link_count"], 2);
    }
    #[test]
    fn paragraphs_have_projected_positions_and_exclude_headings() {
        let document: EmdashDocument = serde_json::from_str(r#"{"route":"/test","title":"Test","body":[{"style":"h2","children":[{"_type":"span","text":"Heading"}]},{"children":[{"_type":"span","text":"First\nline"}]},{"children":[{"_type":"span","text":"Second"}]}]}"#).unwrap();
        let (_, context, _) = content_input_with_context(&document);
        let paragraphs = context.paragraphs.unwrap();
        assert_eq!(paragraphs.len(), 2);
        assert_eq!(paragraphs[0].line, 2);
        assert_eq!(paragraphs[1].line, 4);
        assert_eq!(paragraphs[1].text, "Second");
    }
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
                evidence_sources: None,
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
    fn evidence_anchors_require_caller_supplied_sources() {
        let document: EmdashDocument = serde_json::from_str(r#"{"route":"/test","title":"Test","body":[{"children":[{"_type":"span","text":"The count is 14."}]}]}"#).unwrap();
        let preset = ContentPreset::from_toml(
            r#"
version = 1
name = "evidence"
[[gates]]
id = "anchor"
kind = "evidence_anchors"
source = "study"
claim = "The count is 14."
quote = "14 participants"
"#,
        )
        .unwrap();
        assert_eq!(
            evaluate_content(&document, &preset).unwrap().gates[0].status,
            GateStatus::NotChecked
        );
        let sources = BTreeMap::from([("study".into(), "There were 14 participants.".into())]);
        assert_eq!(
            evaluate_content_with_evidence(&document, &preset, Some(sources))
                .unwrap()
                .gates[0]
                .status,
            GateStatus::Passed
        );
    }
    #[test]
    fn structured_fact_uses_the_same_source_data_in_emdash() {
        let document: EmdashDocument = serde_json::from_str(r#"{"route":"/test","title":"Test","body":[{"children":[{"_type":"span","text":"The study had 14 participants."}]}]}"#).unwrap();
        let preset = ContentPreset::from_toml("version=1\nname='facts'\n[[gates]]\nid='count'\nkind='structured_fact'\nsource='study'\nclaim='The study had 14 participants.'\npointer='/count'\nexpected=14\n").unwrap();
        let sources = BTreeMap::from([("study".into(), "{\"count\":12}".into())]);
        assert_eq!(
            evaluate_content_with_evidence(&document, &preset, Some(sources))
                .unwrap()
                .gates[0]
                .status,
            GateStatus::Failed
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

//! Adapt EmDash portable text to the same content gates used by the CLI.
use crate::{EmdashDocument, PortableTextChild};
use aexeo_core::content_gate::{
    ContentContext, ContentEvaluationContext, ContentGate, ContentInput, ContentParagraph,
    ContentPreset, ContentReport,
};
use anyhow::Result;
use std::collections::BTreeMap;

fn content_metadata(document: &EmdashDocument) -> BTreeMap<String, String> {
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
    metadata
}

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
    let metadata = content_metadata(document);
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

/// Location inside the exact Portable Text snapshot passed to evaluation.
#[derive(Debug, Clone, serde::Serialize)]
pub struct EmdashSourceLocation {
    pub gate_id: String,
    pub finding_index: usize,
    pub block_index: usize,
    pub span_index: usize,
    /// Zero-based UTF-16 code units, matching JavaScript string indexing.
    pub utf16_offset: usize,
}

fn positional_gate(gate: &ContentGate) -> bool {
    matches!(
        gate,
        ContentGate::ForbiddenTerms { .. }
            | ContentGate::RepeatedTerms { .. }
            | ContentGate::SentenceLength { .. }
            | ContentGate::ParagraphLength { .. }
    )
}

/// Map positional prose findings to spans. Synthetic block separators have no
/// location. Indices apply only to this exact document snapshot.
pub fn source_locations(
    document: &EmdashDocument,
    preset: &ContentPreset,
    report: &ContentReport,
) -> Vec<EmdashSourceLocation> {
    if !preset
        .gates
        .iter()
        .zip(&report.gates)
        .any(|(gate, result)| positional_gate(gate) && !result.findings.is_empty())
    {
        return Vec::new();
    }
    let mut projection = String::new();
    let mut spans = Vec::new();
    for (block_index, block) in document.body.iter().enumerate() {
        if block_index > 0 {
            projection.push('\n');
        }
        for (span_index, child) in block.children.iter().enumerate() {
            let PortableTextChild::Span(span) = child;
            let start = projection.len();
            projection.push_str(&span.text);
            if !span.text.is_empty() {
                spans.push((
                    start,
                    projection.len(),
                    block_index,
                    span_index,
                    span.text.as_str(),
                ));
            }
        }
    }
    let mut line_starts = vec![0];
    line_starts.extend(projection.match_indices('\n').map(|(i, _)| i + 1));
    let mut result = Vec::new();
    for (gate, evaluated) in preset.gates.iter().zip(&report.gates) {
        if !positional_gate(gate) {
            continue;
        }
        for (finding_index, finding) in evaluated.findings.iter().enumerate() {
            let Some(line_start) = finding
                .line
                .checked_sub(1)
                .and_then(|i| line_starts.get(i))
                .copied()
            else {
                continue;
            };
            let Some(column) = finding.column.checked_sub(1) else {
                continue;
            };
            let Some(line) = projection
                .get(line_start..)
                .and_then(|s| s.split('\n').next())
            else {
                continue;
            };
            let Some((within_line, _)) = line.char_indices().nth(column) else {
                continue;
            };
            let offset = line_start + within_line;
            let Some(index) = spans
                .partition_point(|&(start, _, _, _, _)| start <= offset)
                .checked_sub(1)
            else {
                continue;
            };
            let (start, end, block_index, span_index, text) = spans[index];
            if offset >= end {
                continue;
            }
            let Some(prefix) = text.get(..offset - start) else {
                continue;
            };
            result.push(EmdashSourceLocation {
                gate_id: evaluated.gate_id.clone(),
                finding_index,
                block_index,
                span_index,
                utf16_offset: prefix.encode_utf16().count(),
            });
        }
    }
    result
}

pub fn evaluate_content(
    document: &EmdashDocument,
    preset: &ContentPreset,
) -> Result<ContentReport> {
    evaluate_content_with_evidence(document, preset, None)
}

/// Evaluate collection-scoped gates over a caller-supplied complete collection.
/// Document and site gates without corresponding inputs remain `not_checked`.
pub fn evaluate_collection(
    documents: &[EmdashDocument],
    preset: &ContentPreset,
) -> Result<ContentReport> {
    let items: Vec<_> = documents
        .iter()
        .map(|document| ContentInput {
            id: document.route.clone(),
            metadata: Some(content_metadata(document)),
            ..Default::default()
        })
        .collect();
    let root = ContentInput {
        id: "collection".into(),
        ..Default::default()
    };
    preset.evaluate_with_capabilities(
        &root,
        &ContentEvaluationContext {
            collection: Some(&items),
            ..Default::default()
        },
    )
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
    fn findings_map_to_their_portable_text_span_and_utf16_offset() {
        let document: EmdashDocument = serde_json::from_str(r#"{"route":"/test","title":"Test","body":[{"children":[{"_type":"span","text":"Heading"}]},{"children":[{"_type":"span","text":"😀 "},{"_type":"span","text":"pré target"}]}]}"#).unwrap();
        let preset = ContentPreset::from_toml("version=1\nname='test'\n[[gates]]\nid='terms'\nkind='forbidden_terms'\nterms=['target']\n").unwrap();
        let report = evaluate_content(&document, &preset).unwrap();
        let positions = source_locations(&document, &preset, &report);
        assert_eq!(positions.len(), 1);
        assert_eq!(
            (
                positions[0].block_index,
                positions[0].span_index,
                positions[0].utf16_offset
            ),
            (1, 1, 4)
        );
    }
    #[test]
    fn source_mapping_counts_utf16_surrogate_pairs() {
        let document: EmdashDocument = serde_json::from_str(r#"{"route":"/test","title":"Test","body":[{"children":[{"_type":"span","text":"😀 target"}]}]}"#).unwrap();
        let preset = ContentPreset::from_toml("version=1\nname='test'\n[[gates]]\nid='terms'\nkind='forbidden_terms'\nterms=['target']\n").unwrap();
        let report = evaluate_content(&document, &preset).unwrap();
        assert_eq!(
            source_locations(&document, &preset, &report)[0].utf16_offset,
            3
        );
    }

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
    fn collection_adapter_checks_duplicate_document_titles() {
        let documents: Vec<EmdashDocument> = serde_json::from_str(
            r#"[{"route":"/a","title":"Same","body":[]},{"route":"/b","title":"Same","body":[]}]"#,
        )
        .unwrap();
        let preset = ContentPreset::from_toml(
            r#"
version = 1
name = "collection"
[[gates]]
id = "unique"
kind = "unique_collection_metadata"
field = "title"
"#,
        )
        .unwrap();
        let report = evaluate_collection(&documents, &preset).unwrap();
        assert_eq!(report.gates[0].status, GateStatus::Failed);
        assert_eq!(report.gates[0].measurements["collection_items"], 2);
        assert_eq!(report.gates[0].findings[0].path, "/b");
    }

    #[test]
    fn collection_gate_is_explicitly_unavailable_for_single_emdash_document() {
        let document: EmdashDocument =
            serde_json::from_str(r#"{"route":"/test","title":"Test","body":[]}"#).unwrap();
        let preset = ContentPreset::from_toml(
            r#"
version = 1
name = "collection"
[[gates]]
id = "unique"
kind = "unique_collection_metadata"
field = "slug"
"#,
        )
        .unwrap();
        let report = evaluate_content(&document, &preset).unwrap();
        assert_eq!(report.gates[0].scope, "collection");
        assert_eq!(report.gates[0].status, GateStatus::NotChecked);
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

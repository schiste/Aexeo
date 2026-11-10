//! Adapt EmDash portable text to the same content gates used by the CLI.
use crate::{EmdashDocument, PortableTextChild};
use aexeo_core::content_gate::{
    ContentContext, ContentEvaluationContext, ContentGate, ContentInput, ContentParagraph,
    ContentPreset, ContentReport, collapse_whitespace,
};
use anyhow::Result;
use std::collections::BTreeMap;

fn is_heading(style: &crate::BlockStyle) -> bool {
    matches!(
        style,
        crate::BlockStyle::H1
            | crate::BlockStyle::H2
            | crate::BlockStyle::H3
            | crate::BlockStyle::H4
            | crate::BlockStyle::H5
            | crate::BlockStyle::H6
    )
}

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
/// Each block end is reported as a structural boundary, like the Markdown adapter.
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
        .filter(|block| is_heading(&block.style))
        .map(|block| {
            collapse_whitespace(
                &block
                    .children
                    .iter()
                    .map(|child| match child {
                        PortableTextChild::Span(span) => span.text.as_str(),
                    })
                    .collect::<String>(),
            )
        })
        .collect();
    let mut line = 1;
    let mut paragraphs = Vec::new();
    let mut link_count = 0;
    let mut block_ends = Vec::with_capacity(document.body.len());
    let mut offset = 0;
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
        offset += block_text.len();
        block_ends.push(offset);
        offset += 1;
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
            block_ends: Some(block_ends),
            ..Default::default()
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
        ContentGate::Custom { .. }
            | ContentGate::ForbiddenTerms { .. }
            | ContentGate::RepeatedTerms { .. }
            | ContentGate::SentenceLength { .. }
            | ContentGate::SentenceCharacterLength { .. }
            | ContentGate::ParagraphLength { .. }
            | ContentGate::RepeatedParagraphOpenings { .. }
            | ContentGate::ParagraphCharacterLength { .. }
    )
}

/// Map prose, heading and invalid link findings to spans. Synthetic block
/// separators have no location. Heading locations identify the first non-empty
/// span of the block; link locations identify the first span of the occurrence.
/// Indices apply only to this exact document snapshot.
pub fn source_locations(
    document: &EmdashDocument,
    preset: &ContentPreset,
    report: &ContentReport,
) -> Vec<EmdashSourceLocation> {
    let needs_prose = preset
        .gates
        .iter()
        .zip(&report.gates)
        .any(|(gate, result)| positional_gate(gate) && !result.findings.is_empty());
    let needs_headings = preset
        .gates
        .iter()
        .zip(&report.gates)
        .any(|(gate, result)| {
            matches!(
                gate,
                ContentGate::HeadingLength { .. } | ContentGate::DuplicateHeadings { .. }
            ) && !result.findings.is_empty()
        });
    let needs_links = preset
        .gates
        .iter()
        .zip(&report.gates)
        .any(|(gate, result)| {
            matches!(gate, ContentGate::LinkDestinations { .. }) && !result.findings.is_empty()
        });
    if !needs_prose && !needs_headings && !needs_links {
        return Vec::new();
    }
    let mut projection = String::new();
    let mut spans = Vec::new();
    for (block_index, block) in document.body.iter().enumerate().filter(|_| needs_prose) {
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
    let line_index = aexeo_core::text_index::LineIndex::new(&projection);
    let heading_blocks: Vec<_> = document
        .body
        .iter()
        .enumerate()
        .filter(|(_, block)| needs_headings && is_heading(&block.style))
        .collect();
    let headings: Vec<String> = heading_blocks
        .iter()
        .map(|(_, block)| {
            block
                .children
                .iter()
                .map(|child| match child {
                    PortableTextChild::Span(span) => span.text.as_str(),
                })
                .collect::<String>()
        })
        .map(|heading| collapse_whitespace(&heading))
        .collect();
    let links = needs_links.then(|| content_links(document));
    let mut result = Vec::new();
    for (gate, evaluated) in preset.gates.iter().zip(&report.gates) {
        if let ContentGate::LinkDestinations {
            allowed_schemes,
            allow_relative,
            ..
        } = gate
        {
            if let Some(links) = &links {
                let invalid: Vec<_> = links
                    .iter()
                    .filter(|(destination, _, _)| {
                        !aexeo_core::content_gate::valid_link_destination(
                            destination,
                            allowed_schemes,
                            *allow_relative,
                        )
                    })
                    .collect();
                // Do not attach locations when the number of findings disagrees.
                if invalid.len() == evaluated.total_findings() {
                    for (finding_index, (_, block_index, span_index)) in invalid
                        .into_iter()
                        .take(evaluated.findings.len())
                        .enumerate()
                    {
                        result.push(EmdashSourceLocation {
                            gate_id: evaluated.gate_id.clone(),
                            finding_index,
                            block_index: *block_index,
                            span_index: *span_index,
                            utf16_offset: 0,
                        });
                    }
                }
            }
            continue;
        }
        if let Some(indices) = aexeo_core::content_gate::heading_finding_indices(gate, &headings) {
            if indices.len() == evaluated.total_findings() {
                for (finding_index, index) in indices
                    .into_iter()
                    .take(evaluated.findings.len())
                    .enumerate()
                {
                    let (block_index, block) = heading_blocks[index];
                    if let Some(span_index) = block.children.iter().position(|child| match child {
                        PortableTextChild::Span(span) => !span.text.is_empty(),
                    }) {
                        result.push(EmdashSourceLocation {
                            gate_id: evaluated.gate_id.clone(),
                            finding_index,
                            block_index,
                            span_index,
                            utf16_offset: 0,
                        });
                    }
                }
            }
            continue;
        }
        if !positional_gate(gate) {
            continue;
        }
        for (finding_index, finding) in evaluated.findings.iter().enumerate() {
            let Some(offset) = line_index.offset(&projection, finding.line, finding.column) else {
                continue;
            };
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

fn content_links(document: &EmdashDocument) -> Vec<(&str, usize, usize)> {
    let mut links = Vec::new();
    for (block_index, block) in document.body.iter().enumerate() {
        let definitions: BTreeMap<_, _> = block
            .mark_defs
            .iter()
            .filter(|definition| definition.mark_type == "link")
            .map(|definition| (definition.key.as_str(), definition))
            .collect();
        let mut previous = std::collections::BTreeSet::new();
        for (span_index, child) in block.children.iter().enumerate() {
            let PortableTextChild::Span(span) = child;
            let current: std::collections::BTreeSet<_> = span
                .marks
                .iter()
                .map(String::as_str)
                .filter(|key| definitions.contains_key(key))
                .collect();
            for key in current.difference(&previous) {
                links.push((
                    definitions[key]
                        .data
                        .get("href")
                        .and_then(|value| value.as_str())
                        .unwrap_or(""),
                    block_index,
                    span_index,
                ));
            }
            previous = current;
        }
    }
    links
}

fn content_link_destinations(document: &EmdashDocument) -> Vec<String> {
    content_links(document)
        .into_iter()
        .map(|(destination, _, _)| destination.to_owned())
        .collect()
}

/// Evaluate with caller-supplied source snapshots; no source is fetched here.
pub fn evaluate_content_with_evidence(
    document: &EmdashDocument,
    preset: &ContentPreset,
    evidence_sources: Option<BTreeMap<String, String>>,
) -> anyhow::Result<ContentReport> {
    evaluate_content_with_body_availability(document, preset, evidence_sources, true)
}

/// Evaluate an EmDash snapshot while explicitly indicating whether its body was
/// supplied. An unavailable body is distinct from an explicitly empty body.
pub fn evaluate_content_with_body_availability(
    document: &EmdashDocument,
    preset: &ContentPreset,
    evidence_sources: Option<BTreeMap<String, String>>,
    body_available: bool,
) -> anyhow::Result<ContentReport> {
    evaluate_content_with_body_capabilities(document, preset, evidence_sources, [body_available; 4])
}

/// Return the exact input visible to extension hosts for this snapshot.
pub fn extension_input(
    document: &EmdashDocument,
    evidence_sources: Option<BTreeMap<String, String>>,
    capabilities: [bool; 4],
) -> ContentInput {
    let mut input = content_input(document);
    input.evidence_sources = evidence_sources;
    if !capabilities[0] {
        input.text = None;
    }
    if !capabilities[1] {
        input.headings = None;
    }
    input
}

/// Capabilities in order: text, headings, paragraphs, links. Partial adapters
/// must not imply that a missing structure is an empty structure.
pub fn evaluate_content_with_body_capabilities(
    document: &EmdashDocument,
    preset: &ContentPreset,
    evidence_sources: Option<BTreeMap<String, String>>,
    capabilities: [bool; 4],
) -> anyhow::Result<ContentReport> {
    evaluate_content_with_extensions(document, preset, evidence_sources, capabilities, None)
}

/// Evaluate a document with an explicitly trusted extension host.
pub fn evaluate_content_with_extensions(
    document: &EmdashDocument,
    preset: &ContentPreset,
    evidence_sources: Option<BTreeMap<String, String>>,
    capabilities: [bool; 4],
    extensions: Option<&dyn aexeo_core::content_gate::ContentGateExtensions>,
) -> anyhow::Result<ContentReport> {
    let [text, headings, paragraphs, links] = capabilities;
    let (mut input, mut context, link_count) = content_input_with_context(document);
    input.evidence_sources = evidence_sources;
    if !text {
        input.text = None;
    }
    if !headings {
        input.headings = None;
    }
    if !paragraphs {
        context.paragraphs = None;
        context.block_ends = None;
    }
    let destinations = (links
        && preset
            .gates
            .iter()
            .any(|gate| matches!(gate, ContentGate::LinkDestinations { .. })))
    .then(|| content_link_destinations(document));
    preset.evaluate_with_destinations(
        &input,
        &ContentEvaluationContext {
            adapter: Some(&context),
            link_count: links.then_some(link_count),
            extensions,
            ..Default::default()
        },
        destinations.as_deref(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use aexeo_core::{
        content_gate::{ContentEvaluationContext, GateStatus},
        content_presets::{builtin_content_presets, resolve_content_preset},
        markdown_adapter::markdown_input_with_links,
    };

    #[test]
    fn markdown_and_portable_text_agree_on_blocks_code_and_wrapped_terms() {
        let source = "## Option `a`\n\n- one two three four\n- five six seven eight\n\nThe best\nprice `wins`\n";
        let document: EmdashDocument = serde_json::from_str(r#"{"route":"/t","title":"T","body":[
            {"style":"h2","children":[{"_type":"span","text":"Option "},{"_type":"span","text":"a","marks":["code"]}]},
            {"listItem":"bullet","children":[{"_type":"span","text":"one two three four"}]},
            {"listItem":"bullet","children":[{"_type":"span","text":"five six seven eight"}]},
            {"children":[{"_type":"span","text":"The best\nprice "},{"_type":"span","text":"wins","marks":["code"]}]}
        ]}"#).unwrap();
        let preset = ContentPreset::from_toml("version=1\nname='t'\n[[gates]]\nid='s'\nkind='sentence_length'\nmax_words=6\n[[gates]]\nid='h'\nkind='required_headings'\nheadings=['Option a']\n[[gates]]\nid='t'\nkind='forbidden_terms'\nterms=['best price','four five']\n[[gates]]\nid='w'\nkind='word_count'\nmin=1\n").unwrap();
        let (input, context, links) = markdown_input_with_links("t.md", source).unwrap();
        let markdown = preset
            .evaluate_with_capabilities(
                &input,
                &ContentEvaluationContext {
                    adapter: Some(&context),
                    link_count: Some(links),
                    ..Default::default()
                },
            )
            .unwrap();
        let emdash = evaluate_content(&document, &preset).unwrap();
        for (left, right) in markdown.gates.iter().zip(&emdash.gates) {
            assert_eq!(left.status, right.status, "{}", left.gate_id);
            assert_eq!(left.measurements, right.measurements, "{}", left.gate_id);
            let positions = |gate: &aexeo_core::content_gate::ContentGateResult| {
                gate.findings
                    .iter()
                    .map(|finding| (finding.message.clone(), finding.line, finding.column))
                    .collect::<Vec<_>>()
            };
            assert_eq!(positions(left), positions(right), "{}", left.gate_id);
        }
        assert_eq!(emdash.gates[0].status, GateStatus::Passed);
        assert_eq!(emdash.gates[0].measurements["sentence_count"], 4);
        assert_eq!(emdash.gates[2].findings.len(), 1);
    }
    #[test]
    fn repeated_paragraph_opening_maps_to_emdash_block() {
        let document: EmdashDocument = serde_json::from_str(r#"{"route":"/t","title":"T","body":[{"children":[{"_type":"span","text":"The team reviews."}]},{"children":[{"_type":"span","text":"The team checks."}]},{"children":[{"_type":"span","text":"The team restores."}]}]}"#).unwrap();
        let preset = ContentPreset::from_toml("version=1\nname='review'\n[[gates]]\nid='opening'\nkind='repeated_paragraph_openings'\nopening_words=2\nmax_occurrences=2\n").unwrap();
        let report = evaluate_content(&document, &preset).unwrap();
        assert_eq!(report.gates[0].status, GateStatus::Failed);
        let locations = source_locations(&document, &preset, &report);
        assert_eq!(locations.len(), 1);
        assert_eq!(
            (
                locations[0].block_index,
                locations[0].span_index,
                locations[0].utf16_offset
            ),
            (2, 0, 0)
        );
        let unavailable =
            evaluate_content_with_body_availability(&document, &preset, None, false).unwrap();
        assert_eq!(unavailable.gates[0].status, GateStatus::NotChecked);
    }

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
    fn heading_findings_map_to_their_emdash_blocks() {
        let document: EmdashDocument = serde_json::from_str(r#"{"route":"/test","title":"Test","body":[{"children":[{"_type":"span","text":"prose"}]},{"style":"h2","children":[{"_type":"span","text":"é😀abc"}]},{"style":"h3","children":[{"_type":"span","text":""},{"_type":"span","text":"É😀abc"}]}]}"#).unwrap();
        let preset = ContentPreset::from_toml("version=1\nname='test'\n[[gates]]\nid='length'\nkind='heading_length'\nmax_chars=4\n[[gates]]\nid='duplicate'\nkind='duplicate_headings'\ncase_sensitive=false\n").unwrap();
        let report = evaluate_content(&document, &preset).unwrap();
        let locations = source_locations(&document, &preset, &report);
        let actual: Vec<_> = locations
            .iter()
            .map(|l| {
                (
                    l.gate_id.as_str(),
                    l.finding_index,
                    l.block_index,
                    l.span_index,
                    l.utf16_offset,
                )
            })
            .collect();
        assert_eq!(
            actual,
            vec![
                ("length", 0, 1, 0, 0),
                ("length", 1, 2, 1, 0),
                ("duplicate", 0, 2, 1, 0)
            ]
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
    fn unavailable_body_is_not_an_empty_body() {
        let document: EmdashDocument =
            serde_json::from_str(r#"{"route":"/test","title":"Test"}"#).unwrap();
        let preset = ContentPreset::from_toml("version=1\nname='body'\n[[gates]]\nid='words'\nkind='word_count'\nmin=1\n[[gates]]\nid='links'\nkind='link_destinations'\nallowed_schemes=['https']\n[[gates]]\nid='headings'\nkind='heading_length'\nmax_chars=20\n[[gates]]\nid='paragraphs'\nkind='paragraph_length'\nmax_words=20\n").unwrap();
        let missing =
            evaluate_content_with_body_availability(&document, &preset, None, false).unwrap();
        assert!(
            missing
                .gates
                .iter()
                .all(|gate| gate.status == GateStatus::NotChecked)
        );
        let partial = evaluate_content_with_body_capabilities(
            &document,
            &preset,
            None,
            [true, false, false, false],
        )
        .unwrap();
        assert_eq!(partial.gates[0].status, GateStatus::Failed);
        assert!(
            partial.gates[1..]
                .iter()
                .all(|gate| gate.status == GateStatus::NotChecked)
        );
        let empty =
            evaluate_content_with_body_availability(&document, &preset, None, true).unwrap();
        assert_eq!(empty.gates[0].status, GateStatus::Failed);
        assert!(
            empty.gates[1..]
                .iter()
                .all(|gate| gate.status == GateStatus::Passed)
        );
    }

    #[test]
    fn destination_gate_checks_only_used_emdash_link_definitions() {
        let document: EmdashDocument = serde_json::from_str(r#"{"route":"/test","title":"Test","body":[{"markDefs":[{"_key":"a","_type":"link","href":"javascript:alert(1)"},{"_key":"unused","_type":"link","href":"https://example.org"}],"children":[{"_type":"span","text":"link","marks":["a"]}]}]}"#).unwrap();
        let preset = ContentPreset::from_toml("version=1\nname='links'\n[[gates]]\nid='links'\nkind='link_destinations'\nallowed_schemes=['https']\n").unwrap();
        let report = evaluate_content(&document, &preset).unwrap();
        assert_eq!(report.gates[0].status, GateStatus::Failed);
        assert_eq!(report.gates[0].measurements["checked_destinations"], 1);
    }

    #[test]
    fn destination_checks_count_each_link_occurrence_like_markdown() {
        let document: EmdashDocument = serde_json::from_str(r#"{"route":"/test","title":"Test","body":[{"markDefs":[{"_key":"a","_type":"link","href":"javascript:bad"},{"_key":"unused","_type":"link","href":"javascript:unused"}],"children":[{"_type":"span","text":"one ","marks":["a"]},{"_type":"span","text":"link","marks":["a","strong"]},{"_type":"span","text":" then "},{"_type":"span","text":"again","marks":["a"]}]}]}"#).unwrap();
        let preset = ContentPreset::from_toml("version=1\nname='links'\n[[gates]]\nid='links'\nkind='link_destinations'\nallowed_schemes=['https']\n").unwrap();
        let emdash = evaluate_content(&document, &preset).unwrap();
        let markdown = "[one link](javascript:bad) then [again](javascript:bad)";
        let destinations = aexeo_core::markdown_adapter::markdown_link_destinations(markdown)
            .unwrap()
            .unwrap();
        assert_eq!(content_link_destinations(&document), destinations);
        assert_eq!(emdash.gates[0].measurements["checked_destinations"], 2);
        assert_eq!(emdash.gates[0].measurements["invalid_destinations"], 2);
        assert_eq!(emdash.gates[0].total_findings(), 2);
    }

    #[test]
    fn invalid_link_locations_follow_occurrences_and_gate_policy() {
        let document: EmdashDocument = serde_json::from_str(r#"{"route":"/test","title":"Test","body":[{"markDefs":[{"_key":"bad","_type":"link","href":"javascript:bad"},{"_key":"good","_type":"link","href":"https://example.org"}],"children":[{"_type":"span","text":"😀 ","marks":["good"]},{"_type":"span","text":"one","marks":["bad"]},{"_type":"span","text":" link","marks":["bad","strong"]},{"_type":"span","text":" and "},{"_type":"span","text":"again","marks":["bad"]}]},{"markDefs":[{"_key":"bad","_type":"link","href":"javascript:bad"}],"children":[{"_type":"span","text":"another","marks":["bad"]}]}]}"#).unwrap();
        let preset = ContentPreset::from_toml("version=1\nname='links'\n[[gates]]\nid='https'\nkind='link_destinations'\nallowed_schemes=['https']\n[[gates]]\nid='javascript'\nkind='link_destinations'\nallowed_schemes=['javascript']\n").unwrap();
        let report = evaluate_content(&document, &preset).unwrap();
        let locations = source_locations(&document, &preset, &report);
        let positions: Vec<_> = locations
            .iter()
            .map(|location| {
                (
                    location.gate_id.as_str(),
                    location.finding_index,
                    location.block_index,
                    location.span_index,
                    location.utf16_offset,
                )
            })
            .collect();
        assert_eq!(
            positions,
            vec![
                ("https", 0, 0, 1, 0),
                ("https", 1, 0, 4, 0),
                ("https", 2, 1, 0, 0),
                ("javascript", 0, 0, 0, 0)
            ]
        );
    }

    #[test]
    fn invalid_link_locations_respect_finding_cap() {
        let mut children = Vec::new();
        for _ in 0..1001 {
            children.push(serde_json::json!({"_type":"span", "text":"bad", "marks":["bad"]}));
            children.push(serde_json::json!({"_type":"span", "text":" "}));
        }
        let document: EmdashDocument = serde_json::from_value(serde_json::json!({"route":"/test", "title":"Test", "body":[{"markDefs":[{"_key":"bad", "_type":"link", "href":"javascript:bad"}], "children":children}]})).unwrap();
        let preset = ContentPreset::from_toml("version=1\nname='links'\n[[gates]]\nid='links'\nkind='link_destinations'\nallowed_schemes=['https']\n").unwrap();
        let report = evaluate_content(&document, &preset).unwrap();
        assert_eq!(report.gates[0].total_findings(), 1001);
        let locations = source_locations(&document, &preset, &report);
        assert_eq!(locations.len(), report.gates[0].findings.len());
        assert_eq!(locations.last().unwrap().span_index, 1998);
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
    fn markdown_and_emdash_share_duplicate_heading_gate() {
        let document: EmdashDocument = serde_json::from_str(r#"{"route":"/test","title":"Test","body":[{"style":"h2","children":[{"_type":"span","text":"Summary"}]},{"style":"h3","children":[{"_type":"span","text":"summary"}]}]}"#).unwrap();
        let preset = ContentPreset::from_toml("version=1\nname='test'\n[[gates]]\nid='unique'\nkind='duplicate_headings'\ncase_sensitive=false\n").unwrap();
        let markdown =
            aexeo_core::markdown_adapter::markdown_input("test.md", "## Summary\n\n### summary")
                .unwrap();
        let from_markdown = preset.evaluate(&markdown).unwrap();
        let from_emdash = evaluate_content(&document, &preset).unwrap();
        assert_eq!(from_markdown.gates[0].status, GateStatus::Failed);
        assert_eq!(from_markdown.gates[0].status, from_emdash.gates[0].status);
        assert_eq!(
            from_markdown.gates[0].measurements,
            from_emdash.gates[0].measurements
        );
    }

    #[test]
    fn markdown_and_emdash_share_sentence_character_limit() {
        let document: EmdashDocument = serde_json::from_str(r#"{"route":"/test","title":"Test","body":[{"children":[{"_type":"span","text":"中文文章測試。 好文。"}]}]}"#).unwrap();
        let preset = ContentPreset::from_toml("version=1\nname='test'\n[[gates]]\nid='length'\nkind='sentence_character_length'\nmax_chars=4\ndelimiters='。'\n").unwrap();
        let (input, context, links) =
            markdown_input_with_links("t.md", "中文文章測試。 好文。").unwrap();
        let markdown = preset
            .evaluate_with_capabilities(
                &input,
                &ContentEvaluationContext {
                    adapter: Some(&context),
                    link_count: Some(links),
                    ..Default::default()
                },
            )
            .unwrap();
        let emdash = evaluate_content(&document, &preset).unwrap();
        assert_eq!(markdown.gates[0].status, GateStatus::Failed);
        assert_eq!(markdown.gates[0].status, emdash.gates[0].status);
        assert_eq!(markdown.gates[0].measurements, emdash.gates[0].measurements);
    }

    #[test]
    fn markdown_and_emdash_share_paragraph_character_limit() {
        let document: EmdashDocument = serde_json::from_str(r#"{"route":"/test","title":"Test","body":[{"children":[{"_type":"span","text":"中文文章測試"}]}]}"#).unwrap();
        let preset = ContentPreset::from_toml("version=1\nname='test'\n[[gates]]\nid='length'\nkind='paragraph_character_length'\nmax_chars=4\n").unwrap();
        let (input, context, links) = markdown_input_with_links("t.md", "中文文章測試").unwrap();
        let markdown = preset
            .evaluate_with_capabilities(
                &input,
                &ContentEvaluationContext {
                    adapter: Some(&context),
                    link_count: Some(links),
                    ..Default::default()
                },
            )
            .unwrap();
        let emdash = evaluate_content(&document, &preset).unwrap();
        assert_eq!(markdown.gates[0].status, GateStatus::Failed);
        assert_eq!(markdown.gates[0].status, emdash.gates[0].status);
        assert_eq!(markdown.gates[0].measurements, emdash.gates[0].measurements);
    }

    #[test]
    fn markdown_and_emdash_share_heading_length_gate() {
        let document: EmdashDocument = serde_json::from_str(r#"{"route":"/test","title":"Test","body":[{"style":"h2","children":[{"_type":"span","text":"é😀abc"}]}]}"#).unwrap();
        let preset = ContentPreset::from_toml(
            "version=1\nname='test'\n[[gates]]\nid='length'\nkind='heading_length'\nmax_chars=4\n",
        )
        .unwrap();
        let markdown =
            aexeo_core::markdown_adapter::markdown_input("test.md", "## é😀abc").unwrap();
        let from_markdown = preset.evaluate(&markdown).unwrap();
        let from_emdash = evaluate_content(&document, &preset).unwrap();
        assert_eq!(from_markdown.gates[0].status, GateStatus::Failed);
        assert_eq!(from_markdown.gates[0].status, from_emdash.gates[0].status);
        assert_eq!(
            from_markdown.gates[0].measurements,
            from_emdash.gates[0].measurements
        );
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

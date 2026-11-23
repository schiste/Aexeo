//! Adapt EmDash portable text to the same content gates used by the CLI.
use crate::{EmdashDocument, PortableTextChild};
use aexeo_core::content_gate::{
    ContentContext, ContentEvaluationContext, ContentGate, ContentInput, ContentParagraph,
    ContentPreset, ContentReport, collapse_whitespace,
};
use anyhow::Result;
use std::collections::BTreeMap;

fn heading_level(style: &crate::BlockStyle) -> Option<u8> {
    match style {
        crate::BlockStyle::H1 => Some(1),
        crate::BlockStyle::H2 => Some(2),
        crate::BlockStyle::H3 => Some(3),
        crate::BlockStyle::H4 => Some(4),
        crate::BlockStyle::H5 => Some(5),
        crate::BlockStyle::H6 => Some(6),
        _ => None,
    }
}

fn reference_key<'a>(
    span: &'a crate::PortableTextSpan,
    link_keys: &std::collections::BTreeSet<&str>,
    citation_keys: &std::collections::BTreeSet<&str>,
) -> Option<&'a str> {
    let mut links = span
        .marks
        .iter()
        .filter(|mark| link_keys.contains(mark.as_str()));
    let first = links.next()?.as_str();
    (links.next().is_none() && citation_keys.contains(first)).then_some(first)
}

fn is_heading(style: &crate::BlockStyle) -> bool {
    heading_level(style).is_some()
}

fn content_metadata(document: &EmdashDocument) -> BTreeMap<String, String> {
    let mut metadata: BTreeMap<String, String> = document.meta.clone();
    metadata.insert("title".into(), document.title.clone());
    if document.description_source_observed == Some(false) {
        metadata.remove("description");
    } else if let Some(value) = &document.description {
        metadata.insert("description".into(), value.clone());
    }
    if let Some(value) = &document.canonical {
        metadata.insert("canonical".into(), value.clone());
    }
    if let Some(value) = &document.lang {
        metadata.insert("lang".into(), value.clone());
    }
    for field in &document.unavailable_metadata_fields {
        metadata.remove(field);
    }
    metadata
}

/// Positions in findings refer to this adapter's projected plain text: spans
/// are concatenated within each block, and blocks are separated by newlines.
/// Each block end is reported as a structural boundary, like the Markdown adapter.
pub fn content_input(document: &EmdashDocument) -> ContentInput {
    content_input_with_context(document).0
}

fn content_input_with_context(
    document: &EmdashDocument,
) -> (ContentInput, ContentContext, usize, Option<String>) {
    let mut text = String::new();
    let mut literal_projection = aexeo_core::content_gate::ReferenceLiteralProjection::default();
    let metadata = content_metadata(document);
    let mut headings = Vec::new();
    let mut line = 1;
    let mut paragraphs = Vec::new();
    let mut link_count = 0;
    let mut block_ends = Vec::with_capacity(document.body.len());
    let mut offset = 0;
    for (index, block) in document.body.iter().enumerate() {
        if index > 0 {
            text.push('\n');
        }
        let link_keys: std::collections::BTreeSet<_> = block
            .mark_defs
            .iter()
            .filter(|def| def.mark_type == "link")
            .map(|def| def.key.as_str())
            .collect();
        // A numeric link to a reference is an annotation, not prose. Mask it
        // with whitespace of the same UTF-8 and UTF-16 width to retain offsets.
        let citation_keys: std::collections::BTreeSet<_> = block
            .mark_defs
            .iter()
            .filter(|def| {
                def.mark_type == "link"
                    && def
                        .data
                        .get("href")
                        .and_then(serde_json::Value::as_str)
                        .is_some_and(aexeo_core::content_gate::numeric_reference_destination)
            })
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
        let block_start = text.len();
        // Evaluate the whole contiguous link label: Portable Text may split a
        // citation across formatting spans, or split a prose label from its year.
        let mut index = 0;
        while index < block.children.len() {
            let PortableTextChild::Span(first) = &block.children[index];
            let Some(key) = first
                .marks
                .iter()
                .find(|key| citation_keys.contains(key.as_str()))
            else {
                text.push_str(&first.text);
                index += 1;
                continue;
            };
            let mut end = index;
            let mut numeric = true;
            let mut has_digit = false;
            while end < block.children.len() {
                let PortableTextChild::Span(span) = &block.children[end];
                if !span.marks.contains(key) {
                    break;
                }
                numeric &= reference_key(span, &link_keys, &citation_keys) == Some(key.as_str());
                match aexeo_core::content_gate::numeric_reference_label_fragment(&span.text) {
                    Some(digit) => has_digit |= digit,
                    None => numeric = false,
                }
                end += 1;
            }
            for child in &block.children[index..end] {
                let PortableTextChild::Span(span) = child;
                if numeric && has_digit {
                    literal_projection.push_masked(&mut text, &span.text);
                } else {
                    text.push_str(&span.text);
                }
            }
            index = end;
        }
        let block_text = text[block_start..].to_owned();
        if is_heading(&block.style) {
            // Headings are an independently available capability. Preserve
            // reference labels even when the host cannot supply body text.
            let heading = block
                .children
                .iter()
                .map(|child| match child {
                    PortableTextChild::Span(span) => span.text.as_str(),
                })
                .collect::<String>();
            headings.push(collapse_whitespace(&heading));
        }
        let next_line = line + block_text.chars().filter(|ch| *ch == '\n').count() + 1;
        offset += block_text.len();
        if matches!(block.style, crate::BlockStyle::Normal) {
            paragraphs.push(ContentParagraph {
                text: block_text,
                line,
                column: 1,
            });
        }
        line = next_line;
        block_ends.push(offset);
        offset += 1;
    }
    let literal_text = literal_projection.finish(&text);
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
            unavailable_metadata_fields: {
                let mut fields = document.unavailable_metadata_fields.clone();
                if document.description_source_observed == Some(false)
                    || (document.description_source_observed.is_none()
                        && document.description.is_none()
                        && !document.meta.contains_key("description"))
                {
                    fields.push("description".into());
                }
                fields
            },
            ..Default::default()
        },
        link_count,
        literal_text,
    )
}

// Must match the adapter-json reader's per-observation limit.
const MAX_ADAPTER_OBSERVATIONS: usize = 100_000;

fn bounded_adapter_projection(mut value: serde_json::Value) -> Result<serde_json::Value> {
    let limit = aexeo_core::content_revision_loop::MAX_DRAFT_BYTES;
    if serde_json::to_vec(&value)?.len() > limit {
        // Native coordinates are optional. Preserve a usable content projection
        // when their serialized overhead would exceed the shared input limit.
        value
            .as_object_mut()
            .and_then(|object| object.remove("native_source_map"));
        anyhow::ensure!(
            serde_json::to_vec(&value)?.len() <= limit,
            "serialized projection exceeds byte limit"
        );
    }
    Ok(value)
}

/// Project CMS metadata without treating unobserved fields as absent.
pub fn content_adapter_projection_with_metadata(
    document: &EmdashDocument,
    capabilities: [bool; 4],
    list_structure_complete: bool,
    observed: Option<&std::collections::BTreeSet<String>>,
) -> Result<serde_json::Value> {
    let mut projection =
        content_adapter_projection(document, capabilities, list_structure_complete)?;
    let mut observed = observed.cloned().unwrap_or_default();
    for unavailable in &document.unavailable_metadata_fields {
        observed.remove(unavailable);
    }
    if document.description_source_observed == Some(false) {
        observed.remove("description");
    }
    anyhow::ensure!(
        observed.len() <= MAX_ADAPTER_OBSERVATIONS,
        "metadata observation limit exceeded"
    );
    let mut metadata = content_metadata(document);
    metadata.retain(|field, _| observed.contains(field));
    projection["metadata"] = serde_json::to_value(metadata)?;
    projection["observed_metadata_fields"] = serde_json::to_value(observed)?;
    bounded_adapter_projection(projection)
}

/// Project observed body capabilities for the CLI host adapter contract.
/// Metadata is omitted by this body-only helper. Native paths identify spans
/// in the projected document, not necessarily the original CMS record.
pub fn content_adapter_projection(
    document: &EmdashDocument,
    capabilities: [bool; 4],
    list_structure_complete: bool,
) -> Result<serde_json::Value> {
    anyhow::ensure!(
        document.body.len() <= MAX_ADAPTER_OBSERVATIONS,
        "body exceeds projection limit"
    );
    // Bound the input before building the masked and literal copies. Separators
    // contribute one byte between blocks; every span is UTF-8 encoded already.
    let source_bytes = document.body.iter().try_fold(0usize, |size, block| {
        block.children.iter().try_fold(size, |size, child| {
            let PortableTextChild::Span(span) = child;
            size.checked_add(span.text.len())
        })
    });
    let source_bytes = source_bytes
        .and_then(|size| size.checked_add(document.body.len().saturating_sub(1)))
        .ok_or_else(|| anyhow::anyhow!("body exceeds projection limit"))?;
    anyhow::ensure!(
        source_bytes <= aexeo_core::content_revision_loop::MAX_DRAFT_BYTES,
        "body exceeds projection limit"
    );
    let (input, context, _, literal) = content_input_with_context(document);
    let [
        text_available,
        headings_available,
        paragraphs_available,
        links_available,
    ] = capabilities;
    let mut out = serde_json::json!({"schema":"aexeo.content-adapter/v1", "text": null});
    if headings_available {
        out["headings"] = serde_json::json!(input.headings);
        out["heading_levels"] = serde_json::json!(
            document
                .body
                .iter()
                .filter_map(|block| heading_level(&block.style))
                .collect::<Vec<_>>()
        );
    }
    if !text_available {
        return bounded_adapter_projection(out);
    }
    let text = input.text.as_deref().unwrap_or("");
    anyhow::ensure!(
        text.len() <= aexeo_core::content_revision_loop::MAX_DRAFT_BYTES,
        "text exceeds projection limit"
    );
    out["text"] = serde_json::json!(text);
    let mut native_spans = Vec::new();
    let mut projected_start = 0usize;
    let mut native_complete = true;
    for (block_index, block) in document.body.iter().enumerate() {
        if block_index > 0 {
            projected_start += 1;
        }
        for (span_index, child) in block.children.iter().enumerate() {
            let PortableTextChild::Span(span) = child;
            let end = projected_start + span.text.len();
            if !span.text.is_empty() {
                if native_spans.len() == MAX_ADAPTER_OBSERVATIONS {
                    native_complete = false;
                    break;
                }
                native_spans.push(serde_json::json!({"start":projected_start,"end":end,"path":["body",block_index,"children",span_index,"text"]}));
            }
            projected_start = end;
        }
        if !native_complete {
            break;
        }
    }
    if native_complete {
        out["native_source_map"] = serde_json::json!({"spans":native_spans});
    }
    if let Some(literal) = literal {
        out["literal_text"] = serde_json::json!(literal);
    }
    let ends = context.block_ends.as_deref().unwrap_or_default();
    out["block_ends"] = serde_json::json!(ends);
    let lists_complete = list_structure_complete && capabilities.iter().all(|value| *value);
    let classifications = portable_text_paragraph_list_items(document, lists_complete);
    if paragraphs_available {
        let mut start = 0;
        let mut paragraphs = Vec::new();
        let mut ordinal = 0;
        let mut complete = true;
        for (block, end) in document.body.iter().zip(ends) {
            if matches!(block.style, crate::BlockStyle::Normal) {
                if start == *end {
                    complete = false;
                    break;
                }
                let mut paragraph = serde_json::json!({"start":start, "end":end});
                if let Some(classification) = classifications
                    .as_ref()
                    .and_then(|items| items.get(ordinal))
                {
                    paragraph["list_item"] = serde_json::json!(classification);
                }
                paragraphs.push(paragraph);
                ordinal += 1;
            }
            start = end.saturating_add(1);
        }
        if complete {
            out["paragraphs"] = serde_json::json!(paragraphs);
        }
    }
    if let Some(lists) = portable_text_structured_lists(document, lists_complete) {
        out["lists"] = serde_json::json!(
            lists
                .iter()
                .map(|list| serde_json::json!({"item_count":list.item_count, "depth":list.depth}))
                .collect::<Vec<_>>()
        );
    }
    if links_available {
        let destinations = content_link_destinations(document);
        anyhow::ensure!(
            destinations.len() <= MAX_ADAPTER_OBSERVATIONS,
            "link observations exceed projection limit"
        );
        out["link_destinations"] = serde_json::json!(destinations);
    }
    bounded_adapter_projection(out)
}

/// Observe explicit Markdown links in complete rich text. Code-marked spans
/// are masked by byte length, preserving coordinates into the original text.
fn literal_markdown_links(document: &EmdashDocument) -> Option<Vec<(usize, usize)>> {
    if document.body.len() > 10_000 {
        return None;
    }
    let mut original = String::new();
    let mut offsets = Vec::new();
    for (index, block) in document.body.iter().enumerate() {
        if index > 0 {
            original.push('\n');
        }
        let block_start = original.len();
        let mut candidate = String::new();
        for child in &block.children {
            let PortableTextChild::Span(span) = child;
            original.push_str(&span.text);
            if span.marks.iter().any(|mark| mark == "code") {
                for ch in span.text.chars() {
                    if ch == '\n' {
                        candidate.push('\n');
                    } else {
                        candidate.extend(std::iter::repeat_n(' ', ch.len_utf8()));
                    }
                }
            } else {
                candidate.push_str(&span.text);
            }
        }
        if original.len() > aexeo_core::content_gate::MAX_CONTENT_INPUT_BYTES {
            return None;
        }
        if !candidate.contains('[') {
            continue;
        }
        for offset in
            aexeo_core::markdown_adapter::literal_markdown_link_offsets(&candidate).ok()?
        {
            offsets.push(block_start + offset);
            if offsets.len() > 100_000 {
                return None;
            }
        }
    }
    let index = aexeo_core::text_index::LineIndex::new(&original);
    offsets
        .into_iter()
        .map(|offset| index.position(&original, offset))
        .collect()
}

/// Observe complete Portable Text list structure. Returns `None` when the host
/// cannot guarantee the body includes every list item, or when nesting is
/// malformed. Positions refer to the projected plain-text snapshot.
pub fn portable_text_structured_lists(
    document: &EmdashDocument,
    complete: bool,
) -> Option<Vec<aexeo_core::markdown_adapter::StructuredList>> {
    portable_text_list_projection(document, complete).map(|(lists, _)| lists)
}

type ListProjection = (
    Vec<aexeo_core::markdown_adapter::StructuredList>,
    Vec<Option<(usize, usize)>>,
);

fn portable_text_list_projection(
    document: &EmdashDocument,
    complete: bool,
) -> Option<ListProjection> {
    use aexeo_core::markdown_adapter::StructuredList;
    if !complete || document.body.len() > 100_000 {
        return None;
    }
    let mut result: Vec<StructuredList> = Vec::new();
    let mut stack: Vec<(crate::ListItem, usize)> = Vec::new();
    let mut locations = Vec::new();
    let mut line = 1;
    for (block_index, block) in document.body.iter().enumerate() {
        if let Some(kind) = &block.list_item {
            let depth = usize::try_from(block.level.unwrap_or(1)).ok()?;
            if depth == 0 || depth > 100 || depth > stack.len() + 1 {
                return None;
            }
            stack.truncate(depth);
            if stack.len() == depth && stack[depth - 1].0 != *kind {
                stack.pop();
            }
            if stack.len() < depth {
                result.push(StructuredList {
                    item_count: 0,
                    depth,
                    line,
                    column: 1,
                });
                locations.push(
                    block
                        .children
                        .iter()
                        .position(|child| match child {
                            PortableTextChild::Span(span) => !span.text.is_empty(),
                        })
                        .map(|span_index| (block_index, span_index)),
                );
                stack.push((kind.clone(), result.len() - 1));
            }
            result[stack[depth - 1].1].item_count += 1;
        } else {
            stack.clear();
        }
        line += 1 + block
            .children
            .iter()
            .map(|child| match child {
                PortableTextChild::Span(span) => span.text.bytes().filter(|b| *b == b'\n').count(),
            })
            .sum::<usize>();
    }
    Some((result, locations))
}

/// Classify projected paragraphs by list membership, in projection order.
///
/// `complete` asserts that the host observed all original body blocks, including
/// custom components. A list item omitted from paragraph projection prevents a
/// complete observation because it could hide a break between prose paragraphs.
pub fn portable_text_paragraph_list_items(
    document: &EmdashDocument,
    complete: bool,
) -> Option<Vec<bool>> {
    portable_text_structured_lists(document, complete)?;
    if document
        .body
        .iter()
        .any(|block| block.list_item.is_some() && !matches!(block.style, crate::BlockStyle::Normal))
    {
        return None;
    }
    Some(
        document
            .body
            .iter()
            .filter_map(|block| {
                matches!(block.style, crate::BlockStyle::Normal)
                    .then_some(block.list_item.is_some())
            })
            .collect(),
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

use aexeo_core::source_mapping::prose_gate_has_positions as positional_gate;

/// Coverage of emitted findings that can have a source position.
/// Findings omitted by the report limit are not included.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SourceMappingCoverage {
    pub eligible: usize,
    pub mapped: usize,
    pub unmapped: usize,
}

pub fn source_mapping_coverage(
    preset: &ContentPreset,
    report: &ContentReport,
    locations: &[EmdashSourceLocation],
) -> SourceMappingCoverage {
    let eligible = aexeo_core::source_mapping::eligible_findings(preset, report);
    SourceMappingCoverage {
        eligible,
        mapped: locations.len(),
        unmapped: eligible.saturating_sub(locations.len()),
    }
}

// Sparse UTF-16 offsets avoid rescanning a long span for every finding. The
// projection and this index must refer to the same unchanged text.
struct Utf16Index {
    checkpoints: Vec<(usize, usize)>,
}

impl Utf16Index {
    fn new(text: &str) -> Self {
        let mut checkpoints = Vec::new();
        let mut utf16 = 0;
        for (index, (offset, character)) in text.char_indices().enumerate() {
            if index % 128 == 0 {
                checkpoints.push((offset, utf16));
            }
            utf16 += character.len_utf16();
        }
        Self { checkpoints }
    }

    fn position(&self, text: &str, offset: usize) -> Option<usize> {
        let checkpoint = self
            .checkpoints
            .partition_point(|&(start, _)| start <= offset)
            .checked_sub(1)?;
        let (start, units) = self.checkpoints[checkpoint];
        Some(units + text.get(start..offset)?.encode_utf16().count())
    }
}

/// Map prose, heading, list and invalid link findings to spans. Synthetic block
/// separators have no location. Heading locations identify the first non-empty
/// span of the block; link locations identify the first span of the occurrence.
/// Indices apply only to this exact document snapshot.
pub fn source_locations(
    document: &EmdashDocument,
    preset: &ContentPreset,
    report: &ContentReport,
) -> Vec<EmdashSourceLocation> {
    source_locations_with_reachability(document, preset, report, None)
}

pub fn source_locations_with_reachability(
    document: &EmdashDocument,
    preset: &ContentPreset,
    report: &ContentReport,
    observations: Option<&[aexeo_core::content_gate::LinkReachabilityObservation]>,
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
                ContentGate::HeadingHierarchy { .. }
                    | ContentGate::HeadingLength { .. }
                    | ContentGate::DuplicateHeadings { .. }
            ) && !result.findings.is_empty()
        });
    let needs_links = preset
        .gates
        .iter()
        .zip(&report.gates)
        .any(|(gate, result)| {
            matches!(
                gate,
                ContentGate::LinkDestinations { .. } | ContentGate::LinkReachability { .. }
            ) && !result.findings.is_empty()
        });
    let needs_lists = preset
        .gates
        .iter()
        .zip(&report.gates)
        .any(|(gate, result)| {
            matches!(gate, ContentGate::ListItemCount { .. }) && !result.findings.is_empty()
        });
    if !needs_prose && !needs_headings && !needs_links && !needs_lists {
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
    // Build sparse indices lazily only for long spans with many findings.
    let many_prose_findings = preset
        .gates
        .iter()
        .zip(&report.gates)
        .filter(|(gate, _)| positional_gate(gate))
        .flat_map(|(_, result)| result.findings.iter())
        .take(8)
        .count()
        == 8;
    let mut utf16_indices: BTreeMap<usize, Option<Utf16Index>> = BTreeMap::new();
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
    let lists = needs_lists
        .then(|| portable_text_list_projection(document, true))
        .flatten();
    let mut result = Vec::new();
    for (gate, evaluated) in preset.gates.iter().zip(&report.gates) {
        if matches!(gate, ContentGate::ListItemCount { .. }) {
            if let Some((lists, locations)) = &lists
                && let Some(indices) =
                    aexeo_core::markdown_adapter::list_finding_indices(lists, gate, evaluated)
            {
                for (finding_index, index) in indices
                    .into_iter()
                    .take(evaluated.findings.len())
                    .enumerate()
                {
                    if let Some((block_index, span_index)) = locations[index] {
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
        if matches!(gate, ContentGate::LinkReachability { .. }) {
            if let (Some(links), Some(observations)) = (&links, observations) {
                use aexeo_core::content_gate::LinkReachabilityObservation;
                let indices: Vec<_> = observations
                    .iter()
                    .enumerate()
                    .filter_map(|(index, observation)| {
                        (*observation == LinkReachabilityObservation::NotFound).then_some(index)
                    })
                    .collect();
                if links.len() == observations.len() && indices.len() == evaluated.total_findings()
                {
                    for (finding_index, index) in indices
                        .into_iter()
                        .take(evaluated.findings.len())
                        .enumerate()
                    {
                        let (_, block_index, span_index) = links[index];
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
        let indices = if let ContentGate::HeadingHierarchy { initial_level, .. } = gate {
            let levels: Vec<_> = heading_blocks
                .iter()
                .filter_map(|(_, block)| heading_level(&block.style))
                .collect();
            Some(
                aexeo_core::content_gate::heading_hierarchy_indices(&levels, *initial_level)
                    .collect(),
            )
        } else {
            aexeo_core::content_gate::heading_finding_indices(gate, &headings)
        };
        if let Some(indices) = indices {
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
            let relative = offset - start;
            let utf16_offset = if many_prose_findings && text.len() >= 8192 {
                let index = utf16_indices
                    .entry(index)
                    .or_insert_with(|| (!text.is_ascii()).then(|| Utf16Index::new(text)));
                if let Some(index) = index {
                    let Some(units) = index.position(text, relative) else {
                        continue;
                    };
                    units
                } else {
                    relative
                }
            } else {
                let Some(prefix) = text.get(..relative) else {
                    continue;
                };
                prefix.encode_utf16().count()
            };
            result.push(EmdashSourceLocation {
                gate_id: evaluated.gate_id.clone(),
                finding_index,
                block_index,
                span_index,
                utf16_offset,
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
    evaluate_collection_with_completeness(documents, preset, true)
}

/// Evaluate an explicitly complete or incomplete collection. Findings from an
/// incomplete collection remain visible, but the gate cannot pass.
pub fn evaluate_collection_with_completeness(
    documents: &[EmdashDocument],
    preset: &ContentPreset,
    complete: bool,
) -> Result<ContentReport> {
    let mut routes = std::collections::BTreeSet::new();
    for document in documents {
        if document.route.trim().is_empty() || !routes.insert(&document.route) {
            anyhow::bail!("collection item IDs must be nonempty and unique");
        }
    }
    let items: Vec<_> = documents
        .iter()
        .map(|document| {
            let mut metadata = content_metadata(document);
            for field in &document.unavailable_metadata_fields {
                metadata.remove(field);
            }
            ContentInput {
                id: document.route.clone(),
                metadata: Some(metadata),
                ..Default::default()
            }
        })
        .collect();
    let root = ContentInput {
        id: "collection".into(),
        ..Default::default()
    };
    preset.evaluate_with_collection_completeness(
        &root,
        &ContentEvaluationContext {
            collection: Some(&items),
            ..Default::default()
        },
        None,
        complete,
        None,
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

pub fn content_link_destinations(document: &EmdashDocument) -> Vec<String> {
    content_links(document)
        .into_iter()
        .map(|(destination, _, _)| destination.to_owned())
        .collect()
}

/// Host-supplied observations tied to an exact JSON document and projected links.
/// This binding prevents stale reuse; it does not authenticate the observer.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoundLinkObservations {
    pub document: serde_json::Value,
    pub destinations: Vec<String>,
    pub observations: Vec<aexeo_core::content_gate::LinkReachabilityObservation>,
}

pub fn bound_link_observations<'a>(
    batch: &'a BoundLinkObservations,
    raw: &serde_json::Value,
    document: &EmdashDocument,
    links_available: bool,
) -> Option<&'a [aexeo_core::content_gate::LinkReachabilityObservation]> {
    if !links_available
        || batch.document != *raw
        || batch.destinations.len() > 10_000
        || batch.destinations != content_link_destinations(document)
        || batch.observations.len() != batch.destinations.len()
    {
        return None;
    }
    Some(&batch.observations)
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
    let (mut input, _, _, literal_text) = content_input_with_context(document);
    if let Some(literal_text) = literal_text {
        input.text = Some(literal_text);
    }
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
    evaluate_content_with_reachability(
        document,
        preset,
        evidence_sources,
        capabilities,
        extensions,
        None,
    )
}

/// Evaluate only observations already bound by the host to this snapshot.
pub fn evaluate_content_with_reachability(
    document: &EmdashDocument,
    preset: &ContentPreset,
    evidence_sources: Option<BTreeMap<String, String>>,
    capabilities: [bool; 4],
    extensions: Option<&dyn aexeo_core::content_gate::ContentGateExtensions>,
    observations: Option<&[aexeo_core::content_gate::LinkReachabilityObservation]>,
) -> anyhow::Result<ContentReport> {
    evaluate_content_with_list_completeness(
        document,
        preset,
        evidence_sources,
        capabilities,
        extensions,
        observations,
        false,
    )
}

/// Evaluate with an explicit assertion that every Portable Text list block in
/// the source snapshot is represented, in order. The host must account for
/// unknown custom components before setting `list_structure_complete`.
pub fn evaluate_content_with_list_completeness(
    document: &EmdashDocument,
    preset: &ContentPreset,
    evidence_sources: Option<BTreeMap<String, String>>,
    capabilities: [bool; 4],
    extensions: Option<&dyn aexeo_core::content_gate::ContentGateExtensions>,
    observations: Option<&[aexeo_core::content_gate::LinkReachabilityObservation]>,
    list_structure_complete: bool,
) -> anyhow::Result<ContentReport> {
    let lists = preset
        .gates
        .iter()
        .any(|gate| matches!(gate, ContentGate::ListItemCount { .. }))
        .then(|| portable_text_structured_lists(document, list_structure_complete))
        .flatten();
    let paragraph_list_items = preset
        .gates
        .iter()
        .any(|gate| matches!(gate, ContentGate::ProseSentenceLengthVariance { .. }))
        .then(|| portable_text_paragraph_list_items(document, list_structure_complete))
        .flatten();
    let [text, headings, paragraphs, links] = capabilities;
    let (mut input, mut context, link_count, literal_text) = content_input_with_context(document);
    if text
        && preset
            .gates
            .iter()
            .any(|gate| matches!(gate, ContentGate::LiteralMarkdownLinks { .. }))
    {
        context.literal_markdown_links = literal_markdown_links(document);
    }
    input.evidence_sources = evidence_sources;
    if !text {
        input.text = None;
    }
    if !headings {
        input.headings = None;
    }
    if !paragraphs {
        context.paragraphs = None;
    }
    let destinations = (links
        && preset.gates.iter().any(|gate| {
            matches!(
                gate,
                ContentGate::LinkDestinations { .. } | ContentGate::LinkReachability { .. }
            )
        }))
    .then(|| content_link_destinations(document));
    let heading_levels: Option<Vec<u8>> = headings.then(|| {
        document
            .body
            .iter()
            .filter_map(|block| heading_level(&block.style))
            .collect()
    });
    preset.evaluate_with_literal_projection(
        &input,
        &ContentEvaluationContext {
            adapter: Some(&context),
            link_count: links.then_some(link_count),
            extensions,
            link_reachability: if links { observations } else { None },
            ..Default::default()
        },
        destinations.as_deref(),
        heading_levels.as_deref(),
        true,
        None,
        lists.as_deref(),
        paragraph_list_items.as_deref(),
        literal_text.as_deref(),
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
    fn literal_markdown_review_uses_rich_text_and_preserves_positions() {
        let document: EmdashDocument = serde_json::from_str(r#"{"route":"/x","title":"T","body":[{"children":[{"_type":"span","text":"é "},{"_type":"span","text":"[Help]("},{"_type":"span","text":"https://example.com)"},{"_type":"span","text":" and `code [ignore](https://example.com)`"},{"_type":"span","text":" [also ignore](https://example.com)","marks":["code"]}]},{"children":[{"_type":"span","text":"native link","marks":["link"]}],"markDefs":[{"_key":"link","_type":"link","href":"https://example.com"}]}]}"#).unwrap();
        let preset = ContentPreset::from_toml(
            "version=1\nname='links'\n[[gates]]\nid='literal'\nkind='literal_markdown_links'\n",
        )
        .unwrap();
        let report =
            evaluate_content_with_body_capabilities(&document, &preset, None, [true; 4]).unwrap();
        assert_eq!(report.gates[0].status, GateStatus::Failed);
        assert!(!report.gates[0].blocking);
        assert_eq!(report.gates[0].findings.len(), 1);
        let locations = source_locations(&document, &preset, &report);
        assert_eq!(
            (
                locations[0].block_index,
                locations[0].span_index,
                locations[0].utf16_offset
            ),
            (0, 1, 0)
        );
        let unavailable = evaluate_content_with_body_capabilities(
            &document,
            &preset,
            None,
            [false, true, true, true],
        )
        .unwrap();
        assert_eq!(unavailable.gates[0].status, GateStatus::NotChecked);
    }

    #[test]
    fn unfinished_markdown_link_candidates_map_to_portable_text_spans() {
        let preset = ContentPreset::from_toml(
            "version=1\nname='links'\n[[gates]]\nid='literal'\nkind='literal_markdown_links'\n",
        )
        .unwrap();
        for target in [
            "https://example.org",
            "HTTP://example.org",
            "HtTpS://example.org",
            "/guide",
        ] {
            let document: EmdashDocument = serde_json::from_value(serde_json::json!({
                "route": "/x", "title": "T", "body": [
                    {"children": [{"_type": "span", "text": "é "},
                        {"_type": "span", "text": format!("[Help]({target}")}]},
                    {"children": [{"_type": "span", "text": format!("[code]({target}"), "marks": ["code"]}]}
                ]
            })).unwrap();
            let report =
                evaluate_content_with_body_capabilities(&document, &preset, None, [true; 4])
                    .unwrap();
            assert_eq!(report.gates[0].status, GateStatus::Failed);
            assert!(!report.gates[0].blocking);
            assert_eq!(report.gates[0].findings.len(), 1);
            let locations = source_locations(&document, &preset, &report);
            assert_eq!(
                (
                    locations[0].block_index,
                    locations[0].span_index,
                    locations[0].utf16_offset
                ),
                (0, 1, 0)
            );
        }
    }

    #[test]
    fn literal_reference_candidates_preserve_cross_span_locations() {
        let document: EmdashDocument = serde_json::from_str(r#"{"route":"/x","title":"T","body":[{"children":[{"_type":"span","text":"é "},{"_type":"span","text":"[Help]["},{"_type":"span","text":"source] [1]"},{"_type":"span","text":" [code][x]","marks":["code"]}]},{"children":[{"_type":"span","text":"[source]: https://example.org"}]}]}"#).unwrap();
        let preset = ContentPreset::from_toml(
            "version=1\nname='links'\n[[gates]]\nid='literal'\nkind='literal_markdown_links'\n",
        )
        .unwrap();
        let report =
            evaluate_content_with_body_capabilities(&document, &preset, None, [true; 4]).unwrap();
        assert_eq!(report.gates[0].findings.len(), 1);
        let locations = source_locations(&document, &preset, &report);
        assert_eq!(
            (
                locations[0].block_index,
                locations[0].span_index,
                locations[0].utf16_offset
            ),
            (0, 1, 0)
        );
    }

    #[test]
    fn paragraph_list_classification_requires_complete_projected_blocks() {
        let document: EmdashDocument = serde_json::from_str(r#"{"route":"/x","title":"T","body":[{"children":[]},{"style":"h2","children":[]},{"listItem":"bullet","level":1,"children":[]},{"listItem":"bullet","level":2,"children":[]},{"children":[]}]}"#).unwrap();
        assert_eq!(portable_text_paragraph_list_items(&document, false), None);
        assert_eq!(
            portable_text_paragraph_list_items(&document, true),
            Some(vec![false, true, true, false])
        );
        assert_eq!(
            content_input_with_context(&document)
                .1
                .paragraphs
                .unwrap()
                .len(),
            4
        );
        let invalid: EmdashDocument = serde_json::from_str(
            r#"{"route":"/x","title":"T","body":[{"listItem":"bullet","level":2,"children":[]}]}"#,
        )
        .unwrap();
        assert_eq!(portable_text_paragraph_list_items(&invalid, true), None);
        let hidden: EmdashDocument = serde_json::from_str(r#"{"route":"/x","title":"T","body":[{"style":"h2","listItem":"bullet","children":[]}]}"#).unwrap();
        assert_eq!(portable_text_paragraph_list_items(&hidden, true), None);
        let empty: EmdashDocument =
            serde_json::from_str(r#"{"route":"/x","title":"T","body":[]}"#).unwrap();
        assert_eq!(
            portable_text_paragraph_list_items(&empty, true),
            Some(vec![])
        );
    }

    #[test]
    fn prose_variance_requires_complete_portable_text_list_observation() {
        let document: EmdashDocument = serde_json::from_str(r#"{"route":"/x","title":"T","body":[{"children":[{"_type":"span","text":"One two. Three four. Five six."}]},{"listItem":"bullet","level":1,"children":[{"_type":"span","text":"List one. List two. List three."}]},{"children":[{"_type":"span","text":"Seven eight. Nine ten. Eleven twelve."}]}]}"#).unwrap();
        let preset = ContentPreset::from_toml("version=1\nname='test'\n[[gates]]\nid='v'\nkind='prose_sentence_length_variance'\nwindow_size=3\nmin_stddev_words=2\n").unwrap();
        let run = |complete, capabilities| {
            evaluate_content_with_list_completeness(
                &document,
                &preset,
                None,
                capabilities,
                None,
                None,
                complete,
            )
            .unwrap()
        };
        let unknown = run(false, [true; 4]);
        assert_eq!(unknown.gates[0].status, GateStatus::NotChecked);
        assert!(unknown.gates[0].findings.is_empty());
        let missing_paragraphs = run(true, [true, true, false, true]);
        assert_eq!(missing_paragraphs.gates[0].status, GateStatus::NotChecked);
        let report = run(true, [true; 4]);
        assert_eq!(report.gates[0].status, GateStatus::Failed);
        assert_eq!(report.gates[0].measurements["list_paragraphs_excluded"], 1);
        assert_eq!(report.gates[0].measurements["windows_checked"], 2);
        let locations = source_locations(&document, &preset, &report);
        assert_eq!(
            locations
                .iter()
                .map(|item| item.block_index)
                .collect::<Vec<_>>(),
            vec![0, 2]
        );
    }

    #[test]
    fn structured_lists_require_complete_valid_body_and_count_nested_items() {
        let document: EmdashDocument = serde_json::from_str(r#"{"route":"/x","title":"T","body":[{"listItem":"bullet","level":1,"children":[]},{"listItem":"bullet","level":2,"children":[]},{"listItem":"bullet","level":2,"children":[]},{"listItem":"bullet","level":1,"children":[]}]}"#).unwrap();
        assert_eq!(portable_text_structured_lists(&document, false), None);
        let lists = portable_text_structured_lists(&document, true).unwrap();
        assert_eq!(
            lists
                .iter()
                .map(|list| (list.depth, list.item_count))
                .collect::<Vec<_>>(),
            vec![(1, 2), (2, 2)]
        );
        let invalid: EmdashDocument = serde_json::from_str(
            r#"{"route":"/x","title":"T","body":[{"listItem":"bullet","level":2,"children":[]}]}"#,
        )
        .unwrap();
        assert_eq!(portable_text_structured_lists(&invalid, true), None);
        let preset = ContentPreset::from_toml("version=1\nname='lists'\n[[gates]]\nid='review'\nkind='list_item_count'\nflagged_counts=[2]\n").unwrap();
        let unknown = evaluate_content_with_list_completeness(
            &document, &preset, None, [true; 4], None, None, false,
        )
        .unwrap();
        assert_eq!(unknown.gates[0].status, GateStatus::NotChecked);
        let complete = evaluate_content_with_list_completeness(
            &document, &preset, None, [true; 4], None, None, true,
        )
        .unwrap();
        assert_eq!(complete.gates[0].status, GateStatus::Failed);
        let malformed = evaluate_content_with_list_completeness(
            &invalid, &preset, None, [true; 4], None, None, true,
        )
        .unwrap();
        assert_eq!(malformed.gates[0].status, GateStatus::NotChecked);
    }

    #[test]
    fn list_findings_map_to_first_nonempty_span_of_each_list_start() {
        let document: EmdashDocument = serde_json::from_str(r#"{"route":"/x","title":"T","body":[
            {"listItem":"bullet","level":1,"children":[{"_type":"span","text":""},{"_type":"span","text":"Outer ✨"}]},
            {"listItem":"bullet","level":2,"children":[{"_type":"span","text":"Nested"}]},
            {"listItem":"bullet","level":2,"children":[{"_type":"span","text":"Nested 2"}]},
            {"listItem":"bullet","level":1,"children":[{"_type":"span","text":"Outer 2"}]},
            {"children":[{"_type":"span","text":"Break"}]},
            {"listItem":"bullet","level":1,"children":[]},
            {"listItem":"bullet","level":1,"children":[{"_type":"span","text":"Later"}]}
        ]}"#).unwrap();
        let preset = ContentPreset::from_toml("version=1\nname='lists'\n[[gates]]\nid='review'\nkind='list_item_count'\nflagged_counts=[2]\n").unwrap();
        let report = evaluate_content_with_list_completeness(
            &document, &preset, None, [true; 4], None, None, true,
        )
        .unwrap();
        assert_eq!(report.gates[0].status, GateStatus::Failed);
        assert_eq!(report.gates[0].findings.len(), 3);
        let positions = source_locations(&document, &preset, &report);
        assert_eq!(
            positions
                .iter()
                .map(|position| (
                    position.finding_index,
                    position.block_index,
                    position.span_index,
                    position.utf16_offset
                ))
                .collect::<Vec<_>>(),
            vec![(0, 0, 1, 0), (1, 1, 0, 0)]
        );
        let incomplete = evaluate_content_with_list_completeness(
            &document, &preset, None, [true; 4], None, None, false,
        )
        .unwrap();
        assert_eq!(incomplete.gates[0].status, GateStatus::NotChecked);
        assert!(source_locations(&document, &preset, &incomplete).is_empty());
    }

    #[test]
    fn pacing_checks_complete_projection_and_withholds_partial_projection() {
        let document: EmdashDocument = serde_json::from_str(r#"{"route":"/x","title":"Title","body":[{"children":[{"_type":"span","text":"four"}]},{"children":[{"_type":"span","text":"context"}]}]}"#).unwrap();
        let preset = ContentPreset::from_toml("version=1\nname='test'\n[[gates]]\nid='pacing'\nkind='editorial_pacing'\nsource='composition'\nprojection_separator=\"\\n\"\nprofile={version=1,name='test',fields=[{name='hook',max_characters=3},{name='context'}]}\n").unwrap();
        let snapshots = BTreeMap::from([(
            "composition".to_string(),
            r#"{"hook":"four","context":"context"}"#.to_string(),
        )]);
        let complete = evaluate_content_with_body_capabilities(
            &document,
            &preset,
            Some(snapshots.clone()),
            [true; 4],
        )
        .unwrap();
        assert_eq!(complete.gates[0].status, GateStatus::Failed);
        assert_eq!(complete.gates[0].findings.len(), 1);
        let partial = evaluate_content_with_body_capabilities(
            &document,
            &preset,
            Some(snapshots),
            [false, true, true, true],
        )
        .unwrap();
        assert_eq!(partial.gates[0].status, GateStatus::NotChecked);
        assert!(partial.gates[0].findings.is_empty());
    }

    #[test]
    fn metadata_length_respects_description_observation() {
        let mut document: EmdashDocument =
            serde_json::from_str(r#"{"route":"/t","title":"Title","body":[]}"#).unwrap();
        let preset = ContentPreset::from_toml("version=1\nname='test'\n[[gates]]\nid='description'\nkind='metadata_length'\nfield='description'\nmax=3\n").unwrap();
        document.description_source_observed = Some(false);
        document.description = Some("long".into());
        assert_eq!(
            evaluate_content(&document, &preset).unwrap().gates[0].status,
            GateStatus::NotChecked
        );
        document.description_source_observed = Some(true);
        let failed = evaluate_content(&document, &preset).unwrap();
        assert_eq!(failed.gates[0].status, GateStatus::Failed);
        assert_eq!(failed.gates[0].measurements["character_count"], 4);
        assert!(source_locations(&document, &preset, &failed).is_empty());
        document.description = None;
        assert_eq!(
            evaluate_content(&document, &preset).unwrap().gates[0].status,
            GateStatus::NotChecked
        );
    }

    #[test]
    fn description_metadata_distinguishes_unknown_absent_and_present() {
        let preset = ContentPreset::from_toml("version=1\nname='test'\n[[gates]]\nid='meta'\nkind='required_metadata'\nfields=['title','description']\n").unwrap();
        let mut document: EmdashDocument =
            serde_json::from_str(r#"{"route":"/x","title":"","body":[]}"#).unwrap();
        let unknown = evaluate_content(&document, &preset).unwrap();
        assert_eq!(unknown.gates[0].status, GateStatus::NotChecked);
        assert_eq!(unknown.gates[0].findings.len(), 1);
        assert!(unknown.gates[0].findings[0].message.contains("title"));
        assert_eq!(unknown.gates[0].measurements["unavailable_fields"], 1);
        document.title = "Title".into();
        document.description_source_observed = Some(true);
        let absent = evaluate_content(&document, &preset).unwrap();
        assert_eq!(absent.gates[0].status, GateStatus::Failed);
        assert!(absent.gates[0].findings[0].message.contains("description"));
        document.description = Some("Observed".into());
        let present = evaluate_content(&document, &preset).unwrap();
        assert_eq!(present.gates[0].status, GateStatus::Passed);
        document.description_source_observed = Some(false);
        let untrusted = evaluate_content(&document, &preset).unwrap();
        assert_eq!(untrusted.gates[0].status, GateStatus::NotChecked);
        assert!(untrusted.gates[0].findings.is_empty());
    }

    #[test]
    fn text_only_capability_preserves_block_boundaries() {
        let document: EmdashDocument = serde_json::from_str(r#"{"route":"/t","title":"T","body":[{"children":[{"_type":"span","text":"one best"}]},{"children":[{"_type":"span","text":"price four"}]}]}"#).unwrap();
        let preset = ContentPreset::from_toml("version=1\nname='test'\n[[gates]]\nid='phrase'\nkind='forbidden_terms'\nterms=['best price']\n[[gates]]\nid='sentence'\nkind='sentence_length'\nmax_words=3\n[[gates]]\nid='paragraph'\nkind='paragraph_count'\nmin=1\n").unwrap();
        let partial = evaluate_content_with_body_capabilities(
            &document,
            &preset,
            None,
            [true, true, false, true],
        )
        .unwrap();
        assert_eq!(partial.gates[0].status, GateStatus::Passed);
        assert_eq!(partial.gates[1].status, GateStatus::Passed);
        assert_eq!(partial.gates[2].status, GateStatus::NotChecked);
        let (input, context, links) =
            markdown_input_with_links("t.md", "one best\n\nprice four").unwrap();
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
        assert_eq!(partial.gates[0].status, markdown.gates[0].status);
        assert_eq!(partial.gates[1].status, markdown.gates[1].status);
    }

    #[test]
    fn paragraph_only_sentences_skip_headings_and_require_paragraphs() {
        let document: EmdashDocument = serde_json::from_str(r#"{"route":"/t","title":"T","body":[{"style":"h2","children":[{"_type":"span","text":"A heading that exceeds limit"}]},{"children":[{"_type":"span","text":"A longer paragraph here."}]}]}"#).unwrap();
        let preset = ContentPreset::from_toml("version=1\nname='t'\n[[gates]]\nid='s'\nkind='sentence_length'\nmax_words=3\nparagraphs_only=true\n").unwrap();
        let full = evaluate_content(&document, &preset).unwrap();
        assert_eq!(full.gates[0].status, GateStatus::Failed);
        assert_eq!(full.gates[0].measurements["sentence_count"], 1);
        assert_eq!(full.gates[0].findings[0].line, 2);
        assert_eq!(
            source_locations(&document, &preset, &full)[0].block_index,
            1
        );
        let partial = evaluate_content_with_body_capabilities(
            &document,
            &preset,
            None,
            [true, true, false, true],
        )
        .unwrap();
        assert_eq!(partial.gates[0].status, GateStatus::NotChecked);
        assert!(partial.gates[0].findings.is_empty());
    }

    #[test]
    fn markdown_and_portable_text_keep_http_urls_in_sentence_measurements() {
        let source = "See https://example.org/a.b?x=1. Next sentence.";
        let document: EmdashDocument = serde_json::from_value(serde_json::json!({
            "route":"/t", "title":"T", "body":[{"children":[
                {"_type":"span", "text":"See https://example."},
                {"_type":"span", "text":"org/a.b?x=1. "},
                {"_type":"span", "text":"Next sentence."}
            ]}]
        }))
        .unwrap();
        let preset = ContentPreset::from_toml(
            "version=1\nname='t'\n[[gates]]\nid='s'\nkind='sentence_length'\nmax_words=1\n",
        )
        .unwrap();
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
        assert_eq!(markdown.gates[0].measurements["sentence_count"], 2);
        assert_eq!(emdash.gates[0].measurements, markdown.gates[0].measurements);
        let locations = source_locations(&document, &preset, &emdash);
        assert_eq!(locations.len(), 2);
        assert_eq!(
            (
                locations[1].block_index,
                locations[1].span_index,
                locations[1].utf16_offset
            ),
            (0, 2, 0)
        );
    }

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
    fn builtin_sentence_openings_maps_and_withholds_unavailable_body() {
        let document: EmdashDocument = serde_json::from_str(r#"{"route":"/t","title":"T","body":[{"children":[{"_type":"span","text":"The team plans. The team acts. THE TEAM checks."}]}]}"#).unwrap();
        let preset =
            resolve_content_preset("sentence-openings-review", &builtin_content_presets()).unwrap();
        let report = evaluate_content(&document, &preset).unwrap();
        assert_eq!(report.gates[0].status, GateStatus::Failed);
        assert!(!report.gates[0].blocking);
        let locations = source_locations(&document, &preset, &report);
        assert_eq!(locations.len(), 1);
        assert_eq!(
            (
                locations[0].block_index,
                locations[0].span_index,
                locations[0].utf16_offset
            ),
            (0, 0, 31)
        );
        let unavailable =
            evaluate_content_with_body_availability(&document, &preset, None, false).unwrap();
        assert_eq!(unavailable.gates[0].status, GateStatus::NotChecked);
    }

    #[test]
    fn builtin_duplicate_review_maps_and_withholds_unavailable_body() {
        let document: EmdashDocument = serde_json::from_str(r#"{"route":"/t","title":"T","body":[{"children":[{"_type":"span","text":"The team checks backups before every planned deployment."}]},{"children":[{"_type":"span","text":"THE TEAM CHECKS BACKUPS BEFORE EVERY PLANNED DEPLOYMENT."}]}]}"#).unwrap();
        let preset =
            resolve_content_preset("duplicate-paragraph-review", &builtin_content_presets())
                .unwrap();
        let report = evaluate_content(&document, &preset).unwrap();
        assert_eq!(report.gates[0].status, GateStatus::Failed);
        assert!(!report.gates[0].blocking);
        let locations = source_locations(&document, &preset, &report);
        assert_eq!(locations.len(), 1);
        assert_eq!(
            (
                locations[0].block_index,
                locations[0].span_index,
                locations[0].utf16_offset
            ),
            (1, 0, 0)
        );
        let unavailable =
            evaluate_content_with_body_availability(&document, &preset, None, false).unwrap();
        assert_eq!(unavailable.gates[0].status, GateStatus::NotChecked);
    }

    #[test]
    fn transition_opening_maps_to_emdash_and_withholds_missing_paragraphs() {
        let document: EmdashDocument = serde_json::from_str(r#"{"route":"/t","title":"T","body":[{"children":[{"_type":"span","text":"First. "},{"_type":"span","text":"Moreover, next."}]}]}"#).unwrap();
        let preset = ContentPreset::from_toml("version=1\nname='test'\n[[gates]]\nid='transitions'\nkind='sentence_opening_terms'\nterms=['Moreover']\n").unwrap();
        let report = evaluate_content(&document, &preset).unwrap();
        assert_eq!(report.gates[0].status, GateStatus::Failed);
        let locations = source_locations(&document, &preset, &report);
        assert_eq!(
            (
                locations[0].block_index,
                locations[0].span_index,
                locations[0].utf16_offset
            ),
            (0, 1, 0)
        );
        let unavailable = evaluate_content_with_body_capabilities(
            &document,
            &preset,
            None,
            [true, true, false, true],
        )
        .unwrap();
        assert_eq!(unavailable.gates[0].status, GateStatus::NotChecked);
    }

    #[test]
    fn inline_triple_candidate_maps_to_source_and_withholds_partial_text() {
        let document: EmdashDocument = serde_json::from_str(r#"{"route":"/t","title":"T","body":[{"children":[{"_type":"span","text":"We have apples, pears, and plums."}]}]}"#).unwrap();
        let preset = ContentPreset::from_toml("version=1\nname='test'\n[[gates]]\nid='triples'\nkind='inline_triple_candidates'\nconjunction='and'\n").unwrap();
        let report = evaluate_content(&document, &preset).unwrap();
        assert_eq!(report.gates[0].status, GateStatus::Failed);
        let locations = source_locations(&document, &preset, &report);
        assert_eq!(
            (
                locations[0].block_index,
                locations[0].span_index,
                locations[0].utf16_offset
            ),
            (0, 0, 0)
        );
        let partial = evaluate_content_with_body_capabilities(
            &document,
            &preset,
            None,
            [true, true, false, true],
        )
        .unwrap();
        assert_eq!(partial.gates[0].status, GateStatus::NotChecked);
    }

    #[test]
    fn duplicate_paragraph_maps_to_emdash_block() {
        let document: EmdashDocument = serde_json::from_str(r#"{"route":"/t","title":"T","body":[{"children":[{"_type":"span","text":"Repeated prose."}]},{"children":[{"_type":"span","text":"Repeated prose."}]}]}"#).unwrap();
        let preset = ContentPreset::from_toml("version=1\nname='review'\n[[gates]]\nid='dupe'\nkind='duplicate_paragraphs'\nmax_occurrences=1\n").unwrap();
        let report = evaluate_content(&document, &preset).unwrap();
        assert_eq!(report.gates[0].status, GateStatus::Failed);
        let locations = source_locations(&document, &preset, &report);
        assert_eq!((locations[0].block_index, locations[0].span_index), (1, 0));
        let unavailable =
            evaluate_content_with_body_availability(&document, &preset, None, false).unwrap();
        assert_eq!(unavailable.gates[0].status, GateStatus::NotChecked);
    }

    #[test]
    fn case_insensitive_repetition_matches_markdown_and_maps_emdash() {
        let document: EmdashDocument = serde_json::from_str(r#"{"route":"/t","title":"T","body":[{"children":[{"_type":"span","text":"Café"}]},{"children":[{"_type":"span","text":"😀 CAFÉ"}]}]}"#).unwrap();
        let preset = ContentPreset::from_toml("version=1\nname='test'\n[[gates]]\nid='repeat'\nkind='repeated_terms'\nterms=['café']\nmax_occurrences=1\ncase_insensitive=true\n").unwrap();
        let report = evaluate_content(&document, &preset).unwrap();
        let (input, context, links) = markdown_input_with_links("t.md", "Café\n\n😀 CAFÉ").unwrap();
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
        assert_eq!(report.gates[0].status, GateStatus::Failed);
        assert_eq!(report.gates[0].status, markdown.gates[0].status);
        assert_eq!(report.gates[0].measurements["matching_occurrences"], 2);
        let locations = source_locations(&document, &preset, &report);
        assert_eq!(locations.len(), 1);
        assert_eq!(
            (
                locations[0].block_index,
                locations[0].span_index,
                locations[0].utf16_offset
            ),
            (1, 0, 3)
        );
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
    fn heading_hierarchy_respects_complete_and_partial_emdash_body() {
        let document: EmdashDocument = serde_json::from_str(r#"{"route":"/test","title":"Test","body":[{"style":"h2","children":[{"_type":"span","text":"One"}]},{"style":"h4","children":[{"_type":"span","text":"Two"}]}]}"#).unwrap();
        let preset = ContentPreset::from_toml(
            "version=1\nname='test'\n[[gates]]\nid='hierarchy'\nkind='heading_hierarchy'\n",
        )
        .unwrap();
        let complete = evaluate_content_with_body_capabilities(
            &document,
            &preset,
            None,
            [true, true, true, true],
        )
        .unwrap();
        assert_eq!(complete.gates[0].status, GateStatus::Failed);
        assert_eq!(complete.gates[0].measurements["skipped_levels"], 1);
        let locations = source_locations(&document, &preset, &complete);
        assert_eq!(locations.len(), 1);
        assert_eq!(
            (
                locations[0].block_index,
                locations[0].span_index,
                locations[0].utf16_offset
            ),
            (1, 0, 0)
        );
        assert_eq!(
            source_mapping_coverage(&preset, &complete, &locations).mapped,
            1
        );
        let partial = evaluate_content_with_body_capabilities(
            &document,
            &preset,
            None,
            [true, false, true, true],
        )
        .unwrap();
        assert_eq!(partial.gates[0].status, GateStatus::NotChecked);
        assert!(partial.gates[0].findings.is_empty());
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
    fn source_mapping_coverage_distinguishes_unmapped_and_nonpositional_findings() {
        let document: EmdashDocument = serde_json::from_str(r#"{"route":"/t","title":"T","body":[{"children":[{"_type":"span","text":"first second"}]}]}"#).unwrap();
        let preset = ContentPreset::from_toml("version=1\nname='test'\n[[gates]]\nid='terms'\nkind='forbidden_terms'\nterms=['first','second']\n[[gates]]\nid='count'\nkind='word_count'\nmin=10\n").unwrap();
        let mut report = evaluate_content(&document, &preset).unwrap();
        assert_eq!(report.gates[0].findings.len(), 2);
        assert_eq!(report.gates[1].findings.len(), 1);
        report.gates[0].findings[1].column = "first second".chars().count() + 1;
        let locations = source_locations(&document, &preset, &report);
        assert_eq!(
            source_mapping_coverage(&preset, &report, &locations),
            SourceMappingCoverage {
                eligible: 2,
                mapped: 1,
                unmapped: 1
            }
        );
    }

    #[test]
    fn utf16_index_matches_reference_across_checkpoints() {
        let text = format!("{}\n{}", "é😀ab".repeat(200), "漢😀z".repeat(200));
        let index = Utf16Index::new(&text);
        for (offset, _) in text.char_indices() {
            assert_eq!(
                index.position(&text, offset),
                Some(text[..offset].encode_utf16().count())
            );
        }
        assert_eq!(
            index.position(&text, text.len()),
            Some(text.encode_utf16().count())
        );
        assert_eq!(index.position(&text, 1), None);
        assert_eq!(index.position(&text, text.len() + 1), None);
    }

    #[test]
    fn source_mapping_handles_long_unicode_spans_and_extreme_coordinates() {
        let prefix = "😀éabc".repeat(500);
        let document: EmdashDocument = serde_json::from_value(serde_json::json!({
            "route": "/test", "title": "Test", "body": [
                {"children": [{"_type": "span", "text": "Earlier 😀"}]},
                {"children": [{"_type": "span", "text": format!("{prefix} target")}]}
            ]
        }))
        .unwrap();
        let preset = ContentPreset::from_toml("version=1\nname='test'\n[[gates]]\nid='terms'\nkind='forbidden_terms'\nterms=['target']\n").unwrap();
        let mut report = evaluate_content(&document, &preset).unwrap();
        let locations = source_locations(&document, &preset, &report);
        assert_eq!(locations[0].block_index, 1);
        assert_eq!(locations[0].utf16_offset, prefix.encode_utf16().count() + 1);
        report.gates[0].findings[0].column = usize::MAX;
        assert!(source_locations(&document, &preset, &report).is_empty());
    }

    #[test]
    fn source_mapping_indexes_many_unicode_findings_across_spans() {
        let body = format!("{} {}", "é😀".repeat(2000), "😀 target é ".repeat(25));
        let document: EmdashDocument = serde_json::from_value(serde_json::json!({
            "route": "/test", "title": "Test", "body": [
                {"children": [{"_type": "span", "text": "前置 😀"}]},
                {"children": [{"_type": "span", "text": &body}, {"_type": "span", "text": &body}]}
            ]
        }))
        .unwrap();
        let preset = ContentPreset::from_toml("version=1\nname='test'\n[[gates]]\nid='terms'\nkind='forbidden_terms'\nterms=['target']\n").unwrap();
        let report = evaluate_content(&document, &preset).unwrap();
        let locations = source_locations(&document, &preset, &report);
        let expected: Vec<_> = body
            .match_indices("target")
            .map(|(offset, _)| body[..offset].encode_utf16().count())
            .collect();
        assert_eq!(locations.len(), expected.len() * 2);
        for (index, location) in locations.iter().enumerate() {
            assert_eq!(location.block_index, 1);
            assert_eq!(location.span_index, index / expected.len());
            assert_eq!(location.utf16_offset, expected[index % expected.len()]);
        }
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
    fn paragraph_count_distinguishes_complete_empty_and_unavailable_bodies() {
        let preset = ContentPreset::from_toml("version=1\nname='count'\n[[gates]]\nid='count'\nkind='paragraph_count'\nmin=2\nmax=2\n").unwrap();
        let document: EmdashDocument = serde_json::from_str(r#"{"route":"/test","title":"Test","body":[{"style":"h2","children":[{"_type":"span","text":"Heading"}]},{"children":[{"_type":"span","text":"First."}]},{"children":[{"_type":"span","text":"Second."}]}]}"#).unwrap();
        let report = evaluate_content(&document, &preset).unwrap();
        assert_eq!(report.gates[0].status, GateStatus::Passed);
        assert_eq!(report.gates[0].measurements["paragraph_count"], 2);
        let unavailable =
            evaluate_content_with_body_availability(&document, &preset, None, false).unwrap();
        assert_eq!(unavailable.gates[0].status, GateStatus::NotChecked);
        assert!(
            !unavailable.gates[0]
                .measurements
                .contains_key("paragraph_count")
        );
        let empty: EmdashDocument =
            serde_json::from_str(r#"{"route":"/test","title":"Test","body":[]}"#).unwrap();
        let report = evaluate_content(&empty, &preset).unwrap();
        assert_eq!(report.gates[0].status, GateStatus::Failed);
        assert_eq!(report.gates[0].measurements["paragraph_count"], 0);
    }

    #[test]
    fn character_count_respects_emdash_body_availability() {
        let document: EmdashDocument = serde_json::from_str(r#"{"route":"/test","title":"Test","body":[{"children":[{"_type":"span","text":"你好 世界"}]}]}"#).unwrap();
        let preset = ContentPreset::from_toml("version=1\nname='test'\n[[gates]]\nid='chars'\nkind='character_count'\nmin=4\nmax=4\nignore_whitespace=true\n").unwrap();
        let report = evaluate_content(&document, &preset).unwrap();
        assert_eq!(report.gates[0].status, GateStatus::Passed);
        assert_eq!(report.gates[0].measurements["character_count"], 4);
        let missing =
            evaluate_content_with_body_availability(&document, &preset, None, false).unwrap();
        assert_eq!(missing.gates[0].status, GateStatus::NotChecked);
        assert!(
            !missing.gates[0]
                .measurements
                .contains_key("character_count")
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
    fn malformed_percent_link_is_reported_and_mapped_in_emdash() {
        let document: EmdashDocument = serde_json::from_str(r#"{"route":"/test","title":"Test","body":[{"markDefs":[{"_key":"bad","_type":"link","href":"/bad%zz"},{"_key":"good","_type":"link","href":"/good%20path"}],"children":[{"_type":"span","text":"bad","marks":["bad"]},{"_type":"span","text":" good","marks":["good"]}]}]}"#).unwrap();
        let preset = ContentPreset::from_toml("version=1\nname='links'\n[[gates]]\nid='links'\nkind='link_destinations'\nallowed_schemes=['https']\n").unwrap();
        let report = evaluate_content(&document, &preset).unwrap();
        assert_eq!(report.gates[0].status, GateStatus::Failed);
        assert_eq!(report.gates[0].measurements["invalid_destinations"], 1);
        assert!(report.gates[0].findings[0].message.contains("percent"));
        let locations = source_locations(&document, &preset, &report);
        assert_eq!(locations.len(), 1);
        assert_eq!((locations[0].block_index, locations[0].span_index), (0, 0));
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
    fn collection_does_not_compare_explicitly_unavailable_metadata() {
        let documents: Vec<EmdashDocument> = serde_json::from_str(
            r#"[{"route":"/a","title":"Same"},{"route":"/b","title":"Same","unavailable_metadata_fields":["title"]}]"#,
        ).unwrap();
        let preset = ContentPreset::from_toml("version=1\nname='test'\n[[gates]]\nid='unique'\nkind='unique_collection_metadata'\nfield='title'\n").unwrap();
        let report = evaluate_collection(&documents, &preset).unwrap();
        assert_eq!(report.gates[0].status, GateStatus::NotChecked);
        assert!(report.gates[0].findings.is_empty());
    }

    #[test]
    fn collection_rejects_empty_and_duplicate_routes() {
        let preset = ContentPreset::from_toml("version=1\nname='test'\n[[gates]]\nid='unique'\nkind='unique_collection_metadata'\nfield='title'\n").unwrap();
        for raw in [
            r#"[{"route":" ","title":"A","body":[]}]"#,
            r#"[{"route":"/same","title":"A","body":[]},{"route":"/same","title":"B","body":[]}]"#,
        ] {
            let documents: Vec<EmdashDocument> = serde_json::from_str(raw).unwrap();
            assert!(evaluate_collection(&documents, &preset).is_err());
        }
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
    fn numeric_reference_markers_do_not_merge_sentences_or_move_sources() {
        let mut children = Vec::new();
        for index in 0..31 {
            children.push(serde_json::json!({"_type":"span", "text": "In 2025."}));
            if index % 5 == 0 {
                children.push(serde_json::json!({"_type":"span", "text": "1", "marks":["ref"]}));
            }
            children.push(serde_json::json!({"_type":"span", "text": " "}));
        }
        let raw = serde_json::json!({"route":"/test", "title":"Test", "body":[{
            "markDefs":[{"_key":"ref", "_type":"link", "href":"#reference-source"}],
            "children":children
        }]});
        let document: EmdashDocument = serde_json::from_value(raw).unwrap();
        let preset = ContentPreset::from_toml(
            "version=1\nname='test'\n[[gates]]\nid='length'\nkind='sentence_length'\nmax_words=1\n",
        )
        .unwrap();
        let report = evaluate_content(&document, &preset).unwrap();
        assert_eq!(report.gates[0].measurements["sentence_count"], 31);
        let locations = source_locations(&document, &preset, &report);
        assert_eq!(locations.len(), 31);
        assert_eq!((locations[1].span_index, locations[1].utf16_offset), (3, 0));
        assert_eq!(document.body[0].children.len(), 69);
    }

    #[test]
    fn adapter_projection_preserves_citations_and_observation_boundaries() {
        let document: EmdashDocument = serde_json::from_value(serde_json::json!({
            "route":"/test", "title":"Test", "body":[{
                "markDefs":[{"_key":"ref", "_type":"link", "href":"#cite_note-source-17"}],
                "children":[{"_type":"span", "text":"Élan."}, {"_type":"span", "text":"[17]", "marks":["ref"]}, {"_type":"span", "text":" Next."}]
            }]
        })).unwrap();
        let projected = content_adapter_projection(&document, [true; 4], true).unwrap();
        assert_eq!(projected["text"], "Élan.     Next.");
        assert_eq!(projected["literal_text"], "Élan.[17] Next.");
        assert_eq!(projected["paragraphs"][0]["start"], 0);
        assert_eq!(projected["paragraphs"][0]["end"], "Élan.     Next.".len());
        assert_eq!(projected["link_destinations"][0], "#cite_note-source-17");
        assert_eq!(projected["paragraphs"][0]["list_item"], false);
        assert!(projected.get("source_map").is_none());
        assert_eq!(
            projected["native_source_map"]["spans"][1]["path"],
            serde_json::json!(["body", 0, "children", 1, "text"])
        );
        assert_eq!(
            projected["native_source_map"]["spans"][1]["start"],
            "Élan.".len()
        );

        let unavailable = content_adapter_projection(&document, [false; 4], false).unwrap();
        assert!(unavailable["text"].is_null());
        assert!(unavailable.get("literal_text").is_none());
        assert!(unavailable.get("native_source_map").is_none());
        assert!(unavailable.get("paragraphs").is_none());
        assert!(unavailable.get("link_destinations").is_none());
    }

    #[test]
    fn adapter_metadata_projection_preserves_observation_coverage_without_body() {
        let document: EmdashDocument = serde_json::from_value(serde_json::json!({
            "route":"/test", "title":"Editorial title", "description":"Fallback",
            "description_source_observed":false, "meta":{"author":"Editor", "secret":"ignored"},
            "unavailable_metadata_fields":["author"]
        }))
        .unwrap();
        let observed = ["title", "description", "author", "reviewed"]
            .into_iter()
            .map(str::to_owned)
            .collect();
        let projection =
            content_adapter_projection_with_metadata(&document, [false; 4], false, Some(&observed))
                .unwrap();
        assert!(projection["text"].is_null());
        assert_eq!(
            projection["metadata"],
            serde_json::json!({"title":"Editorial title"})
        );
        assert_eq!(
            projection["observed_metadata_fields"],
            serde_json::json!(["reviewed", "title"])
        );
        let unknown =
            content_adapter_projection_with_metadata(&document, [false; 4], false, None).unwrap();
        assert_eq!(unknown["metadata"], serde_json::json!({}));
        assert_eq!(unknown["observed_metadata_fields"], serde_json::json!([]));
    }

    #[test]
    fn oversized_native_coordinates_do_not_discard_usable_projection() {
        let limit = aexeo_core::content_revision_loop::MAX_DRAFT_BYTES;
        let value = serde_json::json!({"text":"x".repeat(limit - 100), "native_source_map":{"spans":[{"padding":"y".repeat(200)}]}});
        let projection = bounded_adapter_projection(value).unwrap();
        assert!(projection.get("native_source_map").is_none());
        assert_eq!(projection["text"].as_str().unwrap().len(), limit - 100);
    }

    #[test]
    fn adapter_projection_rejects_too_many_links() {
        let mut children = Vec::new();
        for _ in 0..=MAX_ADAPTER_OBSERVATIONS {
            children.push(serde_json::json!({"_type":"span", "text":"x", "marks":["link"]}));
            children.push(serde_json::json!({"_type":"span", "text":" ", "marks":[]}));
        }
        let document: EmdashDocument = serde_json::from_value(serde_json::json!({
            "route":"/test", "title":"Test", "body":[{
                "children":children, "markDefs":[{"_key":"link", "_type":"link", "href":"https://example.test"}]
            }]
        })).unwrap();
        let error = content_adapter_projection(&document, [true; 4], false).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("link observations exceed projection limit")
        );
    }

    #[test]
    fn adapter_projection_rejects_oversized_input_before_building_views() {
        let document: EmdashDocument = serde_json::from_value(serde_json::json!({
            "route":"/test", "title":"Test", "body":[{"children":[{
                "_type":"span", "text":"x".repeat(aexeo_core::content_revision_loop::MAX_DRAFT_BYTES + 1)
            }]}]
        })).unwrap();
        let error = content_adapter_projection(&document, [true; 4], true).unwrap_err();
        assert!(error.to_string().contains("body exceeds projection limit"));
    }

    #[test]
    fn adapter_projection_withholds_empty_paragraphs_and_unattested_lists() {
        let document: EmdashDocument = serde_json::from_value(serde_json::json!({
            "route":"/test", "title":"Test", "body":[{"children":[]}]
        }))
        .unwrap();
        let projected = content_adapter_projection(&document, [true; 4], false).unwrap();
        assert!(projected.get("paragraphs").is_none());
        assert!(projected.get("lists").is_none());
    }

    #[test]
    fn wikipedia_citations_keep_literal_input_and_portable_text_positions() {
        let document: EmdashDocument = serde_json::from_value(serde_json::json!({
            "route":"/test", "title":"Test", "body":[{
                "markDefs":[{"_key":"ref", "_type":"link", "href":"#cite_note-source-17"}],
                "children":[{"_type":"span", "text":"Élan."}, {"_type":"span", "text":"[17]", "marks":["ref"]}, {"_type":"span", "text":" Next sentence."}]
            }]
        })).unwrap();
        let (input, _, _, literal) = content_input_with_context(&document);
        assert_eq!(input.text.as_deref(), Some("Élan.     Next sentence."));
        assert_eq!(literal.as_deref(), Some("Élan.[17] Next sentence."));
        let preset = ContentPreset::from_toml(
            "version=1\nname='test'\n[[gates]]\nid='length'\nkind='sentence_length'\nmax_words=1\n",
        )
        .unwrap();
        let report = evaluate_content(&document, &preset).unwrap();
        assert_eq!(report.gates[0].measurements["sentence_count"], 2);
        let locations = source_locations(&document, &preset, &report);
        assert_eq!(locations.len(), 1);
        assert_eq!((locations[0].span_index, locations[0].utf16_offset), (2, 1));
    }

    #[test]
    fn arabic_reference_digits_keep_portable_text_positions_and_literal_text() {
        let document: EmdashDocument = serde_json::from_value(serde_json::json!({
            "route":"/test", "title":"Test", "body":[{
                "markDefs":[{"_key":"ref", "_type":"link", "href":"#reference-proof"}],
                "children":[
                    {"_type":"span", "text":"First."},
                    {"_type":"span", "text":"[١", "marks":["ref"]},
                    {"_type":"span", "text":"۲، ٣]", "marks":["ref", "strong"]},
                    {"_type":"span", "text":" Next sentence."}
                ]
            }]
        }))
        .unwrap();
        let (input, _, _, literal) = content_input_with_context(&document);
        assert!(!input.text.as_deref().unwrap().contains("١"));
        assert_eq!(literal.as_deref(), Some("First.[١۲، ٣] Next sentence."));
        let preset = ContentPreset::from_toml("version=1\nname='test'\n[[gates]]\nid='literal'\nkind='required_terms'\nterms=['[١۲، ٣]']\n[[gates]]\nid='length'\nkind='sentence_length'\nmax_words=1\n").unwrap();
        let report = evaluate_content(&document, &preset).unwrap();
        assert_eq!(report.gates[0].status, GateStatus::Passed);
        assert_eq!(report.gates[1].measurements["sentence_count"], 2);
        let locations = source_locations(&document, &preset, &report);
        assert_eq!((locations[0].span_index, locations[0].utf16_offset), (3, 1));
    }

    #[test]
    fn portable_text_decimals_preserve_sentence_and_span_positions() {
        for (first, second) in [("٣.", "١٤"), ("۳.", "۱۴")] {
            let document: EmdashDocument = serde_json::from_value(serde_json::json!({
                "route":"/test", "title":"Test", "body":[{"children":[
                    {"_type":"span", "text":format!("Value {first}")},
                    {"_type":"span", "text":format!("{second} units. ")},
                    {"_type":"span", "text":"Next sentence."}
                ]}]
            }))
            .unwrap();
            let preset = ContentPreset::from_toml(
                "version=1\nname='test'\n[[gates]]\nid='length'\nkind='sentence_character_length'\nmax_chars=5\n",
            ).unwrap();
            let report = evaluate_content(&document, &preset).unwrap();
            assert_eq!(report.gates[0].measurements["sentence_count"], 2);
            let locations = source_locations(&document, &preset, &report);
            assert_eq!(locations.len(), 2);
            assert_eq!((locations[1].span_index, locations[1].utf16_offset), (2, 0));
        }
    }

    #[test]
    fn portable_text_urls_after_punctuation_keep_sentence_and_span_positions() {
        let document: EmdashDocument = serde_json::from_value(serde_json::json!({
            "route":"/test", "title":"Test", "body":[{"children":[
                {"_type":"span", "text":"رابط،https://example"},
                {"_type":"span", "text":".org/v1.2?x=1. "},
                {"_type":"span", "text":"Next sentence."}
            ]}]
        }))
        .unwrap();
        let preset = ContentPreset::from_toml(
            "version=1\nname='test'\n[[gates]]\nid='length'\nkind='sentence_character_length'\nmax_chars=5\n",
        ).unwrap();
        let report = evaluate_content(&document, &preset).unwrap();
        assert_eq!(report.gates[0].measurements["sentence_count"], 2);
        let locations = source_locations(&document, &preset, &report);
        assert_eq!(locations.len(), 2);
        assert_eq!((locations[1].span_index, locations[1].utf16_offset), (2, 0));
    }

    #[test]
    fn segmented_reference_labels_are_masked_only_when_entire_link_is_numeric() {
        let document: EmdashDocument = serde_json::from_value(serde_json::json!({
            "route":"/test", "title":"Test", "body":[{
                "markDefs":[{"_key":"ref", "_type":"link", "href":"#reference-source"}],
                "children":[
                    {"_type":"span", "text":"Élan."},
                    {"_type":"span", "text":"[", "marks":["ref"]},
                    {"_type":"span", "text":"17", "marks":["ref", "strong"]},
                    {"_type":"span", "text":"]", "marks":["ref"]},
                    {"_type":"span", "text":" Next. "},
                    {"_type":"span", "text":"Source ", "marks":["ref"]},
                    {"_type":"span", "text":"2025", "marks":["ref"]}
                ]
            }]
        }))
        .unwrap();
        let (input, _, _, literal) = content_input_with_context(&document);
        assert_eq!(input.text.as_deref(), Some("Élan.     Next. Source 2025"));
        assert_eq!(literal.as_deref(), Some("Élan.[17] Next. Source 2025"));
        let preset = ContentPreset::from_toml("version=1\nname='test'\n[[gates]]\nid='literal'\nkind='required_terms'\nterms=['[17]', '2025']\n[[gates]]\nid='length'\nkind='sentence_length'\nmax_words=20\n").unwrap();
        let report = evaluate_content(&document, &preset).unwrap();
        assert_eq!(report.gates[0].status, GateStatus::Passed);
        assert_eq!(report.gates[1].measurements["sentence_count"], 3);
    }

    #[test]
    fn overlapping_links_keep_the_entire_ambiguous_label() {
        let document: EmdashDocument = serde_json::from_value(serde_json::json!({
            "route":"/test", "title":"Test", "body":[{
                "markDefs":[
                    {"_key":"ref", "_type":"link", "href":"#reference-source"},
                    {"_key":"other", "_type":"link", "href":"/elsewhere"}
                ],
                "children":[
                    {"_type":"span", "text":"Source ", "marks":["ref", "other"]},
                    {"_type":"span", "text":"2025", "marks":["ref"]}
                ]
            }]
        }))
        .unwrap();
        assert_eq!(
            content_input(&document).text.as_deref(),
            Some("Source 2025")
        );
    }

    #[test]
    fn external_results_bind_to_exact_numeric_reference_labels() {
        use aexeo_core::content_extensions::{ExternalGateResult, ExternalGateResults};
        let make_document = |label: &str| -> EmdashDocument {
            serde_json::from_value(serde_json::json!({"route":"/test","title":"Test","body":[{"markDefs":[{"_key":"r","_type":"link","href":"#reference-source"}],"children":[{"_type":"span","text":"Élan."},{"_type":"span","text":label,"marks":["r"]},{"_type":"span","text":" Next."}]}]})).unwrap()
        };
        let first = make_document("[1]");
        let second = make_document("[2]");
        let capabilities = [true; 4];
        let projected = extension_input(&first, None, capabilities);
        assert_eq!(projected.text.as_deref(), Some("Élan.[1] Next."));
        assert_eq!(extension_input(&first, None, [false; 4]).text, None);
        let preset = ContentPreset::from_toml("version=1\nname='test'\n[[gates]]\nid='custom'\nkind='custom'\nextension='test.rule'\n").unwrap();
        let host = ExternalGateResults::new(vec![ExternalGateResult {
            extension: "test.rule".into(),
            parameters: BTreeMap::new(),
            input: projected,
            diagnostics: vec![],
        }])
        .unwrap();
        let current =
            evaluate_content_with_extensions(&first, &preset, None, capabilities, Some(&host))
                .unwrap();
        assert_eq!(current.gates[0].status, GateStatus::Passed);
        let stale =
            evaluate_content_with_extensions(&second, &preset, None, capabilities, Some(&host))
                .unwrap();
        assert_eq!(stale.gates[0].status, GateStatus::NotChecked);
    }

    #[test]
    fn extension_heading_only_input_preserves_reference_labels() {
        let document: EmdashDocument = serde_json::from_str(r##"{"route":"/test","title":"Test","body":[{"style":"h2","markDefs":[{"_key":"r","_type":"link","href":"#reference-source"}],"children":[{"_type":"span","text":"Heading "},{"_type":"span","text":"[1]","marks":["r"]}]}]}"##).unwrap();
        let input = extension_input(&document, None, [false, true, false, false]);
        assert_eq!(input.text, None);
        assert_eq!(input.headings.unwrap(), vec!["Heading [1]"]);
    }

    #[test]
    fn literal_gates_observe_numeric_references_while_rhythm_ignores_them() {
        let document: EmdashDocument = serde_json::from_str(r##"{"route":"/test","title":"Test","body":[{"markDefs":[{"_key":"r","_type":"link","href":"#reference-source"}],"children":[{"_type":"span","text":"Élan."},{"_type":"span","text":"[17]","marks":["r"]},{"_type":"span","text":" Next sentence."}]}]}"##).unwrap();
        let preset = ContentPreset::from_toml("version=1\nname='t'\n[[gates]]\nid='required'\nkind='required_terms'\nterms=['[17]']\n[[gates]]\nid='forbidden'\nkind='forbidden_terms'\nterms=['[17]']\n[[gates]]\nid='length'\nkind='sentence_length'\nmax_words=20\n").unwrap();
        let report = evaluate_content(&document, &preset).unwrap();
        assert_eq!(report.gates[0].status, GateStatus::Passed);
        assert_eq!(report.gates[1].status, GateStatus::Failed);
        assert_eq!(report.gates[2].measurements["sentence_count"], 2);
        let location = source_locations(&document, &preset, &report);
        assert!(
            location
                .iter()
                .any(|item| item.span_index == 1 && item.utf16_offset == 0)
        );
    }

    #[test]
    fn ordinary_numeric_links_remain_prose() {
        let document: EmdashDocument = serde_json::from_str(r##"{"route":"/test","title":"Test","body":[{"markDefs":[{"_key":"a","_type":"link","href":"#section"}],"children":[{"_type":"span","text":"42","marks":["a"]}]}]}"##).unwrap();
        assert_eq!(content_input(&document).text.as_deref(), Some("42"));
    }

    #[test]
    fn projection_preserves_empty_blocks_multispan_text_and_newlines() {
        let document: EmdashDocument = serde_json::from_str(r#"{"route":"/test","title":"Test","body":[{"style":"h2","children":[{"_type":"span","text":" A "},{"_type":"span","text":" B"}]},{"children":[]},{"children":[{"_type":"span","text":"a\n"},{"_type":"span","text":"é"}]}]}"#).unwrap();
        let (input, context, _, _) = content_input_with_context(&document);
        assert_eq!(input.text.as_deref(), Some(" A  B\n\na\né"));
        assert_eq!(
            input.headings.as_deref(),
            Some(["A B".to_string()].as_slice())
        );
        assert_eq!(context.block_ends.as_deref(), Some([5, 6, 11].as_slice()));
        let paragraphs = context.paragraphs.unwrap();
        assert_eq!((paragraphs[0].text.as_str(), paragraphs[0].line), ("", 2));
        assert_eq!(
            (paragraphs[1].text.as_str(), paragraphs[1].line),
            ("a\né", 3)
        );
    }

    #[test]
    fn paragraphs_have_projected_positions_and_exclude_headings() {
        let document: EmdashDocument = serde_json::from_str(r#"{"route":"/test","title":"Test","body":[{"style":"h2","children":[{"_type":"span","text":"Heading"}]},{"children":[{"_type":"span","text":"First\nline"}]},{"children":[{"_type":"span","text":"Second"}]}]}"#).unwrap();
        let (_, context, _, _) = content_input_with_context(&document);
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
    fn required_terms_match_across_markdown_and_emdash() {
        let preset = ContentPreset::from_toml("version=1\nname='test'\n[[gates]]\nid='terms'\nkind='required_terms'\nterms=['café', 'absent']\ncase_insensitive=true\n").unwrap();
        let document: EmdashDocument = serde_json::from_str(r#"{"route":"/test","title":"Test","body":[{"children":[{"_type":"span","text":"CAFÉ"}]}]}"#).unwrap();
        let markdown = aexeo_core::markdown_adapter::markdown_input("test.md", "CAFÉ").unwrap();
        let emdash = evaluate_content(&document, &preset).unwrap();
        let plain = preset.evaluate(&markdown).unwrap();
        assert_eq!(emdash.gates[0].status, GateStatus::Failed);
        assert_eq!(emdash.gates[0].measurements, plain.gates[0].measurements);
        assert!(source_locations(&document, &preset, &emdash).is_empty());
        assert_eq!(
            evaluate_content_with_body_availability(&document, &preset, None, false)
                .unwrap()
                .gates[0]
                .status,
            GateStatus::NotChecked
        );
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
    #[test]
    fn bound_reachability_rejects_stale_or_incomplete_snapshots_and_maps_findings() {
        use aexeo_core::content_gate::LinkReachabilityObservation::{NotChecked, NotFound};
        let raw: serde_json::Value = serde_json::from_str(r#"{"route":"/t","title":"T","body":[{"children":[{"_type":"span","text":"first","marks":["a"]},{"_type":"span","text":" second","marks":["b"]}],"markDefs":[{"_key":"a","_type":"link","href":"https://a.example"},{"_key":"b","_type":"link","href":"https://b.example"}]}]}"#).unwrap();
        let document: EmdashDocument = serde_json::from_value(raw.clone()).unwrap();
        let preset = ContentPreset::from_toml(
            "version=1\nname='links'\n[[gates]]\nid='external'\nkind='link_reachability'\n",
        )
        .unwrap();
        let mut batch = BoundLinkObservations {
            document: raw.clone(),
            destinations: content_link_destinations(&document),
            observations: vec![NotFound, NotChecked],
        };
        assert_eq!(batch.destinations.len(), 2);
        let observations = bound_link_observations(&batch, &raw, &document, true).unwrap();
        let report = evaluate_content_with_reachability(
            &document,
            &preset,
            None,
            [true; 4],
            None,
            Some(observations),
        )
        .unwrap();
        assert_eq!(report.gates[0].status, GateStatus::NotChecked);
        assert_eq!(report.gates[0].findings.len(), 1);
        let locations =
            source_locations_with_reachability(&document, &preset, &report, Some(observations));
        assert_eq!(locations.len(), 1);
        assert_eq!((locations[0].block_index, locations[0].span_index), (0, 0));
        assert!(bound_link_observations(&batch, &raw, &document, false).is_none());
        let mut changed = raw.clone();
        changed["title"] = "changed".into();
        assert!(bound_link_observations(&batch, &changed, &document, true).is_none());
        batch.destinations.reverse();
        assert!(bound_link_observations(&batch, &raw, &document, true).is_none());
        batch.destinations.reverse();
        batch.observations.pop();
        assert!(bound_link_observations(&batch, &raw, &document, true).is_none());
    }
}

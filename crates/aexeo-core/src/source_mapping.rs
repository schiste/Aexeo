//! Shared mapping of content-gate findings to adapter source positions.
use crate::content_gate::ContentInput;

/// Whether a gate's finding coordinates refer to its input prose.
pub fn prose_gate_has_positions(gate: &crate::content_gate::ContentGate) -> bool {
    use crate::content_gate::ContentGate;
    matches!(
        gate,
        ContentGate::Custom { .. }
            | ContentGate::LiteralMarkdownLinks { .. }
            | ContentGate::ForbiddenTerms { .. }
            | ContentGate::RepeatedTerms { .. }
            | ContentGate::SentenceLength { .. }
            | ContentGate::SentenceCharacterLength { .. }
            | ContentGate::ParagraphLength { .. }
            | ContentGate::RepeatedParagraphOpenings { .. }
            | ContentGate::RepeatedSentenceOpenings { .. }
            | ContentGate::SentenceOpeningWindow { .. }
            | ContentGate::SentenceOpeningTerms { .. }
            | ContentGate::RepeatedPunctuation { .. }
            | ContentGate::TrailingIngClause { .. }
            | ContentGate::SimilarSentenceLengths { .. }
            | ContentGate::SentenceLengthVariance { .. }
            | ContentGate::ProseSentenceLengthVariance { .. }
            | ContentGate::InlineTripleCandidates { .. }
            | ContentGate::DuplicateParagraphs { .. }
            | ContentGate::ParagraphCharacterLength { .. }
    )
}

/// Count emitted findings for gates that can have a source position. Findings
/// omitted by a report limit are excluded. An adapter may still be unable to
/// locate any particular finding. The report must be from this preset, in gate order.
pub fn eligible_findings(
    preset: &crate::content_gate::ContentPreset,
    report: &crate::content_gate::ContentReport,
) -> usize {
    use crate::content_gate::ContentGate;
    preset
        .gates
        .iter()
        .zip(&report.gates)
        .filter(|(gate, _)| {
            prose_gate_has_positions(gate)
                || matches!(
                    gate,
                    ContentGate::ListItemCount { .. }
                        | ContentGate::HeadingHierarchy { .. }
                        | ContentGate::HeadingLength { .. }
                        | ContentGate::DuplicateHeadings { .. }
                        | ContentGate::LinkDestinations { .. }
                        | ContentGate::LinkReachability { .. }
                )
        })
        .map(|(_, result)| result.findings.len())
        .sum()
}

/// Adapter-specific exact positions used by the shared gate mapper.
pub(crate) trait ContentSourceMap {
    fn heading_count(&self) -> usize;
    fn link_count(&self) -> usize;
    fn prose_position(
        &self,
        source: &str,
        projected: &str,
        line: usize,
        column: usize,
    ) -> Option<(usize, usize)>;
    fn heading_position(&self, source: &str, index: usize) -> Option<(usize, usize)>;
    fn link_position(&self, source: &str, index: usize) -> Option<(usize, usize)>;
}

pub(crate) fn map_heading_hierarchy_positions(
    map: &dyn ContentSourceMap,
    source: &str,
    levels: &[u8],
    initial_level: Option<u8>,
    result: &crate::content_gate::ContentGateResult,
) -> Option<Vec<Option<(usize, usize)>>> {
    if map.heading_count() != levels.len() || levels.iter().any(|level| !(1..=6).contains(level)) {
        return None;
    }
    let indices: Vec<_> =
        crate::content_gate::heading_hierarchy_indices(levels, initial_level).collect();
    if indices.len() != result.total_findings() {
        return None;
    }
    Some(
        indices
            .into_iter()
            .take(result.findings.len())
            .map(|index| map.heading_position(source, index))
            .collect(),
    )
}

pub(crate) fn map_gate_source_positions(
    map: &dyn ContentSourceMap,
    source: &str,
    input: &ContentInput,
    gate: &crate::content_gate::ContentGate,
    result: &crate::content_gate::ContentGateResult,
    destinations: Option<&[String]>,
    observations: Option<&[crate::content_gate::LinkReachabilityObservation]>,
) -> Option<Vec<Option<(usize, usize)>>> {
    use crate::content_gate::{ContentGate, heading_finding_indices};
    let heading_indices = input
        .headings
        .as_deref()
        .and_then(|headings| heading_finding_indices(gate, headings));
    if heading_indices.is_some() && input.headings.as_ref()?.len() != map.heading_count() {
        return None;
    }
    if matches!(
        gate,
        ContentGate::LinkDestinations { .. } | ContentGate::LinkReachability { .. }
    ) {
        if destinations?.len() != map.link_count() {
            return None;
        }
        if observations.is_some_and(|values| values.len() != map.link_count()) {
            return None;
        }
    }
    let size = result.findings.len();
    let mut positions = vec![None; size];
    if matches!(
        gate,
        ContentGate::LinkDestinations { .. } | ContentGate::LinkReachability { .. }
    ) {
        let indices = link_finding_indices(gate, destinations, observations)?;
        if indices.len() == result.total_findings() {
            for (position, index) in positions.iter_mut().zip(indices) {
                *position = map.link_position(source, index);
            }
        }
    } else if let Some(indices) = heading_indices {
        if indices.len() == result.total_findings() {
            for (position, index) in positions.iter_mut().zip(indices) {
                *position = map.heading_position(source, index);
            }
        }
    } else if prose_gate_has_positions(gate) {
        let projected = input.text.as_deref()?;
        for (position, finding) in positions.iter_mut().zip(&result.findings) {
            *position = map.prose_position(source, projected, finding.line, finding.column);
        }
    } else {
        return None;
    }
    Some(positions)
}

/// Indices of link observations that produce findings, in evaluation order.
pub fn link_finding_indices(
    gate: &crate::content_gate::ContentGate,
    destinations: Option<&[String]>,
    observations: Option<&[crate::content_gate::LinkReachabilityObservation]>,
) -> Option<Vec<usize>> {
    use crate::content_gate::{ContentGate, LinkReachabilityObservation, valid_link_destination};
    match gate {
        ContentGate::LinkDestinations {
            allowed_schemes,
            allow_relative,
            ..
        } => Some(
            destinations?
                .iter()
                .enumerate()
                .filter_map(|(index, value)| {
                    (!valid_link_destination(value, allowed_schemes, *allow_relative))
                        .then_some(index)
                })
                .collect(),
        ),
        ContentGate::LinkReachability { .. } => Some(
            observations?
                .iter()
                .enumerate()
                .filter_map(|(index, observation)| {
                    (*observation == LinkReachabilityObservation::NotFound).then_some(index)
                })
                .collect(),
        ),
        _ => None,
    }
}

/// A location in the exact source snapshot supplied to the mapper.
#[derive(Debug, Clone, serde::Serialize)]
pub struct IncrementalSourceLocation {
    pub gate_id: String,
    pub finding_index: usize,
    pub line: usize,
    pub column: usize,
}

/// Source mapping coverage. A digest identifies the supplied snapshot but does
/// not authenticate it. Unmappable findings are counted and never guessed.
#[derive(Debug, Clone, serde::Serialize)]
pub struct IncrementalSourceMapping {
    pub source_sha256: String,
    pub eligible: usize,
    pub mapped: usize,
    pub unmapped: usize,
    pub locations: Vec<IncrementalSourceLocation>,
}

/// Map a report against its unchanged candidate source. The caller must supply
/// the same resolved item preset, source and link observations used to evaluate
/// the report; report IDs and observation bindings are checked here, but they
/// cannot prove that a caller-supplied report was produced from these bytes.
pub fn map_incremental_report(
    preset: &crate::content_gate::ContentPreset,
    item: &crate::content_candidate::CandidateItem,
    report: &crate::content_gate::ContentReport,
    scope: crate::html_content_adapter::HtmlContentScope,
    observations: Option<&crate::content_gate_verdict::LinkObservationSnapshot>,
) -> anyhow::Result<IncrementalSourceMapping> {
    use crate::content_gate::ContentGate;
    use crate::html_content_adapter as html;
    use crate::markdown_adapter as markdown;
    use anyhow::{Context, bail};
    use sha2::{Digest, Sha256};
    if format!("{:x}", Sha256::digest(item.source.as_bytes())) != item.sha256 {
        bail!("source digest differs from candidate item");
    }
    if report.input_id != item.entity_id
        || report.preset != preset.name
        || report.gates.len() != preset.gates.len()
        || preset
            .gates
            .iter()
            .zip(&report.gates)
            .any(|(gate, result)| gate.id() != result.gate_id)
    {
        bail!("report does not match the item or resolved preset");
    }
    let eligible = eligible_findings(preset, report);
    let mut output = IncrementalSourceMapping {
        source_sha256: item.sha256.clone(),
        eligible,
        mapped: 0,
        unmapped: eligible,
        locations: Vec::new(),
    };
    if eligible == 0 {
        return Ok(output);
    }
    let mut markdown_map = None;
    let mut html_map = None;
    let mut plain_index = None;
    let mut destinations = None;
    let mut levels = None;
    let mut lists = None;
    let input = match item.media_type.as_str() {
        "text/plain" => {
            plain_index = Some(crate::text_index::LineIndex::new(&item.source));
            ContentInput {
                id: item.entity_id.clone(),
                text: Some(item.source.clone()),
                source_text: Some(item.source.clone()),
                ..Default::default()
            }
        }
        "text/markdown" => {
            let adapted = markdown::markdown_project(
                &item.entity_id,
                &item.source,
                markdown::MarkdownProjectionOptions {
                    source_map: true,
                    link_destinations: true,
                    structured_lists: true,
                },
            )
            .context("cannot project Markdown source")?;
            levels = adapted.source_map.heading_levels().map(<[u8]>::to_vec);
            lists = adapted.structured_lists;
            destinations = adapted.destinations;
            markdown_map = Some(adapted.source_map);
            adapted.input
        }
        "text/html" => {
            let project = if preset
                .gates
                .iter()
                .any(|g| matches!(g, ContentGate::LiteralMarkdownLinks { .. }))
            {
                html::html_content_with_literal_markdown_projection
            } else {
                html::html_content_with_literal_projection
            };
            let (adapted, headings, _, _) = project(&item.entity_id, &item.source, scope)
                .map_err(|reason| anyhow::anyhow!(reason))?;
            levels = Some(headings);
            destinations = Some(adapted.destinations);
            if let Some(text) = adapted.input.text.as_deref() {
                lists = html::html_structured_lists(&item.source, text, scope);
                html_map = html::html_source_map_with_scope(&item.source, text, scope);
            }
            adapted.input
        }
        _ => bail!("unsupported source media type"),
    };
    let observed = observations.and_then(|snapshot| {
        (snapshot.version == 1
            && snapshot.source_sha256 == item.sha256
            && destinations.as_ref() == Some(&snapshot.destinations)
            && snapshot.destinations.len() == snapshot.observations.len())
        .then_some(snapshot.observations.as_slice())
    });
    for (gate, result) in preset.gates.iter().zip(&report.gates) {
        let hierarchy = if let (ContentGate::HeadingHierarchy { initial_level, .. }, Some(levels)) =
            (gate, levels.as_deref())
        {
            markdown_map
                .as_ref()
                .and_then(|m| {
                    markdown::hierarchy_source_positions(
                        m,
                        &item.source,
                        levels,
                        *initial_level,
                        result,
                    )
                })
                .or_else(|| {
                    html_map.as_ref().and_then(|m| {
                        html::hierarchy_source_positions(
                            m,
                            &item.source,
                            levels,
                            *initial_level,
                            result,
                        )
                    })
                })
        } else {
            None
        };
        let positions = hierarchy
            .or_else(|| markdown::list_source_positions(lists.as_deref()?, gate, result))
            .or_else(|| {
                markdown_map.as_ref().and_then(|m| {
                    markdown::gate_source_positions(
                        m,
                        &item.source,
                        &input,
                        gate,
                        result,
                        destinations.as_deref(),
                        observed,
                    )
                })
            })
            .or_else(|| {
                html_map.as_ref().and_then(|m| {
                    html::gate_source_positions_with_links(
                        m,
                        &item.source,
                        &input,
                        gate,
                        result,
                        destinations.as_deref(),
                        observed,
                    )
                })
            })
            .or_else(|| {
                crate::plain_text_adapter::gate_source_positions(
                    plain_index.as_ref()?,
                    &item.source,
                    gate,
                    result,
                )
            });
        if let Some(positions) = positions {
            for (finding_index, position) in positions.into_iter().enumerate() {
                if let Some((line, column)) = position {
                    output.locations.push(IncrementalSourceLocation {
                        gate_id: result.gate_id.clone(),
                        finding_index,
                        line,
                        column,
                    });
                }
            }
        }
    }
    output.mapped = output.locations.len();
    output.unmapped = eligible.saturating_sub(output.mapped);
    Ok(output)
}

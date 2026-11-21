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
                    ContentGate::HeadingHierarchy { .. }
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
    use crate::content_gate::{
        ContentGate, LinkReachabilityObservation, heading_finding_indices, valid_link_destination,
    };
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
    if let ContentGate::LinkDestinations {
        allowed_schemes,
        allow_relative,
        ..
    } = gate
    {
        let values = destinations?;
        let indices: Vec<_> = values
            .iter()
            .enumerate()
            .filter_map(|(index, value)| {
                (!valid_link_destination(value, allowed_schemes, *allow_relative)).then_some(index)
            })
            .collect();
        if indices.len() == result.total_findings() {
            for (position, index) in positions.iter_mut().zip(indices) {
                *position = map.link_position(source, index);
            }
        }
    } else if matches!(gate, ContentGate::LinkReachability { .. }) {
        let indices: Vec<_> = observations?
            .iter()
            .enumerate()
            .filter_map(|(index, observation)| {
                (*observation == LinkReachabilityObservation::NotFound).then_some(index)
            })
            .collect();
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

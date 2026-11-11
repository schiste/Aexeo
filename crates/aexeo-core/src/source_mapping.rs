//! Shared mapping of content-gate findings to adapter source positions.
use crate::content_gate::ContentInput;

/// Whether a gate's finding coordinates refer to its input prose.
pub fn prose_gate_has_positions(gate: &crate::content_gate::ContentGate) -> bool {
    use crate::content_gate::ContentGate;
    matches!(
        gate,
        ContentGate::Custom { .. }
            | ContentGate::ForbiddenTerms { .. }
            | ContentGate::RepeatedTerms { .. }
            | ContentGate::SentenceLength { .. }
            | ContentGate::SentenceCharacterLength { .. }
            | ContentGate::ParagraphLength { .. }
            | ContentGate::RepeatedParagraphOpenings { .. }
            | ContentGate::DuplicateParagraphs { .. }
            | ContentGate::ParagraphCharacterLength { .. }
    )
}

/// Adapter-specific exact positions used by the shared gate mapper.
pub(crate) trait ContentSourceMap {
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
        let indices = observations?
            .iter()
            .enumerate()
            .filter_map(|(index, observation)| {
                (*observation == LinkReachabilityObservation::NotFound).then_some(index)
            });
        for (position, index) in positions.iter_mut().zip(indices) {
            *position = map.link_position(source, index);
        }
    } else if let Some(indices) = input
        .headings
        .as_deref()
        .and_then(|headings| heading_finding_indices(gate, headings))
    {
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

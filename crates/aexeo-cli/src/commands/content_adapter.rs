//! Bounded, host-supplied content observations for non-native adapters.
use aexeo_core::{
    content_gate::{ContentContext, ContentInput, ContentParagraph},
    markdown_adapter::StructuredList,
    text_index::LineIndex,
};
use anyhow::{Result, ensure};
use serde::Deserialize;

const MAX_OBSERVATIONS: usize = 100_000;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AdapterInput {
    schema: String,
    text: Option<String>,
    #[serde(default)]
    headings: Option<Vec<String>>,
    #[serde(default)]
    heading_levels: Option<Vec<u8>>,
    #[serde(default)]
    paragraphs: Option<Vec<Paragraph>>,
    #[serde(default)]
    block_ends: Option<Vec<usize>>,
    #[serde(default)]
    lists: Option<Vec<List>>,
    #[serde(default)]
    link_destinations: Option<Vec<String>>,
    #[serde(default)]
    source_map: Option<HostSourceMap>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Paragraph {
    start: usize,
    end: usize,
    #[serde(default)]
    list_item: Option<bool>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct List {
    item_count: usize,
    depth: usize,
}

/// Source bytes and byte-identical host-declared prose and heading locations.
/// Their correspondence to the actual host content remains the host's responsibility.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct HostSourceMap {
    source_text: String,
    spans: Vec<HostSourceSpan>,
    #[serde(default)]
    heading_offsets: Option<Vec<Option<usize>>>,
    #[serde(default)]
    link_offsets: Option<Vec<Option<usize>>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HostSourceSpan {
    projected_start: usize,
    projected_end: usize,
    source_start: usize,
}

impl HostSourceMap {
    fn validate(
        &self,
        projected: &str,
        headings: Option<&[String]>,
        destinations: Option<&[String]>,
    ) -> Result<()> {
        ensure!(
            self.spans.len() <= MAX_OBSERVATIONS,
            "source map observation limit exceeded"
        );
        let (mut previous_end, mut previous_source_end) = (0, 0);
        for span in &self.spans {
            ensure!(
                span.projected_start >= previous_end
                    && span.projected_end > span.projected_start
                    && span.source_start >= previous_source_end,
                "source map spans must be ordered and nonoverlapping"
            );
            let expected = projected
                .get(span.projected_start..span.projected_end)
                .ok_or_else(|| anyhow::anyhow!("invalid projected source map range"))?;
            let source_end = span
                .source_start
                .checked_add(expected.len())
                .ok_or_else(|| anyhow::anyhow!("invalid source map range"))?;
            ensure!(
                self.source_text.get(span.source_start..source_end) == Some(expected),
                "source map bytes do not match projected text"
            );
            previous_end = span.projected_end;
            previous_source_end = source_end;
        }
        if let Some(offsets) = &self.heading_offsets {
            let headings =
                headings.ok_or_else(|| anyhow::anyhow!("heading offsets require headings"))?;
            ensure!(
                offsets.len() == headings.len(),
                "heading offsets must match headings"
            );
            let mut previous_end = 0;
            for (offset, heading) in offsets.iter().zip(headings) {
                if let Some(start) = offset {
                    let end = start
                        .checked_add(heading.len())
                        .ok_or_else(|| anyhow::anyhow!("invalid heading source range"))?;
                    ensure!(
                        !heading.is_empty()
                            && *start >= previous_end
                            && self.source_text.get(*start..end) == Some(heading.as_str()),
                        "heading source bytes must match ordered nonoverlapping headings"
                    );
                    previous_end = end;
                }
            }
        }
        if let Some(offsets) = &self.link_offsets {
            let destinations = destinations
                .ok_or_else(|| anyhow::anyhow!("link offsets require link destinations"))?;
            ensure!(
                offsets.len() == destinations.len(),
                "link offsets must match link destinations"
            );
            let mut previous_end = 0;
            for (offset, destination) in offsets.iter().zip(destinations) {
                if let Some(start) = offset {
                    let end = start
                        .checked_add(destination.len())
                        .ok_or_else(|| anyhow::anyhow!("invalid link source range"))?;
                    ensure!(
                        !destination.is_empty()
                            && *start >= previous_end
                            && self.source_text.get(*start..end) == Some(destination.as_str()),
                        "link source bytes must match ordered nonoverlapping destinations"
                    );
                    previous_end = end;
                }
            }
        }
        Ok(())
    }

    pub(super) fn link_positions(
        &self,
        source_index: &LineIndex,
        destinations: Option<&[String]>,
        observations: Option<&[aexeo_core::content_gate::LinkReachabilityObservation]>,
        gate: &aexeo_core::content_gate::ContentGate,
        result: &aexeo_core::content_gate::ContentGateResult,
    ) -> Option<Vec<Option<(usize, usize)>>> {
        if result.findings.is_empty() {
            return None;
        }
        let offsets = self.link_offsets.as_deref()?;
        let destinations = destinations?;
        if offsets.len() != destinations.len()
            || observations.is_some_and(|values| values.len() != destinations.len())
        {
            return None;
        }
        let indices = aexeo_core::source_mapping::link_finding_indices(
            gate,
            Some(destinations),
            observations,
        )?;
        if indices.len() != result.total_findings() {
            return None;
        }
        Some(
            indices
                .into_iter()
                .take(result.findings.len())
                .map(|index| {
                    source_index.position(&self.source_text, offsets.get(index).copied().flatten()?)
                })
                .collect(),
        )
    }

    pub(super) fn has_link_offsets(&self) -> bool {
        self.link_offsets.is_some()
    }
    pub(super) fn link_offset_count(&self) -> usize {
        self.link_offsets.as_ref().map_or(0, |offsets| {
            offsets.iter().filter(|offset| offset.is_some()).count()
        })
    }

    pub(super) fn heading_positions(
        &self,
        source_index: &LineIndex,
        headings: Option<&[String]>,
        levels: Option<&[u8]>,
        gate: &aexeo_core::content_gate::ContentGate,
        result: &aexeo_core::content_gate::ContentGateResult,
    ) -> Option<Vec<Option<(usize, usize)>>> {
        use aexeo_core::content_gate::{
            ContentGate, heading_finding_indices, heading_hierarchy_indices,
        };
        if result.findings.is_empty() {
            return None;
        }
        let offsets = self.heading_offsets.as_deref()?;
        let headings = headings?;
        let indices: Vec<usize> = if let ContentGate::HeadingHierarchy { initial_level, .. } = gate
        {
            heading_hierarchy_indices(levels?, *initial_level).collect()
        } else {
            heading_finding_indices(gate, headings)?
        };
        if offsets.len() != headings.len() || indices.len() != result.total_findings() {
            return None;
        }
        Some(
            indices
                .into_iter()
                .take(result.findings.len())
                .map(|index| {
                    source_index.position(&self.source_text, offsets.get(index).copied().flatten()?)
                })
                .collect(),
        )
    }

    pub(super) fn has_heading_offsets(&self) -> bool {
        self.heading_offsets.is_some()
    }

    pub(super) fn heading_offset_count(&self) -> usize {
        self.heading_offsets.as_ref().map_or(0, |offsets| {
            offsets.iter().filter(|offset| offset.is_some()).count()
        })
    }

    pub(super) fn gate_positions(
        &self,
        projected: &str,
        projected_index: &LineIndex,
        source_index: &LineIndex,
        gate: &aexeo_core::content_gate::ContentGate,
        result: &aexeo_core::content_gate::ContentGateResult,
    ) -> Option<Vec<Option<(usize, usize)>>> {
        if !aexeo_core::source_mapping::prose_gate_has_positions(gate) {
            return None;
        }
        Some(
            result
                .findings
                .iter()
                .map(|finding| {
                    let offset = projected_index.offset(projected, finding.line, finding.column)?;
                    let index = self
                        .spans
                        .partition_point(|span| span.projected_start <= offset)
                        .checked_sub(1)?;
                    let span = self.spans.get(index)?;
                    if offset >= span.projected_end {
                        return None;
                    }
                    source_index.position(
                        &self.source_text,
                        span.source_start + offset - span.projected_start,
                    )
                })
                .collect(),
        )
    }

    pub(super) fn source_index(&self) -> LineIndex {
        LineIndex::new(&self.source_text)
    }
    pub(super) fn span_count(&self) -> usize {
        self.spans.len()
    }
}

pub(super) struct AdapterProjection {
    pub input: ContentInput,
    pub context: ContentContext,
    pub headings: Option<Vec<u8>>,
    pub lists: Option<Vec<StructuredList>>,
    pub paragraph_list_items: Option<Vec<bool>>,
    pub destinations: Option<Vec<String>>,
    pub source_map: Option<HostSourceMap>,
}

pub(super) fn parse(id: String, data: &str) -> Result<AdapterProjection> {
    let value =
        aexeo_contracts::content_gate_change_validation::parse_unique_json_value(data.as_bytes())?;
    let projection: AdapterInput = serde_json::from_value(value)?;
    ensure!(
        projection.schema == "aexeo.content-adapter/v1",
        "unsupported adapter schema"
    );
    ensure!(
        projection.text.is_some()
            || (projection.paragraphs.is_none()
                && projection.block_ends.is_none()
                && projection.lists.is_none()
                && projection.link_destinations.is_none()
                && projection.source_map.is_none()),
        "text-dependent observations require projected text"
    );
    if let Some(levels) = &projection.heading_levels {
        ensure!(
            projection
                .headings
                .as_ref()
                .is_some_and(|headings| headings.len() == levels.len())
                && levels.iter().all(|level| (1..=6).contains(level)),
            "heading levels require matching headings and values from 1 to 6"
        );
    }
    ensure!(
        projection
            .headings
            .as_ref()
            .is_none_or(|v| v.len() <= MAX_OBSERVATIONS)
            && projection
                .link_destinations
                .as_ref()
                .is_none_or(|v| v.len() <= MAX_OBSERVATIONS)
            && projection
                .lists
                .as_ref()
                .is_none_or(|v| v.len() <= MAX_OBSERVATIONS)
            && projection
                .paragraphs
                .as_ref()
                .is_none_or(|v| v.len() <= MAX_OBSERVATIONS)
            && projection
                .block_ends
                .as_ref()
                .is_none_or(|v| v.len() <= MAX_OBSERVATIONS),
        "adapter observation limit exceeded"
    );
    let text = projection.text.as_deref().unwrap_or("");
    if let Some(map) = &projection.source_map {
        map.validate(
            text,
            projection.headings.as_deref(),
            projection.link_destinations.as_deref(),
        )?;
    }
    if let Some(ends) = &projection.block_ends {
        ensure!(
            ends.windows(2).all(|pair| pair[0] < pair[1])
                && ends
                    .iter()
                    .all(|&end| end <= text.len() && text.is_char_boundary(end)),
            "invalid block boundaries"
        );
    }
    let mut previous_end = 0;
    let mut classified = Vec::new();
    let mut all_classified = true;
    let paragraphs = projection
        .paragraphs
        .map(|items| -> Result<_> {
            let index = LineIndex::new(text);
            let mut result = Vec::with_capacity(items.len());
            for item in items {
                ensure!(
                    item.start >= previous_end && item.end > item.start,
                    "paragraph ranges must be nonempty, ordered and nonoverlapping"
                );
                let value = text
                    .get(item.start..item.end)
                    .ok_or_else(|| anyhow::anyhow!("invalid paragraph range"))?;
                let (line, column) = index
                    .position(text, item.start)
                    .ok_or_else(|| anyhow::anyhow!("invalid paragraph position"))?;
                previous_end = item.end;
                all_classified &= item.list_item.is_some();
                classified.push(item.list_item.unwrap_or(false));
                result.push(ContentParagraph {
                    text: value.to_owned(),
                    line,
                    column,
                });
            }
            Ok(result)
        })
        .transpose()?;
    let paragraph_list_items = paragraphs
        .as_ref()
        .and_then(|_| all_classified.then_some(classified));
    let lists = projection.lists.map(|lists| {
        lists
            .into_iter()
            .map(|list| StructuredList {
                item_count: list.item_count,
                depth: list.depth,
                line: 1,
                column: 1,
            })
            .collect()
    });
    let input = ContentInput {
        id,
        source_text: projection.text.clone(),
        text: projection.text,
        headings: projection.headings,
        ..Default::default()
    };
    Ok(AdapterProjection {
        input,
        context: ContentContext {
            paragraphs,
            block_ends: projection.block_ends,
            ..Default::default()
        },
        headings: projection.heading_levels,
        lists,
        paragraph_list_items,
        destinations: projection.link_destinations,
        source_map: projection.source_map,
    })
}

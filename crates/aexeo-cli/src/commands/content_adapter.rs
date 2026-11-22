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

pub(super) struct AdapterProjection {
    pub input: ContentInput,
    pub context: ContentContext,
    pub headings: Option<Vec<u8>>,
    pub lists: Option<Vec<StructuredList>>,
    pub paragraph_list_items: Option<Vec<bool>>,
    pub destinations: Option<Vec<String>>,
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
                && projection.link_destinations.is_none()),
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
    if let Some(ends) = &projection.block_ends {
        ensure!(
            ends.windows(2).all(|pair| pair[0] < pair[1])
                && ends
                    .iter()
                    .all(|&end| end <= text.len() && text.is_char_boundary(end)),
            "invalid block boundaries"
        );
    }
    let index = projection.paragraphs.as_ref().map(|_| LineIndex::new(text));
    let mut previous_end = 0;
    let mut classified = Vec::new();
    let mut all_classified = true;
    let paragraphs = projection
        .paragraphs
        .map(|items| -> Result<_> {
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
                    .as_ref()
                    .expect("paragraph index")
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
    })
}

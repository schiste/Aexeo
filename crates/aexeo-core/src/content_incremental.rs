//! In-process incremental evaluation with a deliberately narrow proof scope.
//!
//! The opaque baseline owns the exact preset and snapshot evaluated by this
//! process. It cannot be deserialized or supplied as an arbitrary cache.
use std::collections::{BTreeMap, BTreeSet};

use aexeo_contracts::content_gate_protocol::Change;
use serde::Serialize;

use crate::{
    content_candidate::{
        CandidateItem, CandidateOverlay, CandidateSnapshot, ValidatedCandidateSnapshot,
    },
    content_gate::{
        ContentEvaluationContext, ContentGate, ContentInput, ContentPreset, ContentReport,
        GateStatus,
    },
};

/// An unavailable guarantee; this is not a passing result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NotChecked {
    pub reason: String,
}

fn unavailable(reason: impl Into<String>) -> NotChecked {
    NotChecked {
        reason: reason.into(),
    }
}

fn checked_overlay(
    snapshot: &ValidatedCandidateSnapshot,
    change: &Change,
    after: &[CandidateItem],
) -> Result<CandidateOverlay, NotChecked> {
    if change
        .entries
        .iter()
        .flat_map(|entry| [entry.before.as_ref(), entry.after.as_ref()])
        .flatten()
        .any(|state| state.inline.is_none())
    {
        return Err(unavailable("only inline change content is supported"));
    }
    snapshot.overlay(change, after).map_err(unavailable)
}

// Validate the overlay once, then compare every referenced state with the
// corresponding immutable candidate bytes using the caller's confined source.
fn checked_overlay_with_sources<P: crate::content_gate_sources::ConfinedContentFiles>(
    snapshot: &ValidatedCandidateSnapshot,
    change: &Change,
    after: &[CandidateItem],
    provider: &mut P,
    max_state_bytes: usize,
    max_total_bytes: usize,
) -> Result<CandidateOverlay, NotChecked> {
    let overlay = snapshot.overlay(change, after).map_err(unavailable)?;
    crate::content_gate_sources::verify_candidate_referenced_states_after_overlay(
        change,
        snapshot,
        after,
        provider,
        max_state_bytes,
        max_total_bytes,
    )
    .map_err(|error| unavailable(error.to_string()))?;
    Ok(overlay)
}

/// Host-supplied evidence per entity. Neither authenticity nor freshness is established.
pub type IncrementalEvidence = BTreeMap<String, BTreeMap<String, String>>;

/// Reports evaluated by this binary from an owned base and owned preset.
/// Construct once and reuse for multiple proposals against the same base.
/// The cache is intentionally not serializable and has no mutable accessors.
pub struct IncrementalBaseline {
    snapshot: ValidatedCandidateSnapshot,
    preset: ContentPreset,
    reports: BTreeMap<String, ContentReport>,
    mode: ProjectionMode,
    evidence: Option<IncrementalEvidence>,
    metadata_dependent: bool,
    base_metadata: Option<BTreeMap<String, Option<BTreeMap<String, String>>>>,
    link_observations:
        Option<BTreeMap<String, crate::content_gate_verdict::LinkObservationSnapshot>>,
    partial_links: bool,
    partial_results: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ProjectionMode {
    Plain,
    ParagraphText,
    Adapted(crate::html_content_adapter::HtmlContentScope),
}

impl ProjectionMode {
    fn supports(self, media_type: &str) -> bool {
        media_type == "text/plain"
            || (matches!(self, Self::Adapted(_))
                && (matches!(media_type, "text/markdown" | "text/html")
                    || media_type == crate::content_adapter::MEDIA_TYPE))
    }
}

/// Reports for supported independent item gates. In explicitly partial mode,
/// reports can contain unchecked gates; callers must inspect their status.
#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum IncrementalEvaluation {
    Evaluated {
        change_id: String,
        reports: BTreeMap<String, ContentReport>,
        /// Number of candidate entities re-evaluated in this request.
        evaluated: usize,
        /// Number of unchanged candidate reports reused from the baseline.
        reused: usize,
    },
    NotChecked {
        reason: String,
    },
}

// Inventories are validated at baseline construction or by the candidate
// overlay. This check only covers scope and mode-specific applicability.
fn validate_scope(snapshot: &CandidateSnapshot, mode: ProjectionMode) -> Result<(), NotChecked> {
    if snapshot.scope.is_empty() || snapshot.site_id.is_empty() || snapshot.base.revision.is_empty()
    {
        return Err(unavailable("snapshot scope or base is unavailable"));
    }
    if !snapshot.complete {
        return Err(unavailable("snapshot completeness is not attested"));
    }
    if snapshot
        .items
        .iter()
        .any(|item| !mode.supports(&item.media_type))
    {
        return Err(unavailable(
            "source media type is not supported by this projection mode",
        ));
    }
    Ok(())
}

fn is_item_metadata_gate(gate: &ContentGate) -> bool {
    matches!(
        gate,
        ContentGate::RequiredMetadata { .. }
            | ContentGate::MetadataLength { .. }
            | ContentGate::MetadataAge { .. }
    )
}

fn is_evidence_gate(gate: &ContentGate) -> bool {
    matches!(
        gate,
        ContentGate::EditorialPacing { .. }
            | ContentGate::EvidenceAnchors { .. }
            | ContentGate::StructuredFact { .. }
            | ContentGate::EvidenceReview { .. }
            | ContentGate::EvidenceReviewAge { .. }
    )
}

fn validate_preset(
    preset: &ContentPreset,
    mode: ProjectionMode,
    has_metadata: bool,
    has_evidence: bool,
) -> Result<(), NotChecked> {
    preset
        .validate()
        .map_err(|_| unavailable("invalid content preset"))?;
    if preset.gates.iter().any(|gate| {
        let text_gate = matches!(
            gate,
            ContentGate::FinalNewline { .. }
                | ContentGate::WordCount { .. }
                | ContentGate::CharacterCount { .. }
                | ContentGate::SentenceLength {
                    paragraphs_only: false,
                    ..
                }
                | ContentGate::SentenceCharacterLength {
                    paragraphs_only: false,
                    ..
                }
                | ContentGate::RepeatedTerms { .. }
                | ContentGate::RequiredTerms { .. }
                | ContentGate::ForbiddenTerms { .. }
        );
        let paragraph_gate = mode != ProjectionMode::Plain
            && matches!(
                gate,
                ContentGate::ParagraphCount { .. }
                    | ContentGate::ParagraphLength { .. }
                    | ContentGate::ParagraphCharacterLength { .. }
                    | ContentGate::RepeatedParagraphOpenings { .. }
                    | ContentGate::RepeatedSentenceOpenings { .. }
                    | ContentGate::SentenceOpeningWindow { .. }
                    | ContentGate::SentenceOpeningTerms { .. }
                    | ContentGate::RepeatedPunctuation { .. }
                    | ContentGate::TrailingIngClause { .. }
                    | ContentGate::SimilarSentenceLengths { .. }
                    | ContentGate::SentenceLengthVariance { .. }
                    | ContentGate::InlineTripleCandidates { .. }
                    | ContentGate::DuplicateParagraphs { .. }
                    | ContentGate::SentenceLength { .. }
                    | ContentGate::SentenceCharacterLength { .. }
            );
        let structured_gate = matches!(mode, ProjectionMode::Adapted(_))
            && matches!(
                gate,
                ContentGate::ProseSentenceLengthVariance { .. }
                    | ContentGate::LiteralMarkdownLinks { .. }
                    | ContentGate::ListItemCount { .. }
                    | ContentGate::LinkCount { .. }
                    | ContentGate::LinkDestinations { .. }
                    | ContentGate::LinkReachability { .. }
                    | ContentGate::RequiredHeadings { .. }
                    | ContentGate::HeadingLength { .. }
                    | ContentGate::HeadingHierarchy { .. }
                    | ContentGate::DuplicateHeadings { .. }
            );
        let metadata_gate = has_metadata && is_item_metadata_gate(gate);
        let evidence_gate = has_evidence && is_evidence_gate(gate);
        !text_gate && !paragraph_gate && !structured_gate && !metadata_gate && !evidence_gate
    }) {
        return Err(unavailable(
            "preset contains gates outside the supported independent item scope",
        ));
    }
    Ok(())
}

fn evaluate_item(
    preset: &ContentPreset,
    item: &CandidateItem,
    mode: ProjectionMode,
    metadata: Option<&BTreeMap<String, String>>,
    link_snapshot: Option<&crate::content_gate_verdict::LinkObservationSnapshot>,
    evidence: Option<&BTreeMap<String, String>>,
    partial_results: bool,
) -> Result<ContentReport, NotChecked> {
    let classify_prose = preset
        .gates
        .iter()
        .any(|gate| matches!(gate, ContentGate::ProseSentenceLengthVariance { .. }));
    let wants_heading_levels = preset
        .gates
        .iter()
        .any(|gate| matches!(gate, ContentGate::HeadingHierarchy { .. }));
    let wants_destinations = preset.gates.iter().any(|gate| {
        matches!(
            gate,
            ContentGate::LinkDestinations { .. } | ContentGate::LinkReachability { .. }
        )
    });
    let wants_literal = preset.gates.iter().any(|gate| {
        matches!(
            gate,
            ContentGate::RepeatedTerms { .. }
                | ContentGate::RequiredTerms { .. }
                | ContentGate::ForbiddenTerms { .. }
        )
    });
    let mut literal_projection = None;
    let mut destinations = None;
    let mut heading_levels = None;
    let mut paragraph_list_items = None;
    let wants_lists = preset
        .gates
        .iter()
        .any(|gate| matches!(gate, ContentGate::ListItemCount { .. }));
    let mut structured_lists = None;
    let (mut input, context, link_count) = match (mode, item.media_type.as_str()) {
        (ProjectionMode::Adapted(_), crate::content_adapter::MEDIA_TYPE) => {
            let adapted = crate::content_adapter::parse(item.entity_id.clone(), &item.source)
                .map_err(|_| unavailable("host adapter projection is invalid"))?;
            if adapted.input.metadata.is_some() && preset.gates.iter().any(is_item_metadata_gate) {
                return Err(unavailable(
                    "embedded host metadata is not supported for incremental metadata gates",
                ));
            }
            literal_projection = adapted.literal_text;
            structured_lists = adapted.lists;
            heading_levels = adapted.headings;
            paragraph_list_items = adapted.paragraph_list_items;
            let count = adapted.destinations.as_ref().map(Vec::len);
            destinations = adapted.destinations;
            (adapted.input, Some(adapted.context), count)
        }
        (ProjectionMode::Adapted(_), "text/markdown") => {
            let adapted = crate::markdown_adapter::markdown_project(
                &item.entity_id,
                &item.source,
                crate::markdown_adapter::MarkdownProjectionOptions {
                    source_map: classify_prose
                        || wants_heading_levels
                        || wants_literal
                        || wants_destinations,
                    link_destinations: wants_destinations,
                    structured_lists: wants_lists,
                },
            )
            .map_err(|_| unavailable("Markdown projection failed"))?;
            structured_lists = adapted.structured_lists;
            if wants_literal {
                literal_projection = adapted.source_map.literal_projection().map(str::to_owned);
            }
            if classify_prose {
                paragraph_list_items = adapted
                    .source_map
                    .paragraph_list_items()
                    .map(<[bool]>::to_vec);
            }
            if wants_heading_levels {
                heading_levels = adapted.source_map.heading_levels().map(<[u8]>::to_vec);
            }
            let count = adapted
                .source_map
                .links_complete()
                .then_some(adapted.link_count);
            destinations = adapted.destinations;
            (adapted.input, Some(adapted.context), count)
        }
        (ProjectionMode::Adapted(html_scope), "text/html") => {
            let project = if preset
                .gates
                .iter()
                .any(|gate| matches!(gate, ContentGate::LiteralMarkdownLinks { .. }))
            {
                crate::html_content_adapter::html_content_with_literal_markdown_projection
            } else {
                crate::html_content_adapter::html_content_with_literal_projection
            };
            match project(&item.entity_id, &item.source, html_scope) {
                Ok((adapted, levels, lists, literal_text)) => {
                    if wants_lists {
                        structured_lists = adapted.input.text.as_deref().and_then(|projected| {
                            crate::html_content_adapter::html_structured_lists(
                                &item.source,
                                projected,
                                html_scope,
                            )
                        });
                    }
                    literal_projection = literal_text;
                    paragraph_list_items = lists;
                    heading_levels = Some(levels);
                    let count = adapted.destinations.len();
                    if wants_destinations {
                        destinations = Some(adapted.destinations);
                    }
                    (adapted.input, Some(adapted.context), Some(count))
                }
                Err(reason) if partial_results && link_snapshot.is_none() => (
                    ContentInput {
                        id: item.entity_id.clone(),
                        ..Default::default()
                    },
                    Some(crate::content_gate::ContentContext {
                        input_limit: Some(reason.into()),
                        ..Default::default()
                    }),
                    None,
                ),
                Err(reason) => return Err(unavailable(reason)),
            }
        }
        (_, "text/plain") => {
            let paragraphs = mode != ProjectionMode::Plain;
            let needs_source = paragraphs
                || preset
                    .gates
                    .iter()
                    .any(|gate| matches!(gate, ContentGate::FinalNewline { .. }));
            let input = ContentInput {
                id: item.entity_id.clone(),
                text: Some(item.source.clone()),
                source_text: needs_source.then(|| item.source.clone()),
                ..Default::default()
            };
            let context = if paragraphs {
                Some(
                    crate::plain_text_adapter::paragraph_text_context(&item.source)
                        .map_err(unavailable)?,
                )
            } else {
                None
            };
            (input, context, None)
        }
        _ => {
            return Err(unavailable(
                "source media type is not supported by this projection mode",
            ));
        }
    };
    input.metadata = metadata.cloned();
    input.evidence_sources = evidence.cloned();
    if let Some(snapshot) = link_snapshot
        && (snapshot.version != 1
            || snapshot.source_sha256 != item.sha256
            || destinations.as_ref() != Some(&snapshot.destinations)
            || snapshot.destinations.len() != snapshot.observations.len())
    {
        return Err(unavailable(
            "link observation snapshot does not match the item",
        ));
    }
    let capabilities = ContentEvaluationContext {
        link_reachability: link_snapshot.map(|snapshot| snapshot.observations.as_slice()),
        adapter: context.as_ref(),
        link_count,
        ..Default::default()
    };
    let report = preset
        .evaluate_with_literal_projection(
            &input,
            &capabilities,
            destinations.as_deref(),
            heading_levels.as_deref(),
            true,
            None,
            structured_lists.as_deref(),
            paragraph_list_items.as_deref(),
            literal_projection.as_deref(),
        )
        .map_err(|_| unavailable("content evaluation failed"))?;
    if !partial_results
        && report
            .gates
            .iter()
            .any(|gate| gate.status == GateStatus::NotChecked)
    {
        return Err(unavailable("a required item gate was not checked"));
    }
    Ok(report)
}

fn validate_partial_links(
    snapshot: &CandidateSnapshot,
    observed: &BTreeMap<String, crate::content_gate_verdict::LinkObservationSnapshot>,
    unavailable_ids: &BTreeSet<String>,
) -> Result<(), NotChecked> {
    if observed.len().checked_add(unavailable_ids.len()) != Some(snapshot.items.len())
        || snapshot.items.iter().any(|item| {
            observed.contains_key(&item.entity_id) == unavailable_ids.contains(&item.entity_id)
        })
    {
        return Err(unavailable(
            "partial link inventory does not match item identities",
        ));
    }
    Ok(())
}

impl IncrementalBaseline {
    /// Evaluate all base entities once. A host's completeness and link
    /// projection claims are prerequisites, not independently proven facts.
    pub fn prepare(snapshot: CandidateSnapshot, preset: ContentPreset) -> Result<Self, NotChecked> {
        Self::prepare_with_mode(snapshot, preset, ProjectionMode::Plain, None, None, None)
    }

    /// Interpret blank-line-separated plain text as paragraphs using the same
    /// adapter as content-change. This explicit choice is retained in the
    /// opaque baseline and applied identically to every candidate item.
    pub fn prepare_paragraph_text(
        snapshot: CandidateSnapshot,
        preset: ContentPreset,
    ) -> Result<Self, NotChecked> {
        Self::prepare_with_mode(
            snapshot,
            preset,
            ProjectionMode::ParagraphText,
            None,
            None,
            None,
        )
    }

    /// Project Markdown and static HTML using the shared adapters; plain text
    /// uses blank-line-separated paragraph semantics. Missing capabilities fail closed.
    pub fn prepare_adapted(
        snapshot: CandidateSnapshot,
        preset: ContentPreset,
    ) -> Result<Self, NotChecked> {
        Self::prepare_adapted_with_html_scope(
            snapshot,
            preset,
            crate::html_content_adapter::HtmlContentScope::Document,
        )
    }

    /// Add explicit complete-or-unavailable metadata for every base identity.
    /// The host must establish that each projection belongs to the base revision.
    /// A map is an observed full projection (including absent keys); None means unavailable.
    pub fn prepare_adapted_with_metadata(
        snapshot: CandidateSnapshot,
        metadata: BTreeMap<String, Option<BTreeMap<String, String>>>,
        preset: ContentPreset,
    ) -> Result<Self, NotChecked> {
        Self::prepare_adapted_with_metadata_and_html_scope(
            snapshot,
            metadata,
            preset,
            crate::html_content_adapter::HtmlContentScope::Document,
        )
    }

    /// Select the HTML projection scope explicitly for this baseline and all
    /// candidate evaluations. Markdown and plain text keep their adapted modes.
    /// Missing or ambiguous main elements are not checked.
    pub fn prepare_adapted_with_html_scope(
        snapshot: CandidateSnapshot,
        preset: ContentPreset,
        html_scope: crate::html_content_adapter::HtmlContentScope,
    ) -> Result<Self, NotChecked> {
        Self::prepare_with_mode(
            snapshot,
            preset,
            ProjectionMode::Adapted(html_scope),
            None,
            None,
            None,
        )
    }

    /// Select an HTML scope together with host-provided per-item metadata.
    pub fn prepare_adapted_with_metadata_and_html_scope(
        snapshot: CandidateSnapshot,
        metadata: BTreeMap<String, Option<BTreeMap<String, String>>>,
        preset: ContentPreset,
        html_scope: crate::html_content_adapter::HtmlContentScope,
    ) -> Result<Self, NotChecked> {
        Self::prepare_with_mode(
            snapshot,
            preset,
            ProjectionMode::Adapted(html_scope),
            Some(metadata),
            None,
            None,
        )
    }

    /// Prepare an adapted baseline with explicit source-bound observations for every item.
    /// The host is responsible for their provenance and freshness.
    pub fn prepare_adapted_with_link_observations(
        snapshot: CandidateSnapshot,
        preset: ContentPreset,
        html_scope: crate::html_content_adapter::HtmlContentScope,
        metadata: Option<BTreeMap<String, Option<BTreeMap<String, String>>>>,
        observations: BTreeMap<String, crate::content_gate_verdict::LinkObservationSnapshot>,
    ) -> Result<Self, NotChecked> {
        Self::prepare_with_mode(
            snapshot,
            preset,
            ProjectionMode::Adapted(html_scope),
            metadata,
            Some(observations),
            None,
        )
    }

    /// Prepare a baseline with a complete per-entity evidence inventory. The host
    /// must verify provenance, freshness, and revision consistency independently.
    pub fn prepare_adapted_with_evidence(
        snapshot: CandidateSnapshot,
        preset: ContentPreset,
        html_scope: crate::html_content_adapter::HtmlContentScope,
        metadata: Option<BTreeMap<String, Option<BTreeMap<String, String>>>>,
        observations: Option<
            BTreeMap<String, crate::content_gate_verdict::LinkObservationSnapshot>,
        >,
        evidence: IncrementalEvidence,
    ) -> Result<Self, NotChecked> {
        Self::prepare_with_mode(
            snapshot,
            preset,
            ProjectionMode::Adapted(html_scope),
            metadata,
            observations,
            Some(evidence),
        )
    }

    fn prepare_with_mode(
        snapshot: CandidateSnapshot,
        preset: ContentPreset,
        mode: ProjectionMode,
        metadata: Option<BTreeMap<String, Option<BTreeMap<String, String>>>>,
        link_observations: Option<
            BTreeMap<String, crate::content_gate_verdict::LinkObservationSnapshot>,
        >,
        evidence: Option<IncrementalEvidence>,
    ) -> Result<Self, NotChecked> {
        Self::prepare_with_mode_partial(
            snapshot,
            preset,
            mode,
            metadata,
            link_observations,
            evidence,
            false,
            false,
        )
    }

    /// Prepare a baseline where absent map entries are explicitly unavailable.
    /// `unavailable_ids` and `observations` must partition the entire inventory.
    /// A missing link check remains not checked, including for blocking gates.
    pub fn prepare_adapted_with_partial_links(
        snapshot: CandidateSnapshot,
        preset: ContentPreset,
        html_scope: crate::html_content_adapter::HtmlContentScope,
        metadata: Option<BTreeMap<String, Option<BTreeMap<String, String>>>>,
        observations: BTreeMap<String, crate::content_gate_verdict::LinkObservationSnapshot>,
        unavailable_ids: BTreeSet<String>,
        evidence: Option<IncrementalEvidence>,
    ) -> Result<Self, NotChecked> {
        validate_partial_links(&snapshot, &observations, &unavailable_ids)?;
        Self::prepare_with_mode_partial(
            snapshot,
            preset,
            ProjectionMode::Adapted(html_scope),
            metadata,
            Some(observations),
            evidence,
            true,
            true,
        )
    }

    /// Retain individual unchecked gates when other item results can be assessed.
    /// Inventories must still match their identities and observed links must be complete.
    /// Callers must inspect each report and must not interpret an unchecked gate as a pass.
    pub fn prepare_adapted_with_partial_results(
        snapshot: CandidateSnapshot,
        preset: ContentPreset,
        html_scope: crate::html_content_adapter::HtmlContentScope,
        metadata: Option<BTreeMap<String, Option<BTreeMap<String, String>>>>,
        observations: Option<
            BTreeMap<String, crate::content_gate_verdict::LinkObservationSnapshot>,
        >,
        evidence: Option<IncrementalEvidence>,
    ) -> Result<Self, NotChecked> {
        Self::prepare_with_mode_partial(
            snapshot,
            preset,
            ProjectionMode::Adapted(html_scope),
            metadata,
            observations,
            evidence,
            false,
            true,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn prepare_with_mode_partial(
        snapshot: CandidateSnapshot,
        preset: ContentPreset,
        mode: ProjectionMode,
        metadata: Option<BTreeMap<String, Option<BTreeMap<String, String>>>>,
        link_observations: Option<
            BTreeMap<String, crate::content_gate_verdict::LinkObservationSnapshot>,
        >,
        evidence: Option<IncrementalEvidence>,
        partial_links: bool,
        partial_results: bool,
    ) -> Result<Self, NotChecked> {
        let snapshot = ValidatedCandidateSnapshot::new(snapshot).map_err(unavailable)?;
        validate_scope(&snapshot, mode)?;
        if metadata.as_ref().is_some_and(|projections| {
            projections.len() != snapshot.items.len()
                || snapshot
                    .items
                    .iter()
                    .any(|item| !projections.contains_key(&item.entity_id))
        }) {
            return Err(unavailable(
                "metadata inventory does not match base identities",
            ));
        }
        let needs_links = preset
            .gates
            .iter()
            .any(|g| matches!(g, ContentGate::LinkReachability { .. }));
        if needs_links != link_observations.is_some() {
            return Err(unavailable(
                "link observations must be supplied exactly when the reachability gate is selected",
            ));
        }
        if let Some(observations) = &link_observations
            && !partial_links
            && (observations.len() != snapshot.items.len()
                || snapshot
                    .items
                    .iter()
                    .any(|item| !observations.contains_key(&item.entity_id)))
        {
            return Err(unavailable(
                "link observation inventory does not match base identities",
            ));
        }
        let needs_evidence = preset.gates.iter().any(is_evidence_gate);
        if needs_evidence != evidence.is_some() {
            return Err(unavailable(
                "evidence inventory must be supplied exactly when evidence gates are selected",
            ));
        }
        if let Some(map) = &evidence
            && (map.len() != snapshot.items.len()
                || snapshot
                    .items
                    .iter()
                    .any(|item| !map.contains_key(&item.entity_id)))
        {
            return Err(unavailable(
                "evidence inventory does not match base identities",
            ));
        }
        validate_preset(&preset, mode, metadata.is_some(), evidence.is_some())?;
        let metadata_dependent = preset.gates.iter().any(is_item_metadata_gate);
        let mut reports = BTreeMap::new();
        for item in &snapshot.items {
            let fields = metadata
                .as_ref()
                .and_then(|values| values.get(&item.entity_id))
                .and_then(Option::as_ref);
            reports.insert(
                item.entity_id.clone(),
                evaluate_item(
                    &preset,
                    item,
                    mode,
                    fields,
                    link_observations
                        .as_ref()
                        .and_then(|map| map.get(&item.entity_id)),
                    evidence.as_ref().and_then(|map| map.get(&item.entity_id)),
                    partial_results,
                )?,
            );
        }
        Ok(Self {
            snapshot,
            preset,
            reports,
            mode,
            evidence,
            metadata_dependent,
            base_metadata: metadata,
            link_observations,
            partial_links,
            partial_results,
        })
    }

    /// Evaluate a proposal against the *same* owned base and preset. The
    /// overlay and impact are recomputed, not accepted from the caller.
    /// Changed surviving entities are re-evaluated; reports for fully unchanged
    /// items are reused, including link neighbours. Site and collection gates are
    /// never inferred from this result. Errors fail closed as `not_checked`.
    pub fn evaluate(&self, change: &Change, after: &[CandidateItem]) -> IncrementalEvaluation {
        match self.evaluate_checked(change, after) {
            Ok(result) => result,
            Err(error) => IncrementalEvaluation::NotChecked {
                reason: error.reason,
            },
        }
    }

    /// Evaluate referenced states using a host-provided confined source.
    /// The host remains responsible for provenance, freshness and atomicity.
    pub fn evaluate_with_sources<P: crate::content_gate_sources::ConfinedContentFiles>(
        &self,
        change: &Change,
        after: &[CandidateItem],
        provider: &mut P,
        max_state_bytes: usize,
        max_total_bytes: usize,
    ) -> IncrementalEvaluation {
        let result = self
            .validate_change_media(change)
            .and_then(|()| {
                checked_overlay_with_sources(
                    &self.snapshot,
                    change,
                    after,
                    provider,
                    max_state_bytes,
                    max_total_bytes,
                )
            })
            .and_then(|overlay| self.evaluate_overlay(change, &overlay, None, None));
        match result {
            Ok(result) => result,
            Err(error) => IncrementalEvaluation::NotChecked {
                reason: error.reason,
            },
        }
    }

    /// Evaluate using an explicit complete candidate observation inventory. A changed
    /// observation forces reevaluation even when the content itself is unchanged.
    pub fn evaluate_with_link_observations(
        &self,
        change: &Change,
        after: &[CandidateItem],
        observations: &BTreeMap<String, crate::content_gate_verdict::LinkObservationSnapshot>,
    ) -> IncrementalEvaluation {
        let result = self
            .validate_change_media(change)
            .and_then(|()| checked_overlay(&self.snapshot, change, after))
            .and_then(|overlay| self.evaluate_overlay(change, &overlay, Some(observations), None));
        result.unwrap_or_else(|error| IncrementalEvaluation::NotChecked {
            reason: error.reason,
        })
    }

    /// Evaluate source-referenced changes with an explicit candidate observation inventory.
    /// The provider verifies referenced content; it does not verify the observations.
    pub fn evaluate_with_sources_and_link_observations<
        P: crate::content_gate_sources::ConfinedContentFiles,
    >(
        &self,
        change: &Change,
        after: &[CandidateItem],
        provider: &mut P,
        max_state_bytes: usize,
        max_total_bytes: usize,
        observations: &BTreeMap<String, crate::content_gate_verdict::LinkObservationSnapshot>,
    ) -> IncrementalEvaluation {
        let result = self
            .validate_change_media(change)
            .and_then(|()| {
                checked_overlay_with_sources(
                    &self.snapshot,
                    change,
                    after,
                    provider,
                    max_state_bytes,
                    max_total_bytes,
                )
            })
            .and_then(|overlay| self.evaluate_overlay(change, &overlay, Some(observations), None));
        result.unwrap_or_else(|error| IncrementalEvaluation::NotChecked {
            reason: error.reason,
        })
    }

    /// Re-evaluate when an evidence snapshot changes, including for unchanged content.
    /// The host must supply an inventory for every candidate entity and independently
    /// establish source authenticity, freshness and revision consistency.
    pub fn evaluate_with_evidence(
        &self,
        change: &Change,
        after: &[CandidateItem],
        observations: Option<
            &BTreeMap<String, crate::content_gate_verdict::LinkObservationSnapshot>,
        >,
        evidence: &IncrementalEvidence,
    ) -> IncrementalEvaluation {
        let result = self
            .validate_change_media(change)
            .and_then(|()| checked_overlay(&self.snapshot, change, after))
            .and_then(|overlay| {
                self.evaluate_overlay(change, &overlay, observations, Some(evidence))
            });
        result.unwrap_or_else(|error| IncrementalEvaluation::NotChecked {
            reason: error.reason,
        })
    }

    /// Evaluate confined referenced changes with the same complete candidate evidence inventory.
    #[allow(clippy::too_many_arguments)]
    pub fn evaluate_with_sources_and_evidence<
        P: crate::content_gate_sources::ConfinedContentFiles,
    >(
        &self,
        change: &Change,
        after: &[CandidateItem],
        provider: &mut P,
        max_state_bytes: usize,
        max_total_bytes: usize,
        observations: Option<
            &BTreeMap<String, crate::content_gate_verdict::LinkObservationSnapshot>,
        >,
        evidence: &IncrementalEvidence,
    ) -> IncrementalEvaluation {
        let result = self
            .validate_change_media(change)
            .and_then(|()| {
                checked_overlay_with_sources(
                    &self.snapshot,
                    change,
                    after,
                    provider,
                    max_state_bytes,
                    max_total_bytes,
                )
            })
            .and_then(|overlay| {
                self.evaluate_overlay(change, &overlay, observations, Some(evidence))
            });
        result.unwrap_or_else(|error| IncrementalEvaluation::NotChecked {
            reason: error.reason,
        })
    }

    fn evaluate_checked(
        &self,
        change: &Change,
        after: &[CandidateItem],
    ) -> Result<IncrementalEvaluation, NotChecked> {
        self.validate_change_media(change)?;
        let overlay = checked_overlay(&self.snapshot, change, after)?;
        self.evaluate_overlay(change, &overlay, None, None)
    }

    /// Evaluate a candidate with explicit available and unavailable link observations.
    /// The host must separately establish observation provenance and freshness.
    pub fn evaluate_with_partial_links(
        &self,
        change: &Change,
        after: &[CandidateItem],
        observations: &BTreeMap<String, crate::content_gate_verdict::LinkObservationSnapshot>,
        unavailable_ids: &BTreeSet<String>,
        evidence: Option<&IncrementalEvidence>,
    ) -> IncrementalEvaluation {
        let result = (|| {
            if !self.partial_links {
                return Err(unavailable(
                    "baseline does not accept partial link inventories",
                ));
            }
            self.validate_change_media(change)?;
            let overlay = checked_overlay(&self.snapshot, change, after)?;
            self.evaluate_partial_overlay(change, &overlay, observations, unavailable_ids, evidence)
        })();
        result.unwrap_or_else(|error| IncrementalEvaluation::NotChecked {
            reason: error.reason,
        })
    }

    /// Verify referenced content before evaluating an explicitly partial inventory.
    #[allow(clippy::too_many_arguments)]
    pub fn evaluate_with_sources_and_partial_links<
        P: crate::content_gate_sources::ConfinedContentFiles,
    >(
        &self,
        change: &Change,
        after: &[CandidateItem],
        provider: &mut P,
        max_state_bytes: usize,
        max_total_bytes: usize,
        observations: &BTreeMap<String, crate::content_gate_verdict::LinkObservationSnapshot>,
        unavailable_ids: &BTreeSet<String>,
        evidence: Option<&IncrementalEvidence>,
    ) -> IncrementalEvaluation {
        let result = (|| {
            self.validate_change_media(change)?;
            let overlay = checked_overlay_with_sources(
                &self.snapshot,
                change,
                after,
                provider,
                max_state_bytes,
                max_total_bytes,
            )?;
            self.evaluate_partial_overlay(change, &overlay, observations, unavailable_ids, evidence)
        })();
        result.unwrap_or_else(|error| IncrementalEvaluation::NotChecked {
            reason: error.reason,
        })
    }

    fn evaluate_partial_overlay(
        &self,
        change: &Change,
        overlay: &CandidateOverlay,
        observations: &BTreeMap<String, crate::content_gate_verdict::LinkObservationSnapshot>,
        unavailable_ids: &BTreeSet<String>,
        evidence: Option<&IncrementalEvidence>,
    ) -> Result<IncrementalEvaluation, NotChecked> {
        if !self.partial_links {
            return Err(unavailable(
                "baseline does not accept partial link inventories",
            ));
        }
        validate_scope(&overlay.candidate, self.mode)?;
        validate_partial_links(&overlay.candidate, observations, unavailable_ids)?;
        self.evaluate_overlay_inner(change, overlay, Some(observations), evidence)
    }

    fn validate_change_media(&self, change: &Change) -> Result<(), NotChecked> {
        if change
            .entries
            .iter()
            .any(|entry| !self.mode.supports(&entry.media_type))
        {
            return Err(unavailable(
                "change media type is not supported by this projection mode",
            ));
        }
        Ok(())
    }

    fn evaluate_overlay(
        &self,
        change: &Change,
        overlay: &CandidateOverlay,
        observations: Option<
            &BTreeMap<String, crate::content_gate_verdict::LinkObservationSnapshot>,
        >,
        evidence: Option<&IncrementalEvidence>,
    ) -> Result<IncrementalEvaluation, NotChecked> {
        validate_scope(&overlay.candidate, self.mode)?;
        if self.partial_links {
            return Err(unavailable(
                "partial link inventory requires the explicit partial evaluation API",
            ));
        }
        self.evaluate_overlay_inner(change, overlay, observations, evidence)
    }

    fn evaluate_overlay_inner(
        &self,
        change: &Change,
        overlay: &CandidateOverlay,
        observations: Option<
            &BTreeMap<String, crate::content_gate_verdict::LinkObservationSnapshot>,
        >,
        evidence: Option<&IncrementalEvidence>,
    ) -> Result<IncrementalEvaluation, NotChecked> {
        if self.link_observations.is_some() != observations.is_some() {
            return Err(unavailable(
                "candidate link observation inventory is required",
            ));
        }
        if let Some(map) = observations
            && !self.partial_links
            && (map.len() != overlay.candidate.items.len()
                || overlay
                    .candidate
                    .items
                    .iter()
                    .any(|item| !map.contains_key(&item.entity_id)))
        {
            return Err(unavailable(
                "link observation inventory does not match candidate identities",
            ));
        }
        if self.evidence.is_some() != evidence.is_some() {
            return Err(unavailable("candidate evidence inventory is required"));
        }
        if let Some(map) = evidence
            && (map.len() != overlay.candidate.items.len()
                || overlay
                    .candidate
                    .items
                    .iter()
                    .any(|item| !map.contains_key(&item.entity_id)))
        {
            return Err(unavailable(
                "evidence inventory does not match candidate identities",
            ));
        }
        // The accepted gates depend only on the item's own projected content.
        // Changed identities come from the validated change, not link coverage.
        let changed: BTreeSet<&str> = overlay.impact.changed.iter().map(String::as_str).collect();
        let baseline_items: BTreeMap<&str, &CandidateItem> = self
            .snapshot
            .items
            .iter()
            .map(|item| (item.entity_id.as_str(), item))
            .collect();
        let changed_entries: BTreeMap<&str, &aexeo_contracts::content_gate_protocol::Entry> =
            if self.metadata_dependent {
                change
                    .entries
                    .iter()
                    .filter(|entry| {
                        !matches!(
                            entry.operation,
                            aexeo_contracts::content_gate_protocol::Operation::Delete
                        )
                    })
                    .map(|entry| (entry.entity_id.as_str(), entry))
                    .collect()
            } else {
                BTreeMap::new()
            };
        let mut reports = BTreeMap::new();
        let mut evaluated = 0;
        let mut reused = 0;
        for item in &overlay.candidate.items {
            let report = if baseline_items.get(item.entity_id.as_str()).copied() == Some(item)
                && (!self.metadata_dependent || !changed.contains(item.entity_id.as_str()))
                && match (&self.evidence, evidence) {
                    (Some(base), Some(candidate)) => {
                        base.get(&item.entity_id) == candidate.get(&item.entity_id)
                    }
                    (None, None) => true,
                    _ => false,
                }
                && match (&self.link_observations, observations) {
                    (Some(base), Some(candidate)) => {
                        match (base.get(&item.entity_id), candidate.get(&item.entity_id)) {
                            (Some(a), Some(b)) => {
                                a.version == b.version
                                    && a.source_sha256 == b.source_sha256
                                    && a.destinations == b.destinations
                                    && a.observations == b.observations
                            }
                            (None, None) => true,
                            _ => false,
                        }
                    }
                    (None, None) => true,
                    _ => false,
                } {
                reused += 1;
                self.reports
                    .get(&item.entity_id)
                    .ok_or_else(|| unavailable("baseline report is unavailable"))?
                    .clone()
            } else {
                if !changed.contains(item.entity_id.as_str())
                    && baseline_items.get(item.entity_id.as_str()).copied() != Some(item)
                {
                    return Err(unavailable(
                        "baseline item differs from candidate outside the change set",
                    ));
                }
                evaluated += 1;
                let fields = if self.metadata_dependent {
                    if changed.contains(item.entity_id.as_str()) {
                        changed_entries
                            .get(item.entity_id.as_str())
                            .and_then(|entry| entry.metadata.as_ref())
                    } else {
                        self.base_metadata
                            .as_ref()
                            .and_then(|map| map.get(&item.entity_id))
                            .and_then(Option::as_ref)
                    }
                } else {
                    None
                };
                evaluate_item(
                    &self.preset,
                    item,
                    self.mode,
                    fields,
                    observations.and_then(|map| map.get(&item.entity_id)),
                    evidence.and_then(|map| map.get(&item.entity_id)),
                    self.partial_results,
                )?
            };
            reports.insert(item.entity_id.clone(), report);
        }
        Ok(IncrementalEvaluation::Evaluated {
            change_id: change.change_id.clone(),
            reports,
            evaluated,
            reused,
        })
    }
}

/// An owned base for collection-only metadata gates. The host supplies one
/// metadata projection per entity, including explicit unavailable projections.
/// The projection and its relation to the base revision are host assertions.
pub struct CollectionBaseline {
    snapshot: ValidatedCandidateSnapshot,
    metadata: BTreeMap<String, Option<BTreeMap<String, String>>>,
    preset: ContentPreset,
    fields: BTreeSet<String>,
}

/// A host-projected candidate collection that can be borrowed by the existing
/// change-verdict APIs. Completeness is inherited from the host's base claim;
/// it is not independently established by this type.
#[derive(Debug, Clone, Serialize)]
pub struct ProjectedCandidateCollection {
    change_id: String,
    complete: bool,
    items: Vec<ContentInput>,
}

impl ProjectedCandidateCollection {
    pub fn as_change_collection(&self) -> crate::content_gate_verdict::ChangeCollection<'_> {
        crate::content_gate_verdict::ChangeCollection {
            change_id: &self.change_id,
            items: &self.items,
            complete: self.complete,
        }
    }
}

/// A collection report is not a publication decision. Individual gates can
/// be not checked while still retaining findings from observed items.
#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum CollectionEvaluation {
    Report {
        change_id: String,
        report: ContentReport,
    },
    NotChecked {
        reason: String,
    },
}

impl CollectionBaseline {
    /// Own the exact base, collection projections, and preset used for future
    /// candidate evaluations. IDs must exactly match the base inventory.
    pub fn prepare(
        snapshot: CandidateSnapshot,
        metadata: BTreeMap<String, Option<BTreeMap<String, String>>>,
        preset: ContentPreset,
    ) -> Result<Self, NotChecked> {
        let snapshot = ValidatedCandidateSnapshot::new(snapshot).map_err(unavailable)?;
        if snapshot.scope.is_empty()
            || snapshot.site_id.is_empty()
            || snapshot.base.revision.is_empty()
            || !snapshot.complete
        {
            return Err(unavailable("complete snapshot scope and base are required"));
        }
        if metadata.len() != snapshot.items.len()
            || snapshot
                .items
                .iter()
                .any(|item| !metadata.contains_key(&item.entity_id))
        {
            return Err(unavailable(
                "metadata inventory does not match base identities",
            ));
        }
        preset
            .validate()
            .map_err(|_| unavailable("invalid content preset"))?;
        if preset.gates.is_empty() {
            return Err(unavailable(
                "preset contains gates outside collection metadata scope",
            ));
        }
        let mut fields = BTreeSet::new();
        for gate in &preset.gates {
            let ContentGate::UniqueCollectionMetadata {
                field, group_by, ..
            } = gate
            else {
                return Err(unavailable(
                    "preset contains gates outside collection metadata scope",
                ));
            };
            fields.insert(field.clone());
            fields.extend(group_by.iter().cloned());
        }
        Ok(Self {
            snapshot,
            metadata,
            preset,
            fields,
        })
    }

    /// Project the candidate metadata for the existing change-verdict APIs.
    /// The host must verify freshness, authenticity and completeness separately.
    pub fn project_candidate(
        &self,
        change: &Change,
        after: &[CandidateItem],
    ) -> Result<ProjectedCandidateCollection, NotChecked> {
        let overlay = checked_overlay(&self.snapshot, change, after)?;
        self.project_overlay(change, &overlay)
    }

    /// Project candidate metadata after verifying every referenced state.
    pub fn project_candidate_with_sources<P: crate::content_gate_sources::ConfinedContentFiles>(
        &self,
        change: &Change,
        after: &[CandidateItem],
        provider: &mut P,
        max_state_bytes: usize,
        max_total_bytes: usize,
    ) -> Result<ProjectedCandidateCollection, NotChecked> {
        let overlay = checked_overlay_with_sources(
            &self.snapshot,
            change,
            after,
            provider,
            max_state_bytes,
            max_total_bytes,
        )?;
        self.project_overlay(change, &overlay)
    }

    /// Evaluate collection gates with independently supplied referenced bytes.
    /// The host must establish source provenance, freshness and atomicity.
    pub fn evaluate_with_sources<P: crate::content_gate_sources::ConfinedContentFiles>(
        &self,
        change: &Change,
        after: &[CandidateItem],
        provider: &mut P,
        max_state_bytes: usize,
        max_total_bytes: usize,
    ) -> CollectionEvaluation {
        let result = checked_overlay_with_sources(
            &self.snapshot,
            change,
            after,
            provider,
            max_state_bytes,
            max_total_bytes,
        )
        .and_then(|overlay| self.evaluate_overlay(change, &overlay));
        match result {
            Ok(report) => CollectionEvaluation::Report {
                change_id: change.change_id.clone(),
                report,
            },
            Err(error) => CollectionEvaluation::NotChecked {
                reason: error.reason,
            },
        }
    }

    /// Rebuild the candidate metadata collection from the owned base and the
    /// metadata in each change entry, then run the shared collection evaluator.
    pub fn evaluate(&self, change: &Change, after: &[CandidateItem]) -> CollectionEvaluation {
        match self.evaluate_checked(change, after) {
            Ok(report) => CollectionEvaluation::Report {
                change_id: change.change_id.clone(),
                report,
            },
            Err(error) => CollectionEvaluation::NotChecked {
                reason: error.reason,
            },
        }
    }

    fn evaluate_checked(
        &self,
        change: &Change,
        after: &[CandidateItem],
    ) -> Result<ContentReport, NotChecked> {
        let overlay = checked_overlay(&self.snapshot, change, after)?;
        self.evaluate_overlay(change, &overlay)
    }

    fn project_overlay(
        &self,
        change: &Change,
        overlay: &CandidateOverlay,
    ) -> Result<ProjectedCandidateCollection, NotChecked> {
        self.project_overlay_fields(change, overlay, None)
    }

    fn project_overlay_fields(
        &self,
        change: &Change,
        overlay: &CandidateOverlay,
        fields: Option<&BTreeSet<String>>,
    ) -> Result<ProjectedCandidateCollection, NotChecked> {
        // Borrow the owned baseline and change projections until the final
        // collection is assembled; cloning the entire baseline here would
        // copy every metadata value twice for each candidate.
        let mut metadata: BTreeMap<&str, Option<&BTreeMap<String, String>>> = self
            .metadata
            .iter()
            .map(|(id, fields)| (id.as_str(), fields.as_ref()))
            .collect();
        for entry in &change.entries {
            let old_id = match entry.operation {
                aexeo_contracts::content_gate_protocol::Operation::Create => continue,
                aexeo_contracts::content_gate_protocol::Operation::Rename => entry
                    .previous_entity_id
                    .as_deref()
                    .ok_or_else(|| unavailable("missing previous identity"))?,
                _ => &entry.entity_id,
            };
            if metadata.remove(old_id).is_none() {
                return Err(unavailable("base metadata identity unavailable"));
            }
        }
        for entry in &change.entries {
            if !matches!(
                entry.operation,
                aexeo_contracts::content_gate_protocol::Operation::Delete
            ) {
                metadata.insert(entry.entity_id.as_str(), entry.metadata.as_ref());
            }
        }
        let collection: Vec<ContentInput> = overlay
            .candidate
            .items
            .iter()
            .map(|item| ContentInput {
                id: item.entity_id.clone(),
                metadata: metadata
                    .get(item.entity_id.as_str())
                    .copied()
                    .flatten()
                    .map(|metadata| match fields {
                        None => metadata.clone(),
                        Some(fields) => fields
                            .iter()
                            .filter_map(|field| {
                                metadata
                                    .get(field)
                                    .map(|value| (field.clone(), value.clone()))
                            })
                            .collect(),
                    }),
                ..Default::default()
            })
            .collect();
        if metadata.len() != collection.len()
            || collection
                .iter()
                .any(|item| !metadata.contains_key(item.id.as_str()))
        {
            return Err(unavailable("candidate metadata inventory mismatch"));
        }
        Ok(ProjectedCandidateCollection {
            change_id: change.change_id.clone(),
            complete: true,
            items: collection,
        })
    }

    fn evaluate_overlay(
        &self,
        change: &Change,
        overlay: &CandidateOverlay,
    ) -> Result<ContentReport, NotChecked> {
        let projected = self.project_overlay_fields(change, overlay, Some(&self.fields))?;
        let input = ContentInput {
            id: self.snapshot.scope.clone(),
            ..Default::default()
        };
        let context = crate::content_gate::ContentEvaluationContext {
            collection: Some(&projected.items),
            ..Default::default()
        };
        self.preset
            .evaluate_with_collection_completeness(&input, &context, None, true, None)
            .map_err(|_| unavailable("collection evaluation failed"))
    }
}

/// A single resolved preset composed of independent item gates and collection
/// metadata gates, evaluated against the same owned base snapshot.
pub struct ComposedBaseline {
    items: IncrementalBaseline,
    collection: CollectionBaseline,
}

/// Scope-specific reports, not an aggregate pass or publication decision.
#[derive(Debug, Serialize)]
pub struct ComposedEvaluation {
    pub change_id: String,
    pub items: IncrementalEvaluation,
    pub collection: CollectionEvaluation,
}

impl ComposedBaseline {
    /// Prepare a mixed preset using the Markdown, HTML and paragraph-text
    /// adapters. Both item and collection gates must be present. Unsupported
    /// gates, duplicate IDs, and invalid or incomplete inputs fail closed.
    pub fn prepare_adapted(
        snapshot: CandidateSnapshot,
        metadata: BTreeMap<String, Option<BTreeMap<String, String>>>,
        preset: ContentPreset,
    ) -> Result<Self, NotChecked> {
        Self::prepare_adapted_with_html_scope(
            snapshot,
            metadata,
            preset,
            crate::html_content_adapter::HtmlContentScope::Document,
        )
    }

    /// Select the static HTML scope for item gates while preserving the
    /// separately evaluated, host-supplied collection projection.
    pub fn prepare_adapted_with_html_scope(
        snapshot: CandidateSnapshot,
        metadata: BTreeMap<String, Option<BTreeMap<String, String>>>,
        preset: ContentPreset,
        html_scope: crate::html_content_adapter::HtmlContentScope,
    ) -> Result<Self, NotChecked> {
        Self::prepare_with_observations(
            snapshot, metadata, preset, html_scope, None, None, None, false,
        )
    }

    /// Prepare both item and collection scopes with source-bound link observations.
    pub fn prepare_adapted_with_link_observations(
        snapshot: CandidateSnapshot,
        metadata: BTreeMap<String, Option<BTreeMap<String, String>>>,
        preset: ContentPreset,
        html_scope: crate::html_content_adapter::HtmlContentScope,
        observations: BTreeMap<String, crate::content_gate_verdict::LinkObservationSnapshot>,
    ) -> Result<Self, NotChecked> {
        Self::prepare_with_observations(
            snapshot,
            metadata,
            preset,
            html_scope,
            Some(observations),
            None,
            None,
            false,
        )
    }

    /// Prepare item evidence gates and collection gates against the same base.
    /// Evidence provenance, freshness and completeness remain host responsibilities.
    pub fn prepare_adapted_with_evidence(
        snapshot: CandidateSnapshot,
        metadata: BTreeMap<String, Option<BTreeMap<String, String>>>,
        preset: ContentPreset,
        html_scope: crate::html_content_adapter::HtmlContentScope,
        observations: Option<
            BTreeMap<String, crate::content_gate_verdict::LinkObservationSnapshot>,
        >,
        evidence: IncrementalEvidence,
    ) -> Result<Self, NotChecked> {
        Self::prepare_with_observations(
            snapshot,
            metadata,
            preset,
            html_scope,
            observations,
            Some(evidence),
            None,
            false,
        )
    }

    /// Prepare mixed item and collection gates with explicit unavailable link observations.
    #[allow(clippy::too_many_arguments)]
    pub fn prepare_adapted_with_partial_links(
        snapshot: CandidateSnapshot,
        metadata: BTreeMap<String, Option<BTreeMap<String, String>>>,
        preset: ContentPreset,
        html_scope: crate::html_content_adapter::HtmlContentScope,
        observations: BTreeMap<String, crate::content_gate_verdict::LinkObservationSnapshot>,
        unavailable_ids: BTreeSet<String>,
        evidence: Option<IncrementalEvidence>,
    ) -> Result<Self, NotChecked> {
        Self::prepare_with_observations(
            snapshot,
            metadata,
            preset,
            html_scope,
            Some(observations),
            evidence,
            Some(unavailable_ids),
            true,
        )
    }

    /// Preserve unchecked item gates while still evaluating the collection scope.
    /// Inventory identity and completeness requirements remain unchanged.
    pub fn prepare_adapted_with_partial_results(
        snapshot: CandidateSnapshot,
        metadata: BTreeMap<String, Option<BTreeMap<String, String>>>,
        preset: ContentPreset,
        html_scope: crate::html_content_adapter::HtmlContentScope,
        observations: Option<
            BTreeMap<String, crate::content_gate_verdict::LinkObservationSnapshot>,
        >,
        evidence: Option<IncrementalEvidence>,
    ) -> Result<Self, NotChecked> {
        Self::prepare_with_observations(
            snapshot,
            metadata,
            preset,
            html_scope,
            observations,
            evidence,
            None,
            true,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn prepare_with_observations(
        snapshot: CandidateSnapshot,
        metadata: BTreeMap<String, Option<BTreeMap<String, String>>>,
        preset: ContentPreset,
        html_scope: crate::html_content_adapter::HtmlContentScope,
        observations: Option<
            BTreeMap<String, crate::content_gate_verdict::LinkObservationSnapshot>,
        >,
        evidence: Option<IncrementalEvidence>,
        unavailable_ids: Option<BTreeSet<String>>,
        partial_results: bool,
    ) -> Result<Self, NotChecked> {
        preset
            .validate()
            .map_err(|_| unavailable("invalid content preset"))?;
        let mut items = preset.clone();
        let mut collection = preset;
        items
            .gates
            .retain(|gate| !matches!(gate, ContentGate::UniqueCollectionMetadata { .. }));
        collection
            .gates
            .retain(|gate| matches!(gate, ContentGate::UniqueCollectionMetadata { .. }));
        if items.gates.is_empty() || collection.gates.is_empty() {
            return Err(unavailable(
                "mixed preset must contain item and collection gates",
            ));
        }
        let items = if let Some(unavailable_ids) = unavailable_ids {
            let item_metadata = items
                .gates
                .iter()
                .any(is_item_metadata_gate)
                .then(|| metadata.clone());
            IncrementalBaseline::prepare_adapted_with_partial_links(
                snapshot.clone(),
                items,
                html_scope,
                item_metadata,
                observations
                    .ok_or_else(|| unavailable("partial link observations are required"))?,
                unavailable_ids,
                evidence,
            )?
        } else if partial_results {
            let item_metadata = items
                .gates
                .iter()
                .any(is_item_metadata_gate)
                .then(|| metadata.clone());
            IncrementalBaseline::prepare_adapted_with_partial_results(
                snapshot.clone(),
                items,
                html_scope,
                item_metadata,
                observations,
                evidence,
            )?
        } else if let Some(evidence) = evidence {
            let item_metadata = items
                .gates
                .iter()
                .any(is_item_metadata_gate)
                .then(|| metadata.clone());
            IncrementalBaseline::prepare_adapted_with_evidence(
                snapshot.clone(),
                items,
                html_scope,
                item_metadata,
                observations,
                evidence,
            )?
        } else if let Some(observations) = observations {
            let item_metadata = items
                .gates
                .iter()
                .any(is_item_metadata_gate)
                .then(|| metadata.clone());
            IncrementalBaseline::prepare_adapted_with_link_observations(
                snapshot.clone(),
                items,
                html_scope,
                item_metadata,
                observations,
            )?
        } else if items.gates.iter().any(is_item_metadata_gate) {
            IncrementalBaseline::prepare_adapted_with_metadata_and_html_scope(
                snapshot.clone(),
                metadata.clone(),
                items,
                html_scope,
            )?
        } else {
            IncrementalBaseline::prepare_adapted_with_html_scope(
                snapshot.clone(),
                items,
                html_scope,
            )?
        };
        let collection = CollectionBaseline::prepare(snapshot, metadata, collection)?;
        Ok(Self { items, collection })
    }

    /// Project the same candidate collection for an independently executed
    /// change verdict. This does not authenticate or lock the host's inputs.
    pub fn project_candidate(
        &self,
        change: &Change,
        after: &[CandidateItem],
    ) -> Result<ProjectedCandidateCollection, NotChecked> {
        self.collection.project_candidate(change, after)
    }

    /// Evaluate both scopes independently so a missing capability or incomplete
    /// collection cannot be hidden by a successful result in the other scope.
    pub fn evaluate(&self, change: &Change, after: &[CandidateItem]) -> ComposedEvaluation {
        let overlay = checked_overlay(&self.items.snapshot, change, after);
        self.evaluate_with_overlay(change, overlay, None, None, None)
    }

    /// Evaluate both scopes after verifying all host-supplied referenced states.
    /// This does not establish source provenance, freshness or publication safety.
    pub fn evaluate_with_sources<P: crate::content_gate_sources::ConfinedContentFiles>(
        &self,
        change: &Change,
        after: &[CandidateItem],
        provider: &mut P,
        max_state_bytes: usize,
        max_total_bytes: usize,
    ) -> ComposedEvaluation {
        let overlay = checked_overlay_with_sources(
            &self.items.snapshot,
            change,
            after,
            provider,
            max_state_bytes,
            max_total_bytes,
        );
        self.evaluate_with_overlay(change, overlay, None, None, None)
    }

    /// Evaluate both scopes using an explicit complete candidate link inventory.
    pub fn evaluate_with_link_observations(
        &self,
        change: &Change,
        after: &[CandidateItem],
        observations: &BTreeMap<String, crate::content_gate_verdict::LinkObservationSnapshot>,
    ) -> ComposedEvaluation {
        let overlay = checked_overlay(&self.items.snapshot, change, after);
        self.evaluate_with_overlay(change, overlay, Some(observations), None, None)
    }

    /// Evaluate both scopes after checking referenced content and link inventories.
    pub fn evaluate_with_sources_and_link_observations<
        P: crate::content_gate_sources::ConfinedContentFiles,
    >(
        &self,
        change: &Change,
        after: &[CandidateItem],
        provider: &mut P,
        max_state_bytes: usize,
        max_total_bytes: usize,
        observations: &BTreeMap<String, crate::content_gate_verdict::LinkObservationSnapshot>,
    ) -> ComposedEvaluation {
        let overlay = checked_overlay_with_sources(
            &self.items.snapshot,
            change,
            after,
            provider,
            max_state_bytes,
            max_total_bytes,
        );
        self.evaluate_with_overlay(change, overlay, Some(observations), None, None)
    }

    /// Evaluate both scopes with an explicit candidate evidence inventory.
    pub fn evaluate_with_evidence(
        &self,
        change: &Change,
        after: &[CandidateItem],
        observations: Option<
            &BTreeMap<String, crate::content_gate_verdict::LinkObservationSnapshot>,
        >,
        evidence: &IncrementalEvidence,
    ) -> ComposedEvaluation {
        let overlay = checked_overlay(&self.items.snapshot, change, after);
        self.evaluate_with_overlay(change, overlay, observations, Some(evidence), None)
    }

    /// Evaluate both scopes after verifying referenced states, with candidate evidence.
    #[allow(clippy::too_many_arguments)]
    pub fn evaluate_with_sources_and_evidence<
        P: crate::content_gate_sources::ConfinedContentFiles,
    >(
        &self,
        change: &Change,
        after: &[CandidateItem],
        provider: &mut P,
        max_state_bytes: usize,
        max_total_bytes: usize,
        observations: Option<
            &BTreeMap<String, crate::content_gate_verdict::LinkObservationSnapshot>,
        >,
        evidence: &IncrementalEvidence,
    ) -> ComposedEvaluation {
        let overlay = checked_overlay_with_sources(
            &self.items.snapshot,
            change,
            after,
            provider,
            max_state_bytes,
            max_total_bytes,
        );
        self.evaluate_with_overlay(change, overlay, observations, Some(evidence), None)
    }

    /// Project candidate metadata after verifying all host-supplied referenced states.
    pub fn project_candidate_with_sources<P: crate::content_gate_sources::ConfinedContentFiles>(
        &self,
        change: &Change,
        after: &[CandidateItem],
        provider: &mut P,
        max_state_bytes: usize,
        max_total_bytes: usize,
    ) -> Result<ProjectedCandidateCollection, NotChecked> {
        self.collection.project_candidate_with_sources(
            change,
            after,
            provider,
            max_state_bytes,
            max_total_bytes,
        )
    }

    /// Evaluate both scopes with an explicitly complete availability partition.
    pub fn evaluate_with_partial_links(
        &self,
        change: &Change,
        after: &[CandidateItem],
        observations: &BTreeMap<String, crate::content_gate_verdict::LinkObservationSnapshot>,
        unavailable_ids: &BTreeSet<String>,
        evidence: Option<&IncrementalEvidence>,
    ) -> ComposedEvaluation {
        let overlay = checked_overlay(&self.items.snapshot, change, after);
        self.evaluate_with_overlay(
            change,
            overlay,
            Some(observations),
            evidence,
            Some(unavailable_ids),
        )
    }

    /// Verify referenced states and evaluate both scopes with a partial link inventory.
    #[allow(clippy::too_many_arguments)]
    pub fn evaluate_with_sources_and_partial_links<
        P: crate::content_gate_sources::ConfinedContentFiles,
    >(
        &self,
        change: &Change,
        after: &[CandidateItem],
        provider: &mut P,
        max_state_bytes: usize,
        max_total_bytes: usize,
        observations: &BTreeMap<String, crate::content_gate_verdict::LinkObservationSnapshot>,
        unavailable_ids: &BTreeSet<String>,
        evidence: Option<&IncrementalEvidence>,
    ) -> ComposedEvaluation {
        let overlay = checked_overlay_with_sources(
            &self.items.snapshot,
            change,
            after,
            provider,
            max_state_bytes,
            max_total_bytes,
        );
        self.evaluate_with_overlay(
            change,
            overlay,
            Some(observations),
            evidence,
            Some(unavailable_ids),
        )
    }

    fn evaluate_with_overlay(
        &self,
        change: &Change,
        overlay: Result<CandidateOverlay, NotChecked>,
        observations: Option<
            &BTreeMap<String, crate::content_gate_verdict::LinkObservationSnapshot>,
        >,
        evidence: Option<&IncrementalEvidence>,
        unavailable_ids: Option<&BTreeSet<String>>,
    ) -> ComposedEvaluation {
        let item_result = self
            .items
            .validate_change_media(change)
            .and_then(|()| overlay.as_ref().map_err(Clone::clone))
            .and_then(|overlay| {
                if let Some(unavailable_ids) = unavailable_ids {
                    let observations = observations
                        .ok_or_else(|| unavailable("partial link observations are required"))?;
                    self.items.evaluate_partial_overlay(
                        change,
                        overlay,
                        observations,
                        unavailable_ids,
                        evidence,
                    )
                } else {
                    self.items
                        .evaluate_overlay(change, overlay, observations, evidence)
                }
            });
        let collection_result = overlay
            .as_ref()
            .map_err(Clone::clone)
            .and_then(|overlay| self.collection.evaluate_overlay(change, overlay));
        ComposedEvaluation {
            change_id: change.change_id.clone(),
            items: match item_result {
                Ok(result) => result,
                Err(error) => IncrementalEvaluation::NotChecked {
                    reason: error.reason,
                },
            },
            collection: match collection_result {
                Ok(report) => CollectionEvaluation::Report {
                    change_id: change.change_id.clone(),
                    report,
                },
                Err(error) => CollectionEvaluation::NotChecked {
                    reason: error.reason,
                },
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content_candidate::overlay_candidate;
    use aexeo_contracts::content_gate_protocol::{
        Base, BaseKind, ContentState, Entry, Operation, Origin, OriginKind,
    };
    use sha2::{Digest, Sha256};

    #[test]
    fn partial_link_inventory_preserves_other_checks_and_refreshes_availability() {
        use crate::content_gate::LinkReachabilityObservation::Reachable;
        use crate::content_gate_verdict::LinkObservationSnapshot;
        let (mut snapshot, _, mut change) = fixture();
        let mut a = item("a", "[A](https://a.example)", None);
        a.media_type = "text/markdown".into();
        let mut b = item("b", "[B](https://b.example)", None);
        b.media_type = "text/markdown".into();
        snapshot.items = vec![a.clone(), b.clone()];
        let preset = ContentPreset::from_toml("version=1\nname='links'\n[[gates]]\nid='link'\nkind='link_reachability'\nblocking=true\n[[gates]]\nid='words'\nkind='word_count'\nmin=1\n").unwrap();
        let observation = |item: &CandidateItem, url: &str| LinkObservationSnapshot {
            version: 1,
            source_sha256: item.sha256.clone(),
            destinations: vec![url.into()],
            observations: vec![Reachable],
        };
        let observed = BTreeMap::from([("a".into(), observation(&a, "https://a.example"))]);
        let missing = BTreeSet::from(["b".into()]);
        let scope = crate::html_content_adapter::HtmlContentScope::Document;
        assert!(
            IncrementalBaseline::prepare_adapted_with_partial_links(
                snapshot.clone(),
                preset.clone(),
                scope,
                None,
                observed.clone(),
                BTreeSet::new(),
                None
            )
            .is_err()
        );
        assert!(
            IncrementalBaseline::prepare_adapted_with_partial_links(
                snapshot.clone(),
                preset.clone(),
                scope,
                None,
                observed.clone(),
                BTreeSet::from(["a".into()]),
                None
            )
            .is_err()
        );
        let mut base_complete = observed.clone();
        base_complete.insert("b".into(), observation(&b, "https://b.example"));
        let observed_baseline = IncrementalBaseline::prepare_adapted_with_partial_links(
            snapshot.clone(),
            preset.clone(),
            scope,
            None,
            base_complete,
            BTreeSet::new(),
            None,
        )
        .unwrap();
        let baseline = IncrementalBaseline::prepare_adapted_with_partial_links(
            snapshot,
            preset,
            scope,
            None,
            observed.clone(),
            missing.clone(),
            None,
        )
        .unwrap();
        let mut next = item("a", "[new](https://new.example)", None);
        next.media_type = "text/markdown".into();
        let mut update = entry("a", Operation::Update, Some(&a), Some(&next));
        update.media_type = "text/markdown".into();
        change.entries = vec![update];
        let candidate = BTreeMap::from([("a".into(), observation(&next, "https://new.example"))]);
        let IncrementalEvaluation::Evaluated {
            reports,
            evaluated,
            reused,
            ..
        } = baseline.evaluate_with_partial_links(
            &change,
            &[next.clone()],
            &candidate,
            &missing,
            None,
        )
        else {
            panic!("partial candidate must be evaluated")
        };
        assert_eq!((evaluated, reused), (1, 1));
        assert_eq!(reports["b"].gates[0].status, GateStatus::NotChecked);
        assert_eq!(reports["b"].gates[1].status, GateStatus::Passed);
        assert_eq!(reports["b"].summary.blocking_not_checked, 1);
        let IncrementalEvaluation::Evaluated {
            reports,
            evaluated,
            reused,
            ..
        } = observed_baseline.evaluate_with_partial_links(
            &change,
            std::slice::from_ref(&next),
            &candidate,
            &missing,
            None,
        )
        else {
            panic!("newly unavailable candidate must be evaluated")
        };
        assert_eq!((evaluated, reused), (2, 0));
        assert_eq!(reports["b"].gates[0].status, GateStatus::NotChecked);
        let IncrementalEvaluation::Evaluated {
            reports,
            evaluated,
            reused,
            ..
        } = baseline.evaluate_with_partial_links(
            &change,
            std::slice::from_ref(&next),
            &BTreeMap::new(),
            &BTreeSet::from(["a".into(), "b".into()]),
            None,
        )
        else {
            panic!("unavailable candidate must retain partial results")
        };
        assert_eq!((evaluated, reused), (1, 1));
        assert_eq!(reports["a"].gates[0].status, GateStatus::NotChecked);
        assert_eq!(reports["a"].gates[1].status, GateStatus::Passed);
        let mut complete = candidate.clone();
        complete.insert("b".into(), observation(&b, "https://b.example"));
        let IncrementalEvaluation::Evaluated {
            reports,
            evaluated,
            reused,
            ..
        } = baseline.evaluate_with_partial_links(
            &change,
            &[next.clone()],
            &complete,
            &BTreeSet::new(),
            None,
        )
        else {
            panic!("newly available candidate must be evaluated")
        };
        assert_eq!((evaluated, reused), (2, 0));
        assert_eq!(reports["b"].gates[0].status, GateStatus::Passed);
        assert!(matches!(
            baseline.evaluate_with_partial_links(
                &change,
                &[next.clone()],
                &candidate,
                &BTreeSet::new(),
                None
            ),
            IncrementalEvaluation::NotChecked { .. }
        ));
        struct Provider(Vec<u8>);
        impl crate::content_gate_sources::ConfinedContentFiles for Provider {
            type Reader = std::io::Cursor<Vec<u8>>;
            fn open(
                &mut self,
                _: &std::path::Path,
                _: &std::path::Path,
            ) -> std::io::Result<Self::Reader> {
                Ok(std::io::Cursor::new(self.0.clone()))
            }
        }
        let mut referenced = change.clone();
        referenced.content_root = Some("/trusted".into());
        let state = referenced.entries[0].after.as_mut().unwrap();
        state.inline = None;
        state.file = Some("next.md".into());
        let mut provider = Provider(next.source.as_bytes().to_vec());
        assert!(matches!(
            baseline.evaluate_with_sources_and_partial_links(
                &referenced,
                std::slice::from_ref(&next),
                &mut provider,
                1024,
                1024,
                &candidate,
                &missing,
                None
            ),
            IncrementalEvaluation::Evaluated { .. }
        ));
        provider.0 = b"wrong bytes".to_vec();
        assert!(matches!(
            baseline.evaluate_with_sources_and_partial_links(
                &referenced,
                std::slice::from_ref(&next),
                &mut provider,
                1024,
                1024,
                &candidate,
                &missing,
                None
            ),
            IncrementalEvaluation::NotChecked { .. }
        ));
        assert!(matches!(
            baseline.evaluate_with_link_observations(&change, &[next], &candidate),
            IncrementalEvaluation::NotChecked { .. }
        ));
    }

    #[test]
    fn link_observations_are_bound_and_refresh_unchanged_items() {
        use crate::content_gate::LinkReachabilityObservation::{
            NotChecked as Unknown, NotFound, Reachable,
        };
        use crate::content_gate_verdict::LinkObservationSnapshot;
        let (mut snapshot, _, mut change) = fixture();
        let mut a = item("a", "[A](https://a.example)", None);
        a.media_type = "text/markdown".into();
        let mut b = item("b", "[B](https://b.example)", None);
        b.media_type = "text/markdown".into();
        snapshot.items = vec![a.clone(), b.clone()];
        let preset = ContentPreset::from_toml("version=1\nname='links'\n[[gates]]\nid='link'\nkind='link_reachability'\n[[gates]]\nid='metadata'\nkind='required_metadata'\nfields=['description']\n").unwrap();
        let observation = |item: &CandidateItem, url: &str, value| LinkObservationSnapshot {
            version: 1,
            source_sha256: item.sha256.clone(),
            destinations: vec![url.into()],
            observations: vec![value],
        };
        let base = BTreeMap::from([
            ("a".into(), observation(&a, "https://a.example", Reachable)),
            ("b".into(), observation(&b, "https://b.example", Reachable)),
        ]);
        let fields = BTreeMap::from([("description".into(), "observed".into())]);
        let metadata = BTreeMap::from([
            ("a".into(), Some(fields.clone())),
            ("b".into(), Some(fields.clone())),
        ]);
        let baseline = IncrementalBaseline::prepare_adapted_with_link_observations(
            snapshot,
            preset,
            crate::html_content_adapter::HtmlContentScope::Document,
            Some(metadata),
            base.clone(),
        )
        .unwrap();
        let mut next = item("a", "[new](https://new.example)", None);
        next.media_type = "text/markdown".into();
        let mut update = entry("a", Operation::Update, Some(&a), Some(&next));
        update.media_type = "text/markdown".into();
        update.metadata = Some(fields);
        change.entries = vec![update];
        let candidate = BTreeMap::from([
            (
                "a".into(),
                observation(&next, "https://new.example", Reachable),
            ),
            ("b".into(), observation(&b, "https://b.example", NotFound)),
        ]);
        let IncrementalEvaluation::Evaluated {
            reports,
            evaluated,
            reused,
            ..
        } = baseline.evaluate_with_link_observations(&change, &[next.clone()], &candidate)
        else {
            panic!("candidate must be evaluated")
        };
        assert_eq!((evaluated, reused), (2, 0));
        assert_eq!(reports["b"].gates[0].status, GateStatus::Failed);
        assert_eq!(reports["b"].gates[1].status, GateStatus::Passed);
        let mut same = candidate.clone();
        same.insert("b".into(), base["b"].clone());
        assert!(matches!(
            baseline.evaluate_with_link_observations(&change, &[next.clone()], &same),
            IncrementalEvaluation::Evaluated {
                evaluated: 1,
                reused: 1,
                ..
            }
        ));
        assert!(matches!(
            baseline.evaluate(&change, &[next.clone()]),
            IncrementalEvaluation::NotChecked { .. }
        ));
        for alteration in 0..4 {
            let mut bad = candidate.clone();
            let evidence = bad.get_mut("a").unwrap();
            match alteration {
                0 => evidence.source_sha256 = "wrong".into(),
                1 => evidence.destinations[0] = "https://wrong.example".into(),
                2 => evidence.observations.clear(),
                _ => evidence.observations[0] = Unknown,
            }
            assert!(
                matches!(
                    baseline.evaluate_with_link_observations(&change, &[next.clone()], &bad),
                    IncrementalEvaluation::NotChecked { .. }
                ),
                "alteration {alteration}"
            );
        }
        struct Provider(Vec<u8>);
        impl crate::content_gate_sources::ConfinedContentFiles for Provider {
            type Reader = std::io::Cursor<Vec<u8>>;
            fn open(
                &mut self,
                _: &std::path::Path,
                _: &std::path::Path,
            ) -> std::io::Result<Self::Reader> {
                Ok(std::io::Cursor::new(self.0.clone()))
            }
        }
        let mut referenced = change.clone();
        referenced.content_root = Some("/trusted".into());
        let state = referenced.entries[0].after.as_mut().unwrap();
        state.inline = None;
        state.file = Some("next.md".into());
        let mut provider = Provider(next.source.as_bytes().to_vec());
        assert!(matches!(
            baseline.evaluate_with_sources_and_link_observations(
                &referenced,
                &[next.clone()],
                &mut provider,
                1024,
                1024,
                &candidate
            ),
            IncrementalEvaluation::Evaluated { .. }
        ));
        provider.0 = b"wrong bytes".to_vec();
        assert!(matches!(
            baseline.evaluate_with_sources_and_link_observations(
                &referenced,
                &[next.clone()],
                &mut provider,
                1024,
                1024,
                &candidate
            ),
            IncrementalEvaluation::NotChecked { .. }
        ));
        let mut missing = candidate.clone();
        missing.remove("b");
        assert!(matches!(
            baseline.evaluate_with_link_observations(&change, &[next], &missing),
            IncrementalEvaluation::NotChecked { .. }
        ));
    }

    #[test]
    fn editorial_pacing_reuses_only_matching_source_bound_snapshots() {
        let (mut snapshot, _, mut change) = fixture();
        let a = item("a", "alpha", None);
        let b = item("b", "alpha", None);
        snapshot.items = vec![a.clone(), b.clone()];
        let preset = ContentPreset::from_toml(
            "version=1\nname='pacing'\n[[gates]]\nid='pacing'\nkind='editorial_pacing'\nsource='composition'\nprofile={version=1,name='short',fields=[{name='hook',max_characters=4}]}\n"
        ).unwrap();
        let original = BTreeMap::from([("composition".into(), r#"{"hook":"alpha"}"#.into())]);
        let mut evidence = BTreeMap::from([
            ("a".into(), original.clone()),
            ("b".into(), original.clone()),
        ]);
        let baseline = IncrementalBaseline::prepare_adapted_with_evidence(
            snapshot,
            preset.clone(),
            crate::html_content_adapter::HtmlContentScope::Document,
            None,
            None,
            evidence.clone(),
        )
        .unwrap();
        let next = item("a", "bravo", None);
        change.entries = vec![entry("a", Operation::Update, Some(&a), Some(&next))];
        evidence
            .get_mut("a")
            .unwrap()
            .insert("composition".into(), r#"{"hook":"bravo"}"#.into());
        let IncrementalEvaluation::Evaluated {
            reports,
            evaluated,
            reused,
            ..
        } = baseline.evaluate_with_evidence(&change, std::slice::from_ref(&next), None, &evidence)
        else {
            panic!("matching snapshots must be assessed")
        };
        assert_eq!((evaluated, reused), (1, 1));
        for item in [&next, &b] {
            let full = preset
                .evaluate(&ContentInput {
                    id: item.entity_id.clone(),
                    text: Some(item.source.clone()),
                    source_text: Some(item.source.clone()),
                    evidence_sources: evidence.get(&item.entity_id).cloned(),
                    ..Default::default()
                })
                .unwrap();
            assert_eq!(
                serde_json::to_value(&reports[&item.entity_id]).unwrap(),
                serde_json::to_value(&full).unwrap(),
            );
            assert_eq!(full.gates[0].status, GateStatus::Failed);
        }
        // Semantically equivalent but byte-different evidence still invalidates reuse.
        evidence
            .get_mut("b")
            .unwrap()
            .insert("composition".into(), r#"{ "hook": "alpha" }"#.into());
        assert!(matches!(
            baseline.evaluate_with_evidence(&change, std::slice::from_ref(&next), None, &evidence),
            IncrementalEvaluation::Evaluated {
                evaluated: 2,
                reused: 0,
                ..
            }
        ));
        evidence.get_mut("b").unwrap().clear();
        assert!(matches!(
            baseline.evaluate_with_evidence(&change, std::slice::from_ref(&next), None, &evidence),
            IncrementalEvaluation::NotChecked { .. }
        ));
        evidence
            .get_mut("b")
            .unwrap()
            .insert("composition".into(), r#"{"hook":"stale"}"#.into());
        assert!(matches!(
            baseline.evaluate_with_evidence(&change, &[next], None, &evidence),
            IncrementalEvaluation::NotChecked { .. }
        ));
    }

    #[test]
    fn evidence_inventory_controls_incremental_reuse_and_fails_closed() {
        let (mut snapshot, _, mut change) = fixture();
        let a = item("a", "claim", None);
        let b = item("b", "claim", None);
        snapshot.items = vec![a.clone(), b.clone()];
        let preset = ContentPreset::from_toml("version=1\nname='evidence'\n[[gates]]\nid='anchor'\nkind='evidence_anchors'\nsource='study'\nclaim='claim'\nquote='quote'\n").unwrap();
        let sources = BTreeMap::from([("study".into(), "quote".into())]);
        let base = BTreeMap::from([("a".into(), sources.clone()), ("b".into(), sources.clone())]);
        let baseline = IncrementalBaseline::prepare_adapted_with_evidence(
            snapshot.clone(),
            preset.clone(),
            crate::html_content_adapter::HtmlContentScope::Document,
            None,
            None,
            base.clone(),
        )
        .unwrap();
        let next = item("a", "claim revised", None);
        change.entries = vec![entry("a", Operation::Update, Some(&a), Some(&next))];
        let mut candidate = base.clone();
        candidate
            .get_mut("b")
            .unwrap()
            .insert("study".into(), "different".into());
        let IncrementalEvaluation::Evaluated {
            reports,
            evaluated,
            reused,
            ..
        } = baseline.evaluate_with_evidence(&change, std::slice::from_ref(&next), None, &candidate)
        else {
            panic!("expected complete evaluation")
        };
        assert_eq!((evaluated, reused), (2, 0));
        assert_eq!(reports["b"].gates[0].status, GateStatus::Failed);
        for item in [&next, &b] {
            let report = evaluate_item(
                &preset,
                item,
                ProjectionMode::Adapted(crate::html_content_adapter::HtmlContentScope::Document),
                None,
                None,
                candidate.get(&item.entity_id),
                false,
            )
            .unwrap();
            assert_eq!(
                serde_json::to_value(&reports[&item.entity_id]).unwrap(),
                serde_json::to_value(report).unwrap()
            );
        }
        assert!(matches!(
            baseline.evaluate_with_evidence(&change, std::slice::from_ref(&next), None, &base),
            IncrementalEvaluation::Evaluated {
                evaluated: 1,
                reused: 1,
                ..
            }
        ));
        assert!(matches!(
            baseline.evaluate(&change, std::slice::from_ref(&next)),
            IncrementalEvaluation::NotChecked { .. }
        ));
        candidate.remove("b");
        assert!(matches!(
            baseline.evaluate_with_evidence(&change, std::slice::from_ref(&next), None, &candidate),
            IncrementalEvaluation::NotChecked { .. }
        ));
        candidate.insert("b".into(), BTreeMap::new());
        assert!(matches!(
            baseline.evaluate_with_evidence(&change, &[next], None, &candidate),
            IncrementalEvaluation::NotChecked { .. }
        ));
        assert!(
            IncrementalBaseline::prepare_adapted_with_evidence(
                snapshot,
                preset,
                crate::html_content_adapter::HtmlContentScope::Document,
                None,
                None,
                candidate
            )
            .is_err()
        );
    }

    #[test]
    fn composed_evidence_reuses_independent_reports_and_preserves_collection() {
        let (mut snapshot, _, mut change) = fixture();
        let a = item("a", "claim", None);
        let b = item("b", "claim", None);
        snapshot.items = vec![a.clone(), b.clone()];
        let preset = ContentPreset::from_toml("version=1\nname='mixed'\n[[gates]]\nid='anchor'\nkind='evidence_anchors'\nsource='study'\nclaim='claim'\nquote='quote'\n[[gates]]\nid='unique'\nkind='unique_collection_metadata'\nfield='slug'\n").unwrap();
        let metadata = BTreeMap::from([
            (
                "a".into(),
                Some(BTreeMap::from([("slug".into(), "a".into())])),
            ),
            (
                "b".into(),
                Some(BTreeMap::from([("slug".into(), "b".into())])),
            ),
        ]);
        let source = BTreeMap::from([("study".into(), "quote".into())]);
        let base = BTreeMap::from([("a".into(), source.clone()), ("b".into(), source)]);
        let baseline = ComposedBaseline::prepare_adapted_with_evidence(
            snapshot,
            metadata,
            preset,
            crate::html_content_adapter::HtmlContentScope::Document,
            None,
            base.clone(),
        )
        .unwrap();
        let next = item("a", "claim revised", None);
        let mut update = entry("a", Operation::Update, Some(&a), Some(&next));
        update.metadata = Some(BTreeMap::from([("slug".into(), "a".into())]));
        change.entries = vec![update];
        let after = std::slice::from_ref(&next);
        let result = baseline.evaluate_with_evidence(&change, after, None, &base);
        assert!(matches!(
            result.items,
            IncrementalEvaluation::Evaluated {
                evaluated: 1,
                reused: 1,
                ..
            }
        ));
        assert!(matches!(
            result.collection,
            CollectionEvaluation::Report { .. }
        ));
        let mut changed = base.clone();
        changed
            .get_mut("b")
            .unwrap()
            .insert("study".into(), "different".into());
        let result = baseline.evaluate_with_evidence(&change, after, None, &changed);
        let IncrementalEvaluation::Evaluated {
            reports,
            evaluated,
            reused,
            ..
        } = result.items
        else {
            panic!("item result unavailable")
        };
        assert_eq!((evaluated, reused), (2, 0));
        assert_eq!(reports["b"].gates[0].status, GateStatus::Failed);
        changed.remove("b");
        let result = baseline.evaluate_with_evidence(&change, after, None, &changed);
        assert!(matches!(
            result.items,
            IncrementalEvaluation::NotChecked { .. }
        ));
        assert!(matches!(
            result.collection,
            CollectionEvaluation::Report { .. }
        ));
        assert!(matches!(
            baseline.evaluate(&change, after).items,
            IncrementalEvaluation::NotChecked { .. }
        ));
    }

    #[test]
    fn adapted_literal_terms_see_reference_labels() {
        let preset = ContentPreset::from_toml("version=1\nname='t'\n[[gates]]\nid='required'\nkind='required_terms'\nterms=['[17]']\n").unwrap();
        for (media, source) in [
            ("text/markdown", "Élan.[[17]](#reference-source) Next."),
            (
                "text/html",
                "<p>Élan.<a href='#reference-source'>[17]</a> Next.</p>",
            ),
        ] {
            let mut candidate = item("a", source, None);
            candidate.media_type = media.into();
            let report = evaluate_item(
                &preset,
                &candidate,
                ProjectionMode::Adapted(crate::html_content_adapter::HtmlContentScope::Document),
                None,
                None,
                None,
                false,
            )
            .unwrap();
            assert_eq!(report.gates[0].status, GateStatus::Passed, "{media}");
        }
    }

    fn item(id: &str, source: &str, links: Option<Vec<&str>>) -> CandidateItem {
        CandidateItem {
            entity_id: id.into(),
            public_path: format!("/{id}"),
            media_type: "text/plain".into(),
            revision: Some("r1".into()),
            sha256: format!("{:x}", Sha256::digest(source.as_bytes())),
            source: source.into(),
            links: links.map(|v| v.into_iter().map(str::to_owned).collect()),
        }
    }
    fn state(item: &CandidateItem) -> ContentState {
        ContentState {
            revision: item.revision.clone(),
            sha256: item.sha256.clone(),
            inline: Some(item.source.clone()),
            file: None,
            uri: None,
        }
    }
    fn entry(
        id: &str,
        op: Operation,
        before: Option<&CandidateItem>,
        after: Option<&CandidateItem>,
    ) -> Entry {
        Entry {
            entity_id: id.into(),
            operation: op,
            previous_entity_id: None,
            media_type: "text/plain".into(),
            public_path: after.map(|item| item.public_path.clone()),
            metadata: None,
            before: before.map(state),
            after: after.map(state),
        }
    }
    fn fixture() -> (CandidateSnapshot, ContentPreset, Change) {
        let base = Base {
            kind: BaseKind::Opaque,
            revision: "base".into(),
        };
        let snapshot = CandidateSnapshot {
            scope: "published".into(),
            site_id: "test".into(),
            site_base_url: None,
            base: base.clone(),
            complete: true,
            items: vec![
                item("a", "one", Some(vec!["/b"])),
                item("b", "two words", Some(vec![])),
                item("c", "three different words", Some(vec![])),
                item("d", "four rather different words", Some(vec!["/a"])),
                item("e", "untouched", Some(vec![])),
            ],
        };
        let preset=ContentPreset::from_toml("version=1\nname='checks'\n[[gates]]\nid='words'\nkind='word_count'\nmin=2\n[[gates]]\nid='chars'\nkind='character_count'\nmin=4\n").unwrap();
        let change = Change {
            schema: "content-gate.change/v1".into(),
            change_id: "change".into(),
            created_at: "2026-01-01T00:00:00Z".into(),
            origin: Origin {
                kind: OriginKind::Manual,
                system: None,
                reference: None,
            },
            base,
            content_root: None,
            site: None,
            author: None,
            entries: vec![],
            extensions: None,
        };
        (snapshot, preset, change)
    }
    fn full(
        snapshot: &CandidateSnapshot,
        preset: &ContentPreset,
    ) -> BTreeMap<String, ContentReport> {
        snapshot
            .items
            .iter()
            .map(|item| {
                let input = ContentInput {
                    id: item.entity_id.clone(),
                    text: Some(item.source.clone()),
                    ..Default::default()
                };
                (item.entity_id.clone(), preset.evaluate(&input).unwrap())
            })
            .collect()
    }
    #[test]
    fn metadata_only_changes_are_recomputed_and_missing_projections_fail_closed() {
        let (snapshot, _, mut change) = fixture();
        let preset = ContentPreset::from_toml("version=1\nname='metadata'\n[[gates]]\nid='required'\nkind='required_metadata'\nfields=['description']\n[[gates]]\nid='length'\nkind='metadata_length'\nfield='description'\nmin=4\n[[gates]]\nid='age'\nkind='metadata_age'\nfield='reviewed'\nas_of='2026-01-31'\nmax_days=30\n").unwrap();
        let fields = BTreeMap::from([
            ("description".into(), "long enough".into()),
            ("reviewed".into(), "2026-01-20".into()),
        ]);
        let metadata: BTreeMap<_, _> = snapshot
            .items
            .iter()
            .map(|item| (item.entity_id.clone(), Some(fields.clone())))
            .collect();
        assert!(IncrementalBaseline::prepare_adapted(snapshot.clone(), preset.clone()).is_err());
        let mut incomplete = metadata.clone();
        incomplete.remove("a");
        assert!(
            IncrementalBaseline::prepare_adapted_with_metadata(
                snapshot.clone(),
                incomplete,
                preset.clone()
            )
            .is_err()
        );
        let mut unavailable = metadata.clone();
        unavailable.insert("a".into(), None);
        assert!(
            IncrementalBaseline::prepare_adapted_with_metadata(
                snapshot.clone(),
                unavailable,
                preset.clone()
            )
            .is_err()
        );
        let baseline = IncrementalBaseline::prepare_adapted_with_metadata(
            snapshot.clone(),
            metadata,
            preset.clone(),
        )
        .unwrap();
        let next = snapshot.items[0].clone();
        let changed_fields = BTreeMap::from([
            ("description".into(), "new".into()),
            ("reviewed".into(), "2025-01-01".into()),
        ]);
        let mut update = entry(
            "a",
            Operation::Update,
            Some(&snapshot.items[0]),
            Some(&next),
        );
        update.metadata = Some(changed_fields.clone());
        change.entries.push(update);
        let IncrementalEvaluation::Evaluated {
            reports,
            evaluated,
            reused,
            ..
        } = baseline.evaluate(&change, std::slice::from_ref(&next))
        else {
            panic!("expected evaluation")
        };
        assert_eq!((evaluated, reused), (1, snapshot.items.len() - 1));
        assert_eq!(
            reports["a"]
                .gates
                .iter()
                .map(|gate| gate.status)
                .collect::<Vec<_>>(),
            vec![GateStatus::Passed, GateStatus::Failed, GateStatus::Failed]
        );
        let input = ContentInput {
            id: next.entity_id.clone(),
            text: Some(next.source.clone()),
            source_text: Some(next.source.clone()),
            metadata: Some(changed_fields),
            ..Default::default()
        };
        let context = crate::plain_text_adapter::paragraph_text_context(&next.source).unwrap();
        let capabilities = ContentEvaluationContext {
            adapter: Some(&context),
            ..Default::default()
        };
        let full = preset
            .evaluate_with_paragraph_list_items(
                &input,
                &capabilities,
                None,
                None,
                true,
                None,
                None,
                None,
            )
            .unwrap();
        assert_eq!(
            serde_json::to_value(&reports["a"]).unwrap(),
            serde_json::to_value(full).unwrap()
        );
        change.entries[0].metadata = None;
        assert!(matches!(
            baseline.evaluate(&change, &[next]),
            IncrementalEvaluation::NotChecked { .. }
        ));
    }

    #[test]
    fn metadata_follows_create_rename_and_delete_without_inheriting_old_fields() {
        let (snapshot, _, mut change) = fixture();
        let preset = ContentPreset::from_toml("version=1\nname='metadata'\n[[gates]]\nid='required'\nkind='required_metadata'\nfields=['description']\n").unwrap();
        let metadata = snapshot
            .items
            .iter()
            .map(|item| {
                (
                    item.entity_id.clone(),
                    Some(BTreeMap::from([("description".into(), "old".into())])),
                )
            })
            .collect();
        let baseline =
            IncrementalBaseline::prepare_adapted_with_metadata(snapshot.clone(), metadata, preset)
                .unwrap();
        let renamed = item("renamed", &snapshot.items[0].source, Some(vec![]));
        let created = item("created", "new", Some(vec![]));
        let mut rename = entry(
            "renamed",
            Operation::Rename,
            Some(&snapshot.items[0]),
            Some(&renamed),
        );
        rename.previous_entity_id = Some("a".into());
        rename.metadata = Some(BTreeMap::new());
        let mut create = entry("created", Operation::Create, None, Some(&created));
        create.metadata = Some(BTreeMap::from([("description".into(), "new".into())]));
        let delete = entry("b", Operation::Delete, Some(&snapshot.items[1]), None);
        change.entries = vec![rename, create, delete];
        let IncrementalEvaluation::Evaluated {
            reports,
            evaluated,
            reused,
            ..
        } = baseline.evaluate(&change, &[renamed.clone(), created.clone()])
        else {
            panic!("expected evaluation")
        };
        assert_eq!(evaluated, 2);
        assert_eq!(reused, snapshot.items.len() - 2);
        assert!(!reports.contains_key("a") && !reports.contains_key("b"));
        assert_eq!(reports["renamed"].gates[0].status, GateStatus::Failed);
        assert_eq!(reports["created"].gates[0].status, GateStatus::Passed);
        change.entries[0].metadata = None;
        assert!(matches!(
            baseline.evaluate(&change, &[renamed, created]),
            IncrementalEvaluation::NotChecked { .. }
        ));
    }

    #[test]
    fn metadata_absence_is_distinct_from_unavailable_and_composes_with_collection() {
        let (snapshot, _, mut change) = fixture();
        let preset = ContentPreset::from_toml("version=1\nname='mixed'\n[[gates]]\nid='required'\nkind='required_metadata'\nfields=['description']\n[[gates]]\nid='unique'\nkind='unique_collection_metadata'\nfield='slug'\n").unwrap();
        let metadata: BTreeMap<_, _> = snapshot
            .items
            .iter()
            .map(|item| {
                (
                    item.entity_id.clone(),
                    Some(BTreeMap::from([
                        ("description".into(), "text".into()),
                        ("slug".into(), item.entity_id.clone()),
                    ])),
                )
            })
            .collect();
        let baseline =
            ComposedBaseline::prepare_adapted(snapshot.clone(), metadata, preset).unwrap();
        let next = snapshot.items[0].clone();
        let mut update = entry(
            "a",
            Operation::Update,
            Some(&snapshot.items[0]),
            Some(&next),
        );
        update.metadata = Some(BTreeMap::from([("slug".into(), "a".into())]));
        change.entries.push(update);
        let result = baseline.evaluate(&change, std::slice::from_ref(&next));
        let IncrementalEvaluation::Evaluated {
            reports, evaluated, ..
        } = result.items
        else {
            panic!("expected evaluated item report")
        };
        assert_eq!(evaluated, 1);
        assert_eq!(reports["a"].gates[0].status, GateStatus::Failed);
        change.entries[0].metadata = None;
        let result = baseline.evaluate(&change, &[next]);
        assert!(matches!(
            result.items,
            IncrementalEvaluation::NotChecked { .. }
        ));
    }

    #[test]
    fn incremental_host_projection_checks_bound_content_and_unknowns() {
        let (mut snapshot, _, mut change) = fixture();
        snapshot.items.truncate(1);
        let make = |text: Option<&str>| {
            let source = serde_json::json!({
                "schema":"aexeo.content-adapter/v1",
                "text":text,
                "headings":["Heading"],
                "heading_levels":[2],
                "link_destinations":[]
            })
            .to_string();
            let mut result = item("a", &source, None);
            result.media_type = crate::content_adapter::MEDIA_TYPE.into();
            result
        };
        snapshot.items[0] = make(Some("one two"));
        let preset = ContentPreset::from_toml(
            "version=1\nname='words'\n[[gates]]\nid='words'\nkind='word_count'\nmin=2\n",
        )
        .unwrap();
        let baseline = IncrementalBaseline::prepare_adapted(snapshot.clone(), preset).unwrap();
        let next = make(Some("one"));
        let mut update = entry(
            "a",
            Operation::Update,
            Some(&snapshot.items[0]),
            Some(&next),
        );
        update.media_type = crate::content_adapter::MEDIA_TYPE.into();
        change.entries = vec![update];
        let IncrementalEvaluation::Evaluated {
            reports, evaluated, ..
        } = baseline.evaluate(&change, &[next.clone()])
        else {
            panic!("host snapshot should be evaluated");
        };
        assert_eq!(evaluated, 1);
        assert_eq!(reports["a"].gates[0].status, GateStatus::Failed);
        let missing = make(None);
        change.entries[0].after = Some(state(&missing));
        assert!(matches!(
            baseline.evaluate(&change, &[missing]),
            IncrementalEvaluation::NotChecked { .. }
        ));
        change.entries[0].after = Some(state(&next));
        let mut tampered = next.clone();
        tampered.source.push(' ');
        assert!(matches!(
            baseline.evaluate(&change, &[tampered]),
            IncrementalEvaluation::NotChecked { .. }
        ));
        let mut invalid = item(
            "a",
            "{\"schema\":\"aexeo.content-adapter/v1\",\"text\":\"x\",\"text\":\"y\"}",
            None,
        );
        invalid.media_type = crate::content_adapter::MEDIA_TYPE.into();
        change.entries[0].after = Some(state(&invalid));
        assert!(matches!(
            baseline.evaluate(&change, &[invalid]),
            IncrementalEvaluation::NotChecked { .. }
        ));
    }

    #[test]
    fn incremental_host_structures_are_independent_of_text_and_preserve_unknowns() {
        let preset = ContentPreset::from_toml("version=1\nname='structure'\n[[gates]]\nid='heads'\nkind='heading_hierarchy'\n[[gates]]\nid='lists'\nkind='list_item_count'\nflagged_counts=[3]\n[[gates]]\nid='links'\nkind='link_count'\nmin=1\n").unwrap();
        let check = |source: serde_json::Value| {
            let mut candidate = item("a", &source.to_string(), None);
            candidate.media_type = crate::content_adapter::MEDIA_TYPE.into();
            evaluate_item(
                &preset,
                &candidate,
                ProjectionMode::Adapted(crate::html_content_adapter::HtmlContentScope::Document),
                None,
                None,
                None,
                true,
            )
            .unwrap()
        };
        let report = check(serde_json::json!({
            "schema":"aexeo.content-adapter/v1", "text":null,
            "headings":["First", "Skipped"], "heading_levels":[1,3],
            "lists":[{"item_count":3,"depth":1}], "link_destinations":[]
        }));
        assert!(
            report
                .gates
                .iter()
                .all(|gate| gate.status == GateStatus::Failed)
        );
        let missing = check(serde_json::json!({"schema":"aexeo.content-adapter/v1", "text":null}));
        assert!(
            missing
                .gates
                .iter()
                .all(|gate| gate.status == GateStatus::NotChecked)
        );
    }

    #[test]
    fn host_projection_cannot_override_unavailable_external_metadata() {
        let preset = ContentPreset::from_toml("version=1\nname='metadata'\n[[gates]]\nid='required'\nkind='required_metadata'\nfields=['description']\n").unwrap();
        let source = serde_json::json!({
            "schema":"aexeo.content-adapter/v1",
            "text":"body",
            "metadata":{"description":"embedded"},
            "observed_metadata_fields":["description"]
        })
        .to_string();
        let mut candidate = item("a", &source, None);
        candidate.media_type = crate::content_adapter::MEDIA_TYPE.into();
        let result = evaluate_item(
            &preset,
            &candidate,
            ProjectionMode::Adapted(crate::html_content_adapter::HtmlContentScope::Document),
            None,
            None,
            None,
            false,
        );
        assert!(result.is_err());
    }

    #[test]
    fn composed_html_scope_applies_with_and_without_item_metadata() {
        use crate::html_content_adapter::HtmlContentScope;
        for require_metadata in [false, true] {
            let (mut snapshot, _, mut change) = fixture();
            for item in &mut snapshot.items {
                item.media_type = "text/html".into();
                item.source = "<nav>navigation words</nav><main>base words</main>".into();
                item.sha256 = format!("{:x}", Sha256::digest(item.source.as_bytes()));
            }
            let metadata: BTreeMap<_, _> = snapshot
                .items
                .iter()
                .map(|item| {
                    (
                        item.entity_id.clone(),
                        Some(BTreeMap::from([
                            ("slug".into(), item.entity_id.clone()),
                            ("description".into(), "description".into()),
                        ])),
                    )
                })
                .collect();
            let metadata_gate = if require_metadata {
                "[[gates]]\nid='required'\nkind='required_metadata'\nfields=['description']\n"
            } else {
                ""
            };
            let source = format!(
                "version=1\nname='mixed'\n[[gates]]\nid='words'\nkind='word_count'\nmin=2\n{metadata_gate}[[gates]]\nid='unique'\nkind='unique_collection_metadata'\nfield='slug'\n"
            );
            let preset = ContentPreset::from_toml(&source).unwrap();
            let main = ComposedBaseline::prepare_adapted_with_html_scope(
                snapshot.clone(),
                metadata.clone(),
                preset.clone(),
                HtmlContentScope::Main,
            )
            .unwrap();
            let whole =
                ComposedBaseline::prepare_adapted(snapshot.clone(), metadata.clone(), preset)
                    .unwrap();
            let mut updated = snapshot.items[0].clone();
            updated.source = "<nav>extra navigation words</nav><main>one</main>".into();
            updated.sha256 = format!("{:x}", Sha256::digest(updated.source.as_bytes()));
            let mut update = entry(
                "a",
                Operation::Update,
                Some(&snapshot.items[0]),
                Some(&updated),
            );
            update.media_type = updated.media_type.clone();
            update.metadata = Some(BTreeMap::from([
                ("slug".into(), "b".into()),
                ("description".into(), "description".into()),
            ]));
            change.entries = vec![update];
            let result = main.evaluate(&change, std::slice::from_ref(&updated));
            let IncrementalEvaluation::Evaluated { reports, .. } = result.items else {
                panic!("main item unavailable");
            };
            assert_eq!(reports["a"].gates[0].status, GateStatus::Failed);
            let CollectionEvaluation::Report { report, .. } = result.collection else {
                panic!("collection unavailable");
            };
            assert_eq!(report.gates[0].status, GateStatus::Failed);
            let IncrementalEvaluation::Evaluated { reports, .. } = whole
                .evaluate(&change, std::slice::from_ref(&updated))
                .items
            else {
                panic!("whole item unavailable");
            };
            assert_eq!(reports["a"].gates[0].status, GateStatus::Passed);
            updated.source = "<p>No main</p>".into();
            updated.sha256 = format!("{:x}", Sha256::digest(updated.source.as_bytes()));
            change.entries[0].after = Some(state(&updated));
            assert!(matches!(
                main.evaluate(&change, &[updated]).items,
                IncrementalEvaluation::NotChecked { .. }
            ));
        }
    }

    #[test]
    fn composed_candidate_reports_both_scopes_and_preserves_unavailable_collection() {
        let (snapshot, _, mut change) = fixture();
        let preset = ContentPreset::from_toml("version=1\nname='mixed'\n[[gates]]\nid='words'\nkind='word_count'\nmin=2\n[[gates]]\nid='unique'\nkind='unique_collection_metadata'\nfield='slug'\n").unwrap();
        let metadata: BTreeMap<_, _> = snapshot
            .items
            .iter()
            .map(|i| {
                (
                    i.entity_id.clone(),
                    Some(BTreeMap::from([("slug".into(), i.entity_id.clone())])),
                )
            })
            .collect();
        let updated = item("a", "one", Some(vec!["/b"]));
        let mut update = entry(
            "a",
            Operation::Update,
            Some(&snapshot.items[0]),
            Some(&updated),
        );
        update.metadata = Some(BTreeMap::from([("slug".into(), "b".into())]));
        change.entries = vec![update];
        let baseline =
            ComposedBaseline::prepare_adapted(snapshot.clone(), metadata.clone(), preset.clone())
                .unwrap();
        let result = baseline.evaluate(&change, std::slice::from_ref(&updated));
        let IncrementalEvaluation::Evaluated { reports, .. } = result.items else {
            panic!("item evaluation unavailable")
        };
        assert_eq!(reports["a"].gates[0].status, GateStatus::Failed);
        let CollectionEvaluation::Report { report, .. } = result.collection else {
            panic!("collection unavailable")
        };
        assert_eq!(report.gates[0].status, GateStatus::Failed);
        assert_eq!(report.gates[0].findings.len(), 1);
        change.entries[0].metadata = None;
        let result = baseline.evaluate(&change, std::slice::from_ref(&updated));
        assert!(matches!(
            result.items,
            IncrementalEvaluation::Evaluated { .. }
        ));
        let CollectionEvaluation::Report { report, .. } = result.collection else {
            panic!("collection unavailable")
        };
        assert_eq!(report.gates[0].status, GateStatus::NotChecked);
        change.base.revision = "stale".into();
        let result = baseline.evaluate(&change, &[updated]);
        assert!(matches!(
            result.items,
            IncrementalEvaluation::NotChecked { .. }
        ));
        assert!(matches!(
            result.collection,
            CollectionEvaluation::NotChecked { .. }
        ));
        let mut invalid = preset.clone();
        invalid.gates.push(invalid.gates[0].clone());
        assert!(ComposedBaseline::prepare_adapted(snapshot, metadata, invalid).is_err());
    }

    #[test]
    fn shared_overlay_matches_separate_scope_evaluations() {
        let (snapshot, _, mut change) = fixture();
        let preset = ContentPreset::from_toml("version=1\nname='mixed'\n[[gates]]\nid='words'\nkind='word_count'\nmin=2\n[[gates]]\nid='unique'\nkind='unique_collection_metadata'\nfield='slug'\n").unwrap();
        let metadata = snapshot
            .items
            .iter()
            .map(|i| {
                (
                    i.entity_id.clone(),
                    Some(BTreeMap::from([("slug".into(), i.entity_id.clone())])),
                )
            })
            .collect();
        let updated = item("a", "updated words", Some(vec!["/b"]));
        let mut update = entry(
            "a",
            Operation::Update,
            Some(&snapshot.items[0]),
            Some(&updated),
        );
        update.metadata = Some(BTreeMap::from([("slug".into(), "b".into())]));
        change.entries = vec![update];
        let baseline = ComposedBaseline::prepare_adapted(snapshot, metadata, preset).unwrap();
        let mut missing_links = updated.clone();
        missing_links.links = None;
        let mut stale = change.clone();
        stale.base.revision = "stale".into();
        let mut unsupported = change.clone();
        unsupported.entries[0].media_type = "application/pdf".into();
        let mut missing_inline = change.clone();
        missing_inline.entries[0].after.as_mut().unwrap().inline = None;
        for (proposal, after) in [
            (&change, vec![updated.clone()]),
            (&change, vec![missing_links]),
            (&stale, vec![updated.clone()]),
            (&unsupported, vec![updated.clone()]),
            (&missing_inline, vec![updated]),
        ] {
            let expected_items = baseline.items.evaluate(proposal, &after);
            let expected_collection = baseline.collection.evaluate(proposal, &after);
            let actual = baseline.evaluate(proposal, &after);
            assert_eq!(
                serde_json::to_value(actual.items).unwrap(),
                serde_json::to_value(expected_items).unwrap()
            );
            assert_eq!(
                serde_json::to_value(actual.collection).unwrap(),
                serde_json::to_value(expected_collection).unwrap()
            );
        }
    }

    #[test]
    fn projected_collection_integrates_with_bound_candidate_verdict() {
        use crate::content_gate_sources::ConfinedContentFiles;
        use crate::content_gate_verdict::{
            CandidateDecisionContext, OfflineDecisionContext,
            evaluate_experimental_candidate_verdict, revalidate_experimental_candidate_verdict,
        };
        use crate::html_content_adapter::HtmlContentScope;
        use std::{
            io::{self, Cursor},
            path::Path,
        };
        struct NoFiles;
        impl ConfinedContentFiles for NoFiles {
            type Reader = Cursor<Vec<u8>>;
            fn open(&mut self, _: &Path, _: &Path) -> io::Result<Self::Reader> {
                panic!("unexpected file access")
            }
        }
        let (snapshot, _, mut change) = fixture();
        let preset = ContentPreset::from_toml("version=1\nname='mixed'\n[[gates]]\nid='words'\nkind='word_count'\nmin=1\n[[gates]]\nid='unique'\nkind='unique_collection_metadata'\nfield='slug'\nblocking=true\n").unwrap();
        let metadata = snapshot
            .items
            .iter()
            .map(|i| {
                (
                    i.entity_id.clone(),
                    Some(BTreeMap::from([("slug".into(), i.entity_id.clone())])),
                )
            })
            .collect();
        let updated = item("a", "updated words", Some(vec!["/b"]));
        let mut update = entry(
            "a",
            Operation::Update,
            Some(&snapshot.items[0]),
            Some(&updated),
        );
        update.metadata = Some(BTreeMap::from([("slug".into(), "b".into())]));
        change.entries = vec![update];
        let baseline =
            ComposedBaseline::prepare_adapted(snapshot.clone(), metadata, preset.clone()).unwrap();
        let projected = baseline
            .project_candidate(&change, std::slice::from_ref(&updated))
            .unwrap();
        let encoded = serde_json::to_value(&projected).unwrap();
        assert_eq!(encoded["change_id"], change.change_id);
        assert_eq!(encoded["complete"], true);
        assert_eq!(
            encoded["items"].as_array().unwrap().len(),
            snapshot.items.len()
        );
        let decision = OfflineDecisionContext {
            change: &change,
            preset: &preset,
            max_entry_bytes: 1024,
            max_total_bytes: 4096,
            plain_text_paragraphs: true,
            collection: Some(projected.as_change_collection()),
            evaluator_identity: b"test-build",
        };
        let after = [updated];
        let ctx = CandidateDecisionContext {
            decision: &decision,
            base: &snapshot,
            after: &after,
        };
        let verdict = evaluate_experimental_candidate_verdict(
            &ctx,
            &mut NoFiles,
            "2026-01-01T00:00:00Z",
            HtmlContentScope::Document,
        )
        .unwrap();
        assert_eq!(
            verdict.receipt.verdict.status,
            aexeo_contracts::content_gate_protocol::VerdictStatus::Fail
        );
        assert!(
            revalidate_experimental_candidate_verdict(
                &verdict,
                &ctx,
                &mut NoFiles,
                "2026-01-02T00:00:00Z",
                HtmlContentScope::Document
            )
            .is_ok()
        );
        let mut changed_metadata = projected.clone();
        changed_metadata
            .items
            .iter_mut()
            .find(|item| item.id == "e")
            .unwrap()
            .metadata
            .as_mut()
            .unwrap()
            .insert("slug".into(), "different".into());
        let changed_decision = OfflineDecisionContext {
            collection: Some(changed_metadata.as_change_collection()),
            ..decision
        };
        let changed_ctx = CandidateDecisionContext {
            decision: &changed_decision,
            ..ctx
        };
        assert!(
            revalidate_experimental_candidate_verdict(
                &verdict,
                &changed_ctx,
                &mut NoFiles,
                "2026-01-02T00:00:00Z",
                HtmlContentScope::Document
            )
            .is_err()
        );
        let mut missing_change = change.clone();
        missing_change.entries[0].metadata = None;
        let missing_projection = baseline.project_candidate(&missing_change, &after).unwrap();
        let missing_decision = OfflineDecisionContext {
            change: &missing_change,
            preset: &preset,
            max_entry_bytes: 1024,
            max_total_bytes: 4096,
            plain_text_paragraphs: true,
            collection: Some(missing_projection.as_change_collection()),
            evaluator_identity: b"test-build",
        };
        let missing_ctx = CandidateDecisionContext {
            decision: &missing_decision,
            base: &snapshot,
            after: &after,
        };
        let missing_verdict = evaluate_experimental_candidate_verdict(
            &missing_ctx,
            &mut NoFiles,
            "2026-01-01T00:00:00Z",
            HtmlContentScope::Document,
        )
        .unwrap();
        assert_eq!(
            missing_verdict.receipt.verdict.status,
            aexeo_contracts::content_gate_protocol::VerdictStatus::NotChecked
        );
        let mut stale = change.clone();
        stale.base.revision = "stale".into();
        assert!(baseline.project_candidate(&stale, &after).is_err());
    }

    #[test]
    fn projected_rename_swap_integrates_with_candidate_verdict() {
        use crate::content_gate_sources::ConfinedContentFiles;
        use crate::content_gate_verdict::{
            CandidateDecisionContext, OfflineDecisionContext,
            evaluate_experimental_candidate_verdict, revalidate_experimental_candidate_verdict,
        };
        use crate::html_content_adapter::HtmlContentScope;
        use std::{
            io::{self, Cursor},
            path::Path,
        };
        struct NoFiles;
        impl ConfinedContentFiles for NoFiles {
            type Reader = Cursor<Vec<u8>>;
            fn open(&mut self, _: &Path, _: &Path) -> io::Result<Self::Reader> {
                panic!("unexpected file access")
            }
        }
        let (snapshot, _, mut change) = fixture();
        let preset = ContentPreset::from_toml("version=1\nname='collection'\n[[gates]]\nid='unique'\nkind='unique_collection_metadata'\nfield='slug'\nblocking=true\n").unwrap();
        let metadata = snapshot
            .items
            .iter()
            .map(|i| {
                (
                    i.entity_id.clone(),
                    Some(BTreeMap::from([("slug".into(), i.entity_id.clone())])),
                )
            })
            .collect();
        let a = item("a", "new a", Some(vec![]));
        let b = item("b", "new b", Some(vec![]));
        let mut to_a = entry("a", Operation::Rename, Some(&snapshot.items[1]), Some(&a));
        to_a.previous_entity_id = Some("b".into());
        to_a.metadata = Some(BTreeMap::from([("slug".into(), "a".into())]));
        let mut to_b = entry("b", Operation::Rename, Some(&snapshot.items[0]), Some(&b));
        to_b.previous_entity_id = Some("a".into());
        to_b.metadata = Some(BTreeMap::from([("slug".into(), "b".into())]));
        change.entries = vec![to_a, to_b];
        let after = [a, b];
        let baseline =
            CollectionBaseline::prepare(snapshot.clone(), metadata, preset.clone()).unwrap();
        let projected = baseline.project_candidate(&change, &after).unwrap();
        let decision = OfflineDecisionContext {
            change: &change,
            preset: &preset,
            max_entry_bytes: 1024,
            max_total_bytes: 4096,
            plain_text_paragraphs: true,
            collection: Some(projected.as_change_collection()),
            evaluator_identity: b"test-build",
        };
        let ctx = CandidateDecisionContext {
            decision: &decision,
            base: &snapshot,
            after: &after,
        };
        let verdict = evaluate_experimental_candidate_verdict(
            &ctx,
            &mut NoFiles,
            "2026-01-01T00:00:00Z",
            HtmlContentScope::Document,
        )
        .unwrap();
        assert_eq!(
            verdict.receipt.verdict.status,
            aexeo_contracts::content_gate_protocol::VerdictStatus::NotChecked
        );
        assert!(
            verdict
                .receipt
                .verdict
                .checks
                .iter()
                .any(|check| check.id == "collection:0::unique"
                    && check.status == aexeo_contracts::content_gate_protocol::CheckStatus::Passed)
        );
        assert!(
            revalidate_experimental_candidate_verdict(
                &verdict,
                &ctx,
                &mut NoFiles,
                "2026-01-02T00:00:00Z",
                HtmlContentScope::Document,
            )
            .is_ok()
        );
    }

    #[test]
    fn composed_candidate_rejects_unsupported_or_missing_scope() {
        let (snapshot, item_only, _) = fixture();
        let metadata = snapshot
            .items
            .iter()
            .map(|i| (i.entity_id.clone(), None))
            .collect();
        assert!(ComposedBaseline::prepare_adapted(snapshot.clone(), metadata, item_only).is_err());
        let unsupported = ContentPreset::from_toml("version=1\nname='mixed'\n[[gates]]\nid='site'\nkind='site_group'\ngroup='links'\n[[gates]]\nid='unique'\nkind='unique_collection_metadata'\nfield='slug'\n").unwrap();
        let metadata = snapshot
            .items
            .iter()
            .map(|i| (i.entity_id.clone(), None))
            .collect();
        assert!(ComposedBaseline::prepare_adapted(snapshot, metadata, unsupported).is_err());
    }

    #[test]
    fn collection_candidate_matches_full_and_retains_partial_findings() {
        let (snapshot, _, mut change) = fixture();
        let preset = ContentPreset::from_toml("version=1\nname='collection'\n[[gates]]\nid='unique'\nkind='unique_collection_metadata'\nfield='slug'\ngroup_by='lang'\n").unwrap();
        let projection = |slug: &str| {
            Some(BTreeMap::from([
                ("slug".into(), slug.into()),
                ("lang".into(), "en".into()),
                ("unrelated".into(), "preserve for external consumers".into()),
            ]))
        };
        let metadata: BTreeMap<_, _> = snapshot
            .items
            .iter()
            .map(|i| (i.entity_id.clone(), projection(&i.entity_id)))
            .collect();
        let updated = item("a", "updated source", Some(vec![]));
        let created = item("new", "created source", Some(vec![]));
        let renamed = item("renamed", "renamed source", Some(vec![]));
        let mut update = entry(
            "a",
            Operation::Update,
            Some(&snapshot.items[0]),
            Some(&updated),
        );
        update.metadata = projection("duplicate");
        let mut create = entry("new", Operation::Create, None, Some(&created));
        create.metadata = projection("duplicate");
        let mut rename = entry(
            "renamed",
            Operation::Rename,
            Some(&snapshot.items[2]),
            Some(&renamed),
        );
        rename.previous_entity_id = Some("c".into());
        rename.metadata = projection("renamed");
        change.entries = vec![
            update,
            entry("b", Operation::Delete, Some(&snapshot.items[1]), None),
            create,
            rename,
        ];
        let after = [updated, created, renamed];
        let baseline =
            CollectionBaseline::prepare(snapshot.clone(), metadata.clone(), preset.clone())
                .unwrap();
        let public_projection = baseline.project_candidate(&change, &after).unwrap();
        assert!(
            public_projection
                .as_change_collection()
                .items
                .iter()
                .all(|item| {
                    item.metadata
                        .as_ref()
                        .is_some_and(|metadata| metadata.contains_key("unrelated"))
                })
        );
        let CollectionEvaluation::Report { report, .. } = baseline.evaluate(&change, &after) else {
            panic!("expected report")
        };
        let candidate = overlay_candidate(&snapshot, &change, &after)
            .unwrap()
            .candidate;
        let collection: Vec<_> = candidate
            .items
            .iter()
            .map(|item| ContentInput {
                id: item.entity_id.clone(),
                metadata: match item.entity_id.as_str() {
                    "a" | "new" => projection("duplicate"),
                    "renamed" => projection("renamed"),
                    id => metadata[id].clone(),
                },
                ..Default::default()
            })
            .collect();
        let input = ContentInput {
            id: snapshot.scope.clone(),
            ..Default::default()
        };
        let context = crate::content_gate::ContentEvaluationContext {
            collection: Some(&collection),
            ..Default::default()
        };
        let full = preset
            .evaluate_with_collection_completeness(&input, &context, None, true, None)
            .unwrap();
        assert_eq!(
            serde_json::to_value(&report).unwrap(),
            serde_json::to_value(full).unwrap()
        );
        assert_eq!(report.gates[0].status, GateStatus::Failed);
        assert_eq!(report.gates[0].findings.len(), 1);
        change.entries[3].metadata = None;
        let CollectionEvaluation::Report { report, .. } = baseline.evaluate(&change, &after) else {
            panic!("expected partial report")
        };
        assert_eq!(report.gates[0].status, GateStatus::NotChecked);
        assert_eq!(report.gates[0].findings.len(), 1);
        change.base.revision = "stale".into();
        assert!(matches!(
            baseline.evaluate(&change, &after),
            CollectionEvaluation::NotChecked { .. }
        ));
    }

    #[test]
    fn collection_field_selection_preserves_full_reports_and_public_metadata() {
        let (snapshot, _, mut change) = fixture();
        let preset = ContentPreset::from_toml("version=1\nname='collection'\n[[gates]]\nid='slug'\nkind='unique_collection_metadata'\nfield='slug'\ngroup_by='lang'\n[[gates]]\nid='title'\nkind='unique_collection_metadata'\nfield='title'\n").unwrap();
        let metadata = snapshot
            .items
            .iter()
            .map(|item| {
                (
                    item.entity_id.clone(),
                    Some(BTreeMap::from([
                        ("slug".into(), item.entity_id.clone()),
                        ("lang".into(), "en".into()),
                        ("title".into(), "shared".into()),
                        ("extra".into(), "unrelated host metadata".into()),
                    ])),
                )
            })
            .collect();
        let updated = item("a", "updated source", Some(vec![]));
        let mut update = entry(
            "a",
            Operation::Update,
            Some(&snapshot.items[0]),
            Some(&updated),
        );
        let baseline =
            CollectionBaseline::prepare(snapshot.clone(), metadata, preset.clone()).unwrap();
        for fields in [
            None,
            Some(BTreeMap::new()),
            Some(BTreeMap::from([
                ("slug".into(), "b".into()),
                ("lang".into(), "en".into()),
                ("title".into(), "shared".into()),
                ("extra".into(), "retained".into()),
            ])),
            Some(BTreeMap::from([
                ("slug".into(), "x".repeat(16 * 1024 + 1)),
                ("lang".into(), "en".into()),
                ("title".into(), "shared".into()),
                ("extra".into(), "unrelated".into()),
            ])),
        ] {
            update.metadata = fields;
            change.entries = vec![update.clone()];
            let projected = baseline
                .project_candidate(&change, std::slice::from_ref(&updated))
                .unwrap();
            let collection = projected.as_change_collection();
            let input = ContentInput {
                id: snapshot.scope.clone(),
                ..Default::default()
            };
            let context = ContentEvaluationContext {
                collection: Some(collection.items),
                ..Default::default()
            };
            let full = preset
                .evaluate_with_collection_completeness(&input, &context, None, true, None)
                .unwrap();
            let CollectionEvaluation::Report { report, .. } =
                baseline.evaluate(&change, std::slice::from_ref(&updated))
            else {
                panic!("expected collection report")
            };
            assert_eq!(
                serde_json::to_value(report).unwrap(),
                serde_json::to_value(full).unwrap()
            );
            for item in collection.items.iter().filter(|item| item.id != "a") {
                assert_eq!(
                    item.metadata
                        .as_ref()
                        .unwrap()
                        .get("extra")
                        .map(String::as_str),
                    Some("unrelated host metadata")
                );
            }
        }
    }

    #[test]
    fn collection_baseline_rejects_incomplete_or_ambiguous_inventory_and_other_gates() {
        let (snapshot, preset, _) = fixture();
        let unique = ContentPreset::from_toml("version=1\nname='collection'\n[[gates]]\nid='unique'\nkind='unique_collection_metadata'\nfield='slug'\n").unwrap();
        let metadata: BTreeMap<_, _> = snapshot
            .items
            .iter()
            .map(|i| (i.entity_id.clone(), None))
            .collect();
        assert!(CollectionBaseline::prepare(snapshot.clone(), metadata.clone(), preset).is_err());
        let mut missing = metadata.clone();
        missing.remove("a");
        assert!(CollectionBaseline::prepare(snapshot.clone(), missing, unique.clone()).is_err());
        let mut incomplete = snapshot.clone();
        incomplete.complete = false;
        assert!(CollectionBaseline::prepare(incomplete, metadata.clone(), unique.clone()).is_err());
        let mut duplicate = snapshot;
        duplicate.items[1].entity_id = duplicate.items[0].entity_id.clone();
        assert!(CollectionBaseline::prepare(duplicate, metadata, unique).is_err());
    }
    #[test]
    fn referenced_states_are_verified_before_incremental_evaluation() {
        use crate::content_gate_sources::{ConfinedContentFiles, ContentPhase};
        use std::{io, io::Cursor, path::Path};

        struct Provider {
            old: Vec<u8>,
            new: Vec<u8>,
        }
        impl ConfinedContentFiles for Provider {
            type Reader = Cursor<Vec<u8>>;
            fn open(&mut self, root: &Path, path: &Path) -> io::Result<Self::Reader> {
                if root != Path::new("/trusted") || path != Path::new("before.txt") {
                    return Err(io::ErrorKind::NotFound.into());
                }
                Ok(Cursor::new(self.old.clone()))
            }
            fn open_uri(
                &mut self,
                entity: &str,
                phase: ContentPhase,
                uri: &str,
            ) -> io::Result<Self::Reader> {
                if entity != "a"
                    || phase != ContentPhase::After
                    || uri != "https://example.org/after"
                {
                    return Err(io::ErrorKind::NotFound.into());
                }
                Ok(Cursor::new(self.new.clone()))
            }
        }
        let (snapshot, preset, mut change) = fixture();
        let next = item("a", "new article words", Some(vec![]));
        change.entries.push(entry(
            "a",
            Operation::Update,
            Some(&snapshot.items[0]),
            Some(&next),
        ));
        let inline_change = change.clone();
        change.content_root = Some("/trusted".into());
        let before = change.entries[0].before.as_mut().unwrap();
        before.inline = None;
        before.file = Some("before.txt".into());
        let after = change.entries[0].after.as_mut().unwrap();
        after.inline = None;
        after.uri = Some("https://example.org/after".into());
        let after_items = std::slice::from_ref(&next);
        let mut provider = Provider {
            old: snapshot.items[0].source.as_bytes().to_vec(),
            new: next.source.as_bytes().to_vec(),
        };
        let baseline = IncrementalBaseline::prepare(snapshot.clone(), preset).unwrap();
        assert!(matches!(
            baseline.evaluate(&change, after_items),
            IncrementalEvaluation::NotChecked { .. }
        ));
        let expected =
            serde_json::to_value(baseline.evaluate(&inline_change, after_items)).unwrap();
        assert_eq!(
            serde_json::to_value(baseline.evaluate_with_sources(
                &change,
                after_items,
                &mut provider,
                100,
                200
            ))
            .unwrap(),
            expected
        );
        assert!(matches!(
            baseline.evaluate_with_sources(&change, after_items, &mut provider, 2, 200),
            IncrementalEvaluation::NotChecked { .. }
        ));
        assert!(matches!(
            baseline.evaluate_with_sources(&change, after_items, &mut provider, 100, 2),
            IncrementalEvaluation::NotChecked { .. }
        ));
        provider.old = b"wrong old bytes".to_vec();
        assert!(matches!(
            baseline.evaluate_with_sources(&change, after_items, &mut provider, 100, 200),
            IncrementalEvaluation::NotChecked { .. }
        ));
        provider.old = snapshot.items[0].source.as_bytes().to_vec();
        provider.new = b"wrong new bytes".to_vec();
        assert!(matches!(
            baseline.evaluate_with_sources(&change, after_items, &mut provider, 100, 200),
            IncrementalEvaluation::NotChecked { .. }
        ));
        provider.new = next.source.as_bytes().to_vec();
        let metadata: BTreeMap<_, _> = snapshot
            .items
            .iter()
            .map(|i| {
                (
                    i.entity_id.clone(),
                    Some(BTreeMap::from([("slug".into(), i.entity_id.clone())])),
                )
            })
            .collect();
        change.entries[0].metadata = Some(BTreeMap::from([("slug".into(), "a".into())]));
        let unique = ContentPreset::from_toml("version=1\nname='collection'\n[[gates]]\nid='unique'\nkind='unique_collection_metadata'\nfield='slug'\n").unwrap();
        let collection =
            CollectionBaseline::prepare(snapshot.clone(), metadata.clone(), unique).unwrap();
        assert!(
            collection
                .project_candidate_with_sources(&change, after_items, &mut provider, 100, 200)
                .is_ok()
        );
        assert!(matches!(
            collection.evaluate_with_sources(&change, after_items, &mut provider, 100, 200),
            CollectionEvaluation::Report { .. }
        ));
        let mixed = ContentPreset::from_toml("version=1\nname='mixed'\n[[gates]]\nid='words'\nkind='word_count'\nmin=1\n[[gates]]\nid='unique'\nkind='unique_collection_metadata'\nfield='slug'\n").unwrap();
        let composed = ComposedBaseline::prepare_adapted(snapshot, metadata, mixed).unwrap();
        assert!(
            composed
                .project_candidate_with_sources(&change, after_items, &mut provider, 100, 200)
                .is_ok()
        );
        let result = composed.evaluate_with_sources(&change, after_items, &mut provider, 100, 200);
        assert!(matches!(
            result.items,
            IncrementalEvaluation::Evaluated { .. }
        ));
        assert!(matches!(
            result.collection,
            CollectionEvaluation::Report { .. }
        ));
        provider.new = b"wrong".to_vec();
        let result = composed.evaluate_with_sources(&change, after_items, &mut provider, 100, 200);
        assert!(matches!(
            result.items,
            IncrementalEvaluation::NotChecked { .. }
        ));
        assert!(matches!(
            result.collection,
            CollectionEvaluation::NotChecked { .. }
        ));
        assert!(
            composed
                .project_candidate_with_sources(&change, after_items, &mut provider, 100, 200)
                .is_err()
        );
    }

    #[test]
    fn unordered_base_reuses_correct_reports_after_mixed_operations() {
        let (mut snapshot, preset, mut change) = fixture();
        snapshot.items.reverse();
        let get = |id: &str| {
            snapshot
                .items
                .iter()
                .find(|item| item.entity_id == id)
                .unwrap()
        };
        let updated = item("a", "updated with enough words", Some(vec![]));
        let created = item("new", "new", Some(vec![]));
        let renamed = item("renamed", "renamed item", Some(vec![]));
        change.entries.push(entry(
            "a",
            Operation::Update,
            Some(get("a")),
            Some(&updated),
        ));
        change
            .entries
            .push(entry("b", Operation::Delete, Some(get("b")), None));
        change
            .entries
            .push(entry("new", Operation::Create, None, Some(&created)));
        let mut rename = entry("renamed", Operation::Rename, Some(get("c")), Some(&renamed));
        rename.previous_entity_id = Some("c".into());
        change.entries.push(rename);
        let after = [updated, created, renamed];
        let expected = full(
            &overlay_candidate(&snapshot, &change, &after)
                .unwrap()
                .candidate,
            &preset,
        );
        let baseline = IncrementalBaseline::prepare(snapshot, preset).unwrap();
        let IncrementalEvaluation::Evaluated {
            reports,
            evaluated,
            reused,
            ..
        } = baseline.evaluate(&change, &after)
        else {
            panic!("expected evaluated candidate")
        };
        assert_eq!(
            serde_json::to_value(reports).unwrap(),
            serde_json::to_value(expected).unwrap()
        );
        assert_eq!((evaluated, reused), (3, 2));
    }

    #[test]
    fn referenced_incremental_operations_match_inline_and_full_evaluation() {
        use crate::content_gate_sources::{ConfinedContentFiles, ContentPhase};
        use std::{io, io::Cursor, path::Path};
        struct Provider(BTreeMap<(String, bool), Vec<u8>>);
        impl ConfinedContentFiles for Provider {
            type Reader = Cursor<Vec<u8>>;
            fn open(&mut self, _: &Path, _: &Path) -> io::Result<Self::Reader> {
                Err(io::ErrorKind::Unsupported.into())
            }
            fn open_uri(
                &mut self,
                entity: &str,
                phase: ContentPhase,
                _: &str,
            ) -> io::Result<Self::Reader> {
                self.0
                    .get(&(entity.to_owned(), phase == ContentPhase::After))
                    .cloned()
                    .map(Cursor::new)
                    .ok_or_else(|| io::ErrorKind::NotFound.into())
            }
        }
        let (snapshot, preset, mut change) = fixture();
        let updated = item("a", "new source words", Some(vec![]));
        let created = item("new", "created item", Some(vec![]));
        let renamed = item("renamed", "renamed entry", Some(vec![]));
        change.entries.push(entry(
            "a",
            Operation::Update,
            Some(&snapshot.items[0]),
            Some(&updated),
        ));
        change.entries.push(entry(
            "b",
            Operation::Delete,
            Some(&snapshot.items[1]),
            None,
        ));
        change
            .entries
            .push(entry("new", Operation::Create, None, Some(&created)));
        let mut rename = entry(
            "renamed",
            Operation::Rename,
            Some(&snapshot.items[2]),
            Some(&renamed),
        );
        rename.previous_entity_id = Some("c".into());
        change.entries.push(rename);
        let after = [updated, created, renamed];
        let expected = full(
            &overlay_candidate(&snapshot, &change, &after)
                .unwrap()
                .candidate,
            &preset,
        );
        let mut provider = Provider(BTreeMap::new());
        for entry in &mut change.entries {
            for (is_after, state) in [(false, &mut entry.before), (true, &mut entry.after)] {
                if let Some(state) = state {
                    let source = state.inline.take().unwrap();
                    provider
                        .0
                        .insert((entry.entity_id.clone(), is_after), source.into_bytes());
                    state.uri = Some(format!(
                        "https://example.org/{}/{}",
                        entry.entity_id, is_after
                    ));
                }
            }
        }
        let baseline = IncrementalBaseline::prepare(snapshot, preset).unwrap();
        let IncrementalEvaluation::Evaluated {
            reports,
            evaluated,
            reused,
            ..
        } = baseline.evaluate_with_sources(&change, &after, &mut provider, 100, 1000)
        else {
            panic!("expected verified evaluation")
        };
        assert_eq!(
            serde_json::to_value(reports).unwrap(),
            serde_json::to_value(expected).unwrap()
        );
        assert_eq!((evaluated, reused), (3, 2));
        // Renames bind the before bytes to the old candidate identity but ask
        // the host for the new change-entry identity and before phase.
        provider.0.remove(&("renamed".into(), false));
        assert!(matches!(
            baseline.evaluate_with_sources(&change, &after, &mut provider, 100, 1000),
            IncrementalEvaluation::NotChecked { .. }
        ));
    }

    #[test]
    fn incremental_equals_full_after_update_delete_create_and_rename() {
        let (snapshot, preset, mut change) = fixture();
        let updated = item("a", "new source words", Some(vec!["/c"]));
        let created = item("new", "created item", Some(vec![]));
        let renamed = item("renamed", "renamed entry", Some(vec![]));
        change.entries.push(entry(
            "a",
            Operation::Update,
            Some(&snapshot.items[0]),
            Some(&updated),
        ));
        change.entries.push(entry(
            "b",
            Operation::Delete,
            Some(&snapshot.items[1]),
            None,
        ));
        change
            .entries
            .push(entry("new", Operation::Create, None, Some(&created)));
        let mut rename = entry(
            "renamed",
            Operation::Rename,
            Some(&snapshot.items[2]),
            Some(&renamed),
        );
        rename.previous_entity_id = Some("c".into());
        change.entries.push(rename);
        let after = [updated, created, renamed];
        let overlay = overlay_candidate(&snapshot, &change, &after).unwrap();
        let expected = full(&overlay.candidate, &preset);
        let baseline = IncrementalBaseline::prepare(snapshot, preset).unwrap();
        for _ in 0..2 {
            let IncrementalEvaluation::Evaluated {
                reports,
                evaluated,
                reused,
                ..
            } = baseline.evaluate(&change, &after)
            else {
                panic!("expected evaluated")
            };
            assert_eq!(
                serde_json::to_value(&reports).unwrap(),
                serde_json::to_value(&expected).unwrap()
            );
            assert_eq!(evaluated + reused, reports.len());
            assert!(reused > 0);
        }
    }
    #[test]
    fn independent_gates_match_full_evaluation_without_route_link_projections() {
        let (mut snapshot, preset, mut change) = fixture();
        for item in &mut snapshot.items {
            item.links = None;
        }
        let updated = item("a", "updated article", None);
        let created = item("new", "new article", None);
        let renamed = item("renamed", "renamed article", None);
        change.entries.push(entry(
            "a",
            Operation::Update,
            Some(&snapshot.items[0]),
            Some(&updated),
        ));
        change.entries.push(entry(
            "b",
            Operation::Delete,
            Some(&snapshot.items[1]),
            None,
        ));
        change
            .entries
            .push(entry("new", Operation::Create, None, Some(&created)));
        let mut rename = entry(
            "renamed",
            Operation::Rename,
            Some(&snapshot.items[2]),
            Some(&renamed),
        );
        rename.previous_entity_id = Some("c".into());
        change.entries.push(rename);
        let after = [updated, created, renamed];
        let overlay = overlay_candidate(&snapshot, &change, &after).unwrap();
        assert!(!overlay.impact.link_impact_complete);
        let expected = full(&overlay.candidate, &preset);
        let baseline = IncrementalBaseline::prepare(snapshot, preset).unwrap();
        let IncrementalEvaluation::Evaluated {
            reports,
            evaluated,
            reused,
            ..
        } = baseline.evaluate(&change, &after)
        else {
            panic!("independent item evaluation should not require a route graph")
        };
        assert_eq!(
            serde_json::to_value(reports).unwrap(),
            serde_json::to_value(expected).unwrap()
        );
        assert_eq!(evaluated, 3);
        assert_eq!(reused, 2);
    }

    #[test]
    fn unchanged_link_neighbours_reuse_independent_item_reports() {
        let (snapshot, preset, mut change) = fixture();
        let updated = item("a", "new source words", Some(vec!["/c"]));
        change.entries.push(entry(
            "a",
            Operation::Update,
            Some(&snapshot.items[0]),
            Some(&updated),
        ));
        let overlay =
            overlay_candidate(&snapshot, &change, std::slice::from_ref(&updated)).unwrap();
        assert!(overlay.impact.affected.iter().any(|id| id == "d"));
        let expected = full(&overlay.candidate, &preset);
        let baseline = IncrementalBaseline::prepare(snapshot, preset).unwrap();
        let IncrementalEvaluation::Evaluated {
            reports,
            evaluated,
            reused,
            ..
        } = baseline.evaluate(&change, &[updated])
        else {
            panic!("expected evaluation")
        };
        assert_eq!(
            serde_json::to_value(reports).unwrap(),
            serde_json::to_value(expected).unwrap()
        );
        assert_eq!(evaluated, 1);
        assert_eq!(reused, 4);
    }
    #[test]
    fn text_only_gates_match_full_evaluation_for_each_gate() {
        for gate in [
            "kind='sentence_length'\nmax_words=2\nabbreviations=['Dr.']",
            "kind='sentence_character_length'\nmax_chars=8\ndelimiters='.!?'",
            "kind='repeated_terms'\nterms=['café']\nmax_occurrences=1\ncase_insensitive=true",
            "kind='required_terms'\nterms=['café', 'absent']\ncase_insensitive=true",
            "kind='forbidden_terms'\nterms=['café']\ncase_insensitive=true",
        ] {
            let (snapshot, _, mut change) = fixture();
            let preset = ContentPreset::from_toml(&format!(
                "version=1\nname='text'\n[[gates]]\nid='check'\n{gate}\n"
            ))
            .unwrap();
            let updated = item(
                "a",
                "Dr. Café CAFÉ. Other words here!\r\nNext sentence.",
                Some(vec![]),
            );
            change.entries.push(entry(
                "a",
                Operation::Update,
                Some(&snapshot.items[0]),
                Some(&updated),
            ));
            let after = [updated];
            let candidate = overlay_candidate(&snapshot, &change, &after)
                .unwrap()
                .candidate;
            let expected = full(&candidate, &preset);
            let baseline = IncrementalBaseline::prepare(snapshot, preset).unwrap();
            let IncrementalEvaluation::Evaluated {
                reports,
                evaluated,
                reused,
                ..
            } = baseline.evaluate(&change, &after)
            else {
                panic!("gate did not evaluate: {gate}");
            };
            assert_eq!(
                serde_json::to_value(&reports).unwrap(),
                serde_json::to_value(&expected).unwrap(),
                "{gate}"
            );
            assert_eq!(evaluated + reused, candidate.items.len());
            assert!(evaluated > 0 && reused > 0, "{gate}");
        }
    }
    #[test]
    fn plain_final_newline_matches_full_after_all_change_operations() {
        let (mut snapshot, _, mut change) = fixture();
        for (original, source) in
            snapshot
                .items
                .iter_mut()
                .zip(["old\n", "remove", "rename\r\n", "", "unchanged\n"])
        {
            *original = item(&original.entity_id, source, None);
        }
        let preset = ContentPreset::from_toml(
            "version=1\nname='newline'\n[[gates]]\nid='newline'\nkind='final_newline'\n",
        )
        .unwrap();
        let updated = item("a", "café without newline", None);
        let created = item("new", "created\n", None);
        let renamed = item("renamed", "renamed\r\n", None);
        change.entries.push(entry(
            "a",
            Operation::Update,
            Some(&snapshot.items[0]),
            Some(&updated),
        ));
        change.entries.push(entry(
            "b",
            Operation::Delete,
            Some(&snapshot.items[1]),
            None,
        ));
        change
            .entries
            .push(entry("new", Operation::Create, None, Some(&created)));
        let mut rename = entry(
            "renamed",
            Operation::Rename,
            Some(&snapshot.items[2]),
            Some(&renamed),
        );
        rename.previous_entity_id = Some("c".into());
        change.entries.push(rename);
        let after = [updated, created, renamed];
        let candidate = overlay_candidate(&snapshot, &change, &after)
            .unwrap()
            .candidate;
        let expected: BTreeMap<_, _> = candidate
            .items
            .iter()
            .map(|item| {
                let input = ContentInput {
                    id: item.entity_id.clone(),
                    text: Some(item.source.clone()),
                    source_text: Some(item.source.clone()),
                    ..Default::default()
                };
                (item.entity_id.clone(), preset.evaluate(&input).unwrap())
            })
            .collect();
        let baseline = IncrementalBaseline::prepare(snapshot, preset).unwrap();
        let IncrementalEvaluation::Evaluated {
            reports,
            evaluated,
            reused,
            ..
        } = baseline.evaluate(&change, &after)
        else {
            panic!("expected complete evaluation")
        };
        assert_eq!(
            serde_json::to_value(&reports).unwrap(),
            serde_json::to_value(expected).unwrap()
        );
        assert_eq!((evaluated, reused), (3, 2));
        assert_eq!(reports["a"].gates[0].status, GateStatus::Failed);
        assert_eq!(reports["d"].gates[0].status, GateStatus::Passed);
        assert_eq!(reports["renamed"].gates[0].status, GateStatus::Passed);
    }

    #[test]
    fn paragraph_text_gates_match_fresh_evaluation_after_change() {
        for gate in [
            "kind='paragraph_count'\nmin=2",
            "kind='paragraph_length'\nmax_words=3",
            "kind='paragraph_character_length'\nmax_chars=12",
            "kind='repeated_paragraph_openings'\nopening_words=1\nmax_occurrences=1",
            "kind='repeated_sentence_openings'\nopening_words=1\nmax_consecutive=1",
            "kind='sentence_opening_window'\nopening_words=1\nwindow_size=4\nmax_occurrences=1",
            "kind='sentence_opening_terms'\nterms=['Moreover']\nmax_per_paragraph=0",
            "kind='repeated_punctuation'\nmark='—'\nmax_per_paragraph=0",
            "kind='trailing_ing_clause'",
            "kind='similar_sentence_lengths'\nrun_length=3\ntolerance_words=1",
            "kind='sentence_length_variance'\nwindow_size=4\nrandom_min_stddev_words=1\nrandom_max_stddev_words=3\nrandom_seed=42",
            "kind='inline_triple_candidates'\nconjunction='and'",
            "kind='duplicate_paragraphs'\nmax_occurrences=1\nmin_chars=4",
            "kind='sentence_length'\nmax_words=2\nparagraphs_only=true",
            "kind='sentence_character_length'\nmax_chars=8\nparagraphs_only=true",
            "kind='final_newline'",
        ] {
            let (mut snapshot, _, mut change) = fixture();
            for original in &mut snapshot.items {
                *original = item(
                    &original.entity_id,
                    "The team works. The team plans. Another team acts. More people help.\n\nMoreover, ideas matter.\n",
                    Some(vec![]),
                );
            }
            let preset = ContentPreset::from_toml(&format!(
                "version=1\nname='paragraphs'\n[[gates]]\nid='check'\n{gate}\n"
            ))
            .unwrap();
            let updated = item(
                "a",
                "Café works. Café plans. Café acts. Café checks.\r\n\r\nMoreover, one, two, and three — help, making progress.",
                Some(vec![]),
            );
            change.entries.push(entry(
                "a",
                Operation::Update,
                Some(&snapshot.items[0]),
                Some(&updated),
            ));
            let candidate = overlay_candidate(&snapshot, &change, std::slice::from_ref(&updated))
                .unwrap()
                .candidate;
            let expected: BTreeMap<_, _> = candidate
                .items
                .iter()
                .map(|item| {
                    let input = ContentInput {
                        id: item.entity_id.clone(),
                        text: Some(item.source.clone()),
                        source_text: Some(item.source.clone()),
                        ..Default::default()
                    };
                    let context =
                        crate::plain_text_adapter::paragraph_text_context(&item.source).unwrap();
                    (
                        item.entity_id.clone(),
                        preset
                            .evaluate_with_context(&input, None, None, Some(&context))
                            .unwrap(),
                    )
                })
                .collect();
            let baseline = IncrementalBaseline::prepare_paragraph_text(snapshot, preset).unwrap();
            let IncrementalEvaluation::Evaluated {
                reports,
                evaluated,
                reused,
                ..
            } = baseline.evaluate(&change, &[updated])
            else {
                panic!("paragraph gate did not evaluate: {gate}");
            };
            assert_eq!(
                serde_json::to_value(&reports).unwrap(),
                serde_json::to_value(&expected).unwrap(),
                "{gate}"
            );
            assert_eq!(evaluated + reused, candidate.items.len());
            assert!(evaluated > 0 && reused > 0, "{gate}");
        }
    }

    #[test]
    fn paragraph_mode_reuses_only_unchanged_items_for_all_operations() {
        let (snapshot, _, mut change) = fixture();
        let preset = ContentPreset::from_toml("version=1\nname='paragraphs'\n[[gates]]\nid='p'\nkind='paragraph_count'\nmin=2\n[[gates]]\nid='n'\nkind='final_newline'\n").unwrap();
        let updated = item("a", "Updated text.\n\nMore text.\n", Some(vec![]));
        let created = item("new", "New.\n\nContent.\n", Some(vec![]));
        let renamed = item("renamed", "Renamed.\n\nContent.\n", Some(vec![]));
        change.entries.push(entry(
            "a",
            Operation::Update,
            Some(&snapshot.items[0]),
            Some(&updated),
        ));
        change.entries.push(entry(
            "b",
            Operation::Delete,
            Some(&snapshot.items[1]),
            None,
        ));
        change
            .entries
            .push(entry("new", Operation::Create, None, Some(&created)));
        let mut rename = entry(
            "renamed",
            Operation::Rename,
            Some(&snapshot.items[2]),
            Some(&renamed),
        );
        rename.previous_entity_id = Some("c".into());
        change.entries.push(rename);
        let after = [updated, created, renamed];
        let candidate = overlay_candidate(&snapshot, &change, &after)
            .unwrap()
            .candidate;
        let expected: BTreeMap<_, _> = candidate
            .items
            .iter()
            .map(|item| {
                let input = ContentInput {
                    id: item.entity_id.clone(),
                    text: Some(item.source.clone()),
                    source_text: Some(item.source.clone()),
                    ..Default::default()
                };
                let context =
                    crate::plain_text_adapter::paragraph_text_context(&item.source).unwrap();
                (
                    item.entity_id.clone(),
                    preset
                        .evaluate_with_context(&input, None, None, Some(&context))
                        .unwrap(),
                )
            })
            .collect();
        let baseline = IncrementalBaseline::prepare_paragraph_text(snapshot, preset).unwrap();
        let IncrementalEvaluation::Evaluated {
            reports,
            evaluated,
            reused,
            ..
        } = baseline.evaluate(&change, &after)
        else {
            panic!("expected evaluated")
        };
        assert_eq!(
            serde_json::to_value(&reports).unwrap(),
            serde_json::to_value(&expected).unwrap()
        );
        assert_eq!(evaluated + reused, candidate.items.len());
        assert!(reused > 0);
    }

    #[test]
    fn adapted_incremental_matches_full_for_mixed_formats_and_operations() {
        let (mut snapshot, _, mut change) = fixture();
        for (index, item) in snapshot.items.iter_mut().enumerate() {
            item.media_type = if index % 2 == 0 {
                "text/markdown"
            } else {
                "text/html"
            }
            .into();
            item.source = if index % 2 == 0 {
                "# Heading\n\nSome [text](/b).\n"
            } else {
                "<h1>Heading</h1><p>Some <a href='/b'>text</a>.</p>\n"
            }
            .into();
            item.sha256 = format!("{:x}", Sha256::digest(item.source.as_bytes()));
            item.links = None;
        }
        let preset = ContentPreset::from_toml("version=1\nname='mixed'\n[[gates]]\nid='words'\nkind='word_count'\nmin=2\n[[gates]]\nid='paras'\nkind='paragraph_count'\nmin=1\n[[gates]]\nid='heads'\nkind='heading_length'\nmax_chars=5\n[[gates]]\nid='links'\nkind='link_count'\nmin=1\n[[gates]]\nid='newline'\nkind='final_newline'\n").unwrap();
        let adapted_item = |id: &str, media: &str, source: &str| {
            let mut value = item(id, source, None);
            value.media_type = media.into();
            value
        };
        let updated = adapted_item(
            "a",
            "text/markdown",
            "# New heading\n\nNew [link](/e) and words.\n",
        );
        let created = adapted_item(
            "new",
            "text/html",
            "<h1>Created</h1><p><a href='/e'>Created text</a>.</p>\n",
        );
        let renamed = adapted_item(
            "renamed",
            "text/markdown",
            "# Renamed\n\n[Read](/e) this text.\n",
        );
        let mut update = entry(
            "a",
            Operation::Update,
            Some(&snapshot.items[0]),
            Some(&updated),
        );
        update.media_type = updated.media_type.clone();
        let mut delete = entry("b", Operation::Delete, Some(&snapshot.items[1]), None);
        delete.media_type = snapshot.items[1].media_type.clone();
        let mut create = entry("new", Operation::Create, None, Some(&created));
        create.media_type = created.media_type.clone();
        let mut rename = entry(
            "renamed",
            Operation::Rename,
            Some(&snapshot.items[2]),
            Some(&renamed),
        );
        rename.media_type = renamed.media_type.clone();
        rename.previous_entity_id = Some("c".into());
        change.entries = vec![update, delete, create, rename];
        let after = [updated, created, renamed];
        let overlay = overlay_candidate(&snapshot, &change, &after).unwrap();
        assert!(!overlay.impact.link_impact_complete);
        let candidate = overlay.candidate;
        let expected: BTreeMap<_, _> = candidate
            .items
            .iter()
            .map(|item| {
                let (input, context, link_count) = if item.media_type == "text/markdown" {
                    crate::markdown_adapter::markdown_input_with_links(
                        &item.entity_id,
                        &item.source,
                    )
                    .unwrap()
                } else {
                    let result =
                        crate::html_content_adapter::html_content(&item.entity_id, &item.source)
                            .unwrap();
                    (result.input, result.context, result.destinations.len())
                };
                (
                    item.entity_id.clone(),
                    preset
                        .evaluate_with_links(&input, None, None, Some(&context), Some(link_count))
                        .unwrap(),
                )
            })
            .collect();
        let baseline = IncrementalBaseline::prepare_adapted(snapshot, preset).unwrap();
        let IncrementalEvaluation::Evaluated {
            reports,
            evaluated,
            reused,
            ..
        } = baseline.evaluate(&change, &after)
        else {
            panic!("expected evaluated");
        };
        assert_eq!(
            serde_json::to_value(&reports).unwrap(),
            serde_json::to_value(&expected).unwrap()
        );
        assert_eq!(evaluated + reused, candidate.items.len());
        assert!(evaluated > 0 && reused > 0);
    }

    #[test]
    fn adapted_gates_match_full_evaluation_individually_for_markdown_and_html() {
        for media in ["text/markdown", "text/html"] {
            for gate in [
                "kind='word_count'\nmin=2",
                "kind='character_count'\nmin=3",
                "kind='paragraph_count'\nmin=1",
                "kind='paragraph_length'\nmax_words=3",
                "kind='paragraph_character_length'\nmax_chars=12",
                "kind='repeated_terms'\nterms=['team']\nmax_occurrences=1",
                "kind='repeated_paragraph_openings'\nopening_words=1\nmax_occurrences=1",
                "kind='repeated_sentence_openings'\nopening_words=1\nmax_consecutive=1",
                "kind='sentence_opening_window'\nopening_words=1\nwindow_size=4\nmax_occurrences=1",
                "kind='sentence_opening_terms'\nterms=['Moreover']\nmax_per_paragraph=0",
                "kind='repeated_punctuation'\nmark='—'\nmax_per_paragraph=0",
                "kind='trailing_ing_clause'",
                "kind='similar_sentence_lengths'\nrun_length=3\ntolerance_words=1",
                "kind='sentence_length_variance'\nwindow_size=4\nrandom_min_stddev_words=1\nrandom_max_stddev_words=3\nrandom_seed=42",
                "kind='inline_triple_candidates'\nconjunction='and'",
                "kind='duplicate_paragraphs'\nmax_occurrences=1\nmin_chars=4",
                "kind='sentence_length'\nmax_words=2\nparagraphs_only=true",
                "kind='sentence_character_length'\nmax_chars=8\nparagraphs_only=true",
                "kind='sentence_length'\nmax_words=2",
                "kind='sentence_character_length'\nmax_chars=8",
                "kind='required_terms'\nterms=['team']",
                "kind='forbidden_terms'\nterms=['team']",
                "kind='required_headings'\nheadings=['Heading']",
                "kind='heading_length'\nmax_chars=5",
                "kind='duplicate_headings'",
                "kind='link_count'\nmin=1",
                "kind='final_newline'",
            ] {
                let (mut snapshot, _, mut change) = fixture();
                let source = if media == "text/markdown" {
                    "# Heading\n\nThe team works. The team plans. [Read](/a) this. More people help.\n"
                } else {
                    "<h1>Heading</h1><p>The team works. The team plans. <a href='/a'>Read</a> this. More people help.</p>\n"
                };
                for item in &mut snapshot.items {
                    item.media_type = media.into();
                    item.source = source.into();
                    item.sha256 = format!("{:x}", Sha256::digest(source.as_bytes()));
                }
                let preset = ContentPreset::from_toml(&format!(
                    "version=1\nname='single'\n[[gates]]\nid='check'\n{gate}\n"
                ))
                .unwrap();
                let mut next = snapshot.items[0].clone();
                next.source = next.source.replace("works", "changes");
                next.sha256 = format!("{:x}", Sha256::digest(next.source.as_bytes()));
                let mut update = entry(
                    "a",
                    Operation::Update,
                    Some(&snapshot.items[0]),
                    Some(&next),
                );
                update.media_type = media.into();
                change.entries.push(update);
                let candidate = overlay_candidate(&snapshot, &change, std::slice::from_ref(&next))
                    .unwrap()
                    .candidate;
                let expected: BTreeMap<_, _> = candidate
                    .items
                    .iter()
                    .map(|item| {
                        let (input, context, count) = if media == "text/markdown" {
                            crate::markdown_adapter::markdown_input_with_links(
                                &item.entity_id,
                                &item.source,
                            )
                            .unwrap()
                        } else {
                            let html = crate::html_content_adapter::html_content(
                                &item.entity_id,
                                &item.source,
                            )
                            .unwrap();
                            (html.input, html.context, html.destinations.len())
                        };
                        (
                            item.entity_id.clone(),
                            preset
                                .evaluate_with_links(
                                    &input,
                                    None,
                                    None,
                                    Some(&context),
                                    Some(count),
                                )
                                .unwrap(),
                        )
                    })
                    .collect();
                let baseline = IncrementalBaseline::prepare_adapted(snapshot, preset).unwrap();
                let IncrementalEvaluation::Evaluated {
                    reports, reused, ..
                } = baseline.evaluate(&change, &[next])
                else {
                    panic!("not checked: {media} {gate}")
                };
                assert_eq!(
                    serde_json::to_value(reports).unwrap(),
                    serde_json::to_value(expected).unwrap(),
                    "{media} {gate}"
                );
                assert!(reused > 0, "{media} {gate}");
            }
        }
    }

    #[test]
    fn adapted_incremental_reuses_mixed_plain_markdown_and_html_items() {
        let (mut snapshot, _, mut change) = fixture();
        for (index, item) in snapshot.items.iter_mut().enumerate() {
            let (media, source) = match index % 3 {
                0 => ("text/plain", "Plain paragraph.\n"),
                1 => ("text/markdown", "# Heading\n\nMarkdown paragraph.\n"),
                _ => ("text/html", "<h1>Heading</h1><p>HTML paragraph.</p>\n"),
            };
            item.media_type = media.into();
            item.source = source.into();
            item.sha256 = format!("{:x}", Sha256::digest(source.as_bytes()));
        }
        let preset = ContentPreset::from_toml("version=1\nname='mixed'\n[[gates]]\nid='paras'\nkind='paragraph_count'\nmin=1\n[[gates]]\nid='words'\nkind='word_count'\nmin=2\n").unwrap();
        let mut next = snapshot.items[0].clone();
        next.source = "New plain paragraph.\n".into();
        next.sha256 = format!("{:x}", Sha256::digest(next.source.as_bytes()));
        change.entries.push(entry(
            "a",
            Operation::Update,
            Some(&snapshot.items[0]),
            Some(&next),
        ));
        let candidate = overlay_candidate(&snapshot, &change, std::slice::from_ref(&next))
            .unwrap()
            .candidate;
        let expected: BTreeMap<_, _> = candidate
            .items
            .iter()
            .map(|item| {
                let (input, context, count) = match item.media_type.as_str() {
                    "text/plain" => (
                        ContentInput {
                            id: item.entity_id.clone(),
                            text: Some(item.source.clone()),
                            source_text: Some(item.source.clone()),
                            ..Default::default()
                        },
                        crate::plain_text_adapter::paragraph_text_context(&item.source).unwrap(),
                        None,
                    ),
                    "text/markdown" => {
                        let (input, context, count) =
                            crate::markdown_adapter::markdown_input_with_links(
                                &item.entity_id,
                                &item.source,
                            )
                            .unwrap();
                        (input, context, Some(count))
                    }
                    "text/html" => {
                        let html = crate::html_content_adapter::html_content(
                            &item.entity_id,
                            &item.source,
                        )
                        .unwrap();
                        let count = html.destinations.len();
                        (html.input, html.context, Some(count))
                    }
                    _ => unreachable!(),
                };
                (
                    item.entity_id.clone(),
                    preset
                        .evaluate_with_links(&input, None, None, Some(&context), count)
                        .unwrap(),
                )
            })
            .collect();
        let baseline = IncrementalBaseline::prepare_adapted(snapshot, preset).unwrap();
        let IncrementalEvaluation::Evaluated {
            reports, reused, ..
        } = baseline.evaluate(&change, &[next])
        else {
            panic!("expected evaluation")
        };
        assert_eq!(
            serde_json::to_value(reports).unwrap(),
            serde_json::to_value(expected).unwrap()
        );
        assert!(reused > 0);
    }

    #[test]
    fn markdown_lists_incremental_matches_full_and_refuses_unknown_structure() {
        let (mut snapshot, _, mut change) = fixture();
        for item in &mut snapshot.items {
            item.media_type = "text/markdown".into();
            item.source = "# Heading\n\n- one\n- two\n- three\n".into();
            item.sha256 = format!("{:x}", Sha256::digest(item.source.as_bytes()));
        }
        let preset = ContentPreset::from_toml("version=1\nname='mixed'\n[[gates]]\nid='lists'\nkind='list_item_count'\nflagged_counts=[3]\n[[gates]]\nid='heads'\nkind='heading_hierarchy'\n").unwrap();
        let baseline =
            IncrementalBaseline::prepare_adapted(snapshot.clone(), preset.clone()).unwrap();
        let mut next = snapshot.items[0].clone();
        next.source = "# Heading\n\n- one\n- two\n".into();
        next.sha256 = format!("{:x}", Sha256::digest(next.source.as_bytes()));
        let mut update = entry(
            "a",
            Operation::Update,
            Some(&snapshot.items[0]),
            Some(&next),
        );
        update.media_type = next.media_type.clone();
        change.entries.push(update);
        let candidate = overlay_candidate(&snapshot, &change, std::slice::from_ref(&next))
            .unwrap()
            .candidate;
        let expected: BTreeMap<_, _> = candidate
            .items
            .iter()
            .map(|item| {
                let (input, context, count, map) =
                    crate::markdown_adapter::markdown_input_with_source_map(
                        &item.entity_id,
                        &item.source,
                    )
                    .unwrap();
                let lists = crate::markdown_adapter::markdown_structured_lists(&item.source)
                    .unwrap()
                    .unwrap();
                let caps = ContentEvaluationContext {
                    adapter: Some(&context),
                    link_count: Some(count),
                    ..Default::default()
                };
                (
                    item.entity_id.clone(),
                    preset
                        .evaluate_with_structured_lists(
                            &input,
                            &caps,
                            None,
                            map.heading_levels(),
                            true,
                            None,
                            Some(&lists),
                        )
                        .unwrap(),
                )
            })
            .collect();
        let IncrementalEvaluation::Evaluated {
            reports,
            evaluated,
            reused,
            ..
        } = baseline.evaluate(&change, std::slice::from_ref(&next))
        else {
            panic!("expected evaluated")
        };
        assert_eq!(
            serde_json::to_value(&reports).unwrap(),
            serde_json::to_value(&expected).unwrap()
        );
        assert_eq!(reports["a"].gates[0].status, GateStatus::Passed);
        assert!(evaluated > 0 && reused > 0);
        next.source = "<ul><li>unknown</li></ul>".into();
        next.sha256 = format!("{:x}", Sha256::digest(next.source.as_bytes()));
        change.entries[0].after = Some(state(&next));
        assert!(matches!(
            baseline.evaluate(&change, &[next]),
            IncrementalEvaluation::NotChecked { .. }
        ));
        let mut html = snapshot;
        html.items[0].media_type = "text/html".into();
        html.items[0].source = "<ul hidden><li>unknown</li></ul>".into();
        html.items[0].sha256 = format!("{:x}", Sha256::digest(html.items[0].source.as_bytes()));
        assert!(IncrementalBaseline::prepare_adapted(html, preset).is_err());
    }

    #[test]
    fn html_main_incremental_matches_full_and_rejects_missing_scope() {
        use crate::html_content_adapter::{
            HtmlContentScope, html_content_with_paragraph_list_items, html_structured_lists,
        };
        let (mut snapshot, _, mut change) = fixture();
        for item in &mut snapshot.items {
            item.media_type = "text/html".into();
            item.source = "<nav><a href='/nav'>navigation ignored</a></nav><main><p>Before <a href='/article'>source</a>.</p><ul><li>one</li><li>two</li><li>three</li></ul></main><footer>footer ignored</footer>".into();
            item.sha256 = format!("{:x}", Sha256::digest(item.source.as_bytes()));
        }
        let preset = ContentPreset::from_toml("version=1\nname='main'\n[[gates]]\nid='words'\nkind='word_count'\nmin=1\n[[gates]]\nid='lists'\nkind='list_item_count'\nflagged_counts=[3]\n[[gates]]\nid='links'\nkind='link_count'\nmin=1\n").unwrap();
        let markdown = snapshot.items.last_mut().unwrap();
        markdown.media_type = "text/markdown".into();
        markdown.source = "[source](/article)\n\n- one\n- two\n- three\n".into();
        markdown.sha256 = format!("{:x}", Sha256::digest(markdown.source.as_bytes()));
        let baseline = IncrementalBaseline::prepare_adapted_with_html_scope(
            snapshot.clone(),
            preset.clone(),
            HtmlContentScope::Main,
        )
        .unwrap();
        let mut next = snapshot.items[0].clone();
        next.source = "<nav><a href='/nav'>ignored</a></nav><main><p>Après <a href='/article'>source</a>.</p><ul><li>one</li><li>two</li></ul></main><footer>ignored</footer>".into();
        next.sha256 = format!("{:x}", Sha256::digest(next.source.as_bytes()));
        let mut update = entry(
            "a",
            Operation::Update,
            Some(&snapshot.items[0]),
            Some(&next),
        );
        update.media_type = next.media_type.clone();
        change.entries.push(update);
        let (adapted, _, _) = html_content_with_paragraph_list_items(
            &next.entity_id,
            &next.source,
            HtmlContentScope::Main,
        )
        .unwrap();
        let lists = html_structured_lists(
            &next.source,
            adapted.input.text.as_deref().unwrap(),
            HtmlContentScope::Main,
        )
        .unwrap();
        let caps = ContentEvaluationContext {
            adapter: Some(&adapted.context),
            link_count: Some(adapted.destinations.len()),
            ..Default::default()
        };
        let expected = preset
            .evaluate_with_structured_lists(
                &adapted.input,
                &caps,
                None,
                None,
                true,
                None,
                Some(&lists),
            )
            .unwrap();
        let IncrementalEvaluation::Evaluated {
            reports, reused, ..
        } = baseline.evaluate(&change, std::slice::from_ref(&next))
        else {
            panic!("expected evaluated");
        };
        assert_eq!(
            serde_json::to_value(&reports["a"]).unwrap(),
            serde_json::to_value(expected).unwrap()
        );
        assert_eq!(reports["a"].gates[2].measurements["link_count"], 1);
        assert!(reused > 0);

        for invalid in [
            "<p>No main</p>",
            "<main>One</main><main>Two</main>",
            "<main hidden>Hidden</main>",
        ] {
            next.source = invalid.into();
            next.sha256 = format!("{:x}", Sha256::digest(next.source.as_bytes()));
            change.entries[0].after = Some(state(&next));
            assert!(
                matches!(
                    baseline.evaluate(&change, std::slice::from_ref(&next)),
                    IncrementalEvaluation::NotChecked { .. }
                ),
                "{invalid}"
            );
        }
        snapshot.items[0].source = "<p>No main</p>".into();
        snapshot.items[0].sha256 =
            format!("{:x}", Sha256::digest(snapshot.items[0].source.as_bytes()));
        assert!(
            IncrementalBaseline::prepare_adapted_with_html_scope(
                snapshot,
                preset,
                HtmlContentScope::Main,
            )
            .is_err()
        );
    }

    #[test]
    fn literal_html_links_incremental_matches_full_and_withholds_unknown_inputs() {
        use crate::html_content_adapter::{
            HtmlContentScope, html_content_with_literal_markdown_projection,
        };
        let (mut snapshot, _, mut change) = fixture();
        for item in &mut snapshot.items {
            item.media_type = "text/html".into();
            item.source =
                "<main><p>Read [source](/guide).</p><code>[sample](/code)</code></main>".into();
            item.sha256 = format!("{:x}", Sha256::digest(item.source.as_bytes()));
        }
        let preset = ContentPreset::from_toml("version=1\nname='links'\n[[gates]]\nid='literal'\nkind='literal_markdown_links'\n[[gates]]\nid='words'\nkind='word_count'\nmin=1\n").unwrap();
        let baseline = IncrementalBaseline::prepare_adapted_with_html_scope(
            snapshot.clone(),
            preset.clone(),
            HtmlContentScope::Main,
        )
        .unwrap();
        let mut next = snapshot.items[0].clone();
        next.source =
            "<main><p>Read <strong>[new]</strong>(/guide).</p><a href='/ok'>[real](/ok)</a></main>"
                .into();
        next.sha256 = format!("{:x}", Sha256::digest(next.source.as_bytes()));
        let mut update = entry(
            "a",
            Operation::Update,
            Some(&snapshot.items[0]),
            Some(&next),
        );
        update.media_type = next.media_type.clone();
        change.entries.push(update);
        let (adapted, _, _, _) = html_content_with_literal_markdown_projection(
            &next.entity_id,
            &next.source,
            HtmlContentScope::Main,
        )
        .unwrap();
        let expected = preset
            .evaluate_with_context(&adapted.input, None, None, Some(&adapted.context))
            .unwrap();
        let IncrementalEvaluation::Evaluated {
            reports,
            evaluated,
            reused,
            ..
        } = baseline.evaluate(&change, std::slice::from_ref(&next))
        else {
            panic!("expected evaluated")
        };
        assert_eq!(
            serde_json::to_value(&reports["a"]).unwrap(),
            serde_json::to_value(expected).unwrap()
        );
        assert_eq!(reports["a"].gates[0].findings.len(), 1);
        assert_eq!(evaluated, 1);
        assert_eq!(reused, snapshot.items.len() - 1);
        for source in ["<main><p><b>[source](/x)</p></b></main>", "<p>No main</p>"] {
            next.source = source.into();
            next.sha256 = format!("{:x}", Sha256::digest(next.source.as_bytes()));
            change.entries[0].after = Some(state(&next));
            assert!(
                matches!(
                    baseline.evaluate(&change, std::slice::from_ref(&next)),
                    IncrementalEvaluation::NotChecked { .. }
                ),
                "{source}"
            );
        }
        let mut unknown = snapshot;
        unknown.items[0].media_type = "text/markdown".into();
        assert!(
            IncrementalBaseline::prepare_adapted_with_html_scope(
                unknown,
                preset,
                HtmlContentScope::Main
            )
            .is_err()
        );
    }

    #[test]
    fn html_lists_incremental_matches_full_and_withholds_hidden_structure() {
        let (mut snapshot, _, mut change) = fixture();
        for item in &mut snapshot.items {
            item.media_type = "text/html".into();
            item.source = "<h1>Heading</h1><ul><li>one</li><li>two</li><li>three</li></ul>".into();
            item.sha256 = format!("{:x}", Sha256::digest(item.source.as_bytes()));
        }
        let preset = ContentPreset::from_toml("version=1\nname='lists'\n[[gates]]\nid='lists'\nkind='list_item_count'\nflagged_counts=[3]\n").unwrap();
        let baseline =
            IncrementalBaseline::prepare_adapted(snapshot.clone(), preset.clone()).unwrap();
        let mut next = snapshot.items[0].clone();
        next.source = "<h1>Heading</h1><ul><li>one</li><li>two</li></ul>".into();
        next.sha256 = format!("{:x}", Sha256::digest(next.source.as_bytes()));
        let mut update = entry(
            "a",
            Operation::Update,
            Some(&snapshot.items[0]),
            Some(&next),
        );
        update.media_type = next.media_type.clone();
        change.entries.push(update);
        let scope = crate::html_content_adapter::HtmlContentScope::Document;
        let (adapted, _, _) = crate::html_content_adapter::html_content_with_paragraph_list_items(
            &next.entity_id,
            &next.source,
            scope,
        )
        .unwrap();
        let lists = crate::html_content_adapter::html_structured_lists(
            &next.source,
            adapted.input.text.as_deref().unwrap(),
            scope,
        )
        .unwrap();
        let caps = ContentEvaluationContext {
            adapter: Some(&adapted.context),
            link_count: Some(adapted.destinations.len()),
            ..Default::default()
        };
        let expected = preset
            .evaluate_with_structured_lists(
                &adapted.input,
                &caps,
                None,
                None,
                true,
                None,
                Some(&lists),
            )
            .unwrap();
        let IncrementalEvaluation::Evaluated {
            reports, reused, ..
        } = baseline.evaluate(&change, std::slice::from_ref(&next))
        else {
            panic!("expected evaluated")
        };
        assert_eq!(
            serde_json::to_value(&reports["a"]).unwrap(),
            serde_json::to_value(expected).unwrap()
        );
        assert_eq!(reports["a"].gates[0].status, GateStatus::Passed);
        assert!(reused > 0);
        next.source = "<ul hidden><li>unknown</li></ul>".into();
        next.sha256 = format!("{:x}", Sha256::digest(next.source.as_bytes()));
        change.entries[0].after = Some(state(&next));
        assert!(matches!(
            baseline.evaluate(&change, &[next]),
            IncrementalEvaluation::NotChecked { .. }
        ));
    }

    #[test]
    fn heading_hierarchy_incremental_matches_full_and_withholds_unknown_levels() {
        let (mut snapshot, _, mut change) = fixture();
        for (index, item) in snapshot.items.iter_mut().enumerate() {
            let (media, source) = if index % 2 == 0 {
                ("text/markdown", "# One\n\n## Two\n\nText.\n")
            } else {
                ("text/html", "<h1>One</h1><h3>Three</h3><p>Text.</p>")
            };
            item.media_type = media.into();
            item.source = source.into();
            item.sha256 = format!("{:x}", Sha256::digest(source.as_bytes()));
        }
        let preset = ContentPreset::from_toml("version=1\nname='mixed'\n[[gates]]\nid='hierarchy'\nkind='heading_hierarchy'\ninitial_level=1\n[[gates]]\nid='words'\nkind='word_count'\nmin=1\n").unwrap();
        let baseline =
            IncrementalBaseline::prepare_adapted(snapshot.clone(), preset.clone()).unwrap();
        let mut updated = snapshot.items[0].clone();
        updated.source = "# One\n\n### Skipped\n\nUpdated.\n".into();
        updated.sha256 = format!("{:x}", Sha256::digest(updated.source.as_bytes()));
        let mut update = entry(
            "a",
            Operation::Update,
            Some(&snapshot.items[0]),
            Some(&updated),
        );
        update.media_type = updated.media_type.clone();
        change.entries.push(update);
        let candidate = overlay_candidate(&snapshot, &change, std::slice::from_ref(&updated))
            .unwrap()
            .candidate;
        let expected: BTreeMap<_, _> = candidate
            .items
            .iter()
            .map(|item| {
                let (input, context, count, levels) = if item.media_type == "text/markdown" {
                    let (input, context, count, map) =
                        crate::markdown_adapter::markdown_input_with_source_map(
                            &item.entity_id,
                            &item.source,
                        )
                        .unwrap();
                    (
                        input,
                        context,
                        count,
                        map.heading_levels().unwrap().to_vec(),
                    )
                } else {
                    let (html, levels) =
                        crate::html_content_adapter::html_content_with_scope_and_heading_levels(
                            &item.entity_id,
                            &item.source,
                            crate::html_content_adapter::HtmlContentScope::Document,
                        )
                        .unwrap();
                    let count = html.destinations.len();
                    (html.input, html.context, count, levels)
                };
                let capabilities = ContentEvaluationContext {
                    adapter: Some(&context),
                    link_count: Some(count),
                    ..Default::default()
                };
                (
                    item.entity_id.clone(),
                    preset
                        .evaluate_with_heading_levels(
                            &input,
                            &capabilities,
                            None,
                            Some(&levels),
                            true,
                            None,
                        )
                        .unwrap(),
                )
            })
            .collect();
        let IncrementalEvaluation::Evaluated {
            reports,
            evaluated,
            reused,
            ..
        } = baseline.evaluate(&change, std::slice::from_ref(&updated))
        else {
            panic!("expected evaluated")
        };
        assert_eq!(
            serde_json::to_value(&reports).unwrap(),
            serde_json::to_value(&expected).unwrap()
        );
        assert_eq!(reports["a"].gates[0].status, GateStatus::Failed);
        assert!(evaluated > 0 && reused > 0);

        let mut unknown = updated.clone();
        unknown.source = "# One\n\n<h3>Unobserved</h3>\n".into();
        unknown.sha256 = format!("{:x}", Sha256::digest(unknown.source.as_bytes()));
        change.entries.clear();
        let mut update = entry(
            "a",
            Operation::Update,
            Some(&snapshot.items[0]),
            Some(&unknown),
        );
        update.media_type = unknown.media_type.clone();
        change.entries.push(update);
        assert!(matches!(
            baseline.evaluate(&change, &[unknown]),
            IncrementalEvaluation::NotChecked { .. }
        ));
        let plain = fixture().0;
        assert!(IncrementalBaseline::prepare_adapted(plain, preset).is_err());
    }

    #[test]
    fn link_destinations_incremental_matches_full_and_withholds_unknown() {
        let (mut snapshot, _, mut change) = fixture();
        for (i, item) in snapshot.items.iter_mut().enumerate() {
            let (media, source) = if i % 2 == 0 {
                (
                    "text/markdown",
                    "# Heading\n\n[link](https://example.org)\n",
                )
            } else {
                (
                    "text/html",
                    "<h1>Heading</h1><a href=\"https://example.org\">link</a>",
                )
            };
            item.media_type = media.into();
            item.source = source.into();
            item.sha256 = format!("{:x}", Sha256::digest(source.as_bytes()));
        }
        let preset = ContentPreset::from_toml("version=1\nname='links'\n[[gates]]\nid='destinations'\nkind='link_destinations'\nallowed_schemes=['https']\nallow_relative=false\n[[gates]]\nid='heads'\nkind='heading_hierarchy'\n").unwrap();
        let baseline =
            IncrementalBaseline::prepare_adapted(snapshot.clone(), preset.clone()).unwrap();
        let mut next = snapshot.items[0].clone();
        next.source = "# Heading\n\n[link](http://example.org)\n".into();
        next.sha256 = format!("{:x}", Sha256::digest(next.source.as_bytes()));
        let mut update = entry(
            "a",
            Operation::Update,
            Some(&snapshot.items[0]),
            Some(&next),
        );
        update.media_type = next.media_type.clone();
        change.entries.push(update);
        let candidate = overlay_candidate(&snapshot, &change, std::slice::from_ref(&next))
            .unwrap()
            .candidate;
        let expected: BTreeMap<_, _> = candidate
            .items
            .iter()
            .map(|item| {
                let (input, context, count, destinations, levels) =
                    if item.media_type == "text/markdown" {
                        let adapted =
                        crate::markdown_adapter::markdown_input_with_source_map_and_destinations(
                            &item.entity_id,
                            &item.source,
                        )
                        .unwrap();
                        let levels = adapted.source_map.heading_levels().unwrap().to_vec();
                        (
                            adapted.input,
                            adapted.context,
                            adapted.link_count,
                            adapted.destinations.unwrap(),
                            levels,
                        )
                    } else {
                        let (adapted, levels, _) =
                            crate::html_content_adapter::html_content_with_paragraph_list_items(
                                &item.entity_id,
                                &item.source,
                                crate::html_content_adapter::HtmlContentScope::Document,
                            )
                            .unwrap();
                        let count = adapted.destinations.len();
                        (
                            adapted.input,
                            adapted.context,
                            count,
                            adapted.destinations,
                            levels,
                        )
                    };
                let caps = ContentEvaluationContext {
                    adapter: Some(&context),
                    link_count: Some(count),
                    ..Default::default()
                };
                (
                    item.entity_id.clone(),
                    preset
                        .evaluate_with_heading_levels(
                            &input,
                            &caps,
                            Some(&destinations),
                            Some(&levels),
                            true,
                            None,
                        )
                        .unwrap(),
                )
            })
            .collect();
        let IncrementalEvaluation::Evaluated {
            reports,
            evaluated,
            reused,
            ..
        } = baseline.evaluate(&change, std::slice::from_ref(&next))
        else {
            panic!("expected evaluated")
        };
        assert_eq!(
            serde_json::to_value(&reports).unwrap(),
            serde_json::to_value(expected).unwrap()
        );
        assert_eq!(reports["a"].gates[0].status, GateStatus::Failed);
        assert!(evaluated > 0 && reused > 0);
        next.source = "# Heading\n\n<a href=\"https://example.org\">unobserved</a>".into();
        next.sha256 = format!("{:x}", Sha256::digest(next.source.as_bytes()));
        change.entries[0].after = Some(state(&next));
        assert!(matches!(
            baseline.evaluate(&change, &[next]),
            IncrementalEvaluation::NotChecked { .. }
        ));
        let mut html = snapshot.items[1].clone();
        html.source = "<h1>Heading</h1><a href=\"http://example.org\">link</a>".into();
        html.sha256 = format!("{:x}", Sha256::digest(html.source.as_bytes()));
        let mut update = entry(
            "b",
            Operation::Update,
            Some(&snapshot.items[1]),
            Some(&html),
        );
        update.media_type = html.media_type.clone();
        change.entries[0] = update;
        let IncrementalEvaluation::Evaluated {
            reports, reused, ..
        } = baseline.evaluate(&change, &[html])
        else {
            panic!("expected evaluated HTML")
        };
        assert_eq!(reports["b"].gates[0].status, GateStatus::Failed);
        assert!(reused > 0);
        assert!(IncrementalBaseline::prepare_adapted(fixture().0, preset).is_err());
    }

    #[test]
    fn adapted_incremental_refuses_unobserved_html_links() {
        let (mut snapshot, _, _) = fixture();
        let preset = ContentPreset::from_toml(
            "version=1\nname='links'\n[[gates]]\nid='links'\nkind='link_count'\nmin=1\n",
        )
        .unwrap();
        for item in &mut snapshot.items {
            item.media_type = "text/markdown".into();
            item.source = "<a href=\"https://example.org\">source</a>".into();
            item.sha256 = format!("{:x}", Sha256::digest(item.source.as_bytes()));
        }
        assert!(IncrementalBaseline::prepare_adapted(snapshot, preset).is_err());
    }

    #[test]
    fn adapted_incremental_refuses_missing_capabilities_and_unknown_formats() {
        let (mut snapshot, _, mut change) = fixture();
        let preset = ContentPreset::from_toml(
            "version=1\nname='links'\n[[gates]]\nid='links'\nkind='link_count'\nmin=1\n",
        )
        .unwrap();
        assert!(IncrementalBaseline::prepare_adapted(snapshot.clone(), preset.clone()).is_err());
        for item in &mut snapshot.items {
            item.media_type = "text/markdown".into();
            item.source = "[link](/a)\n".into();
            item.sha256 = format!("{:x}", Sha256::digest(item.source.as_bytes()));
        }
        let baseline =
            IncrementalBaseline::prepare_adapted(snapshot.clone(), preset.clone()).unwrap();
        let mut next = item("a", "unknown", Some(vec![]));
        next.media_type = "application/json".into();
        let mut update = entry(
            "a",
            Operation::Update,
            Some(&snapshot.items[0]),
            Some(&next),
        );
        update.media_type = next.media_type.clone();
        change.entries.push(update);
        assert!(matches!(
            baseline.evaluate(&change, &[next]),
            IncrementalEvaluation::NotChecked { .. }
        ));
        let plain = item("a", "plain content", Some(vec![]));
        let mut plain_change = change.clone();
        plain_change.entries[0] = entry(
            "a",
            Operation::Update,
            Some(&snapshot.items[0]),
            Some(&plain),
        );
        assert!(matches!(
            baseline.evaluate(&plain_change, &[plain]),
            IncrementalEvaluation::NotChecked { .. }
        ));
        snapshot.complete = false;
        assert!(IncrementalBaseline::prepare_adapted(snapshot, preset).is_err());
    }

    #[test]
    fn paragraph_mode_rejects_unavailable_or_external_dependencies() {
        let (mut snapshot, _, mut change) = fixture();
        for original in &mut snapshot.items {
            *original = item(&original.entity_id, "Team writes.", Some(vec![]));
        }
        let preset = ContentPreset::from_toml("version=1\nname='opening'\n[[gates]]\nid='x'\nkind='repeated_paragraph_openings'\nopening_words=2\nmax_occurrences=1\n").unwrap();
        assert!(IncrementalBaseline::prepare(snapshot.clone(), preset.clone()).is_err());
        let baseline =
            IncrementalBaseline::prepare_paragraph_text(snapshot.clone(), preset).unwrap();
        let next = item("a", "中文句子。下一句。", Some(vec![]));
        change.entries.push(entry(
            "a",
            Operation::Update,
            Some(&snapshot.items[0]),
            Some(&next),
        ));
        assert!(matches!(
            baseline.evaluate(&change, &[next]),
            IncrementalEvaluation::NotChecked { .. }
        ));
        let external = ContentPreset::from_toml(
            "version=1\nname='external'\n[[gates]]\nid='x'\nkind='custom'\nextension='provider'\n",
        )
        .unwrap();
        assert!(IncrementalBaseline::prepare_paragraph_text(snapshot, external).is_err());
    }

    #[test]
    fn incomplete_dependencies_and_unsupported_scope_are_never_passed() {
        let (mut snapshot, preset, mut change) = fixture();
        snapshot.complete = false;
        assert!(IncrementalBaseline::prepare(snapshot.clone(), preset.clone()).is_err());
        snapshot.complete = true;
        snapshot.items[0].links = None;
        assert!(IncrementalBaseline::prepare(snapshot.clone(), preset.clone()).is_ok());
        snapshot.items[0].links = Some(vec![]);
        for gate in [
            "kind='sentence_length'\nmax_words=2\nparagraphs_only=true",
            "kind='sentence_character_length'\nmax_chars=8\nparagraphs_only=true",
            "kind='metadata_length'\nfield='title'\nmin=1",
            "kind='unique_collection_metadata'\nfield='title'",
            "kind='site_audit'",
            "kind='custom'\nextension='remote'",
        ] {
            let source = format!("version=1\nname='unsupported'\n[[gates]]\nid='other'\n{gate}\n");
            let unsupported = ContentPreset::from_toml(&source).unwrap();
            assert!(IncrementalBaseline::prepare(snapshot.clone(), unsupported).is_err());
        }
        let mut next = item("a", "new text", None);
        change.entries.push(entry(
            "a",
            Operation::Update,
            Some(&snapshot.items[0]),
            Some(&next),
        ));
        let baseline = IncrementalBaseline::prepare(snapshot.clone(), preset).unwrap();
        let mut deletion = change.clone();
        deletion.entries = vec![entry(
            "b",
            Operation::Delete,
            Some(&snapshot.items[1]),
            None,
        )];
        deletion.entries[0].media_type = "text/html".into();
        assert!(matches!(
            baseline.evaluate(&deletion, &[]),
            IncrementalEvaluation::NotChecked { .. }
        ));
        assert!(matches!(
            baseline.evaluate(&change, std::slice::from_ref(&next)),
            IncrementalEvaluation::Evaluated { .. }
        ));
        next.links = Some(vec![]);
        next.media_type = "text/html".into();
        change.entries[0].media_type = "text/html".into();
        assert!(matches!(
            baseline.evaluate(&change, &[next]),
            IncrementalEvaluation::NotChecked { .. }
        ));
    }
    #[test]
    fn stale_or_tampered_proposal_is_not_checked() {
        let (snapshot, preset, mut change) = fixture();
        let next = item("a", "new text", Some(vec![]));
        change.entries.push(entry(
            "a",
            Operation::Update,
            Some(&snapshot.items[0]),
            Some(&next),
        ));
        let baseline = IncrementalBaseline::prepare(snapshot, preset).unwrap();
        change.base.revision = "stale".into();
        assert!(matches!(
            baseline.evaluate(&change, std::slice::from_ref(&next)),
            IncrementalEvaluation::NotChecked { .. }
        ));
        change.base.revision = "base".into();
        change.entries[0].after.as_mut().unwrap().inline = None;
        change.entries[0].after.as_mut().unwrap().uri = Some("https://example.test/source".into());
        assert!(matches!(
            baseline.evaluate(&change, std::slice::from_ref(&next)),
            IncrementalEvaluation::NotChecked { .. }
        ));
        change.entries[0].after.as_mut().unwrap().uri = None;
        change.entries[0].after.as_mut().unwrap().inline = Some("tampered".into());
        assert!(matches!(
            baseline.evaluate(&change, &[next]),
            IncrementalEvaluation::NotChecked { .. }
        ));
    }
    #[test]
    fn adapted_prose_variance_reuses_only_complete_structured_observations() {
        let (mut snapshot, _, mut change) = fixture();
        let markdown = "One two. Three four. Five six.\n\n- List one. List two. List three.\n\nSeven eight. Nine ten. Eleven twelve.\n";
        let html = "<p>One two. Three four. Five six.</p><ul><li><p>List one. List two. List three.</p></li></ul><p>Seven eight. Nine ten. Eleven twelve.</p>";
        for (index, item) in snapshot.items.iter_mut().enumerate() {
            let (media, source) = if index % 2 == 0 {
                ("text/markdown", markdown)
            } else {
                ("text/html", html)
            };
            item.media_type = media.into();
            item.source = source.into();
            item.sha256 = format!("{:x}", Sha256::digest(source.as_bytes()));
        }
        let preset = ContentPreset::from_toml("version=1\nname='prose'\n[[gates]]\nid='v'\nkind='prose_sentence_length_variance'\nwindow_size=3\nmin_stddev_words=2\n[[gates]]\nid='hierarchy'\nkind='heading_hierarchy'\n").unwrap();
        let mut next = snapshot.items[0].clone();
        next.source = format!(
            "{}\nAnother sentence. Another short sentence. Another last sentence.\n",
            markdown
        );
        next.sha256 = format!("{:x}", Sha256::digest(next.source.as_bytes()));
        change.entries.push(entry(
            "a",
            Operation::Update,
            Some(&snapshot.items[0]),
            Some(&next),
        ));
        change.entries.last_mut().unwrap().media_type = next.media_type.clone();
        let candidate = overlay_candidate(&snapshot, &change, std::slice::from_ref(&next))
            .unwrap()
            .candidate;
        let expected: BTreeMap<_, _> = candidate
            .items
            .iter()
            .map(|item| {
                let (input, context, levels, lists) = if item.media_type == "text/markdown" {
                    let (input, context, _, map) =
                        crate::markdown_adapter::markdown_input_with_source_map(
                            &item.entity_id,
                            &item.source,
                        )
                        .unwrap();
                    (
                        input,
                        context,
                        map.heading_levels().unwrap().to_vec(),
                        map.paragraph_list_items().unwrap().to_vec(),
                    )
                } else {
                    let (adapted, levels, lists) =
                        crate::html_content_adapter::html_content_with_paragraph_list_items(
                            &item.entity_id,
                            &item.source,
                            crate::html_content_adapter::HtmlContentScope::Document,
                        )
                        .unwrap();
                    (adapted.input, adapted.context, levels, lists.unwrap())
                };
                let caps = ContentEvaluationContext {
                    adapter: Some(&context),
                    ..Default::default()
                };
                let report = preset
                    .evaluate_with_paragraph_list_items(
                        &input,
                        &caps,
                        None,
                        Some(&levels),
                        false,
                        None,
                        None,
                        Some(&lists),
                    )
                    .unwrap();
                (item.entity_id.clone(), report)
            })
            .collect();
        let baseline =
            IncrementalBaseline::prepare_adapted(snapshot.clone(), preset.clone()).unwrap();
        let IncrementalEvaluation::Evaluated {
            reports,
            evaluated,
            reused,
            ..
        } = baseline.evaluate(&change, std::slice::from_ref(&next))
        else {
            panic!("expected evaluation")
        };
        assert_eq!(
            serde_json::to_value(reports).unwrap(),
            serde_json::to_value(expected).unwrap()
        );
        assert!(evaluated > 0 && reused > 0);
        let mut uncertain = next.clone();
        uncertain.source = "Prose. Another. Third.\n\n- \n".into();
        uncertain.sha256 = format!("{:x}", Sha256::digest(uncertain.source.as_bytes()));
        let mut uncertain_change = change.clone();
        uncertain_change.entries[0].after = Some(state(&uncertain));
        assert!(matches!(
            baseline.evaluate(&uncertain_change, &[uncertain]),
            IncrementalEvaluation::NotChecked { .. }
        ));
        for (media, source) in [
            ("text/markdown", "Prose. Another. Third.\n\n- \n"),
            (
                "text/html",
                "<p>Prose. Another. Third.</p><ul><li>unrepresented</li></ul>",
            ),
            ("text/plain", "Prose. Another. Third."),
        ] {
            let mut incomplete = snapshot.clone();
            incomplete.items[0].media_type = media.into();
            incomplete.items[0].source = source.into();
            incomplete.items[0].sha256 = format!("{:x}", Sha256::digest(source.as_bytes()));
            assert!(
                IncrementalBaseline::prepare_adapted(incomplete, preset.clone()).is_err(),
                "{media}"
            );
        }
        assert!(IncrementalBaseline::prepare_paragraph_text(snapshot, preset).is_err());
    }
}

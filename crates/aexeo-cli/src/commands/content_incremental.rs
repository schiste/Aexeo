//! Bounded, experimental batch access to the in-process incremental evaluator.
use super::{
    common::required_arg,
    content_change::{HostFiles, configure_host_content_map},
};
use aexeo_contracts::{
    content_gate_change_validation::{parse_strict_validated_change, parse_unique_json_value},
    content_gate_protocol::Change,
};
use aexeo_core::{
    content_candidate::{CandidateItem, CandidateSnapshot},
    content_gate::{ContentGate, ContentPreset, MAX_CONTENT_INPUT_BYTES},
    content_gate_verdict::LinkObservationSnapshot,
    content_incremental::{
        CollectionBaseline, CollectionEvaluation, ComposedBaseline, IncrementalBaseline,
        IncrementalEvaluation, IncrementalEvidence, NotChecked,
    },
    content_presets::{builtin_content_presets, resolve_content_preset},
    html_content_adapter::HtmlContentScope,
};
use anyhow::{Result, bail};
use clap::ArgMatches;
use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    io::{self, Read, Write},
    path::Path,
};

const MAX_BYTES: u64 = 16 * 1024 * 1024;
const MAX_ITEMS: usize = 256;
const MAX_PROPOSALS: usize = 16;
const MAX_TOTAL_CONTENT_BYTES: usize = 64 * 1024 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Batch {
    schema: String,
    snapshot: CandidateSnapshot,
    preset: String,
    #[serde(default)]
    preset_sources: BTreeMap<String, String>,
    mode: Mode,
    #[serde(default)]
    allow_partial_results: bool,
    #[serde(default, deserialize_with = "metadata_if_present")]
    base_metadata: Option<Metadata>,
    #[serde(default, deserialize_with = "observations_if_present")]
    base_link_observations: Option<BTreeMap<String, LinkObservationSnapshot>>,
    #[serde(default, deserialize_with = "unavailable_if_present")]
    base_link_unavailable: Option<BTreeSet<String>>,
    #[serde(default, deserialize_with = "evidence_if_present")]
    base_evidence: Option<IncrementalEvidence>,
    proposals: Vec<Proposal>,
}
type Metadata = BTreeMap<String, Option<BTreeMap<String, String>>>;

fn metadata_if_present<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Metadata>, D::Error> {
    Metadata::deserialize(deserializer).map(Some)
}

fn observations_if_present<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<BTreeMap<String, LinkObservationSnapshot>>, D::Error> {
    BTreeMap::deserialize(deserializer).map(Some)
}

fn unavailable_if_present<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<BTreeSet<String>>, D::Error> {
    let values = Vec::<String>::deserialize(deserializer)?;
    let count = values.len();
    let values: BTreeSet<_> = values.into_iter().collect();
    if values.len() != count {
        return Err(serde::de::Error::custom(
            "duplicate unavailable link identity",
        ));
    }
    Ok(Some(values))
}

fn evidence_if_present<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<IncrementalEvidence>, D::Error> {
    IncrementalEvidence::deserialize(deserializer).map(Some)
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Mode {
    Plain,
    ParagraphText,
    Adapted,
    AdaptedMain,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Proposal {
    change: Change,
    after: Vec<CandidateItem>,
    #[serde(default, deserialize_with = "observations_if_present")]
    link_observations: Option<BTreeMap<String, LinkObservationSnapshot>>,
    #[serde(default, deserialize_with = "unavailable_if_present")]
    link_unavailable: Option<BTreeSet<String>>,
    #[serde(default, deserialize_with = "evidence_if_present")]
    evidence: Option<IncrementalEvidence>,
}

enum Baseline {
    Items(IncrementalBaseline),
    Collection(CollectionBaseline),
    Composed(Box<ComposedBaseline>),
}

#[allow(clippy::too_many_arguments)]
fn prepare_baseline(
    snapshot: CandidateSnapshot,
    metadata: Option<Metadata>,
    preset: ContentPreset,
    mode: Mode,
    observations: Option<BTreeMap<String, LinkObservationSnapshot>>,
    evidence: Option<IncrementalEvidence>,
    unavailable: Option<BTreeSet<String>>,
    allow_partial_results: bool,
) -> Result<Baseline, NotChecked> {
    if unavailable.is_some() && observations.is_none() {
        return Err(NotChecked {
            reason: "partial link inventory requires observations".into(),
        });
    }
    let collection_gates = preset
        .gates
        .iter()
        .filter(|gate| matches!(gate, ContentGate::UniqueCollectionMetadata { .. }))
        .count();
    if collection_gates > 0 {
        let metadata = metadata.ok_or_else(|| NotChecked {
            reason: "base metadata inventory is required for collection gates".into(),
        })?;
        if collection_gates == preset.gates.len() {
            if observations.is_some() || evidence.is_some() {
                return Err(NotChecked {
                    reason: "link observations and evidence require item gates".into(),
                });
            }
            return CollectionBaseline::prepare(snapshot, metadata, preset)
                .map(Baseline::Collection);
        }
        let scope = match mode {
            Mode::Adapted => HtmlContentScope::Document,
            Mode::AdaptedMain => HtmlContentScope::Main,
            _ => {
                return Err(NotChecked {
                    reason: "mixed item and collection gates require an adapted mode".into(),
                });
            }
        };
        let composed = if let Some(unavailable) = unavailable {
            ComposedBaseline::prepare_adapted_with_partial_links(
                snapshot,
                metadata,
                preset,
                scope,
                observations.ok_or_else(|| NotChecked {
                    reason: "partial link inventory requires observations".into(),
                })?,
                unavailable,
                evidence,
            )
        } else if allow_partial_results {
            ComposedBaseline::prepare_adapted_with_partial_results(
                snapshot,
                metadata,
                preset,
                scope,
                observations,
                evidence,
            )
        } else if let Some(evidence) = evidence {
            ComposedBaseline::prepare_adapted_with_evidence(
                snapshot,
                metadata,
                preset,
                scope,
                observations,
                evidence,
            )
        } else if let Some(observations) = observations {
            ComposedBaseline::prepare_adapted_with_link_observations(
                snapshot,
                metadata,
                preset,
                scope,
                observations,
            )
        } else {
            ComposedBaseline::prepare_adapted_with_html_scope(snapshot, metadata, preset, scope)
        };
        return composed.map(|baseline| Baseline::Composed(Box::new(baseline)));
    }
    if let Some(unavailable) = unavailable {
        let scope = match mode {
            Mode::Adapted => HtmlContentScope::Document,
            Mode::AdaptedMain => HtmlContentScope::Main,
            _ => {
                return Err(NotChecked {
                    reason: "link observations require an adapted mode".into(),
                });
            }
        };
        return IncrementalBaseline::prepare_adapted_with_partial_links(
            snapshot,
            preset,
            scope,
            metadata,
            observations.ok_or_else(|| NotChecked {
                reason: "partial link inventory requires observations".into(),
            })?,
            unavailable,
            evidence,
        )
        .map(Baseline::Items);
    }
    if allow_partial_results {
        let scope = match mode {
            Mode::Adapted => HtmlContentScope::Document,
            Mode::AdaptedMain => HtmlContentScope::Main,
            _ => {
                return Err(NotChecked {
                    reason: "partial results require an adapted mode".into(),
                });
            }
        };
        return IncrementalBaseline::prepare_adapted_with_partial_results(
            snapshot,
            preset,
            scope,
            metadata,
            observations,
            evidence,
        )
        .map(Baseline::Items);
    }
    if let Some(evidence) = evidence {
        let scope = match mode {
            Mode::Adapted => HtmlContentScope::Document,
            Mode::AdaptedMain => HtmlContentScope::Main,
            _ => {
                return Err(NotChecked {
                    reason: "evidence requires an adapted mode".into(),
                });
            }
        };
        return IncrementalBaseline::prepare_adapted_with_evidence(
            snapshot,
            preset,
            scope,
            metadata,
            observations,
            evidence,
        )
        .map(Baseline::Items);
    }
    if let Some(observations) = observations {
        let scope = match mode {
            Mode::Adapted => HtmlContentScope::Document,
            Mode::AdaptedMain => HtmlContentScope::Main,
            _ => {
                return Err(NotChecked {
                    reason: "link observations require an adapted mode".into(),
                });
            }
        };
        return IncrementalBaseline::prepare_adapted_with_link_observations(
            snapshot,
            preset,
            scope,
            metadata,
            observations,
        )
        .map(Baseline::Items);
    }
    let baseline = match (mode, metadata) {
        (Mode::Plain, None) => IncrementalBaseline::prepare(snapshot, preset),
        (Mode::ParagraphText, None) => {
            IncrementalBaseline::prepare_paragraph_text(snapshot, preset)
        }
        (Mode::Adapted, None) => IncrementalBaseline::prepare_adapted(snapshot, preset),
        (Mode::AdaptedMain, None) => IncrementalBaseline::prepare_adapted_with_html_scope(
            snapshot,
            preset,
            HtmlContentScope::Main,
        ),
        (Mode::Adapted | Mode::AdaptedMain, Some(metadata)) => {
            let scope = if matches!(mode, Mode::AdaptedMain) {
                HtmlContentScope::Main
            } else {
                HtmlContentScope::Document
            };
            IncrementalBaseline::prepare_adapted_with_metadata_and_html_scope(
                snapshot, metadata, preset, scope,
            )
        }
        (_, Some(_)) => {
            return Err(NotChecked {
                reason: "base metadata requires an adapted mode".into(),
            });
        }
    };
    baseline.map(Baseline::Items)
}

#[derive(Default)]
struct Totals {
    not_checked: usize,
    evaluated: usize,
    reused: usize,
    blocking_failed: bool,
}
impl Totals {
    fn items(&mut self, result: &IncrementalEvaluation) -> bool {
        match result {
            IncrementalEvaluation::NotChecked { .. } => true,
            IncrementalEvaluation::Evaluated {
                evaluated,
                reused,
                reports,
                ..
            } => {
                self.evaluated += evaluated;
                self.reused += reused;
                self.blocking_failed |= reports.values().any(|r| r.summary.blocking_failed > 0);
                reports.values().any(|r| r.summary.not_checked > 0)
            }
        }
    }
    fn collection(&mut self, result: &CollectionEvaluation) -> bool {
        match result {
            CollectionEvaluation::NotChecked { .. } => true,
            CollectionEvaluation::Report { report, .. } => {
                self.blocking_failed |= report.summary.blocking_failed > 0;
                report.summary.not_checked > 0
            }
        }
    }
}

// Extract through the same Markdown and HTML adapters used by incremental
// evaluation. Unobserved link structures never become empty inventories.
fn observation_template(
    items: &[CandidateItem],
    scope: HtmlContentScope,
) -> Result<(BTreeMap<String, LinkObservationSnapshot>, BTreeSet<String>)> {
    use aexeo_core::content_gate::LinkReachabilityObservation;
    let mut snapshots = BTreeMap::new();
    let mut unavailable = BTreeSet::new();
    let mut seen = BTreeSet::new();
    for item in items {
        if !seen.insert(item.entity_id.clone()) {
            bail!("duplicate item identity in link observation inventory");
        }
        let destinations = match item.media_type.as_str() {
            "text/markdown" => {
                aexeo_core::markdown_adapter::markdown_input_with_source_map_and_destinations(
                    &item.entity_id,
                    &item.source,
                )?
                .destinations
            }
            "text/html" => aexeo_core::html_content_adapter::html_content_with_scope(
                &item.entity_id,
                &item.source,
                scope,
            )
            .ok()
            .map(|projection| projection.destinations),
            "text/plain" => None,
            _ => bail!("link observation templates require supported adapted items"),
        };
        let Some(destinations) = destinations else {
            unavailable.insert(item.entity_id.clone());
            continue;
        };
        let observations = vec![LinkReachabilityObservation::NotChecked; destinations.len()];
        if snapshots
            .insert(
                item.entity_id.clone(),
                LinkObservationSnapshot {
                    version: 1,
                    source_sha256: item.sha256.clone(),
                    destinations,
                    observations,
                },
            )
            .is_some()
        {
            bail!("duplicate item identity in link observation inventory");
        }
    }
    Ok((snapshots, unavailable))
}

fn export_observation_templates(
    batch: &Batch,
    providers: &mut [Option<HostFiles>],
    telemetry: bool,
    started: std::time::Instant,
) -> Result<i32> {
    use aexeo_core::{
        content_candidate::overlay_candidate, content_gate_sources::verify_candidate_file_states,
    };
    let scope = match batch.mode {
        Mode::Adapted => HtmlContentScope::Document,
        Mode::AdaptedMain => HtmlContentScope::Main,
        _ => bail!("link observation templates require an adapted mode"),
    };
    if !batch.snapshot.complete {
        bail!("base inventory completeness is not attested");
    }
    if batch.base_evidence.is_some() || batch.proposals.iter().any(|p| p.evidence.is_some()) {
        bail!("remove evidence inventories before exporting link templates");
    }
    if batch.base_link_observations.is_some()
        || batch.base_link_unavailable.is_some()
        || batch
            .proposals
            .iter()
            .any(|p| p.link_observations.is_some() || p.link_unavailable.is_some())
    {
        bail!("remove existing link observations before exporting templates");
    }
    let (base, base_unavailable) = observation_template(&batch.snapshot.items, scope)?;
    let mut retained_bytes = serde_json::to_vec(&base)?
        .len()
        .checked_add(serde_json::to_vec(&base_unavailable)?.len())
        .ok_or_else(|| anyhow::anyhow!("link observation templates exceed size limit"))?;
    if retained_bytes as u64 > MAX_BYTES {
        bail!("link observation templates exceed size limit");
    }
    let base_items: BTreeMap<_, _> = batch
        .snapshot
        .items
        .iter()
        .map(|item| (item.entity_id.as_str(), item))
        .collect();
    let mut projected = 0usize;
    let mut reused = 0usize;
    let base_unavailable_count = base_unavailable.len();
    let mut candidate_unavailable_count = 0usize;
    let mut candidates = Vec::with_capacity(batch.proposals.len());
    for (index, proposal) in batch.proposals.iter().enumerate() {
        let overlay = overlay_candidate(&batch.snapshot, &proposal.change, &proposal.after)
            .map_err(|_| anyhow::anyhow!("candidate overlay is unavailable"))?;
        let referenced = proposal
            .change
            .entries
            .iter()
            .flat_map(|entry| [entry.before.as_ref(), entry.after.as_ref()])
            .flatten()
            .any(|state| state.inline.is_none());
        if referenced {
            let provider = providers[index]
                .as_mut()
                .ok_or_else(|| anyhow::anyhow!("referenced content provider is unavailable"))?;
            verify_candidate_file_states(
                &proposal.change,
                &batch.snapshot,
                &proposal.after,
                provider,
                MAX_CONTENT_INPUT_BYTES,
                MAX_TOTAL_CONTENT_BYTES,
            )
            .map_err(|_| anyhow::anyhow!("referenced candidate content could not be verified"))?;
        }
        let mut observations = BTreeMap::new();
        let mut unavailable = BTreeSet::new();
        for item in &overlay.candidate.items {
            if base_items.get(item.entity_id.as_str()).copied() == Some(item) {
                reused += 1;
                if let Some(snapshot) = base.get(&item.entity_id) {
                    observations.insert(item.entity_id.clone(), snapshot.clone());
                } else if base_unavailable.contains(&item.entity_id) {
                    unavailable.insert(item.entity_id.clone());
                } else {
                    bail!("base link observation template is unavailable");
                }
            } else {
                projected += 1;
                let (mut projected_observations, projected_unavailable) =
                    observation_template(std::slice::from_ref(item), scope)?;
                if let Some(snapshot) = projected_observations.remove(&item.entity_id) {
                    observations.insert(item.entity_id.clone(), snapshot);
                } else if projected_unavailable.contains(&item.entity_id) {
                    unavailable.insert(item.entity_id.clone());
                } else {
                    bail!("candidate link observation template is unavailable");
                }
            }
        }
        candidate_unavailable_count += unavailable.len();
        let mut row = serde_json::json!({"proposal_index":index, "change_id":proposal.change.change_id,
            "link_observations":observations});
        // Include the field even when empty if the base opts into partial inventories.
        if !base_unavailable.is_empty() || !unavailable.is_empty() {
            row["link_unavailable"] = serde_json::json!(unavailable);
        }
        retained_bytes = retained_bytes
            .checked_add(serde_json::to_vec(&row)?.len())
            .ok_or_else(|| anyhow::anyhow!("link observation templates exceed size limit"))?;
        if retained_bytes as u64 > MAX_BYTES {
            bail!("link observation templates exceed size limit");
        }
        candidates.push(row);
    }
    let partial = !base_unavailable.is_empty()
        || candidates
            .iter()
            .any(|row| row.get("link_unavailable").is_some());
    if partial {
        for row in &mut candidates {
            if row.get("link_unavailable").is_none() {
                row["link_unavailable"] = serde_json::json!([]);
            }
        }
    }
    let count: usize = base
        .values()
        .map(|snapshot| snapshot.destinations.len())
        .sum();
    let mut value = serde_json::json!({"schema":"aexeo.content-incremental-link-templates/v1-experimental", "base_link_observations":base, "proposals":candidates});
    if partial {
        value["base_link_unavailable"] = serde_json::json!(base_unavailable);
    }
    let output = serde_json::to_vec(&value)?;
    if output.len() as u64 > MAX_BYTES {
        bail!("link observation templates exceed size limit");
    }
    let mut stdout = io::BufWriter::new(io::stdout().lock());
    stdout.write_all(&output)?;
    writeln!(stdout)?;
    stdout.flush()?;
    if telemetry {
        eprintln!(
            "{}",
            serde_json::json!({"schema":"aexeo.content-incremental-link-template-telemetry/v1", "duration_ms":started.elapsed().as_millis(), "base_items":batch.snapshot.items.len(), "base_destinations":count, "base_unavailable":base_unavailable_count, "candidate_unavailable":candidate_unavailable_count, "proposals":batch.proposals.len(), "projected_items":projected, "reused_items":reused})
        );
    }
    Ok(0)
}

pub fn command_content_incremental(args: &ArgMatches) -> Result<i32> {
    let started = std::time::Instant::now();
    let mut bytes = Vec::new();
    let input = required_arg(args, "input")?;
    if input == "-" {
        io::stdin()
            .lock()
            .take(MAX_BYTES + 1)
            .read_to_end(&mut bytes)?;
    } else {
        File::open(input)?
            .take(MAX_BYTES + 1)
            .read_to_end(&mut bytes)?;
    }
    if bytes.len() as u64 > MAX_BYTES {
        bail!("incremental batch exceeds size limit");
    }
    let value = parse_unique_json_value(&bytes)?;
    let batch: Batch = serde_json::from_value(value.clone())?;
    if batch.schema != "aexeo.content-incremental-batch/v1-experimental" {
        bail!("unsupported incremental batch schema");
    }
    if batch.proposals.is_empty()
        || batch.proposals.len() > MAX_PROPOSALS
        || batch.snapshot.items.len() > MAX_ITEMS
        || batch.proposals.iter().any(|p| p.after.len() > MAX_ITEMS)
    {
        bail!("incremental batch exceeds item or proposal limits, or contains no proposals");
    }
    // Deserialize alone is not the strict protocol validator: enforce the same
    // change validation used by content-impact before evaluating any proposal.
    for proposal in value["proposals"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("missing proposals array"))?
    {
        parse_strict_validated_change(&serde_json::to_vec(&proposal["change"])?)?;
    }
    // Map paths are host-selected command arguments, never read from the
    // untrusted batch. Validate all maps before producing any result rows.
    let root = args.get_one::<String>("content-root").map(String::as_str);
    let mut maps = BTreeMap::new();
    if let Some(values) = args.get_many::<String>("host-content-map") {
        for value in values {
            let (index, path) = value.split_once('=').ok_or_else(|| {
                anyhow::anyhow!("invalid proposal content map; expected INDEX=PATH")
            })?;
            let index: usize = index
                .parse()
                .map_err(|_| anyhow::anyhow!("invalid proposal content map index"))?;
            if index >= batch.proposals.len()
                || path.is_empty()
                || maps.insert(index, path).is_some()
            {
                bail!("invalid or duplicate proposal content map index");
            }
        }
    }
    let mut providers = Vec::with_capacity(batch.proposals.len());
    for (index, proposal) in batch.proposals.iter().enumerate() {
        let mut provider = root
            .map(|root| HostFiles::new(Some(root), None))
            .transpose()?;
        if let Some(path) = maps.get(&index) {
            let root = root.ok_or_else(|| anyhow::anyhow!("content root is required"))?;
            configure_host_content_map(
                provider
                    .as_mut()
                    .ok_or_else(|| anyhow::anyhow!("content provider unavailable"))?,
                Path::new(path),
                Path::new(root),
                &proposal.change,
                None,
            )?;
        }
        providers.push(provider);
    }
    if args.get_flag("export-link-observations") {
        return export_observation_templates(
            &batch,
            &mut providers,
            args.get_flag("telemetry"),
            started,
        );
    }
    let referenced_states = batch
        .proposals
        .iter()
        .flat_map(|p| &p.change.entries)
        .flat_map(|entry| [entry.before.as_ref(), entry.after.as_ref()])
        .flatten()
        .filter(|state| state.inline.is_none())
        .count();
    let base_evidence_supplied = batch.base_evidence.is_some();
    let proposals_with_evidence = batch
        .proposals
        .iter()
        .filter(|p| p.evidence.is_some())
        .count();
    let base_link_unavailable_count = batch.base_link_unavailable.as_ref().map(BTreeSet::len);
    let candidate_link_unavailable_count: usize = batch
        .proposals
        .iter()
        .filter_map(|proposal| proposal.link_unavailable.as_ref())
        .map(BTreeSet::len)
        .sum();
    let base_link_observations_supplied = batch.base_link_observations.is_some();
    let proposals_with_link_observations = batch
        .proposals
        .iter()
        .filter(|p| p.link_observations.is_some())
        .count();
    let mut sources = builtin_content_presets();
    sources.extend(batch.preset_sources);
    let preset = resolve_content_preset(&batch.preset, &sources)?;
    let source_snapshot = args
        .get_flag("source-locations")
        .then(|| batch.snapshot.clone());
    let item_preset = source_snapshot.as_ref().map(|_| {
        let mut item_preset = preset.clone();
        item_preset
            .gates
            .retain(|gate| !matches!(gate, ContentGate::UniqueCollectionMetadata { .. }));
        item_preset
    });
    let baseline = prepare_baseline(
        batch.snapshot,
        batch.base_metadata,
        preset,
        batch.mode,
        batch.base_link_observations,
        batch.base_evidence,
        batch.base_link_unavailable,
        batch.allow_partial_results,
    );
    let mut stdout = io::BufWriter::new(io::stdout().lock());
    let mut totals = Totals::default();
    let mut mapping_eligible = 0usize;
    let mut mapping_mapped = 0usize;
    let mut mapping_unavailable_items = 0usize;
    let mut mapping_unavailable_proposals = 0usize;
    let mut mapping_duration_ms = 0u128;
    let mut native_mapping_eligible = 0usize;
    let mut native_mapping_mapped = 0usize;
    let mut native_mapping_unavailable = 0usize;
    for (index, proposal) in batch.proposals.iter().enumerate() {
        let (result, unchecked) = match &baseline {
            Err(error) => (
                serde_json::json!({"status":"not_checked", "reason":error.reason}),
                true,
            ),
            Ok(Baseline::Items(baseline)) => {
                let result = if let Some(unavailable) = &proposal.link_unavailable {
                    if let Some(observations) = &proposal.link_observations {
                        if let Some(provider) = providers[index].as_mut() {
                            baseline.evaluate_with_sources_and_partial_links(
                                &proposal.change,
                                &proposal.after,
                                provider,
                                MAX_CONTENT_INPUT_BYTES,
                                MAX_TOTAL_CONTENT_BYTES,
                                observations,
                                unavailable,
                                proposal.evidence.as_ref(),
                            )
                        } else {
                            baseline.evaluate_with_partial_links(
                                &proposal.change,
                                &proposal.after,
                                observations,
                                unavailable,
                                proposal.evidence.as_ref(),
                            )
                        }
                    } else {
                        IncrementalEvaluation::NotChecked {
                            reason: "partial link inventory requires observations".into(),
                        }
                    }
                } else if let Some(evidence) = &proposal.evidence {
                    if let Some(provider) = providers[index].as_mut() {
                        baseline.evaluate_with_sources_and_evidence(
                            &proposal.change,
                            &proposal.after,
                            provider,
                            MAX_CONTENT_INPUT_BYTES,
                            MAX_TOTAL_CONTENT_BYTES,
                            proposal.link_observations.as_ref(),
                            evidence,
                        )
                    } else {
                        baseline.evaluate_with_evidence(
                            &proposal.change,
                            &proposal.after,
                            proposal.link_observations.as_ref(),
                            evidence,
                        )
                    }
                } else if let Some(observations) = &proposal.link_observations {
                    if let Some(provider) = providers[index].as_mut() {
                        baseline.evaluate_with_sources_and_link_observations(
                            &proposal.change,
                            &proposal.after,
                            provider,
                            MAX_CONTENT_INPUT_BYTES,
                            MAX_TOTAL_CONTENT_BYTES,
                            observations,
                        )
                    } else {
                        baseline.evaluate_with_link_observations(
                            &proposal.change,
                            &proposal.after,
                            observations,
                        )
                    }
                } else if let Some(provider) = providers[index].as_mut() {
                    baseline.evaluate_with_sources(
                        &proposal.change,
                        &proposal.after,
                        provider,
                        MAX_CONTENT_INPUT_BYTES,
                        MAX_TOTAL_CONTENT_BYTES,
                    )
                } else {
                    baseline.evaluate(&proposal.change, &proposal.after)
                };
                let unchecked = totals.items(&result);
                (serde_json::to_value(result)?, unchecked)
            }
            Ok(Baseline::Collection(_))
                if proposal.link_observations.is_some()
                    || proposal.link_unavailable.is_some()
                    || proposal.evidence.is_some() =>
            {
                (
                    serde_json::json!({"status":"not_checked", "reason":"link observations and evidence require item gates"}),
                    true,
                )
            }
            Ok(Baseline::Collection(baseline)) => {
                let result = if let Some(provider) = providers[index].as_mut() {
                    baseline.evaluate_with_sources(
                        &proposal.change,
                        &proposal.after,
                        provider,
                        MAX_CONTENT_INPUT_BYTES,
                        MAX_TOTAL_CONTENT_BYTES,
                    )
                } else {
                    baseline.evaluate(&proposal.change, &proposal.after)
                };
                let unchecked = totals.collection(&result);
                (
                    serde_json::json!({"status":"collection", "collection":result}),
                    unchecked,
                )
            }
            Ok(Baseline::Composed(_))
                if proposal.link_unavailable.is_some() && proposal.link_observations.is_none() =>
            {
                (
                    serde_json::json!({"status":"not_checked", "reason":"partial link inventory requires observations"}),
                    true,
                )
            }
            Ok(Baseline::Composed(baseline)) => {
                let result = if let (Some(unavailable), Some(observations)) =
                    (&proposal.link_unavailable, &proposal.link_observations)
                {
                    if let Some(provider) = providers[index].as_mut() {
                        baseline.evaluate_with_sources_and_partial_links(
                            &proposal.change,
                            &proposal.after,
                            provider,
                            MAX_CONTENT_INPUT_BYTES,
                            MAX_TOTAL_CONTENT_BYTES,
                            observations,
                            unavailable,
                            proposal.evidence.as_ref(),
                        )
                    } else {
                        baseline.evaluate_with_partial_links(
                            &proposal.change,
                            &proposal.after,
                            observations,
                            unavailable,
                            proposal.evidence.as_ref(),
                        )
                    }
                } else if let Some(evidence) = &proposal.evidence {
                    if let Some(provider) = providers[index].as_mut() {
                        baseline.evaluate_with_sources_and_evidence(
                            &proposal.change,
                            &proposal.after,
                            provider,
                            MAX_CONTENT_INPUT_BYTES,
                            MAX_TOTAL_CONTENT_BYTES,
                            proposal.link_observations.as_ref(),
                            evidence,
                        )
                    } else {
                        baseline.evaluate_with_evidence(
                            &proposal.change,
                            &proposal.after,
                            proposal.link_observations.as_ref(),
                            evidence,
                        )
                    }
                } else if let Some(observations) = &proposal.link_observations {
                    if let Some(provider) = providers[index].as_mut() {
                        baseline.evaluate_with_sources_and_link_observations(
                            &proposal.change,
                            &proposal.after,
                            provider,
                            MAX_CONTENT_INPUT_BYTES,
                            MAX_TOTAL_CONTENT_BYTES,
                            observations,
                        )
                    } else {
                        baseline.evaluate_with_link_observations(
                            &proposal.change,
                            &proposal.after,
                            observations,
                        )
                    }
                } else if let Some(provider) = providers[index].as_mut() {
                    baseline.evaluate_with_sources(
                        &proposal.change,
                        &proposal.after,
                        provider,
                        MAX_CONTENT_INPUT_BYTES,
                        MAX_TOTAL_CONTENT_BYTES,
                    )
                } else {
                    baseline.evaluate(&proposal.change, &proposal.after)
                };
                // Evaluate both sides even when one is unchecked: preserve all observed diagnostics.
                let item_unchecked = totals.items(&result.items);
                let collection_unchecked = totals.collection(&result.collection);
                (
                    serde_json::json!({"status":"composed", "change_id":result.change_id, "items":result.items, "collection":result.collection}),
                    item_unchecked || collection_unchecked,
                )
            }
        };
        totals.not_checked += usize::from(unchecked);
        let mut row = serde_json::json!({"schema":"aexeo.content-incremental-result/v1-experimental", "proposal_index":index, "result":result});
        if let (Some(snapshot), Some(item_preset)) = (&source_snapshot, &item_preset) {
            let mapping_started = std::time::Instant::now();
            let scope = if matches!(batch.mode, Mode::AdaptedMain) {
                HtmlContentScope::Main
            } else {
                HtmlContentScope::Document
            };
            let reports = if row["result"]["status"] == "composed" {
                &row["result"]["items"]["reports"]
            } else {
                &row["result"]["reports"]
            };
            let mut mapping = serde_json::Map::new();
            let mut mapping_status = if row["result"]["status"] == "collection" {
                "not_applicable"
            } else {
                "unavailable"
            };
            if let Some(reports) = reports.as_object() {
                match aexeo_core::content_candidate::overlay_candidate(
                    snapshot,
                    &proposal.change,
                    &proposal.after,
                ) {
                    Ok(overlay) => {
                        mapping_status = "available";
                        for item in &overlay.candidate.items {
                            let Some(value) = reports.get(&item.entity_id) else {
                                mapping_status = "partial";
                                mapping_unavailable_items += 1;
                                mapping.insert(
                                    item.entity_id.clone(),
                                    serde_json::json!({"status":"unavailable"}),
                                );
                                continue;
                            };
                            let report = serde_json::from_value::<
                                aexeo_core::content_gate::ContentReport,
                            >(value.clone());
                            let observations = proposal
                                .link_observations
                                .as_ref()
                                .and_then(|m| m.get(&item.entity_id));
                            let is_native =
                                item.media_type == aexeo_core::content_adapter::MEDIA_TYPE;
                            let mapped =
                                report
                                    .as_ref()
                                    .ok()
                                    .filter(|_| !is_native)
                                    .and_then(|report| {
                                        aexeo_core::source_mapping::map_incremental_report(
                                            item_preset,
                                            item,
                                            report,
                                            scope,
                                            observations,
                                        )
                                        .ok()
                                    });
                            let mut item_mapping = match mapped {
                                Some(value) => {
                                    mapping_eligible += value.eligible;
                                    mapping_mapped += value.mapped;
                                    serde_json::json!({"status":"available", "coverage":value})
                                }
                                None => {
                                    mapping_status = "partial";
                                    mapping_unavailable_items += 1;
                                    serde_json::json!({"status":"unavailable"})
                                }
                            };
                            if is_native {
                                let native = report.as_ref().ok().and_then(|report| {
                                    aexeo_core::source_mapping::map_incremental_native_report(
                                        item_preset,
                                        item,
                                        report,
                                        observations,
                                    )
                                    .ok()
                                });
                                match native {
                                    Some(coverage) => {
                                        native_mapping_eligible += coverage.eligible;
                                        native_mapping_mapped += coverage.mapped;
                                        item_mapping["native_coverage"] =
                                            serde_json::to_value(coverage)?;
                                    }
                                    None => native_mapping_unavailable += 1,
                                }
                            }
                            mapping.insert(item.entity_id.clone(), item_mapping);
                        }
                    }
                    Err(_) => {
                        mapping_unavailable_proposals += 1;
                    }
                }
            } else if mapping_status == "unavailable" {
                mapping_unavailable_proposals += 1;
            }
            row["source_mapping"] = serde_json::json!({"status":mapping_status,"items":mapping});
            mapping_duration_ms += mapping_started.elapsed().as_millis();
        }
        serde_json::to_writer(&mut stdout, &row)?;
        writeln!(stdout)?;
    }
    stdout.flush()?;
    if args.get_flag("telemetry") {
        eprintln!(
            "{}",
            serde_json::json!({"schema":"aexeo.content-incremental-telemetry/v1", "duration_ms":started.elapsed().as_millis(), "allow_partial_results":batch.allow_partial_results, "input_bytes":bytes.len(), "proposals":batch.proposals.len(), "not_checked":totals.not_checked, "evaluated":totals.evaluated, "reused":totals.reused, "blocking_failed":totals.blocking_failed, "referenced_states":referenced_states, "base_evidence_supplied":base_evidence_supplied, "proposals_with_evidence":proposals_with_evidence, "base_link_observations_supplied":base_link_observations_supplied, "base_link_unavailable_count":base_link_unavailable_count, "candidate_link_unavailable_count":candidate_link_unavailable_count, "proposals_with_link_observations":proposals_with_link_observations, "mapped_proposals":maps.len(), "content_root_supplied":root.is_some(), "source_mapping_enabled":source_snapshot.is_some(), "source_mapping_eligible":mapping_eligible, "source_mapping_mapped":mapping_mapped, "source_mapping_unavailable":mapping_unavailable_items + mapping_unavailable_proposals, "source_mapping_unavailable_items":mapping_unavailable_items, "source_mapping_unavailable_proposals":mapping_unavailable_proposals, "source_mapping_duration_ms":mapping_duration_ms, "native_source_mapping_eligible":native_mapping_eligible, "native_source_mapping_mapped":native_mapping_mapped, "native_source_mapping_unavailable":native_mapping_unavailable})
        );
    }
    Ok(if totals.not_checked != 0 {
        2
    } else if totals.blocking_failed {
        1
    } else {
        0
    })
}

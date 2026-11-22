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
        IncrementalEvaluation, NotChecked,
    },
    content_presets::{builtin_content_presets, resolve_content_preset},
    html_content_adapter::HtmlContentScope,
};
use anyhow::{Result, bail};
use clap::ArgMatches;
use serde::Deserialize;
use std::{
    collections::BTreeMap,
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
    #[serde(default, deserialize_with = "metadata_if_present")]
    base_metadata: Option<Metadata>,
    #[serde(default, deserialize_with = "observations_if_present")]
    base_link_observations: Option<BTreeMap<String, LinkObservationSnapshot>>,
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
}

enum Baseline {
    Items(IncrementalBaseline),
    Collection(CollectionBaseline),
    Composed(Box<ComposedBaseline>),
}

fn prepare_baseline(
    snapshot: CandidateSnapshot,
    metadata: Option<Metadata>,
    preset: ContentPreset,
    mode: Mode,
    observations: Option<BTreeMap<String, LinkObservationSnapshot>>,
) -> Result<Baseline, NotChecked> {
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
            if observations.is_some() {
                return Err(NotChecked {
                    reason: "link observations require a reachability gate".into(),
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
        let composed = if let Some(observations) = observations {
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
                false
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
    let referenced_states = batch
        .proposals
        .iter()
        .flat_map(|p| &p.change.entries)
        .flat_map(|entry| [entry.before.as_ref(), entry.after.as_ref()])
        .flatten()
        .filter(|state| state.inline.is_none())
        .count();
    let base_link_observations_supplied = batch.base_link_observations.is_some();
    let proposals_with_link_observations = batch
        .proposals
        .iter()
        .filter(|p| p.link_observations.is_some())
        .count();
    let mut sources = builtin_content_presets();
    sources.extend(batch.preset_sources);
    let preset = resolve_content_preset(&batch.preset, &sources)?;
    let baseline = prepare_baseline(
        batch.snapshot,
        batch.base_metadata,
        preset,
        batch.mode,
        batch.base_link_observations,
    );
    let mut stdout = io::BufWriter::new(io::stdout().lock());
    let mut totals = Totals::default();
    for (index, proposal) in batch.proposals.iter().enumerate() {
        let (result, unchecked) = match &baseline {
            Err(error) => (
                serde_json::json!({"status":"not_checked", "reason":error.reason}),
                true,
            ),
            Ok(Baseline::Items(baseline)) => {
                let result = if let Some(observations) = &proposal.link_observations {
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
            Ok(Baseline::Collection(_)) if proposal.link_observations.is_some() => (
                serde_json::json!({"status":"not_checked", "reason":"link observations require a reachability gate"}),
                true,
            ),
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
            Ok(Baseline::Composed(baseline)) => {
                let result = if let Some(observations) = &proposal.link_observations {
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
        serde_json::to_writer(
            &mut stdout,
            &serde_json::json!({"schema":"aexeo.content-incremental-result/v1-experimental", "proposal_index":index, "result":result}),
        )?;
        writeln!(stdout)?;
    }
    stdout.flush()?;
    if args.get_flag("telemetry") {
        eprintln!(
            "{}",
            serde_json::json!({"schema":"aexeo.content-incremental-telemetry/v1", "duration_ms":started.elapsed().as_millis(), "input_bytes":bytes.len(), "proposals":batch.proposals.len(), "not_checked":totals.not_checked, "evaluated":totals.evaluated, "reused":totals.reused, "blocking_failed":totals.blocking_failed, "referenced_states":referenced_states, "base_link_observations_supplied":base_link_observations_supplied, "proposals_with_link_observations":proposals_with_link_observations, "mapped_proposals":maps.len(), "content_root_supplied":root.is_some()})
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

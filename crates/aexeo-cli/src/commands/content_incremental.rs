//! Bounded, experimental batch access to the in-process incremental evaluator.
use super::common::required_arg;
use aexeo_contracts::{
    content_gate_change_validation::{parse_strict_validated_change, parse_unique_json_value},
    content_gate_protocol::Change,
};
use aexeo_core::{
    content_candidate::{CandidateItem, CandidateSnapshot},
    content_gate::{ContentGate, ContentPreset},
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
};

const MAX_BYTES: u64 = 16 * 1024 * 1024;
const MAX_ITEMS: usize = 256;
const MAX_PROPOSALS: usize = 16;

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
    proposals: Vec<Proposal>,
}
type Metadata = BTreeMap<String, Option<BTreeMap<String, String>>>;

fn metadata_if_present<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Metadata>, D::Error> {
    Metadata::deserialize(deserializer).map(Some)
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
        return ComposedBaseline::prepare_adapted_with_html_scope(
            snapshot, metadata, preset, scope,
        )
        .map(|baseline| Baseline::Composed(Box::new(baseline)));
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
    let mut sources = builtin_content_presets();
    sources.extend(batch.preset_sources);
    let preset = resolve_content_preset(&batch.preset, &sources)?;
    let baseline = prepare_baseline(batch.snapshot, batch.base_metadata, preset, batch.mode);
    let mut stdout = io::BufWriter::new(io::stdout().lock());
    let mut totals = Totals::default();
    for (index, proposal) in batch.proposals.iter().enumerate() {
        let (result, unchecked) = match &baseline {
            Err(error) => (
                serde_json::json!({"status":"not_checked", "reason":error.reason}),
                true,
            ),
            Ok(Baseline::Items(baseline)) => {
                let result = baseline.evaluate(&proposal.change, &proposal.after);
                let unchecked = totals.items(&result);
                (serde_json::to_value(result)?, unchecked)
            }
            Ok(Baseline::Collection(baseline)) => {
                let result = baseline.evaluate(&proposal.change, &proposal.after);
                let unchecked = totals.collection(&result);
                (
                    serde_json::json!({"status":"collection", "collection":result}),
                    unchecked,
                )
            }
            Ok(Baseline::Composed(baseline)) => {
                let result = baseline.evaluate(&proposal.change, &proposal.after);
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
            serde_json::json!({"schema":"aexeo.content-incremental-telemetry/v1", "duration_ms":started.elapsed().as_millis(), "input_bytes":bytes.len(), "proposals":batch.proposals.len(), "not_checked":totals.not_checked, "evaluated":totals.evaluated, "reused":totals.reused, "blocking_failed":totals.blocking_failed})
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

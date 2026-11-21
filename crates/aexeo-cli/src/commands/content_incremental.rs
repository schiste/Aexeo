//! Bounded, experimental batch access to the in-process incremental evaluator.
use super::common::required_arg;
use aexeo_contracts::{
    content_gate_change_validation::{parse_strict_validated_change, parse_unique_json_value},
    content_gate_protocol::Change,
};
use aexeo_core::{
    content_candidate::{CandidateItem, CandidateSnapshot},
    content_incremental::{IncrementalBaseline, IncrementalEvaluation},
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
    proposals: Vec<Proposal>,
}
#[derive(Deserialize)]
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

pub fn command_content_incremental(args: &ArgMatches) -> Result<i32> {
    let started = std::time::Instant::now();
    let mut bytes = Vec::new();
    File::open(required_arg(args, "input")?)?
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)?;
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
    let baseline = match batch.mode {
        Mode::Plain => IncrementalBaseline::prepare(batch.snapshot, preset),
        Mode::ParagraphText => IncrementalBaseline::prepare_paragraph_text(batch.snapshot, preset),
        Mode::Adapted => IncrementalBaseline::prepare_adapted(batch.snapshot, preset),
        Mode::AdaptedMain => IncrementalBaseline::prepare_adapted_with_html_scope(
            batch.snapshot,
            preset,
            HtmlContentScope::Main,
        ),
    };
    let mut stdout = io::BufWriter::new(io::stdout().lock());
    let mut not_checked = 0usize;
    let mut evaluated = 0usize;
    let mut reused = 0usize;
    let mut blocking_failed = false;
    match baseline {
        Err(error) => {
            not_checked = batch.proposals.len();
            for (index, _) in batch.proposals.iter().enumerate() {
                serde_json::to_writer(
                    &mut stdout,
                    &serde_json::json!({"schema":"aexeo.content-incremental-result/v1-experimental", "proposal_index":index, "result":{"status":"not_checked", "reason":error.reason}}),
                )?;
                writeln!(stdout)?;
            }
        }
        Ok(baseline) => {
            for (index, proposal) in batch.proposals.iter().enumerate() {
                let result = baseline.evaluate(&proposal.change, &proposal.after);
                match &result {
                    IncrementalEvaluation::NotChecked { .. } => not_checked += 1,
                    IncrementalEvaluation::Evaluated {
                        evaluated: e,
                        reused: r,
                        reports,
                        ..
                    } => {
                        evaluated += e;
                        reused += r;
                        blocking_failed |= reports
                            .values()
                            .any(|report| report.summary.blocking_failed > 0);
                    }
                }
                serde_json::to_writer(
                    &mut stdout,
                    &serde_json::json!({"schema":"aexeo.content-incremental-result/v1-experimental", "proposal_index":index, "result":result}),
                )?;
                writeln!(stdout)?;
            }
        }
    }
    stdout.flush()?;
    if args.get_flag("telemetry") {
        eprintln!(
            "{}",
            serde_json::json!({"schema":"aexeo.content-incremental-telemetry/v1", "duration_ms":started.elapsed().as_millis(), "input_bytes":bytes.len(), "proposals":batch.proposals.len(), "not_checked":not_checked, "evaluated":evaluated, "reused":reused, "blocking_failed":blocking_failed})
        );
    }
    Ok(if not_checked != 0 {
        2
    } else if blocking_failed {
        1
    } else {
        0
    })
}

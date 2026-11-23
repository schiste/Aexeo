//! Explicit JSON Lines protocol for agent-driven revisions. No source is modified.
use super::exit_code::{EXIT_FINDINGS, EXIT_SUCCESS, EXIT_UNSUPPORTED};
use aexeo_core::{
    content_gate::{ContentReport, GateStatus},
    content_revision_loop::{self, MAX_DRAFT_BYTES, ReviewScope, RevisionStop},
    error::UserContext,
    user_bail,
};
use anyhow::{Context, Result};
use clap::ArgMatches;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    cell::RefCell,
    fs::File,
    io::{self, BufRead, Read, Write},
    process::{Command, Stdio},
};

// A JSON string can encode each byte as a six-byte Unicode escape.
const MAX_PROPOSAL_BYTES: u64 = (MAX_DRAFT_BYTES as u64) * 6 + 1024;
const MAX_REPORT_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
enum Proposal {
    Submit {
        candidate: String,
        link_observations: Option<String>,
    },
    Patch {
        base_sha256: String,
        start_byte: usize,
        end_byte: usize,
        replacement: String,
        link_observations: Option<String>,
    },
    Decline {},
}

fn read_draft(path: &str) -> Result<String> {
    let mut data = Vec::new();
    File::open(path)
        .user_context("opening initial draft")?
        .take(MAX_DRAFT_BYTES as u64 + 1)
        .read_to_end(&mut data)
        .user_context("reading initial draft")?;
    if data.len() > MAX_DRAFT_BYTES {
        user_bail!("initial draft exceeds byte limit");
    }
    String::from_utf8(data).user_context("initial draft is not UTF-8")
}

/// Reuse the real content CLI, including adapter/source mapping and presets.
/// The child reads its complete input before emitting JSON. A writer thread keeps
/// pipe traffic concurrent and stdout is bounded; no arbitrary program is run.
fn evaluate(
    candidate: &str,
    args: &ArgMatches,
    link_observations: Option<&str>,
) -> Result<(Value, ContentReport)> {
    let mut command = Command::new(std::env::current_exe().context("locating current executable")?);
    command
        .args(["content", "-", "--format", "json", "--preset"])
        .arg(
            args.get_one::<String>("preset")
                .user_context("missing preset")?,
        )
        .arg("--input-format")
        .arg(
            args.get_one::<String>("input-format")
                .user_context("missing input format")?,
        );
    if let Some(sources) = args.get_many::<String>("preset-source") {
        for source in sources {
            command.arg("--preset-source").arg(source);
        }
    }
    if let Some(scope) = args.get_one::<String>("html-scope") {
        command.arg("--html-scope").arg(scope);
    }
    if let Some(metadata) = args.get_one::<String>("metadata") {
        command.arg("--metadata").arg(metadata);
    }
    if let Some(path) = link_observations {
        command.arg("--link-observations").arg(path);
    }
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .user_context("starting content evaluator")?;
    let mut input = child.stdin.take().context("missing evaluator stdin")?;
    let output = child.stdout.take().context("missing evaluator stdout")?;
    let (read_result, write_result) = std::thread::scope(|scope| {
        let writer = scope.spawn(move || input.write_all(candidate.as_bytes()));
        let mut bytes = Vec::new();
        let result = output
            .take(MAX_REPORT_BYTES + 1)
            .read_to_end(&mut bytes)
            .map(|_| bytes);
        (result, writer.join())
    });
    let data = match read_result {
        Ok(data) if data.len() as u64 <= MAX_REPORT_BYTES => data,
        Ok(_) => {
            let _ = child.kill();
            let _ = child.wait();
            user_bail!("content report exceeds byte limit");
        }
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error.into());
        }
    };
    let status = child.wait().user_context("waiting for content evaluator")?;
    write_result
        .map_err(|_| anyhow::anyhow!("content evaluator input writer panicked"))?
        .user_context("writing evaluator input")?;
    if !matches!(
        status.code(),
        Some(EXIT_SUCCESS | EXIT_FINDINGS | EXIT_UNSUPPORTED)
    ) {
        user_bail!("content evaluator failed; see stderr");
    }
    let report: Value =
        serde_json::from_slice(&data).user_context("content evaluator returned invalid JSON")?;
    let typed: ContentReport =
        ContentReport::deserialize(&report).user_context("invalid content report")?;
    let expected_exit = if typed
        .gates
        .iter()
        .any(|gate| gate.blocking && gate.status == GateStatus::NotChecked)
    {
        EXIT_UNSUPPORTED
    } else if typed
        .gates
        .iter()
        .any(|gate| gate.blocking && gate.status == GateStatus::Failed)
    {
        EXIT_FINDINGS
    } else {
        EXIT_SUCCESS
    };
    if status.code() != Some(expected_exit) {
        user_bail!("content evaluator exit does not match the gate report");
    }
    let hash = super::feedback::text_hash(candidate.as_bytes());
    if report
        .pointer("/input_fingerprint/text_sha256")
        .and_then(Value::as_str)
        != Some(hash.as_str())
    {
        user_bail!("content evaluator did not bind the complete candidate");
    }
    Ok((report, typed))
}

pub fn command_content_revise(args: &ArgMatches) -> Result<i32> {
    let path = args
        .get_one::<String>("input")
        .user_context("missing initial draft")?;
    if path == "-" {
        user_bail!("--input must be a file; stdin is used for proposals");
    }
    if args.contains_id("html-scope")
        && args.get_one::<String>("input-format").map(String::as_str) != Some("html")
    {
        user_bail!("--html-scope requires HTML input");
    }
    let started = args.get_flag("telemetry").then(std::time::Instant::now);
    let initial = read_draft(path)?;
    let scope = match args.get_one::<String>("scope").map(String::as_str) {
        Some("blocking") => ReviewScope::Blocking,
        _ => ReviewScope::All,
    };
    let budget = *args
        .get_one::<u32>("max-revisions")
        .user_context("missing revision budget")?;
    let binding = RefCell::new(None::<(Value, Value, Value)>);
    let current_report = RefCell::new(None::<Value>);
    let observations = RefCell::new(args.get_one::<String>("link-observations").cloned());
    let stdin = io::stdin();
    let mut stdin = stdin.lock();
    let stdout = io::stdout();
    let mut stdout = stdout.lock();
    let mut proposal_number = 0usize;
    let mut proposals_with_observations = 0usize;
    let mut patch_proposals = 0usize;
    let mut evaluation_ms = 0.0;
    let outcome = content_revision_loop::revise(
        &initial,
        budget as usize,
        scope,
        |candidate| {
            let evaluation_started = started.map(|_| std::time::Instant::now());
            let (report, typed) = evaluate(candidate, args, observations.borrow().as_deref())?;
            if let Some(time) = evaluation_started {
                evaluation_ms += time.elapsed().as_secs_f64() * 1000.0;
            }
            let policy = report
                .get("resolved_policy_fingerprint")
                .filter(|v| v.get("sha256").and_then(Value::as_str).is_some())
                .user_context("evaluator omitted policy fingerprint")?
                .clone();
            let metadata = report
                .pointer("/input_fingerprint/metadata_sha256")
                .user_context("evaluator omitted metadata fingerprint")?
                .clone();
            let metadata_coverage = report
                .pointer("/input_fingerprint/metadata_coverage_sha256")
                .user_context("evaluator omitted metadata coverage fingerprint")?
                .clone();
            let now = (policy, metadata, metadata_coverage);
            let mut binding = binding.borrow_mut();
            if binding.as_ref().is_some_and(|expected| *expected != now) {
                user_bail!("policy, metadata or metadata coverage changed during revision");
            }
            *binding = Some(now);
            *current_report.borrow_mut() = Some(report);
            Ok(typed)
        },
        |candidate, _| {
            proposal_number += 1;
            let candidate_sha256 = super::feedback::text_hash(candidate.as_bytes());
            serde_json::to_writer(
                &mut stdout,
                &json!({"event":"proposal_requested", "version":1, "proposal":proposal_number, "candidate_sha256":candidate_sha256, "report":current_report.borrow().as_ref()}),
            )?;
            writeln!(stdout)?;
            stdout.flush()?;
            let mut data = Vec::new();
            (&mut stdin)
                .take(MAX_PROPOSAL_BYTES + 1)
                .read_until(b'\n', &mut data)
                .user_context("reading proposal")?;
            if data.is_empty() {
                return Ok(None);
            }
            if data.len() as u64 > MAX_PROPOSAL_BYTES {
                user_bail!("proposal exceeds JSON byte limit");
            }
            let proposal: Proposal =
                serde_json::from_slice(&data).user_context("invalid revision proposal")?;
            Ok(match proposal {
                Proposal::Submit {
                    candidate,
                    link_observations,
                } => {
                    proposals_with_observations += usize::from(link_observations.is_some());
                    *observations.borrow_mut() = link_observations;
                    Some(candidate)
                }
                Proposal::Patch {
                    base_sha256,
                    start_byte,
                    end_byte,
                    replacement,
                    link_observations,
                } => {
                    if base_sha256 != candidate_sha256 {
                        user_bail!("patch base digest does not match the current candidate");
                    }
                    if start_byte > end_byte
                        || end_byte > candidate.len()
                        || !candidate.is_char_boundary(start_byte)
                        || !candidate.is_char_boundary(end_byte)
                    {
                        user_bail!("patch range must use valid UTF-8 byte boundaries");
                    }
                    let revised_len = candidate.len() - (end_byte - start_byte);
                    if replacement.len() > MAX_DRAFT_BYTES - revised_len {
                        user_bail!("patched draft exceeds byte limit");
                    }
                    patch_proposals += 1;
                    let mut revised = String::with_capacity(revised_len + replacement.len());
                    revised.push_str(&candidate[..start_byte]);
                    revised.push_str(&replacement);
                    revised.push_str(&candidate[end_byte..]);
                    proposals_with_observations += usize::from(link_observations.is_some());
                    *observations.borrow_mut() = link_observations;
                    Some(revised)
                }
                Proposal::Decline {} => None,
            })
        },
    )?;
    let stop = match outcome.stop {
        RevisionStop::Satisfied => "satisfied",
        RevisionStop::Incomplete => "incomplete",
        RevisionStop::BudgetExhausted => "budget_exhausted",
        RevisionStop::Declined => "declined",
        RevisionStop::Cycle => "cycle",
    };
    serde_json::to_writer(
        &mut stdout,
        &json!({"event":"complete", "version":1, "stop":stop, "evaluations":outcome.evaluations, "proposals":outcome.proposals, "candidate_sha256":super::feedback::text_hash(outcome.candidate.as_bytes()), "report":current_report.borrow().as_ref()}),
    )?;
    writeln!(stdout)?;
    stdout.flush()?;
    if args.get_flag("telemetry") {
        eprintln!(
            "{}",
            json!({"event":"content_revision_loop", "version":1, "stop":stop, "evaluations":outcome.evaluations, "proposals":outcome.proposals, "initial_link_observations":args.contains_id("link-observations"), "proposals_with_link_observations":proposals_with_observations, "patch_proposals":patch_proposals, "evaluation_ms":evaluation_ms, "elapsed_ms":started.map(|time| time.elapsed().as_secs_f64() * 1000.0)})
        );
    }
    Ok(match outcome.stop {
        RevisionStop::Satisfied => EXIT_SUCCESS,
        RevisionStop::Incomplete => EXIT_UNSUPPORTED,
        _ => EXIT_FINDINGS,
    })
}

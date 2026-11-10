use std::{
    fs::{self, OpenOptions},
    io::Write,
    time::Instant,
};

use aexeo_core::error::UserContext;
use anyhow::{Context, Result, bail};
use clap::ArgMatches;
use ring::digest::{SHA256, digest};
use serde_json::{Map, Value, json};

use crate::commands::common::required_arg;

pub(super) fn metadata_hash(value: &Value) -> Result<String> {
    let object = value
        .as_object()
        .context("metadata must be an object of string fields")?;
    let mut fields: Vec<_> = object.iter().collect();
    fields.sort_unstable_by(|a, b| a.0.cmp(b.0));
    let mut context = ring::digest::Context::new(&SHA256);
    for (key, value) in fields {
        let value = value
            .as_str()
            .context("metadata must be an object of string fields")?;
        for field in [key.as_bytes(), value.as_bytes()] {
            context.update(&(field.len() as u64).to_be_bytes());
            context.update(field);
        }
    }
    Ok(context
        .finish()
        .as_ref()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

pub(super) fn text_hash(text: &[u8]) -> String {
    digest(&SHA256, text)
        .as_ref()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn build_note(
    text: &[u8],
    report: &Value,
    gate: &str,
    example: &str,
    args: &ArgMatches,
) -> Result<Value> {
    std::str::from_utf8(text).context("text is not UTF-8")?;
    if gate.trim().is_empty() || example.trim().is_empty() {
        bail!("gate and example identifiers must be nonempty");
    }
    let gates = report
        .get("gates")
        .and_then(Value::as_array)
        .context("report has no gates array")?;
    let matching: Vec<_> = gates
        .iter()
        .filter(|g| g.get("gate_id").and_then(Value::as_str) == Some(gate))
        .collect();
    if matching.len() != 1 {
        bail!("report must contain exactly one matching gate");
    }
    let status = matching[0]
        .get("status")
        .and_then(Value::as_str)
        .context("gate has no status")?;
    if !["passed", "failed", "not_checked"].contains(&status) {
        bail!("gate has invalid status");
    }
    let hash = text_hash(text);
    let mut context = Map::new();
    for (argument, key) in [
        ("language", "language"),
        ("content-type", "content_type"),
        ("preset", "preset"),
        ("config-version", "config_version"),
        ("tool-version", "tool_version"),
    ] {
        if let Some(value) = args.get_one::<String>(argument) {
            context.insert(key.into(), json!(value));
        }
    }
    let reason = args
        .get_one::<String>("reason-file")
        .map(fs::read_to_string)
        .transpose()?;
    let mut note = json!({"version":1,"kind":"reported_human_feedback","gate_id":gate,"example_id":example,
        "text_sha256":hash,"model_status":status,"human_status":args.get_one::<String>("human-status"),
        "reviewer":args.get_one::<String>("reviewer"),"method":args.get_one::<String>("method"),"reason":reason,"context":context});
    let metadata_digest = args
        .get_one::<String>("metadata")
        .map(|path| -> Result<String> {
            let metadata: Value =
                serde_json::from_slice(&fs::read(path).context("reading metadata")?)
                    .context("parsing metadata")?;
            metadata_hash(&metadata)
        })
        .transpose()?;
    if let Some(digest) = &metadata_digest {
        note["metadata_sha256"] = json!(digest);
    }
    let binding = match report.get("input_fingerprint") {
        None => "unverified",
        Some(fingerprint) => {
            if fingerprint.get("version").and_then(Value::as_u64) != Some(1)
                || fingerprint.get("text_sha256").and_then(Value::as_str) != Some(hash.as_str())
                || fingerprint.get("metadata_sha256") != Some(&json!(metadata_digest))
            {
                bail!("report input fingerprint does not match the text and metadata");
            }
            "verified"
        }
    };
    note["report_binding"] = json!(binding);
    Ok(note)
}

pub fn command_feedback(args: &ArgMatches) -> Result<i32> {
    let started = Instant::now();
    let text = fs::read(required_arg(args, "text")?).context("reading text")?;
    let report: Value =
        serde_json::from_slice(&fs::read(required_arg(args, "report")?).context("reading report")?)
            .user_context("parsing report")?;
    let note = build_note(
        &text,
        &report,
        required_arg(args, "gate")?,
        required_arg(args, "example")?,
        args,
    )
    .user_context("invalid feedback input")?;
    write_new(required_arg(args, "output")?, &note)?;
    if args.get_flag("telemetry") {
        eprintln!(
            "{}",
            json!({"event":"content_review_note", "version":1,"operation":"capture", "elapsed_ms":started.elapsed().as_secs_f64()*1000.0})
        );
    }
    Ok(0)
}

fn export_review(corpus: &Value, note: &Value) -> Result<Value> {
    use std::collections::{HashMap, HashSet};
    if corpus.get("version") != Some(&json!(1)) {
        bail!("unsupported corpus version");
    }
    let gate = corpus.get("gate_id").context("missing corpus gate")?;
    let examples = corpus
        .get("examples")
        .and_then(Value::as_array)
        .context("missing examples")?;
    let mut ids = HashSet::new();
    let mut splits = HashMap::new();
    let mut matching = None;
    let synthetic =
        corpus.get("fixture_kind").and_then(Value::as_str) == Some("synthetic_regression");
    for item in examples {
        let id = item
            .get("id")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .context("invalid example ID")?;
        if !ids.insert(id) {
            bail!("duplicate example ID");
        }
        let split = item
            .get("split")
            .and_then(Value::as_str)
            .context("missing split")?;
        if !["development", "holdout"].contains(&split) {
            bail!("invalid split");
        }
        let text = item
            .get("text")
            .and_then(Value::as_str)
            .context("invalid text")?;
        if let Some(metadata) = item.get("metadata") {
            metadata_hash(metadata)?;
        }
        if splits
            .insert(text, split)
            .is_some_and(|previous| previous != split)
        {
            bail!("identical example text appears in multiple splits");
        }
        let expected = item
            .get("expected")
            .and_then(Value::as_str)
            .context("missing expected status")?;
        if !["passed", "failed", "not_checked"].contains(&expected) {
            bail!("invalid expected status");
        }
        let review_status = item
            .get("review_status")
            .map_or(Some("not_reviewed"), Value::as_str)
            .context("invalid review status")?;
        if !["reviewed", "not_reviewed"].contains(&review_status)
            || (synthetic && review_status == "reviewed")
        {
            bail!("invalid review status");
        }
        if review_status == "reviewed" {
            let review = item.get("review").context("missing review provenance")?;
            for key in ["reviewer", "method"] {
                if !review
                    .get(key)
                    .and_then(Value::as_str)
                    .is_some_and(|v| !v.trim().is_empty())
                {
                    bail!("missing review provenance");
                }
            }
        }
        if note.get("example_id").and_then(Value::as_str) == Some(id) {
            matching = Some(item);
        }
    }
    if note.get("version") != Some(&json!(1))
        || note.get("kind").and_then(Value::as_str) != Some("reported_human_feedback")
    {
        bail!("unsupported feedback note");
    }
    if note.get("gate_id") != Some(gate) {
        bail!("feedback gate does not match corpus");
    }
    let status = note
        .get("human_status")
        .and_then(Value::as_str)
        .context("missing human status")?;
    if !["passed", "failed"].contains(&status) {
        bail!("an explicit human pass or fail is required");
    }
    for key in ["reviewer", "method"] {
        if !note
            .get(key)
            .and_then(Value::as_str)
            .is_some_and(|v| !v.trim().is_empty())
        {
            bail!("reviewer and method provenance are required");
        }
    }
    let item = matching.context("review log references an unknown example")?;
    let text = item["text"].as_str().context("invalid text")?;
    let hash = digest(&SHA256, text.as_bytes())
        .as_ref()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    if note.get("text_sha256").and_then(Value::as_str) != Some(hash.as_str()) {
        bail!("review event does not match the current example text");
    }
    let metadata_digest = item.get("metadata").map(metadata_hash).transpose()?;
    if note.get("metadata_sha256").and_then(Value::as_str) != metadata_digest.as_deref()
        || (metadata_digest.is_none() && note.get("metadata_sha256").is_some())
    {
        bail!("review event does not match the current example metadata");
    }
    let model = note
        .get("model_status")
        .and_then(Value::as_str)
        .context("missing model status")?;
    if !["passed", "failed", "not_checked"].contains(&model) {
        bail!("invalid model status");
    }
    let mut event = json!({"example_id":note["example_id"],"text_sha256":hash,"model_status":model,"reviewer_status":status,"reviewer":note["reviewer"],"method":note["method"]});
    if let Some(binding) = note.get("report_binding") {
        if !["verified", "unverified"].contains(&binding.as_str().unwrap_or("")) {
            bail!("invalid report binding");
        }
        event["report_binding"] = binding.clone();
    }
    if let Some(metadata_digest) = metadata_digest {
        event["metadata_sha256"] = json!(metadata_digest);
    }
    Ok(json!({"version":1,"gate_id":gate,"events":[event]}))
}

pub fn command_feedback_export(args: &ArgMatches) -> Result<i32> {
    let started = Instant::now();
    let read = |name| -> Result<Value> {
        serde_json::from_slice(&fs::read(required_arg(args, name)?)?)
            .user_context("invalid JSON input")
    };
    let corpus = read("corpus")?;
    let note = read("note")?;
    let event = export_review(&corpus, &note).user_context("invalid feedback export")?;
    write_new(required_arg(args, "output")?, &event)?;
    if args.get_flag("telemetry") {
        eprintln!(
            "{}",
            json!({"event":"content_review_note","version":1,"operation":"export-review","elapsed_ms":started.elapsed().as_secs_f64()*1000.0})
        );
    }
    Ok(0)
}

fn write_new(path: &str, value: &Value) -> Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut output = options.open(path).context("creating feedback output")?;
    serde_json::to_writer_pretty(&mut output, value)?;
    output.write_all(b"\n")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn metadata_hash_is_order_independent_and_matches_portable_format() {
        let expected = "c2dd6ca92cc1c5c1a1208812e55ddfac14cbdbcb54ef0f9441d0deb31e3d541e";
        assert_eq!(
            metadata_hash(&json!({"titre":"Été", "lang":"fr"})).unwrap(),
            expected
        );
        assert!(metadata_hash(&json!({"n": 1})).is_err());
    }
    #[test]
    fn capture_rejects_ambiguous_gates_and_hashes_exact_bytes() {
        let args = crate::cli::build_cli()
            .try_get_matches_from([
                "aexeo",
                "feedback-capture",
                "--text",
                "x",
                "--report",
                "x",
                "--gate",
                "g",
                "--example",
                "e",
                "--output",
                "x",
            ])
            .unwrap();
        let args = args.subcommand_matches("feedback-capture").unwrap();
        let note = build_note(
            b"abc\r\n",
            &json!({"gates":[{"gate_id":"g","status":"passed"}]}),
            "g",
            "e",
            args,
        )
        .unwrap();
        assert_eq!(
            note["text_sha256"],
            "552bab6864c7a7b69a502ed1854b9245c0e1a30f008aaa0b281da62585fdb025"
        );
        assert!(!note.to_string().contains("abc"));
        assert!(build_note(b"abc", &json!({"gates":[{"gate_id":"g","status":"passed"},{"gate_id":"g","status":"passed"}]}), "g", "e", args).is_err());
    }
}

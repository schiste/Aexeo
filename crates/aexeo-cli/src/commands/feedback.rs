use std::{
    fs::{self, OpenOptions},
    io::Write,
    time::Instant,
};

use anyhow::{Context, Result, bail};
use clap::ArgMatches;
use ring::digest::{SHA256, digest};
use serde_json::{Map, Value, json};

use crate::commands::common::required_arg;

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
    let hash = digest(&SHA256, text)
        .as_ref()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
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
    Ok(
        json!({"version":1,"kind":"reported_human_feedback","gate_id":gate,"example_id":example,
        "text_sha256":hash,"model_status":status,"human_status":args.get_one::<String>("human-status"),
        "reviewer":args.get_one::<String>("reviewer"),"method":args.get_one::<String>("method"),"reason":reason,"context":context}),
    )
}

pub fn command_feedback(args: &ArgMatches) -> Result<i32> {
    let started = Instant::now();
    let text = fs::read(required_arg(args, "text")?).context("reading text")?;
    let report: Value =
        serde_json::from_slice(&fs::read(required_arg(args, "report")?).context("reading report")?)
            .context("parsing report")?;
    let note = build_note(
        &text,
        &report,
        required_arg(args, "gate")?,
        required_arg(args, "example")?,
        args,
    )?;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut output = options
        .open(required_arg(args, "output")?)
        .context("creating feedback note")?;
    serde_json::to_writer_pretty(&mut output, &note)?;
    output.write_all(b"\n")?;
    if args.get_flag("telemetry") {
        eprintln!(
            "{}",
            json!({"event":"content_review_note", "version":1,"operation":"capture", "elapsed_ms":started.elapsed().as_secs_f64()*1000.0})
        );
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;
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

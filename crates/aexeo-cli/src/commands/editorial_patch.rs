use crate::commands::common::required_arg;
use aexeo_core::editorial_patch::{self, EditorialEdit, EditorialPatch, MAX_PATCH_BYTES};
use anyhow::{Context, Result, ensure};
use clap::ArgMatches;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
};

fn read_bounded(path: &str) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    File::open(path)?
        .take((MAX_PATCH_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= MAX_PATCH_BYTES, "input exceeds size limit");
    Ok(bytes)
}

fn parse<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T> {
    let value = aexeo_contracts::content_gate_change_validation::parse_unique_json_value(bytes)
        .map_err(|_| anyhow::anyhow!("invalid JSON or duplicate object key"))?;
    Ok(serde_json::from_value(value)?)
}

pub fn command_editorial_patch(matches: &ArgMatches) -> Result<i32> {
    let source_path = required_arg(matches, "source")?;
    let source = String::from_utf8(read_bounded(source_path)?).context("source must be UTF-8")?;
    let operation = required_arg(matches, "operation")?;
    let edits_count;
    match operation {
        "propose" => {
            let path = required_arg(matches, "edits")?;
            let edits: Vec<EditorialEdit> = parse(&read_bounded(path)?)?;
            let patch = editorial_patch::propose(&source, edits)?;
            edits_count = patch.edits.len();
            println!("{}", serde_json::to_string_pretty(&patch)?);
        }
        "review" | "apply" => {
            let path = required_arg(matches, "patch")?;
            let patch_bytes = read_bounded(path)?;
            let patch_hash: String = ring::digest::digest(&ring::digest::SHA256, &patch_bytes)
                .as_ref()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect();
            let patch: EditorialPatch = parse(&patch_bytes)?;
            let result = editorial_patch::apply(&source, &patch)?;
            edits_count = patch.edits.len();
            if operation == "review" {
                print!("{result}");
                eprintln!("reviewed proposal SHA-256: {patch_hash}");
            } else {
                ensure!(
                    required_arg(matches, "confirm")? == patch_hash,
                    "proposal does not match reviewed SHA-256"
                );
                let output = required_arg(matches, "output")?;
                ensure!(
                    read_bounded(source_path)? == source.as_bytes(),
                    "source changed during application"
                );
                let mut options = OpenOptions::new();
                options.write(true).create_new(true);
                #[cfg(unix)]
                options.mode(0o600);
                let mut file = options
                    .open(output)
                    .context("cannot create output; existing files are never overwritten")?;
                file.write_all(result.as_bytes())?;
                file.sync_all()?;
            }
        }
        _ => unreachable!(),
    }
    if matches.get_flag("telemetry") {
        eprintln!(
            "{}",
            serde_json::json!({"operation": operation, "edits": edits_count, "status": "ok"})
        );
    }
    Ok(0)
}

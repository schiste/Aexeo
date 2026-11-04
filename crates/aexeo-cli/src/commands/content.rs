use super::exit_code::{EXIT_FINDINGS, EXIT_SUCCESS, EXIT_UNSUPPORTED};
use aexeo_core::{
    content_gate::{ContentInput, GateStatus},
    content_presets::{builtin_content_presets, resolve_content_preset},
};
use anyhow::{Context, Result, bail};
use clap::ArgMatches;
use std::{
    collections::BTreeMap,
    fs,
    io::{self, Read},
};

pub fn command_content(args: &ArgMatches) -> Result<i32> {
    let mut sources = builtin_content_presets();
    if let Some(entries) = args.get_many::<String>("preset-source") {
        for entry in entries {
            let (name, path) = entry
                .split_once('=')
                .context("--preset-source requires NAME=PATH")?;
            if name.is_empty() || path.is_empty() {
                bail!("--preset-source requires NAME=PATH");
            }
            let source =
                fs::read_to_string(path).with_context(|| format!("reading preset {path}"))?;
            sources.insert(name.to_owned(), source);
        }
    }
    let preset_name = args.get_one::<String>("preset").unwrap();
    let preset = resolve_content_preset(preset_name, &sources)?;
    if args.get_flag("explain") {
        println!("{}", toml_output(&preset)?);
        return Ok(EXIT_SUCCESS);
    }
    let path = args.get_one::<String>("path").unwrap();
    let text = if path == "-" {
        let mut text = String::new();
        io::stdin().read_to_string(&mut text)?;
        text
    } else {
        fs::read_to_string(path).with_context(|| format!("reading content {path}"))?
    };
    let metadata: Option<BTreeMap<String, String>> = args
        .get_one::<String>("metadata")
        .map(|path| -> Result<_> {
            let input =
                fs::read_to_string(path).with_context(|| format!("reading metadata {path}"))?;
            Ok(serde_json::from_str(&input)
                .context("metadata must be a JSON object of string values")?)
        })
        .transpose()?;
    let report = preset.evaluate(&ContentInput {
        id: path.clone(),
        text: Some(text),
        metadata,
    })?;
    if args.get_one::<String>("format").map(String::as_str) == Some("json") {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        for gate in &report.gates {
            println!("{}: {:?}", gate.gate_id, gate.status);
            if let Some(reason) = &gate.reason {
                println!("  {reason}");
            }
            for finding in &gate.findings {
                println!(
                    "  {}:{}:{}: {}",
                    finding.path, finding.line, finding.column, finding.message
                );
            }
        }
    }
    Ok(
        if report
            .gates
            .iter()
            .any(|gate| gate.status == GateStatus::NotChecked)
        {
            EXIT_UNSUPPORTED
        } else if report
            .gates
            .iter()
            .any(|gate| gate.status == GateStatus::Failed)
        {
            EXIT_FINDINGS
        } else {
            EXIT_SUCCESS
        },
    )
}

fn toml_output(preset: &aexeo_core::content_gate::ContentPreset) -> Result<String> {
    // JSON preserves the fully resolved structure without requiring a CLI TOML dependency.
    Ok(serde_json::to_string_pretty(preset)?)
}

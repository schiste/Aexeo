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
    if args.get_flag("list-site-groups") {
        let groups = aexeo_core::static_check::native_site_rule_groups();
        if args.get_one::<String>("format").map(String::as_str) == Some("json") {
            println!("{}", serde_json::to_string_pretty(&groups)?);
        } else {
            for name in groups {
                println!("{name}");
            }
        }
        return Ok(EXIT_SUCCESS);
    }
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
    if args.get_flag("list-presets") {
        if args.get_one::<String>("format").map(String::as_str) == Some("json") {
            println!(
                "{}",
                serde_json::to_string_pretty(&sources.keys().collect::<Vec<_>>())?
            );
        } else {
            for name in sources.keys() {
                println!("{name}");
            }
        }
        return Ok(EXIT_SUCCESS);
    }
    let preset_name = args.get_one::<String>("preset").unwrap();
    let preset = resolve_content_preset(preset_name, &sources)?;
    if args.get_flag("explain") {
        println!("{}", toml_output(&preset)?);
        return Ok(EXIT_SUCCESS);
    }
    let path = args.get_one::<String>("path").unwrap();
    let text = if args.get_flag("no-text") {
        None
    } else if path == "-" {
        let mut text = String::new();
        io::stdin().read_to_string(&mut text)?;
        Some(text)
    } else {
        Some(fs::read_to_string(path).with_context(|| format!("reading content {path}"))?)
    };
    let metadata: Option<BTreeMap<String, String>> = args
        .get_one::<String>("metadata")
        .map(|path| -> Result<_> {
            let input =
                fs::read_to_string(path).with_context(|| format!("reading metadata {path}"))?;
            serde_json::from_str(&input).context("metadata must be a JSON object of string values")
        })
        .transpose()?;
    let site = args
        .get_one::<String>("site")
        .map(|path| -> Result<_> {
            let root = std::path::Path::new(path);
            let explicit = args
                .get_one::<String>("site-config")
                .map(std::path::Path::new);
            let config = aexeo_core::config::load_config(root, explicit)?;
            let site_root = aexeo_core::adapter::resolve_static_site_root(root, &config)?;
            Ok((aexeo_core::load_site(&site_root)?, config))
        })
        .transpose()?;
    let id = if args.get_flag("no-text") {
        args.get_one::<String>("site").unwrap_or(path).clone()
    } else {
        path.clone()
    };
    let mut input = match (
        args.get_one::<String>("input-format").map(String::as_str),
        text,
    ) {
        (Some("markdown"), Some(text)) => aexeo_core::markdown_adapter::markdown_input(id, &text)?,
        (_, text) => ContentInput {
            id,
            text,
            headings: None,
            metadata: None,
        },
    };
    input.metadata = metadata;
    let report =
        preset.evaluate_with_site(&input, site.as_ref().map(|(site, config)| (site, config)))?;
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
            .any(|gate| gate.blocking && gate.status == GateStatus::NotChecked)
        {
            EXIT_UNSUPPORTED
        } else if report
            .gates
            .iter()
            .any(|gate| gate.blocking && gate.status == GateStatus::Failed)
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

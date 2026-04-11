use anyhow::{Result, bail};
use clap::ArgMatches;
use seogeo_core::config::load_config_with_diagnostics;
use seogeo_core::{render_resolved_config_json, render_resolved_config_toml};
use std::path::PathBuf;

use crate::output::{emit_config_warnings, render_config_command_json};

fn canonicalize_or_keep(path: &str) -> PathBuf {
    PathBuf::from(path)
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from(path))
}

pub fn command_config(submatches: &ArgMatches) -> Result<i32> {
    match submatches.subcommand() {
        Some(("print", print_matches)) => command_config_print(print_matches),
        Some((other, _)) => bail!("unsupported config command: {}", other),
        None => bail!("missing config subcommand"),
    }
}

fn command_config_print(submatches: &ArgMatches) -> Result<i32> {
    let root = canonicalize_or_keep(submatches.get_one::<String>("path").unwrap());
    let explicit_config = submatches.get_one::<String>("config").map(PathBuf::from);
    let loaded = load_config_with_diagnostics(&root, explicit_config.as_deref())?;
    let config = loaded.config;
    let warnings = loaded.warnings;
    match submatches
        .get_one::<String>("format")
        .map(String::as_str)
        .unwrap_or("toml")
    {
        "json" => {
            let rendered =
                serde_json::from_str::<serde_json::Value>(&render_resolved_config_json(&config)?)?;
            println!(
                "{}",
                render_config_command_json("config print", rendered, warnings)?
            );
        }
        "toml" => {
            emit_config_warnings(&warnings);
            println!("{}", render_resolved_config_toml(&config)?);
        }
        other => bail!("unsupported config format: {}", other),
    }
    Ok(0)
}

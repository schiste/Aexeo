use anyhow::{Result, bail};
use clap::ArgMatches;
use seogeo_core::config::load_config;
use seogeo_core::{render_resolved_config_json, render_resolved_config_toml};
use std::path::PathBuf;

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
    let config = load_config(&root, explicit_config.as_deref())?;
    match submatches
        .get_one::<String>("format")
        .map(String::as_str)
        .unwrap_or("toml")
    {
        "json" => println!("{}", render_resolved_config_json(&config)?),
        "toml" => println!("{}", render_resolved_config_toml(&config)?),
        other => bail!("unsupported config format: {}", other),
    }
    Ok(0)
}

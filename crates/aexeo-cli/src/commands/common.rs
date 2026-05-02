use anyhow::{Result, anyhow};
use clap::ArgMatches;
use std::path::PathBuf;

pub fn required_arg<'a>(matches: &'a ArgMatches, name: &str) -> Result<&'a str> {
    matches
        .get_one::<String>(name)
        .map(String::as_str)
        .ok_or_else(|| anyhow!("missing required CLI argument '{}'", name))
}

pub fn canonicalize_or_keep(path: &str) -> PathBuf {
    PathBuf::from(path)
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from(path))
}

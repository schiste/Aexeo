use aexeo_core::config::ConfigWarning;
use anyhow::{Result, anyhow};
use clap::ArgMatches;
use std::path::PathBuf;

use super::exit_code::EXIT_UNSUPPORTED;
use crate::output::render_failed_command_json;

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

/// Report a failed command on the streams its format promises: JSON output
/// keeps stdout parseable and stderr free of plain text, so JSON callers get
/// the structured failure on stdout. Every other format keeps stdout for its
/// own document and reports on stderr. The cause chain is always included.
pub fn report_failure(
    command: &str,
    format: &str,
    error: &anyhow::Error,
    warnings: Vec<ConfigWarning>,
) -> Result<()> {
    match format {
        "json" => println!(
            "{}",
            render_failed_command_json(command, format!("{error:#}"), warnings)?
        ),
        _ => eprintln!("{error:#}"),
    }
    Ok(())
}

/// Report an operation on external input (a site, a file, a remote service)
/// that could not be carried out, as distinct from one that found problems.
pub fn emit_failure(command: &str, format: &str, error: anyhow::Error) -> Result<i32> {
    report_failure(command, format, &error, Vec::new())?;
    Ok(EXIT_UNSUPPORTED)
}

use anyhow::{Result, bail};

use crate::commands::{
    common::required_arg,
    config::command_config,
    docs::{command_diff, command_docs, command_quality, command_trend},
    listings::{command_adapters, command_plugin_check, command_rules},
    runtime::{command_crawl, command_verify},
    static_site::{command_baseline, command_check, command_fix, command_generate},
};

pub fn dispatch(matches: clap::ArgMatches) -> Result<i32> {
    match matches.subcommand() {
        Some(("rules", submatches)) => command_rules(submatches),
        Some(("adapters", submatches)) => command_adapters(submatches),
        Some(("config", submatches)) => command_config(submatches),
        Some(("check", submatches)) => command_check(submatches),
        Some(("quality", submatches)) => command_quality(
            required_arg(submatches, "path")?,
            required_arg(submatches, "format")?,
        ),
        Some(("docs", submatches)) => command_docs(
            required_arg(submatches, "action")?,
            required_arg(submatches, "path")?,
            required_arg(submatches, "format")?,
        ),
        Some(("diff", submatches)) => command_diff(
            required_arg(submatches, "baseline")?,
            required_arg(submatches, "current")?,
            required_arg(submatches, "format")?,
        ),
        Some(("trend", submatches)) => command_trend(
            required_arg(submatches, "command_name")?,
            required_arg(submatches, "path")?,
            required_arg(submatches, "format")?,
        ),
        Some(("generate", submatches)) => command_generate(submatches),
        Some(("fix", submatches)) => command_fix(submatches),
        Some(("baseline", submatches)) => command_baseline(submatches),
        Some(("crawl", submatches)) => command_crawl(submatches),
        Some(("verify", submatches)) => command_verify(submatches),
        Some(("plugin-check", submatches)) => command_plugin_check(submatches),
        Some((other, _)) => bail!("unsupported command: {}", other),
        None => bail!("missing command"),
    }
}

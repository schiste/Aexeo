use anyhow::{Result, bail};
use clap::ArgMatches;

use crate::commands::{
    config::command_config,
    docs::{command_diff, command_docs, command_quality, command_trend},
    listings::{command_adapters, command_plugin_check, command_rules},
    runtime::{command_crawl, command_verify},
    static_site::{command_baseline, command_check, command_fix, command_generate},
};

pub fn dispatch(matches: clap::ArgMatches) -> Result<i32> {
    match matches.subcommand() {
        Some(("rules", _)) => Ok(command_rules()),
        Some(("adapters", _)) => Ok(command_adapters()),
        Some(("config", submatches)) => command_config(submatches),
        Some(("check", submatches)) => command_check(submatches),
        Some(("quality", submatches)) => command_quality(
            submatches.get_one::<String>("path").unwrap(),
            submatches.get_one::<String>("format").unwrap(),
        ),
        Some(("docs", submatches)) => command_docs(
            submatches.get_one::<String>("action").unwrap(),
            submatches.get_one::<String>("path").unwrap(),
        ),
        Some(("diff", submatches)) => command_diff(
            submatches.get_one::<String>("baseline").unwrap(),
            submatches.get_one::<String>("current").unwrap(),
            submatches.get_one::<String>("format").unwrap(),
        ),
        Some(("trend", submatches)) => command_trend(
            submatches.get_one::<String>("command_name").unwrap(),
            submatches.get_one::<String>("path").unwrap(),
            submatches.get_one::<String>("format").unwrap(),
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

use anyhow::{Result, bail};

use crate::commands::{
    common::required_arg,
    config::command_config,
    docs::{command_diff, command_docs, command_quality, command_report_render, command_trend},
    facts::command_facts,
    integrations::{
        command_bing_ai, command_indexnow, command_publish_hook, command_search_console,
        command_snippet,
    },
    intelligence::command_intelligence,
    listings::{command_adapters, command_plugin_check, command_rules},
    runtime::{
        command_crawl, command_doctor, command_perf_baseline, command_perf_diff,
        command_profile_runtime, command_verify,
    },
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
        Some(("report", submatches)) => match submatches.subcommand() {
            Some(("render", render_matches)) => command_report_render(
                required_arg(render_matches, "audit")?,
                required_arg(render_matches, "format")?,
            ),
            Some((other, _)) => bail!("unsupported report subcommand: {}", other),
            None => bail!("missing report subcommand"),
        },
        Some(("doctor", submatches)) => command_doctor(submatches),
        Some(("profile", submatches)) => match submatches.subcommand() {
            Some(("runtime", runtime_matches)) => command_profile_runtime(runtime_matches),
            Some((other, _)) => bail!("unsupported profile subcommand: {}", other),
            None => bail!("missing profile subcommand"),
        },
        Some(("perf", submatches)) => match submatches.subcommand() {
            Some(("baseline", baseline_matches)) => command_perf_baseline(baseline_matches),
            Some(("diff", diff_matches)) => command_perf_diff(diff_matches),
            Some((other, _)) => bail!("unsupported perf subcommand: {}", other),
            None => bail!("missing perf subcommand"),
        },
        Some(("trend", submatches)) => command_trend(
            required_arg(submatches, "command_name")?,
            required_arg(submatches, "path")?,
            required_arg(submatches, "format")?,
        ),
        Some(("generate", submatches)) => command_generate(submatches),
        Some(("snippet", submatches)) => command_snippet(submatches),
        Some(("intelligence", submatches)) => command_intelligence(submatches),
        Some(("indexnow", submatches)) => command_indexnow(submatches),
        Some(("bing-ai", submatches)) => command_bing_ai(submatches),
        Some(("search-console", submatches)) => command_search_console(submatches),
        Some(("publish-hook", submatches)) => command_publish_hook(submatches),
        Some(("fix", submatches)) => command_fix(submatches),
        Some(("baseline", submatches)) => command_baseline(submatches),
        Some(("crawl", submatches)) => command_crawl(submatches),
        Some(("verify", submatches)) => command_verify(submatches),
        Some(("plugin-check", submatches)) => command_plugin_check(submatches),
        Some(("facts", submatches)) => command_facts(submatches),
        Some((other, _)) => bail!("unsupported command: {}", other),
        None => bail!("missing command"),
    }
}

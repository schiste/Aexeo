use std::{fs::File, io::Read, time::Duration};

use super::common::required_arg;
use aexeo_core::{
    error::UserContext,
    markdown_adapter::markdown_link_destinations,
    runtime::link_reachability::{ReachabilityStatus, check_link_reachability},
};
use anyhow::{Context, Result, bail};
use clap::ArgMatches;
use serde_json::json;

pub fn command_link_reachability(args: &ArgMatches) -> Result<i32> {
    const MAX_BYTES: u64 = 16 * 1024 * 1024;
    let path = required_arg(args, "input")?;
    let mut data = Vec::new();
    File::open(path)
        .user_context("opening link input")?
        .take(MAX_BYTES + 1)
        .read_to_end(&mut data)?;
    if data.len() as u64 > MAX_BYTES {
        bail!("link input exceeds the size limit");
    }
    let destinations: Vec<String> = match required_arg(args, "input-format")? {
        "markdown" => {
            markdown_link_destinations(std::str::from_utf8(&data).context("input must be UTF-8")?)?
                .context("Markdown link destinations are unavailable")?
        }
        "json" => serde_json::from_slice(&data)
            .user_context("expected a JSON array of link destinations")?,
        _ => unreachable!(),
    };
    let report = check_link_reachability(
        &destinations,
        *args.get_one::<usize>("max-checks").unwrap(),
        Duration::from_secs(*args.get_one::<u64>("timeout").unwrap()),
    )?;
    if args.get_flag("telemetry") {
        let count = |status| {
            report
                .observations
                .iter()
                .filter(|item| item.status == status)
                .count()
        };
        eprintln!(
            "{}",
            json!({"event":"link_reachability", "version":1, "elapsed_ms": report.elapsed_ms,
            "destinations":report.observations.len(), "network_checks":report.network_checks,
            "reachable":count(ReachabilityStatus::Reachable), "not_found":count(ReachabilityStatus::NotFound),
            "uncertain":count(ReachabilityStatus::Uncertain), "skipped":count(ReachabilityStatus::Skipped)})
        );
    }
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(0)
}

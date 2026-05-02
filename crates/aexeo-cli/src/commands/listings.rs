use aexeo_core::{list_adapter_names, list_rule_group_names, validate_python_plugin_module};
use anyhow::Result;
use clap::ArgMatches;

use crate::commands::common::required_arg;
use crate::output::{render_list_command_json, render_plugin_check_command_json};

pub fn command_rules(submatches: &ArgMatches) -> Result<i32> {
    let names = list_rule_group_names();
    match submatches
        .get_one::<String>("format")
        .map(String::as_str)
        .unwrap_or("text")
    {
        "json" => println!("{}", render_list_command_json("rules", names)?),
        _ => {
            for name in names {
                println!("{}", name);
            }
        }
    }
    Ok(0)
}

pub fn command_adapters(submatches: &ArgMatches) -> Result<i32> {
    let names = list_adapter_names();
    match submatches
        .get_one::<String>("format")
        .map(String::as_str)
        .unwrap_or("text")
    {
        "json" => println!("{}", render_list_command_json("adapters", names)?),
        _ => {
            for name in names {
                println!("{}", name);
            }
        }
    }
    Ok(0)
}

pub fn command_plugin_check(submatches: &ArgMatches) -> Result<i32> {
    let manifest = validate_python_plugin_module(required_arg(submatches, "module_name")?)?;
    match submatches
        .get_one::<String>("format")
        .map(String::as_str)
        .unwrap_or("text")
    {
        "json" => println!("{}", render_plugin_check_command_json(manifest)?),
        _ => println!(
            "{} {} [{}] capabilities={}",
            manifest.name,
            manifest.version,
            manifest.namespace,
            manifest.capabilities.join(",")
        ),
    }
    Ok(0)
}

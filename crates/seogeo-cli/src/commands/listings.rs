use anyhow::Result;
use clap::ArgMatches;
use seogeo_core::{list_adapter_names, list_rule_group_names, validate_python_plugin_module};

use crate::commands::common::required_arg;

pub fn command_rules() -> i32 {
    for name in list_rule_group_names() {
        println!("{}", name);
    }
    0
}

pub fn command_adapters() -> i32 {
    for name in list_adapter_names() {
        println!("{}", name);
    }
    0
}

pub fn command_plugin_check(submatches: &ArgMatches) -> Result<i32> {
    let manifest = validate_python_plugin_module(required_arg(submatches, "module_name")?)?;
    println!(
        "{} {} [{}] capabilities={}",
        manifest.name,
        manifest.version,
        manifest.namespace,
        manifest.capabilities.join(",")
    );
    Ok(0)
}

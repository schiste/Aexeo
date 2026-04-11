use anyhow::Result;
use clap::ArgMatches;
use seogeo_core::{list_adapter_names, list_rule_group_names, validate_python_plugin_module};

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
    let manifest =
        validate_python_plugin_module(submatches.get_one::<String>("module_name").unwrap())?;
    println!(
        "{} {} [{}] capabilities={}",
        manifest.name,
        manifest.version,
        manifest.namespace,
        manifest.capabilities.join(",")
    );
    Ok(0)
}

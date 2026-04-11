use anyhow::Result;
use seogeo_core::config::ConfigWarning;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
struct ConfigCommandOutput<T> {
    command: &'static str,
    success: bool,
    warnings: Vec<ConfigWarning>,
    config: T,
}

pub fn render_config_command_json<T: Serialize>(
    command: &'static str,
    config: T,
    warnings: Vec<ConfigWarning>,
) -> Result<String> {
    Ok(serde_json::to_string_pretty(&ConfigCommandOutput {
        command,
        success: true,
        warnings,
        config,
    })?)
}

pub fn emit_config_warnings(warnings: &[ConfigWarning]) {
    for warning in warnings {
        eprintln!("{} {}", warning.code, warning.message);
    }
}

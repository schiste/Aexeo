use aexeo_core::PluginManifestCheck;
use aexeo_core::config::ConfigWarning;
use anyhow::Result;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
struct ListCommandOutput<T> {
    command: &'static str,
    success: bool,
    items: T,
}

#[derive(Debug, Clone, Serialize)]
struct TextCommandOutput {
    command: &'static str,
    success: bool,
    kind: String,
    output: String,
    warnings: Vec<ConfigWarning>,
}

#[derive(Debug, Clone, Serialize)]
struct PathsCommandOutput {
    command: &'static str,
    success: bool,
    action: String,
    paths: Vec<String>,
    warnings: Vec<ConfigWarning>,
}

#[derive(Debug, Clone, Serialize)]
struct PathCommandOutput {
    command: &'static str,
    success: bool,
    path: String,
    warnings: Vec<ConfigWarning>,
}

#[derive(Debug, Clone, Serialize)]
struct PluginCheckCommandOutput {
    command: &'static str,
    success: bool,
    plugin: PluginManifestCheck,
}

#[derive(Debug, Clone, Serialize)]
struct DataCommandOutput<T> {
    command: &'static str,
    success: bool,
    result: T,
    warnings: Vec<ConfigWarning>,
}

pub fn render_list_command_json<T: Serialize>(command: &'static str, items: T) -> Result<String> {
    Ok(serde_json::to_string_pretty(&ListCommandOutput {
        command,
        success: true,
        items,
    })?)
}

pub fn render_text_command_json(
    command: &'static str,
    kind: &str,
    output: String,
    warnings: Vec<ConfigWarning>,
) -> Result<String> {
    Ok(serde_json::to_string_pretty(&TextCommandOutput {
        command,
        success: true,
        kind: kind.to_string(),
        output,
        warnings,
    })?)
}

pub fn render_paths_command_json(
    command: &'static str,
    action: &str,
    success: bool,
    paths: Vec<String>,
    warnings: Vec<ConfigWarning>,
) -> Result<String> {
    Ok(serde_json::to_string_pretty(&PathsCommandOutput {
        command,
        success,
        action: action.to_string(),
        paths,
        warnings,
    })?)
}

pub fn render_path_command_json(
    command: &'static str,
    success: bool,
    path: String,
    warnings: Vec<ConfigWarning>,
) -> Result<String> {
    Ok(serde_json::to_string_pretty(&PathCommandOutput {
        command,
        success,
        path,
        warnings,
    })?)
}

pub fn render_plugin_check_command_json(plugin: PluginManifestCheck) -> Result<String> {
    Ok(serde_json::to_string_pretty(&PluginCheckCommandOutput {
        command: "plugin-check",
        success: true,
        plugin,
    })?)
}

pub fn render_data_command_json<T: Serialize>(
    command: &'static str,
    success: bool,
    result: T,
    warnings: Vec<ConfigWarning>,
) -> Result<String> {
    Ok(serde_json::to_string_pretty(&DataCommandOutput {
        command,
        success,
        result,
        warnings,
    })?)
}

use anyhow::{Result, bail};
use serde::Serialize;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use toml::Value;

const PLUGIN_API_VERSION: usize = 1;
const REGISTERED_PLUGIN_SETTINGS_SCHEMAS: &[PluginSettingsSchema] = &[];

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PluginManifestCheck {
    pub name: String,
    pub namespace: String,
    pub version: String,
    pub capabilities: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginSettingsSchema {
    pub namespace: &'static str,
    pub allowed_keys: &'static [&'static str],
}

pub fn registered_plugin_settings_schemas() -> &'static [PluginSettingsSchema] {
    REGISTERED_PLUGIN_SETTINGS_SCHEMAS
}

fn python_module_candidates(module_name: &str) -> Vec<PathBuf> {
    let relative = module_name.replace('.', "/");
    vec![
        PathBuf::from(format!("{}.py", relative)),
        PathBuf::from(relative).join("__init__.py"),
    ]
}

fn resolve_module_file(module_name: &str, search_roots: &[PathBuf]) -> Result<PathBuf> {
    for root in search_roots {
        for candidate in python_module_candidates(module_name) {
            let path = root.join(&candidate);
            if path.exists() {
                return Ok(path);
            }
        }
    }
    bail!(
        "plugin '{}' could not be resolved from current search paths",
        module_name
    )
}

fn current_search_roots() -> Vec<PathBuf> {
    let mut roots = vec![std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))];
    if let Some(paths) = std::env::var_os("PYTHONPATH") {
        roots.extend(std::env::split_paths(&paths));
    }
    roots
}

fn extract_assignment_value(raw: &str, name: &str) -> Option<String> {
    let marker = format!("{} =", name);
    let start = raw.find(&marker)? + marker.len();
    let rest = raw[start..].trim_start();
    if rest.starts_with('{') {
        let mut depth = 0usize;
        for (index, ch) in rest.char_indices() {
            match ch {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(rest[..=index].to_string());
                    }
                }
                _ => {}
            }
        }
        return None;
    }
    rest.lines().next().map(|line| line.trim().to_string())
}

fn extract_string_field(raw: &str, key: &str) -> Option<String> {
    for quote in ['\'', '"'] {
        let marker = format!("{quote}{key}{quote}:");
        if let Some(index) = raw.find(&marker) {
            let rest = raw[index + marker.len()..].trim_start();
            if let Some(stripped) = rest.strip_prefix(quote)
                && let Some(end) = stripped.find(quote)
            {
                return Some(stripped[..end].to_string());
            }
        }
    }
    None
}

fn extract_usize_field(raw: &str, key: &str) -> Option<usize> {
    for quote in ['\'', '"'] {
        let marker = format!("{quote}{key}{quote}:");
        if let Some(index) = raw.find(&marker) {
            let rest = raw[index + marker.len()..].trim_start();
            let digits: String = rest.chars().take_while(|ch| ch.is_ascii_digit()).collect();
            if !digits.is_empty() {
                return digits.parse().ok();
            }
        }
    }
    None
}

fn extract_capabilities(raw: &str) -> Vec<String> {
    for quote in ['\'', '"'] {
        let marker = format!("{quote}capabilities{quote}:");
        if let Some(index) = raw.find(&marker) {
            let rest = raw[index + marker.len()..].trim_start();
            if let Some(stripped) = rest.strip_prefix('[')
                && let Some(end) = stripped.find(']')
            {
                return stripped[..end]
                    .split(',')
                    .filter_map(|item| {
                        let trimmed = item.trim().trim_matches('\'').trim_matches('"');
                        (!trimmed.is_empty()).then(|| trimmed.to_string())
                    })
                    .collect();
            }
        }
    }
    Vec::new()
}

fn validate_module_file(module_name: &str, path: &Path) -> Result<PluginManifestCheck> {
    let raw = fs::read_to_string(path)?;
    let api_version = extract_assignment_value(&raw, "AEXEO_PLUGIN_API_VERSION")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(PLUGIN_API_VERSION);
    let Some(manifest_raw) = extract_assignment_value(&raw, "AEXEO_PLUGIN_MANIFEST") else {
        bail!("plugin '{}' must expose AEXEO_PLUGIN_MANIFEST", module_name);
    };
    let namespace = extract_string_field(&manifest_raw, "namespace").unwrap_or_default();
    if namespace.is_empty() || !namespace.contains('.') {
        bail!("plugin '{}' must declare a dotted namespace", module_name);
    }
    let min_api =
        extract_usize_field(&manifest_raw, "api_version_min").unwrap_or(PLUGIN_API_VERSION);
    let max_api =
        extract_usize_field(&manifest_raw, "api_version_max").unwrap_or(PLUGIN_API_VERSION);
    if api_version != PLUGIN_API_VERSION
        || !(min_api <= PLUGIN_API_VERSION && PLUGIN_API_VERSION <= max_api)
    {
        bail!(
            "plugin '{}' targets incompatible API version range",
            module_name
        );
    }
    if !raw.contains("def aexeo_register(") {
        bail!(
            "plugin '{}' does not expose a callable aexeo_register(registry)",
            module_name
        );
    }
    Ok(PluginManifestCheck {
        name: extract_string_field(&manifest_raw, "name")
            .unwrap_or_else(|| module_name.to_string()),
        namespace,
        version: extract_string_field(&manifest_raw, "version")
            .unwrap_or_else(|| "0.0.0".to_string()),
        capabilities: extract_capabilities(&manifest_raw),
    })
}

pub fn validate_python_plugin_module(module_name: &str) -> Result<PluginManifestCheck> {
    let roots = current_search_roots();
    let path = resolve_module_file(module_name, &roots)?;
    validate_module_file(module_name, &path)
}

pub fn validate_plugin_settings(
    plugins: &[String],
    plugin_settings: &BTreeMap<String, BTreeMap<String, Value>>,
) -> Result<()> {
    for (namespace, settings) in plugin_settings {
        if !plugins.iter().any(|plugin| plugin == namespace) {
            bail!(
                "plugin_settings.{} requires `plugins` to include '{}'",
                namespace,
                namespace
            );
        }

        let Some(schema) = REGISTERED_PLUGIN_SETTINGS_SCHEMAS
            .iter()
            .find(|schema| schema.namespace == namespace)
        else {
            bail!(
                "plugin_settings.{} is not supported because plugin '{}' does not publish a registered settings schema",
                namespace,
                namespace
            );
        };

        for key in settings.keys() {
            if !schema.allowed_keys.contains(&key.as_str()) {
                bail!(
                    "unknown config key '{}' in [plugin_settings.{}]",
                    key,
                    namespace
                );
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{resolve_module_file, validate_module_file, validate_plugin_settings};
    use std::collections::BTreeMap;
    use std::fs;
    use toml::Value;

    #[test]
    fn validates_literal_python_plugin_manifest() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        let package_dir = root.join("example");
        fs::create_dir_all(&package_dir).unwrap();
        fs::write(
            package_dir.join("plugin.py"),
            "AEXEO_PLUGIN_API_VERSION = 1\nAEXEO_PLUGIN_MANIFEST = {'name':'Example Plugin','namespace':'example.plugin','version':'1.0.0','capabilities':['rules','adapters']}\ndef aexeo_register(registry):\n    return None\n",
        )
        .unwrap();
        let path = resolve_module_file("example.plugin", &[root.to_path_buf()]).unwrap();
        let manifest = validate_module_file("example.plugin", &path).unwrap();
        assert_eq!(manifest.namespace, "example.plugin");
        assert_eq!(
            manifest.capabilities,
            vec!["rules".to_string(), "adapters".to_string()]
        );
    }

    #[test]
    fn rejects_settings_for_undeclared_plugin_namespace() {
        let error = validate_plugin_settings(
            &[],
            &BTreeMap::from([(
                "example.plugin".to_string(),
                BTreeMap::from([("enabled".to_string(), Value::Boolean(true))]),
            )]),
        )
        .expect_err("settings without declared plugin should fail");
        assert!(
            error
                .to_string()
                .contains("requires `plugins` to include 'example.plugin'")
        );
    }
}

use anyhow::{Context, Result, bail};
use std::fs;
use std::path::{Component, Path, PathBuf};
use toml::Value;

use super::Config;

fn normalize_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            _ => normalized.push(component.as_os_str()),
        }
    }
    normalized
}

fn merge_toml(base: Value, overlay: Value) -> Value {
    match (base, overlay) {
        (Value::Table(mut base_table), Value::Table(overlay_table)) => {
            for (key, overlay_value) in overlay_table {
                let merged_value = if let Some(base_value) = base_table.remove(&key) {
                    merge_toml(base_value, overlay_value)
                } else {
                    overlay_value
                };
                base_table.insert(key, merged_value);
            }
            Value::Table(base_table)
        }
        (_, overlay) => overlay,
    }
}

fn resolve_extend_paths(value: Option<&Value>, config_path: &Path) -> Result<Vec<PathBuf>> {
    let base_dir = config_path.parent().unwrap_or_else(|| Path::new("."));
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    match value {
        Value::String(path) => Ok(vec![normalize_path(&base_dir.join(path))]),
        Value::Array(items) => items
            .iter()
            .map(|item| match item {
                Value::String(path) => Ok(normalize_path(&base_dir.join(path))),
                _ => bail!(
                    "config extends entries must be strings in {}",
                    config_path.display()
                ),
            })
            .collect(),
        _ => bail!(
            "config extends must be a string or array of strings in {}",
            config_path.display()
        ),
    }
}

fn load_config_value(config_path: &Path, seen: &mut Vec<PathBuf>) -> Result<Value> {
    let normalized_path = config_path
        .canonicalize()
        .unwrap_or_else(|_| normalize_path(config_path));
    if seen.contains(&normalized_path) {
        bail!(
            "cyclic config extends detected at {}",
            normalized_path.display()
        );
    }

    let text = fs::read_to_string(&normalized_path)
        .with_context(|| format!("failed to read config at {}", normalized_path.display()))?;
    let parsed = toml::from_str::<Value>(&text).with_context(|| {
        format!(
            "failed to parse TOML config at {}",
            normalized_path.display()
        )
    })?;
    let Value::Table(_) = &parsed else {
        bail!(
            "config root must be a TOML table at {}",
            normalized_path.display()
        );
    };

    seen.push(normalized_path.clone());
    let mut merged = Value::Table(toml::map::Map::new());
    for parent_path in resolve_extend_paths(parsed.get("extends"), &normalized_path)? {
        if !parent_path.exists() {
            bail!("extended config does not exist: {}", parent_path.display());
        }
        merged = merge_toml(merged, load_config_value(&parent_path, seen)?);
    }
    let _ = seen.pop();

    Ok(merge_toml(merged, parsed))
}

pub fn load_config(root: &Path, explicit_path: Option<&Path>) -> Result<Config> {
    let config_path = explicit_path
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("seogeo.toml"));
    if !config_path.exists() {
        return Ok(Config::default());
    }
    let merged = load_config_value(&config_path, &mut Vec::new())?;
    merged.try_into::<Config>().with_context(|| {
        format!(
            "failed to deserialize merged config at {}",
            config_path.display()
        )
    })
}

#[cfg(test)]
mod tests {
    use super::load_config;
    use crate::config::default_rule_switches;
    use std::fs;

    #[test]
    fn loads_defaults_when_config_is_missing() {
        let temp_dir = tempfile::tempdir().unwrap();
        let config = load_config(temp_dir.path(), None).unwrap();
        assert_eq!(config.adapter, "auto");
        assert_eq!(config.audit_log_limit, 5);
        assert!(
            default_rule_switches()
                .get("html")
                .copied()
                .unwrap_or(false)
        );
        assert_eq!(config.default_twitter_card, "summary");
    }

    #[test]
    fn loads_simple_toml_config() {
        let temp_dir = tempfile::tempdir().unwrap();
        fs::write(
            temp_dir.path().join("seogeo.toml"),
            r#"
site_url = "https://example.com"
profile = "chau7"
audit_log_limit = 9
[severity_overrides]
SEO001 = "warning"
[checks]
html = true
links = false
"#,
        )
        .unwrap();
        let config = load_config(temp_dir.path(), None).unwrap();
        assert_eq!(config.site_url.as_deref(), Some("https://example.com"));
        assert_eq!(config.profile, "chau7");
        assert_eq!(config.audit_log_limit, 9);
        assert_eq!(
            config.severity_overrides.get("SEO001").map(String::as_str),
            Some("warning")
        );
        assert_eq!(config.checks.get("links").copied(), Some(false));
    }

    #[test]
    fn merges_parent_configs_before_current_file() {
        let temp_dir = tempfile::tempdir().unwrap();
        fs::write(
            temp_dir.path().join("base.toml"),
            r#"
profile = "chau7"
[severity_overrides]
SEO001 = "warning"
[checks]
html = false
links = true
"#,
        )
        .unwrap();
        fs::write(
            temp_dir.path().join("seogeo.toml"),
            r#"
extends = ["base.toml"]
site_url = "https://example.com"
[severity_overrides]
SEO002 = "warning"
[checks]
links = false
"#,
        )
        .unwrap();

        let config = load_config(temp_dir.path(), None).unwrap();
        assert_eq!(config.site_url.as_deref(), Some("https://example.com"));
        assert_eq!(config.profile, "chau7");
        assert_eq!(
            config.severity_overrides.get("SEO001").map(String::as_str),
            Some("warning")
        );
        assert_eq!(
            config.severity_overrides.get("SEO002").map(String::as_str),
            Some("warning")
        );
        assert_eq!(config.checks.get("html").copied(), Some(false));
        assert_eq!(config.checks.get("links").copied(), Some(false));
    }

    #[test]
    fn rejects_cyclic_extends_chains() {
        let temp_dir = tempfile::tempdir().unwrap();
        fs::write(temp_dir.path().join("a.toml"), r#"extends = ["b.toml"]"#).unwrap();
        fs::write(temp_dir.path().join("b.toml"), r#"extends = ["a.toml"]"#).unwrap();

        let error = load_config(temp_dir.path(), Some(&temp_dir.path().join("a.toml")))
            .expect_err("cyclic extends should fail");
        assert!(error.to_string().contains("cyclic config extends"));
    }
}

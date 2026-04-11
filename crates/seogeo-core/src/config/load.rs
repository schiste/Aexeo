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

fn insert_if_absent(root: &mut toml::map::Map<String, Value>, key: &str, value: Value) {
    root.entry(key.to_string()).or_insert(value);
}

fn move_table_field_if_absent(
    root: &mut toml::map::Map<String, Value>,
    table: &mut toml::map::Map<String, Value>,
    source_key: &str,
    target_key: &str,
) {
    if let Some(value) = table.remove(source_key) {
        insert_if_absent(root, target_key, value);
    }
}

fn merge_bool_rule_toggles(
    root: &mut toml::map::Map<String, Value>,
    rules_table: &mut toml::map::Map<String, Value>,
) {
    let checks_table = root
        .entry("checks".to_string())
        .or_insert_with(|| Value::Table(toml::map::Map::new()))
        .as_table_mut()
        .expect("checks is always a table");
    for key in [
        "html",
        "links",
        "sitemap",
        "robots",
        "social",
        "schema",
        "llm",
        "content",
        "structure",
    ] {
        if matches!(rules_table.get(key), Some(Value::Boolean(_)))
            && let Some(value) = rules_table.remove(key)
        {
            checks_table.entry(key.to_string()).or_insert(value);
        }
    }
}

fn normalize_versioned_surface(mut merged: Value) -> Result<Value> {
    let Value::Table(ref mut root) = merged else {
        bail!("config root must be a TOML table");
    };

    let _ = root.remove("version");

    if let Some(Value::Table(mut table)) = root.remove("site") {
        move_table_field_if_absent(root, &mut table, "url", "site_url");
        move_table_field_if_absent(root, &mut table, "source_dir", "source_dir");
        move_table_field_if_absent(root, &mut table, "adapter", "adapter");
        move_table_field_if_absent(root, &mut table, "canonical_style", "canonical_style");
    }

    if let Some(Value::Table(mut table)) = root.remove("runtime") {
        move_table_field_if_absent(root, &mut table, "engine", "browser_engine");
        move_table_field_if_absent(root, &mut table, "wait_until", "browser_wait_until");
        move_table_field_if_absent(root, &mut table, "headers", "crawl_headers");
        move_table_field_if_absent(root, &mut table, "cookies", "crawl_cookies");
        move_table_field_if_absent(root, &mut table, "basic_auth", "crawl_basic_auth");
        move_table_field_if_absent(root, &mut table, "seeds", "crawl_seeds");
        move_table_field_if_absent(
            root,
            &mut table,
            "include_patterns",
            "crawl_include_patterns",
        );
        move_table_field_if_absent(
            root,
            &mut table,
            "exclude_patterns",
            "crawl_exclude_patterns",
        );
        move_table_field_if_absent(root, &mut table, "use_sitemap", "crawl_use_sitemap");
        move_table_field_if_absent(root, &mut table, "capture_trace", "crawl_capture_trace");
        move_table_field_if_absent(
            root,
            &mut table,
            "capture_screenshot",
            "crawl_capture_screenshot",
        );
        move_table_field_if_absent(root, &mut table, "capture_console", "crawl_capture_console");
        move_table_field_if_absent(root, &mut table, "capture_network", "crawl_capture_network");
        move_table_field_if_absent(root, &mut table, "artifact_dir", "crawl_artifact_dir");
    }

    if let Some(Value::Table(mut table)) = root.remove("policy") {
        move_table_field_if_absent(root, &mut table, "ignore_rules", "ignore_rules");
        move_table_field_if_absent(root, &mut table, "ignore_paths", "ignore_paths");
        move_table_field_if_absent(root, &mut table, "severity_overrides", "severity_overrides");
        move_table_field_if_absent(root, &mut table, "suppressions", "suppressions");
        move_table_field_if_absent(
            root,
            &mut table,
            "route_policy_overrides",
            "route_policy_overrides",
        );
    }

    if let Some(Value::Table(mut table)) = root.remove("output") {
        move_table_field_if_absent(root, &mut table, "baseline_file", "baseline_file");
        move_table_field_if_absent(root, &mut table, "audit_log_limit", "audit_log_limit");
    }

    if let Some(Value::Table(mut table)) = root.remove("quality") {
        move_table_field_if_absent(root, &mut table, "typecheck_command", "typecheck_command");
        move_table_field_if_absent(root, &mut table, "coverage_threshold", "coverage_threshold");
        move_table_field_if_absent(
            root,
            &mut table,
            "complexity_threshold",
            "complexity_threshold",
        );
        move_table_field_if_absent(
            root,
            &mut table,
            "performance_budget_file",
            "performance_budget_file",
        );
    }

    if let Some(Value::Table(mut rules_table)) = root.remove("rules") {
        merge_bool_rule_toggles(root, &mut rules_table);

        if let Some(Value::Table(mut table)) = rules_table.remove("content") {
            move_table_field_if_absent(root, &mut table, "min_page_size", "min_page_size");
            move_table_field_if_absent(
                root,
                &mut table,
                "required_feature_markers",
                "required_feature_markers",
            );
        }
        if let Some(Value::Table(mut table)) = rules_table.remove("links") {
            move_table_field_if_absent(root, &mut table, "min_inbound_links", "min_inbound_links");
            move_table_field_if_absent(
                root,
                &mut table,
                "link_suggestion_count",
                "link_suggestion_count",
            );
            move_table_field_if_absent(
                root,
                &mut table,
                "enable_link_autofix",
                "enable_link_autofix",
            );
            move_table_field_if_absent(
                root,
                &mut table,
                "related_links_heading",
                "related_links_heading",
            );
            move_table_field_if_absent(root, &mut table, "weak_anchor_text", "weak_anchor_text");
        }
        if let Some(Value::Table(mut table)) = rules_table.remove("schema") {
            move_table_field_if_absent(root, &mut table, "required_types", "required_schema_types");
            move_table_field_if_absent(
                root,
                &mut table,
                "required_families",
                "required_schema_families",
            );
            move_table_field_if_absent(
                root,
                &mut table,
                "require_breadcrumb_schema",
                "require_breadcrumb_schema",
            );
            move_table_field_if_absent(
                root,
                &mut table,
                "require_title_alignment",
                "require_schema_title_alignment",
            );
        }
        if let Some(Value::Table(mut table)) = rules_table.remove("social") {
            move_table_field_if_absent(
                root,
                &mut table,
                "require_open_graph",
                "require_open_graph",
            );
            move_table_field_if_absent(
                root,
                &mut table,
                "require_twitter_card",
                "require_twitter_card",
            );
            move_table_field_if_absent(
                root,
                &mut table,
                "default_twitter_card",
                "default_twitter_card",
            );
            move_table_field_if_absent(
                root,
                &mut table,
                "require_social_images",
                "require_social_images",
            );
            move_table_field_if_absent(
                root,
                &mut table,
                "require_twitter_image",
                "require_twitter_image",
            );
        }
        if let Some(Value::Table(mut table)) = rules_table.remove("robots") {
            move_table_field_if_absent(
                root,
                &mut table,
                "require_sitemap",
                "require_robots_sitemap",
            );
            move_table_field_if_absent(
                root,
                &mut table,
                "require_meta_consistency",
                "require_meta_robots_consistency",
            );
        }
        if let Some(Value::Table(mut table)) = rules_table.remove("html") {
            move_table_field_if_absent(root, &mut table, "require_html_lang", "require_html_lang");
            move_table_field_if_absent(
                root,
                &mut table,
                "require_hreflang_self",
                "require_hreflang_self",
            );
        }
        if let Some(Value::Table(mut table)) = rules_table.remove("structure") {
            move_table_field_if_absent(
                root,
                &mut table,
                "repeatable_data_ui",
                "repeatable_data_ui",
            );
            move_table_field_if_absent(
                root,
                &mut table,
                "utility_route_patterns",
                "utility_route_patterns",
            );
            move_table_field_if_absent(
                root,
                &mut table,
                "min_block_text_length",
                "min_block_text_length",
            );
            move_table_field_if_absent(root, &mut table, "min_answer_blocks", "min_answer_blocks");
            move_table_field_if_absent(
                root,
                &mut table,
                "require_fact_consistency",
                "require_fact_consistency",
            );
        }
    }

    Ok(merged)
}

pub fn load_config(root: &Path, explicit_path: Option<&Path>) -> Result<Config> {
    let config_path = explicit_path
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("seogeo.toml"));
    if !config_path.exists() {
        return Ok(Config::default());
    }
    let merged = normalize_versioned_surface(load_config_value(&config_path, &mut Vec::new())?)?;
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

    #[test]
    fn loads_versioned_nested_surface() {
        let temp_dir = tempfile::tempdir().unwrap();
        fs::write(
            temp_dir.path().join("seogeo.toml"),
            r#"
version = 1

[site]
url = "https://example.com"
source_dir = "dist"
adapter = "astro-dist"

[runtime]
engine = "http"
use_sitemap = false
seeds = ["/docs"]

[policy]
ignore_rules = ["SCH012"]

[policy.severity_overrides]
SEO001 = "warning"

[rules]
html = true
links = false

[rules.schema]
required_types = ["Organization"]
require_title_alignment = false

[output]
baseline_file = "baseline.json"
audit_log_limit = 9

[quality]
coverage_threshold = 90
"#,
        )
        .unwrap();

        let config = load_config(temp_dir.path(), None).unwrap();
        assert_eq!(config.site_url.as_deref(), Some("https://example.com"));
        assert_eq!(config.source_dir, "dist");
        assert_eq!(config.adapter, "astro-dist");
        assert_eq!(config.browser_engine, "http");
        assert!(!config.crawl_use_sitemap);
        assert_eq!(config.crawl_seeds, vec!["/docs".to_string()]);
        assert_eq!(config.ignore_rules, vec!["SCH012".to_string()]);
        assert_eq!(
            config.severity_overrides.get("SEO001").map(String::as_str),
            Some("warning")
        );
        assert_eq!(config.checks.get("links").copied(), Some(false));
        assert_eq!(
            config.required_schema_types,
            vec!["Organization".to_string()]
        );
        assert!(!config.require_schema_title_alignment);
        assert_eq!(config.baseline_file, "baseline.json");
        assert_eq!(config.audit_log_limit, 9);
        assert_eq!(config.coverage_threshold, 90);
    }
}

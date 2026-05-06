use anyhow::{Context, Result, bail};
use serde::Serialize;
use std::fs;
use std::path::{Component, Path, PathBuf};
use toml::Value;

use super::Config;
use crate::plugin::validate_plugin_settings;

const VERSIONED_TOP_LEVEL_TABLES: &[&str] =
    &["site", "runtime", "policy", "rules", "output", "quality"];
// `[accessibility]` lives outside the versioned set because it has
// no flat-key counterpart — there's nothing in old configs for the
// version=1 gate to protect against. Users can adopt A11Y settings
// on any config shape without needing to migrate to versioned form.
const UNVERSIONED_NEW_TABLES: &[&str] = &["accessibility"];
const FLAT_TOP_LEVEL_KEYS: &[&str] = &[
    "site_url",
    "source_dir",
    "profile",
    "adapter",
    "plugins",
    "canonical_style",
    "extends",
    "audit_log_limit",
    "browser_engine",
    "browser_wait_until",
    "baseline_file",
    "max_workers",
    "enable_cache",
    "cache_dir",
    "cache_ttl_seconds",
    "crawl_headers",
    "crawl_cookies",
    "crawl_basic_auth",
    "crawl_seeds",
    "crawl_include_patterns",
    "crawl_exclude_patterns",
    "crawl_use_sitemap",
    "crawl_capture_trace",
    "crawl_capture_screenshot",
    "crawl_capture_console",
    "crawl_capture_network",
    "crawl_artifact_dir",
    "ignore_rules",
    "ignore_paths",
    "severity_overrides",
    "suppressions",
    "checks",
    "orphan_exclude",
    "repeatable_data_ui",
    "utility_route_patterns",
    "route_policy_overrides",
    "min_inbound_links",
    "link_suggestion_count",
    "enable_link_autofix",
    "related_links_heading",
    "min_page_size",
    "required_feature_markers",
    "min_block_text_length",
    "min_answer_blocks",
    "require_fact_consistency",
    "required_schema_types",
    "required_schema_families",
    "require_breadcrumb_schema",
    "require_schema_title_alignment",
    "require_html_lang",
    "require_hreflang_self",
    "require_meta_robots_consistency",
    "require_open_graph",
    "require_twitter_card",
    "default_twitter_card",
    "require_social_images",
    "require_twitter_image",
    "require_robots_sitemap",
    "weak_anchor_text",
    "plugin_settings",
    "typecheck_command",
    "coverage_threshold",
    "complexity_threshold",
    "performance_budget_file",
];
const DEPRECATED_FLAT_KEYS: &[&str] = &[
    "site_url",
    "source_dir",
    "adapter",
    "canonical_style",
    "audit_log_limit",
    "browser_engine",
    "browser_wait_until",
    "baseline_file",
    "crawl_headers",
    "crawl_cookies",
    "crawl_basic_auth",
    "crawl_seeds",
    "crawl_include_patterns",
    "crawl_exclude_patterns",
    "crawl_use_sitemap",
    "crawl_capture_trace",
    "crawl_capture_screenshot",
    "crawl_capture_console",
    "crawl_capture_network",
    "crawl_artifact_dir",
    "ignore_rules",
    "ignore_paths",
    "severity_overrides",
    "suppressions",
    "checks",
    "orphan_exclude",
    "repeatable_data_ui",
    "utility_route_patterns",
    "route_policy_overrides",
    "min_inbound_links",
    "link_suggestion_count",
    "enable_link_autofix",
    "related_links_heading",
    "min_page_size",
    "required_feature_markers",
    "min_block_text_length",
    "min_answer_blocks",
    "require_fact_consistency",
    "required_schema_types",
    "required_schema_families",
    "require_breadcrumb_schema",
    "require_schema_title_alignment",
    "require_html_lang",
    "require_hreflang_self",
    "require_meta_robots_consistency",
    "require_open_graph",
    "require_twitter_card",
    "default_twitter_card",
    "require_social_images",
    "require_twitter_image",
    "require_robots_sitemap",
    "weak_anchor_text",
    "typecheck_command",
    "coverage_threshold",
    "complexity_threshold",
    "performance_budget_file",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConfigWarning {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct LoadedConfig {
    pub config: Config,
    pub warnings: Vec<ConfigWarning>,
}

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
    let checks_value = root
        .entry("checks".to_string())
        .or_insert_with(|| Value::Table(toml::map::Map::new()));
    let Some(checks_table) = checks_value.as_table_mut() else {
        return;
    };
    for key in [
        "html",
        "links",
        "sitemap",
        "robots",
        "social",
        "schema",
        "llm",
        "surfaces",
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

fn validate_allowed_keys(
    table: &toml::map::Map<String, Value>,
    allowed: &[&str],
    context: &str,
) -> Result<()> {
    for key in table.keys() {
        if !allowed.contains(&key.as_str()) {
            bail!("unknown config key '{}' in {}", key, context);
        }
    }
    Ok(())
}

fn expect_table<'a>(
    value: &'a Value,
    key: &str,
    context: &str,
) -> Result<&'a toml::map::Map<String, Value>> {
    value
        .as_table()
        .ok_or_else(|| anyhow::anyhow!("config key '{}' in {} must be a TOML table", key, context))
}

fn validate_rules_table(table: &toml::map::Map<String, Value>) -> Result<()> {
    validate_allowed_keys(
        table,
        &[
            "html",
            "links",
            "sitemap",
            "robots",
            "social",
            "schema",
            "llm",
            "surfaces",
            "content",
            "structure",
        ],
        "[rules]",
    )?;

    for (key, value) in table {
        if matches!(value, Value::Boolean(_)) {
            continue;
        }
        match key.as_str() {
            "content" => validate_allowed_keys(
                expect_table(value, key, "[rules]")?,
                &["enabled", "min_page_size", "required_feature_markers"],
                "[rules.content]",
            )?,
            "links" => validate_allowed_keys(
                expect_table(value, key, "[rules]")?,
                &[
                    "enabled",
                    "min_inbound_links",
                    "link_suggestion_count",
                    "enable_link_autofix",
                    "related_links_heading",
                    "weak_anchor_text",
                ],
                "[rules.links]",
            )?,
            "schema" => validate_allowed_keys(
                expect_table(value, key, "[rules]")?,
                &[
                    "enabled",
                    "required_types",
                    "required_families",
                    "require_breadcrumb_schema",
                    "require_title_alignment",
                ],
                "[rules.schema]",
            )?,
            "social" => validate_allowed_keys(
                expect_table(value, key, "[rules]")?,
                &[
                    "enabled",
                    "require_open_graph",
                    "require_twitter_card",
                    "default_twitter_card",
                    "require_social_images",
                    "require_twitter_image",
                ],
                "[rules.social]",
            )?,
            "robots" => validate_allowed_keys(
                expect_table(value, key, "[rules]")?,
                &["enabled", "require_sitemap", "require_meta_consistency"],
                "[rules.robots]",
            )?,
            "html" => validate_allowed_keys(
                expect_table(value, key, "[rules]")?,
                &["enabled", "require_html_lang", "require_hreflang_self"],
                "[rules.html]",
            )?,
            "structure" => validate_allowed_keys(
                expect_table(value, key, "[rules]")?,
                &[
                    "enabled",
                    "repeatable_data_ui",
                    "utility_route_patterns",
                    "min_block_text_length",
                    "min_answer_blocks",
                    "require_fact_consistency",
                ],
                "[rules.structure]",
            )?,
            "sitemap" | "llm" | "surfaces" => validate_allowed_keys(
                expect_table(value, key, "[rules]")?,
                &["enabled"],
                &format!("[rules.{}]", key),
            )?,
            _ => bail!(
                "config key '{}' in [rules] must be a boolean or nested TOML table",
                key
            ),
        }
    }

    Ok(())
}

fn validate_versioned_sections(root: &toml::map::Map<String, Value>) -> Result<()> {
    if let Some(value) = root.get("site") {
        validate_allowed_keys(
            expect_table(value, "site", "config root")?,
            &["url", "source_dir", "adapter", "canonical_style"],
            "[site]",
        )?;
    }
    if let Some(value) = root.get("runtime") {
        validate_allowed_keys(
            expect_table(value, "runtime", "config root")?,
            &[
                "engine",
                "wait_until",
                "headers",
                "cookies",
                "basic_auth",
                "seeds",
                "include_patterns",
                "exclude_patterns",
                "use_sitemap",
                "capture_trace",
                "capture_screenshot",
                "capture_console",
                "capture_network",
                "artifact_dir",
            ],
            "[runtime]",
        )?;
    }
    if let Some(value) = root.get("policy") {
        validate_allowed_keys(
            expect_table(value, "policy", "config root")?,
            &[
                "ignore_rules",
                "ignore_paths",
                "severity_overrides",
                "suppressions",
                "route_policy_overrides",
            ],
            "[policy]",
        )?;
    }
    if let Some(value) = root.get("rules") {
        validate_rules_table(expect_table(value, "rules", "config root")?)?;
    }
    if let Some(value) = root.get("output") {
        validate_allowed_keys(
            expect_table(value, "output", "config root")?,
            &["baseline_file", "audit_log_limit"],
            "[output]",
        )?;
    }
    if let Some(value) = root.get("quality") {
        validate_allowed_keys(
            expect_table(value, "quality", "config root")?,
            &[
                "typecheck_command",
                "coverage_threshold",
                "complexity_threshold",
                "performance_budget_file",
            ],
            "[quality]",
        )?;
    }
    if let Some(value) = root.get("accessibility") {
        validate_allowed_keys(
            expect_table(value, "accessibility", "config root")?,
            &["strict"],
            "[accessibility]",
        )?;
    }
    Ok(())
}

fn validate_config_surface(value: &Value) -> Result<()> {
    let root = value
        .as_table()
        .ok_or_else(|| anyhow::anyhow!("config root must be a TOML table"))?;
    let version = match root.get("version") {
        None => None,
        Some(Value::Integer(version)) => Some(*version),
        Some(_) => bail!("config version must be an integer"),
    };
    let has_versioned_tables = VERSIONED_TOP_LEVEL_TABLES
        .iter()
        .any(|key| root.contains_key(*key));

    match version {
        Some(1) => {
            let mut allowed_keys = FLAT_TOP_LEVEL_KEYS.to_vec();
            allowed_keys.extend(VERSIONED_TOP_LEVEL_TABLES);
            allowed_keys.extend(UNVERSIONED_NEW_TABLES);
            allowed_keys.push("version");
            validate_allowed_keys(root, &allowed_keys, "config root")?;
            validate_versioned_sections(root)?;
        }
        Some(other) => bail!("unsupported config version {}; expected 1", other),
        None => {
            if has_versioned_tables {
                bail!("nested config tables require `version = 1`");
            }
            let mut allowed_keys = FLAT_TOP_LEVEL_KEYS.to_vec();
            allowed_keys.extend(UNVERSIONED_NEW_TABLES);
            validate_allowed_keys(root, &allowed_keys, "config root")?;
            validate_versioned_sections(root)?;
        }
    }

    Ok(())
}

fn deprecation_warning(message: impl Into<String>) -> ConfigWarning {
    ConfigWarning {
        code: "CFGDEP001".to_string(),
        message: message.into(),
    }
}

fn collect_deprecation_warnings(value: &Value) -> Vec<ConfigWarning> {
    let Some(root) = value.as_table() else {
        return Vec::new();
    };
    let mut warnings = Vec::new();

    for key in DEPRECATED_FLAT_KEYS {
        if root.contains_key(*key) {
            warnings.push(deprecation_warning(format!(
                "legacy flat config key '{}' is deprecated; prefer the versioned nested surface with `version = 1`",
                key
            )));
        }
    }

    if let Some(Value::Table(rules)) = root.get("rules") {
        for (rule_name, rule_value) in rules {
            if matches!(rule_value, Value::Boolean(_)) {
                warnings.push(deprecation_warning(format!(
                    "legacy rule toggle `rules.{}` is deprecated; prefer `[rules.{}] enabled = ...`",
                    rule_name, rule_name
                )));
            }
        }
    }

    warnings
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
            move_table_field_if_absent_checks(root, &mut table, "content");
            move_table_field_if_absent(root, &mut table, "min_page_size", "min_page_size");
            move_table_field_if_absent(
                root,
                &mut table,
                "required_feature_markers",
                "required_feature_markers",
            );
        }
        if let Some(Value::Table(mut table)) = rules_table.remove("links") {
            move_table_field_if_absent_checks(root, &mut table, "links");
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
            move_table_field_if_absent_checks(root, &mut table, "schema");
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
            move_table_field_if_absent_checks(root, &mut table, "social");
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
            move_table_field_if_absent_checks(root, &mut table, "robots");
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
            move_table_field_if_absent_checks(root, &mut table, "html");
            move_table_field_if_absent(root, &mut table, "require_html_lang", "require_html_lang");
            move_table_field_if_absent(
                root,
                &mut table,
                "require_hreflang_self",
                "require_hreflang_self",
            );
        }
        if let Some(Value::Table(mut table)) = rules_table.remove("structure") {
            move_table_field_if_absent_checks(root, &mut table, "structure");
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
        for key in ["sitemap", "llm", "surfaces"] {
            if let Some(Value::Table(mut table)) = rules_table.remove(key) {
                move_table_field_if_absent_checks(root, &mut table, key);
            }
        }
    }

    Ok(merged)
}

fn move_table_field_if_absent_checks(
    root: &mut toml::map::Map<String, Value>,
    table: &mut toml::map::Map<String, Value>,
    rule_name: &str,
) {
    let Some(value) = table.remove("enabled") else {
        return;
    };
    let checks_value = root
        .entry("checks".to_string())
        .or_insert_with(|| Value::Table(toml::map::Map::new()));
    let Some(checks_table) = checks_value.as_table_mut() else {
        return;
    };
    checks_table.entry(rule_name.to_string()).or_insert(value);
}

pub fn load_config_with_diagnostics(
    root: &Path,
    explicit_path: Option<&Path>,
) -> Result<LoadedConfig> {
    let config_path = explicit_path
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("aexeo.toml"));
    if !config_path.exists() {
        return Ok(LoadedConfig {
            config: Config::default(),
            warnings: Vec::new(),
        });
    }
    let merged = load_config_value(&config_path, &mut Vec::new())?;
    validate_config_surface(&merged)?;
    let warnings = collect_deprecation_warnings(&merged);
    let merged = normalize_versioned_surface(merged)?;
    let config = merged.try_into::<Config>().with_context(|| {
        format!(
            "failed to deserialize merged config at {}",
            config_path.display()
        )
    })?;
    validate_plugin_settings(&config.plugins, &config.plugin_settings)?;
    Ok(LoadedConfig { config, warnings })
}

pub fn load_config(root: &Path, explicit_path: Option<&Path>) -> Result<Config> {
    Ok(load_config_with_diagnostics(root, explicit_path)?.config)
}

#[cfg(test)]
mod tests {
    use super::{load_config, load_config_with_diagnostics};
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
            temp_dir.path().join("aexeo.toml"),
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
            temp_dir.path().join("aexeo.toml"),
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
            temp_dir.path().join("aexeo.toml"),
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

[rules.html]
enabled = true

[rules.links]
enabled = false

[rules.schema]
enabled = true
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

    #[test]
    fn rejects_unsupported_config_version() {
        let temp_dir = tempfile::tempdir().unwrap();
        fs::write(temp_dir.path().join("aexeo.toml"), "version = 2\n").unwrap();

        let error =
            load_config(temp_dir.path(), None).expect_err("unsupported version should fail");
        assert!(error.to_string().contains("unsupported config version"));
    }

    #[test]
    fn rejects_nested_surface_without_version() {
        let temp_dir = tempfile::tempdir().unwrap();
        fs::write(
            temp_dir.path().join("aexeo.toml"),
            r#"
[site]
url = "https://example.com"
"#,
        )
        .unwrap();

        let error = load_config(temp_dir.path(), None)
            .expect_err("nested config without version should fail");
        assert!(
            error
                .to_string()
                .contains("nested config tables require `version = 1`")
        );
    }

    #[test]
    fn rejects_unknown_top_level_key() {
        let temp_dir = tempfile::tempdir().unwrap();
        fs::write(
            temp_dir.path().join("aexeo.toml"),
            r#"
site_url = "https://example.com"
unknown_key = true
"#,
        )
        .unwrap();

        let error = load_config(temp_dir.path(), None).expect_err("unknown keys should fail");
        assert!(
            error
                .to_string()
                .contains("unknown config key 'unknown_key'")
        );
    }

    #[test]
    fn rejects_unknown_nested_key() {
        let temp_dir = tempfile::tempdir().unwrap();
        fs::write(
            temp_dir.path().join("aexeo.toml"),
            r#"
version = 1

[rules.links]
unexpected = true
"#,
        )
        .unwrap();

        let error =
            load_config(temp_dir.path(), None).expect_err("unknown nested keys should fail");
        assert!(
            error
                .to_string()
                .contains("unknown config key 'unexpected' in [rules.links]")
        );
    }

    #[test]
    fn reports_flat_key_deprecation_warnings() {
        let temp_dir = tempfile::tempdir().unwrap();
        fs::write(
            temp_dir.path().join("aexeo.toml"),
            r#"
site_url = "https://example.com"
browser_engine = "http"
"#,
        )
        .unwrap();

        let loaded = load_config_with_diagnostics(temp_dir.path(), None).unwrap();
        let messages = loaded
            .warnings
            .iter()
            .map(|warning| warning.message.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            loaded
                .warnings
                .iter()
                .filter(|warning| warning.code == "CFGDEP001")
                .count(),
            2
        );
        assert!(messages.iter().any(|message| message.contains("site_url")));
        assert!(
            messages
                .iter()
                .any(|message| message.contains("browser_engine"))
        );
    }

    #[test]
    fn reports_legacy_rule_toggle_deprecation_warnings() {
        let temp_dir = tempfile::tempdir().unwrap();
        fs::write(
            temp_dir.path().join("aexeo.toml"),
            r#"
version = 1

[rules]
html = true
"#,
        )
        .unwrap();

        let loaded = load_config_with_diagnostics(temp_dir.path(), None).unwrap();
        assert!(loaded.warnings.iter().any(|warning| {
            warning.code == "CFGDEP001" && warning.message.contains("rules.html")
        }));
    }

    #[test]
    fn rejects_plugin_settings_for_undeclared_plugin() {
        let temp_dir = tempfile::tempdir().unwrap();
        fs::write(
            temp_dir.path().join("aexeo.toml"),
            r#"
version = 1

[plugin_settings."example.plugin"]
enabled = true
"#,
        )
        .unwrap();

        let error =
            load_config(temp_dir.path(), None).expect_err("undeclared plugin settings should fail");
        assert!(
            error
                .to_string()
                .contains("plugin_settings.example.plugin requires `plugins`")
        );
    }

    #[test]
    fn accessibility_section_defaults_to_smart_mode() {
        let temp_dir = tempfile::tempdir().unwrap();
        let config = load_config(temp_dir.path(), None).unwrap();
        assert!(
            !config.accessibility.strict,
            "default accessibility mode is smart (skips canonically decorative images)"
        );
    }

    #[test]
    fn loads_accessibility_strict_from_nested_toml() {
        let temp_dir = tempfile::tempdir().unwrap();
        fs::write(
            temp_dir.path().join("aexeo.toml"),
            r#"
[accessibility]
strict = true
"#,
        )
        .unwrap();
        let config = load_config(temp_dir.path(), None).unwrap();
        assert!(
            config.accessibility.strict,
            "[accessibility].strict = true should set Config.accessibility.strict"
        );
    }

    #[test]
    fn rejects_plugin_settings_without_registered_schema() {
        let temp_dir = tempfile::tempdir().unwrap();
        fs::write(
            temp_dir.path().join("aexeo.toml"),
            r#"
version = 1
plugins = ["example.plugin"]

[plugin_settings."example.plugin"]
enabled = true
"#,
        )
        .unwrap();

        let error =
            load_config(temp_dir.path(), None).expect_err("unregistered plugin schema should fail");
        assert!(
            error
                .to_string()
                .contains("does not publish a registered settings schema")
        );
    }
}

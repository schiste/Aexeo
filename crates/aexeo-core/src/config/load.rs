use anyhow::{Context, Result, bail};
use serde::Serialize;
use std::fs;
use std::path::{Component, Path, PathBuf};
use toml::Value;

use super::Config;
use super::docs::versioned_location;
use crate::error::into_user_error;
use crate::plugin::validate_plugin_settings;

const VERSIONED_TOP_LEVEL_TABLES: &[&str] =
    &["site", "runtime", "policy", "rules", "output", "quality"];
const OTHER_TOP_LEVEL_TABLES: &[&str] = &[
    "accessibility",
    "agent_discovery",
    "editorial",
    "images",
    "route_kinds",
];
/// Settings that belong to the config as a whole, not to one section.
const TOP_LEVEL_KEYS: &[&str] = &[
    "profile",
    "plugins",
    "extends",
    "max_workers",
    "plugin_settings",
];
/// Rule groups that only have an on/off switch under `[rules.<group>]`.
const ENABLE_ONLY_RULE_GROUPS: &[&str] = &[
    "sitemap",
    "llm",
    "surfaces",
    "editorial",
    "headers",
    "well_known",
    "accessibility",
    "agent_discovery",
    "images",
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
            "images",
            "editorial",
            "structure",
            "headers",
            "well_known",
            "accessibility",
            "agent_discovery",
        ],
        "[rules]",
    )?;

    for (key, value) in table {
        if matches!(value, Value::Boolean(_)) {
            bail!("`rules.{key}` must be a table: use `[rules.{key}]` with `enabled = ...`");
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
                    "orphan_exclude",
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
            group if ENABLE_ONLY_RULE_GROUPS.contains(&group) => validate_allowed_keys(
                expect_table(value, key, "[rules]")?,
                &["enabled"],
                &format!("[rules.{}]", key),
            )?,
            _ => bail!(
                "config key '{}' in [rules] must be a nested TOML table",
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
                "ignore_robots",
                "crawl_delay_ms",
                "capture_trace",
                "capture_screenshot",
                "capture_console",
                "capture_network",
                "artifact_dir",
                "allow_private_network",
                "max_response_bytes",
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
            &["performance_budget_file"],
            "[quality]",
        )?;
    }
    if let Some(value) = root.get("images") {
        validate_allowed_keys(
            expect_table(value, "images", "config root")?,
            &[
                "max_bytes",
                "max_alt_characters",
                "prefer_modern_formats",
                "require_dimensions",
                "require_image_sitemap",
                "require_image_object_metadata",
                "recommend_figure_captions",
            ],
            "[images]",
        )?;
    }
    if let Some(value) = root.get("accessibility") {
        validate_allowed_keys(
            expect_table(value, "accessibility", "config root")?,
            &["strict"],
            "[accessibility]",
        )?;
    }
    if let Some(value) = root.get("agent_discovery") {
        validate_allowed_keys(
            expect_table(value, "agent_discovery", "config root")?,
            &["enabled"],
            "[agent_discovery]",
        )?;
    }
    if let Some(value) = root.get("route_kinds") {
        for (name, kind) in expect_table(value, "route_kinds", "config root")? {
            validate_allowed_keys(
                expect_table(kind, name, "[route_kinds]")?,
                &["match", "skip_rules", "noindex"],
                "[route_kinds.<name>]",
            )?;
        }
    }
    if let Some(value) = root.get("editorial") {
        let editorial = expect_table(value, "editorial", "config root")?;
        validate_allowed_keys(editorial, &["routes"], "[editorial]")?;
        if let Some(routes_value) = editorial.get("routes") {
            let routes = expect_table(routes_value, "routes", "[editorial]")?;
            for (route, brief_value) in routes {
                let brief = expect_table(brief_value, route, "[editorial.routes]")?;
                validate_allowed_keys(
                    brief,
                    &[
                        "answer_summary_id",
                        "target_questions",
                        "claims_requiring_evidence",
                    ],
                    "[editorial.routes.<route>]",
                )?;
                if let Some(questions_value) = brief.get("target_questions") {
                    let questions = questions_value.as_array().ok_or_else(|| {
                        anyhow::anyhow!(
                            "config key 'target_questions' in [editorial.routes.{}] must be an array",
                            route
                        )
                    })?;
                    for question in questions {
                        let question = question.as_table().ok_or_else(|| {
                            anyhow::anyhow!(
                                "target_questions entries in [editorial.routes.{}] must be tables",
                                route
                            )
                        })?;
                        validate_allowed_keys(
                            question,
                            &["heading_id", "question"],
                            "[editorial.routes.<route>.target_questions]",
                        )?;
                    }
                }
            }
        }
    }
    Ok(())
}

/// Names the new home of a key from the removed flat format, so an old config
/// fails with the fix instead of a bare "unknown key".
fn reject_removed_keys(root: &toml::map::Map<String, Value>) -> Result<()> {
    for key in root.keys() {
        if key == "checks" {
            bail!("`[checks]` was removed: use `[rules.<group>]` with `enabled = ...`");
        }
        if let Some([table @ .., name]) = versioned_location(key) {
            bail!(
                "config key '{key}' was removed: use `{name}` under `[{}]`",
                table.join(".")
            );
        }
    }
    Ok(())
}

fn validate_config_surface(value: &Value) -> Result<()> {
    let root = value
        .as_table()
        .ok_or_else(|| anyhow::anyhow!("config root must be a TOML table"))?;
    reject_removed_keys(root)?;
    match root.get("version") {
        // An empty file is the same as no file: every default applies.
        None if root.is_empty() => return Ok(()),
        None => bail!("config must declare `version = 1`"),
        Some(Value::Integer(1)) => {}
        Some(Value::Integer(other)) => bail!("unsupported config version {other}; expected 1"),
        Some(_) => bail!("config version must be an integer"),
    }
    validate_allowed_keys(root, &accepted_root_keys(), "config root")?;
    validate_versioned_sections(root)
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
        move_table_field_if_absent(root, &mut table, "ignore_robots", "crawl_ignore_robots");
        move_table_field_if_absent(root, &mut table, "crawl_delay_ms", "crawl_delay_ms");
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
        move_table_field_if_absent(
            root,
            &mut table,
            "allow_private_network",
            "crawl_allow_private_network",
        );
        move_table_field_if_absent(
            root,
            &mut table,
            "max_response_bytes",
            "crawl_max_response_bytes",
        );
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
        move_table_field_if_absent(
            root,
            &mut table,
            "performance_budget_file",
            "performance_budget_file",
        );
    }

    if let Some(Value::Table(mut rules_table)) = root.remove("rules") {
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
            move_table_field_if_absent(root, &mut table, "orphan_exclude", "orphan_exclude");
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
        for key in ENABLE_ONLY_RULE_GROUPS {
            if let Some(Value::Table(mut table)) = rules_table.remove(*key) {
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
    read_config_with_diagnostics(root, explicit_path).map_err(into_user_error)
}

fn read_config_with_diagnostics(root: &Path, explicit_path: Option<&Path>) -> Result<LoadedConfig> {
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
    config_from_merged(merged, &config_path.display().to_string())
}

/// Validates and resolves a config whose `extends` have already been merged.
pub(super) fn config_from_merged(merged: Value, source: &str) -> Result<LoadedConfig> {
    validate_config_surface(&merged)?;
    let merged = normalize_versioned_surface(merged)?;
    let config = merged
        .try_into::<Config>()
        .with_context(|| format!("failed to deserialize merged config at {source}"))?;
    validate_plugin_settings(&config.plugins, &config.plugin_settings)?;
    Ok(LoadedConfig {
        config,
        warnings: Vec::new(),
    })
}

/// Every key accepted at the root of a `version = 1` config.
pub(super) fn accepted_root_keys() -> Vec<&'static str> {
    let mut keys = TOP_LEVEL_KEYS.to_vec();
    keys.extend(VERSIONED_TOP_LEVEL_TABLES);
    keys.extend(OTHER_TOP_LEVEL_TABLES);
    keys.push("version");
    keys
}

pub fn load_config(root: &Path, explicit_path: Option<&Path>) -> Result<Config> {
    Ok(load_config_with_diagnostics(root, explicit_path)?.config)
}

#[cfg(test)]
mod tests {
    use super::load_config;
    use crate::config::default_rule_switches;
    use std::fs;

    #[test]
    fn loads_route_kinds_from_a_real_config_file() {
        let temp_dir = tempfile::tempdir().unwrap();
        fs::write(
            temp_dir.path().join("aexeo.toml"),
            "version = 1\n[route_kinds.manifesto]\nmatch = [\"/foundations/\"]\nskip_rules = [\"GEO007\"]\nnoindex = true\n",
        )
        .unwrap();
        let config = load_config(temp_dir.path(), None).unwrap();
        let kind = &config.route_kinds["manifesto"];
        assert_eq!(kind.r#match, ["/foundations/"]);
        assert_eq!(kind.skip_rules, ["GEO007"]);
        assert!(kind.noindex);
    }

    #[test]
    fn rejects_unknown_keys_inside_a_route_kind() {
        let temp_dir = tempfile::tempdir().unwrap();
        fs::write(
            temp_dir.path().join("aexeo.toml"),
            "version = 1\n[route_kinds.manifesto]\nskip = [\"GEO007\"]\n",
        )
        .unwrap();
        let error = load_config(temp_dir.path(), None).unwrap_err();
        assert!(format!("{error:#}").contains("unknown config key 'skip' in [route_kinds.<name>]"));
    }

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
    fn loads_image_quality_policy_options() {
        let temp_dir = tempfile::tempdir().unwrap();
        fs::write(
            temp_dir.path().join("aexeo.toml"),
            r#"
version = 1
[images]
max_bytes = 140000
max_alt_characters = 150
prefer_modern_formats = false
require_dimensions = false
require_image_sitemap = true
require_image_object_metadata = true
recommend_figure_captions = false
"#,
        )
        .unwrap();

        let config = load_config(temp_dir.path(), None).unwrap();

        assert_eq!(config.images.max_bytes, 140_000);
        assert_eq!(config.images.max_alt_characters, 150);
        assert!(!config.images.prefer_modern_formats);
        assert!(!config.images.require_dimensions);
        assert!(config.images.require_image_sitemap);
        assert!(config.images.require_image_object_metadata);
        assert!(!config.images.recommend_figure_captions);
    }

    #[test]
    fn loads_simple_toml_config() {
        let temp_dir = tempfile::tempdir().unwrap();
        fs::write(
            temp_dir.path().join("aexeo.toml"),
            r#"
version = 1
profile = "chau7"
[site]
url = "https://example.com"
[output]
audit_log_limit = 9
[policy.severity_overrides]
SEO001 = "warning"
[rules.html]
enabled = true
[rules.links]
enabled = false
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
version = 1
profile = "chau7"
[policy.severity_overrides]
SEO001 = "warning"
[rules.html]
enabled = false
[rules.links]
enabled = true
"#,
        )
        .unwrap();
        fs::write(
            temp_dir.path().join("aexeo.toml"),
            r#"
version = 1
extends = ["base.toml"]
[site]
url = "https://example.com"
[policy.severity_overrides]
SEO002 = "warning"
[rules.links]
enabled = false
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
allow_private_network = true
max_response_bytes = 1024

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
performance_budget_file = "custom-budget.json"
"#,
        )
        .unwrap();

        let config = load_config(temp_dir.path(), None).unwrap();
        assert_eq!(config.site_url.as_deref(), Some("https://example.com"));
        assert_eq!(config.source_dir, "dist");
        assert_eq!(config.adapter, "astro-dist");
        assert_eq!(config.browser_engine, "http");
        assert!(!config.crawl_use_sitemap);
        assert!(config.crawl_allow_private_network);
        assert_eq!(config.crawl_max_response_bytes, 1024);
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
        assert_eq!(config.performance_budget_file, "custom-budget.json");
    }

    #[test]
    fn loads_versioned_images_rule_group_toggle() {
        let temp_dir = tempfile::tempdir().unwrap();
        fs::write(
            temp_dir.path().join("aexeo.toml"),
            r#"
version = 1

[rules.images]
enabled = false
"#,
        )
        .unwrap();

        let config = load_config(temp_dir.path(), None).unwrap();

        assert_eq!(config.checks.get("images").copied(), Some(false));
    }

    #[test]
    fn rejects_the_boolean_images_rule_group_shorthand() {
        let temp_dir = tempfile::tempdir().unwrap();
        fs::write(
            temp_dir.path().join("aexeo.toml"),
            r#"
version = 1

[rules]
images = false
"#,
        )
        .unwrap();

        let error = load_config(temp_dir.path(), None).unwrap_err();

        assert!(
            format!("{error:#}").contains("use `[rules.images]` with `enabled = ...`"),
            "{error:#}"
        );
    }

    #[test]
    fn rejects_unknown_image_settings() {
        let temp_dir = tempfile::tempdir().unwrap();
        fs::write(
            temp_dir.path().join("aexeo.toml"),
            "version = 1\n[images]\nmax_kilobytes = 10\n",
        )
        .unwrap();

        let error = load_config(temp_dir.path(), None).unwrap_err();

        assert!(
            format!("{error:#}").contains("unknown config key 'max_kilobytes' in [images]"),
            "{error:#}"
        );
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

        let error =
            load_config(temp_dir.path(), None).expect_err("config without version should fail");
        assert!(error.to_string().contains("must declare `version = 1`"));
    }

    #[test]
    fn rejects_unknown_top_level_key() {
        let temp_dir = tempfile::tempdir().unwrap();
        fs::write(
            temp_dir.path().join("aexeo.toml"),
            r#"
version = 1
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

    fn load_error(text: &str) -> String {
        let temp_dir = tempfile::tempdir().unwrap();
        fs::write(temp_dir.path().join("aexeo.toml"), text).unwrap();
        load_config(temp_dir.path(), None)
            .expect_err("config should be rejected")
            .to_string()
    }

    #[test]
    fn a_removed_flat_key_fails_and_names_its_new_location() {
        for (text, expected) in [
            (
                "site_url = \"https://example.com\"\n",
                "use `url` under `[site]`",
            ),
            ("crawl_headers = {}\n", "use `headers` under `[runtime]`"),
            (
                "orphan_exclude = []\n",
                "use `orphan_exclude` under `[rules.links]`",
            ),
            ("[checks]\nhtml = false\n", "use `[rules.<group>]`"),
            ("version = 1\n[rules]\nhtml = true\n", "use `[rules.html]`"),
        ] {
            let message = load_error(text);
            assert!(message.contains(expected), "{text}: {message}");
        }
    }

    #[test]
    fn a_config_without_a_version_is_rejected_but_an_empty_one_is_not() {
        assert!(load_error("profile = \"chau7\"\n").contains("must declare `version = 1`"));

        let temp_dir = tempfile::tempdir().unwrap();
        fs::write(temp_dir.path().join("aexeo.toml"), "").unwrap();
        load_config(temp_dir.path(), None).unwrap();
    }

    #[test]
    fn a_child_config_inherits_the_version_of_its_parent() {
        let temp_dir = tempfile::tempdir().unwrap();
        fs::write(temp_dir.path().join("base.toml"), "version = 1\n").unwrap();
        fs::write(
            temp_dir.path().join("aexeo.toml"),
            "extends = [\"base.toml\"]\nprofile = \"chau7\"\n",
        )
        .unwrap();
        assert_eq!(load_config(temp_dir.path(), None).unwrap().profile, "chau7");
    }

    /// `orphan_exclude` and these groups had no nested form before the flat format
    /// was removed.
    #[test]
    fn every_rule_group_and_orphan_exclude_have_a_nested_form() {
        let temp_dir = tempfile::tempdir().unwrap();
        fs::write(
            temp_dir.path().join("aexeo.toml"),
            r#"
version = 1
[rules.links]
orphan_exclude = ["offline.html"]
[rules.headers]
enabled = false
[rules.well_known]
enabled = false
[rules.accessibility]
enabled = false
[rules.agent_discovery]
enabled = false
"#,
        )
        .unwrap();
        let config = load_config(temp_dir.path(), None).unwrap();
        assert_eq!(config.orphan_exclude, ["offline.html"]);
        for group in ["headers", "well_known", "accessibility", "agent_discovery"] {
            assert_eq!(config.checks.get(group), Some(&false), "{group}");
        }
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
version = 1
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

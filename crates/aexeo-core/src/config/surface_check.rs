//! Keeps the three descriptions of the config surface in agreement: the loader,
//! the generated JSON Schema, and the field docs. The two were written by hand
//! and drifted apart in both directions, so `docs check` runs this.

use anyhow::{Result, bail};
use serde_json::Value;
use std::collections::BTreeSet;

use super::docs::{config_field_docs, render_config_schema, versioned_location};
use super::load::{accepted_root_keys, config_from_merged};

pub const COMPLETE_EXAMPLE: &str = include_str!("complete_example.toml");

/// Documented, but loading it is refused until a plugin registers a schema.
const RESERVED_KEYS: &[&str] = &["plugin_settings"];

/// Fails when the complete example stops passing the schema or the loader, when
/// it omits a documented key, or when the schema and loader accept different
/// root keys.
pub fn check_config_surface() -> Result<()> {
    let schema: Value = serde_json::from_str(&render_config_schema()?)?;
    let example: toml::Value = toml::from_str(COMPLETE_EXAMPLE)?;
    let mut problems = schema_errors(&serde_json::to_value(&example)?, &schema, "$");

    if let Err(error) = config_from_merged(example.clone(), "the complete config example") {
        problems.push(format!("the loader rejects the example: {error:#}"));
    }

    for field in config_field_docs() {
        if !RESERVED_KEYS.contains(&field.key) && !example_sets(&example, field.key) {
            problems.push(format!(
                "documented key '{}' is missing from the complete example",
                field.key
            ));
        }
    }

    let schema_keys: BTreeSet<&str> = schema["properties"]
        .as_object()
        .map(|properties| properties.keys().map(String::as_str).collect())
        .unwrap_or_default();
    let loader_keys: BTreeSet<&str> = accepted_root_keys().into_iter().collect();
    for key in schema_keys.difference(&loader_keys) {
        problems.push(format!(
            "root key '{key}' is in the schema but the loader rejects it"
        ));
    }
    for key in loader_keys.difference(&schema_keys) {
        problems.push(format!(
            "root key '{key}' loads but is missing from the schema"
        ));
    }

    if problems.is_empty() {
        return Ok(());
    }
    bail!("config surface drift:\n  {}", problems.join("\n  "))
}

fn example_sets(example: &toml::Value, key: &str) -> bool {
    let mut current = example;
    let path = versioned_location(key).unwrap_or(&[]);
    let segments = if path.is_empty() { &[key][..] } else { path };
    for segment in segments {
        match current.get(segment) {
            Some(next) => current = next,
            None => return false,
        }
    }
    true
}

/// Validates against the JSON Schema keywords the generated schema uses. An
/// unsupported keyword is reported, so the schema cannot outgrow this silently.
fn schema_errors(value: &Value, schema: &Value, path: &str) -> Vec<String> {
    let Some(keywords) = schema.as_object() else {
        return vec![format!("{path}: schema is not an object")];
    };
    let matches = |candidate: &Value| schema_errors(value, candidate, path).is_empty();
    let mut errors = Vec::new();
    for (keyword, argument) in keywords {
        match keyword.as_str() {
            "$schema" | "title" | "description" | "then" => {}
            "type" if !has_type(value, argument) => {
                errors.push(format!("{path}: expected type {argument}"));
            }
            "type" => {}
            "const" if value != argument => {
                errors.push(format!("{path}: expected constant {argument}"));
            }
            "const" => {}
            "minimum" => {
                if let (Some(actual), Some(minimum)) = (value.as_f64(), argument.as_f64())
                    && actual < minimum
                {
                    errors.push(format!("{path}: below minimum {minimum}"));
                }
            }
            "maxProperties" => {
                if let (Some(object), Some(maximum)) = (value.as_object(), argument.as_u64())
                    && object.len() as u64 > maximum
                {
                    errors.push(format!("{path}: more than {maximum} properties"));
                }
            }
            "required" => {
                for name in argument.as_array().into_iter().flatten() {
                    let name = name.as_str().unwrap_or_default();
                    if value.is_object() && value.get(name).is_none() {
                        errors.push(format!("{path}: missing required key '{name}'"));
                    }
                }
            }
            "properties" | "additionalProperties" => {
                errors.extend(object_errors(value, keywords, keyword, path));
            }
            "items" => {
                for (index, item) in value.as_array().into_iter().flatten().enumerate() {
                    errors.extend(schema_errors(item, argument, &format!("{path}[{index}]")));
                }
            }
            "oneOf" => {
                let passing = argument
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|s| matches(s));
                if passing.count() != 1 {
                    errors.push(format!("{path}: does not match exactly one allowed shape"));
                }
            }
            "anyOf" => {
                if !argument.as_array().into_iter().flatten().any(matches) {
                    errors.push(format!("{path}: matches no allowed shape"));
                }
            }
            "allOf" => {
                for candidate in argument.as_array().into_iter().flatten() {
                    errors.extend(schema_errors(value, candidate, path));
                }
            }
            "if" => {
                if let Some(then) = keywords.get("then")
                    && matches(argument)
                {
                    errors.extend(schema_errors(value, then, path));
                }
            }
            other => errors.push(format!("{path}: unsupported schema keyword '{other}'")),
        }
    }
    errors
}

/// `properties` and `additionalProperties` are evaluated together, once, under
/// whichever of the two keywords comes first.
fn object_errors(
    value: &Value,
    keywords: &serde_json::Map<String, Value>,
    keyword: &str,
    path: &str,
) -> Vec<String> {
    let properties = keywords.get("properties").and_then(Value::as_object);
    if keyword == "properties" && keywords.contains_key("additionalProperties") {
        return Vec::new();
    }
    let additional = keywords.get("additionalProperties");
    let mut errors = Vec::new();
    for (key, item) in value.as_object().into_iter().flatten() {
        let child = format!("{path}.{key}");
        match (properties.and_then(|known| known.get(key)), additional) {
            (Some(schema), _) => errors.extend(schema_errors(item, schema, &child)),
            (None, Some(Value::Bool(false))) => errors.push(format!("{child}: unknown key")),
            (None, Some(schema)) if schema.is_object() => {
                errors.extend(schema_errors(item, schema, &child));
            }
            (None, _) => {}
        }
    }
    errors
}

fn has_type(value: &Value, expected: &Value) -> bool {
    if let Some(alternatives) = expected.as_array() {
        return alternatives.iter().any(|one| has_type(value, one));
    }
    match expected.as_str() {
        Some("object") => value.is_object(),
        Some("array") => value.is_array(),
        Some("string") => value.is_string(),
        Some("boolean") => value.is_boolean(),
        Some("integer") => value.is_i64() || value.is_u64(),
        Some("number") => value.is_number(),
        Some("null") => value.is_null(),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use serde_json::json;

    fn schema() -> Value {
        serde_json::from_str(&render_config_schema().unwrap()).unwrap()
    }

    fn example() -> toml::Value {
        toml::from_str(COMPLETE_EXAMPLE).unwrap()
    }

    fn remove_path(root: &mut toml::Value, path: &[&str]) {
        let (last, parents) = path.split_last().unwrap();
        let mut current = root;
        for parent in parents {
            current = current.get_mut(parent).unwrap();
        }
        assert!(
            current.as_table_mut().unwrap().remove(*last).is_some(),
            "{path:?} is not in the complete example"
        );
    }

    #[test]
    fn the_config_surface_agrees_with_itself() {
        check_config_surface().unwrap();
    }

    /// A key that loads but is ignored would still pass the schema and loader.
    /// Removing each key has to change the resulting config.
    #[test]
    fn every_documented_key_in_the_example_takes_effect() {
        let baseline = config_from_merged(example(), "example").unwrap().config;
        for field in config_field_docs() {
            if RESERVED_KEYS.contains(&field.key) {
                continue;
            }
            let mut value = example();
            let path = versioned_location(field.key).unwrap_or(&[]);
            let top_level = [field.key];
            remove_path(&mut value, if path.is_empty() { &top_level } else { path });
            let changed = config_from_merged(value, "example").unwrap().config;
            assert_ne!(baseline, changed, "'{}' had no effect", field.key);
        }
    }

    /// Lists every field with no `..`, so adding one to `Config` does not compile
    /// until it is added here, and this assertion then requires it to be documented.
    #[test]
    fn every_config_field_is_documented() {
        let Config {
            site_url: _,
            source_dir: _,
            profile: _,
            adapter: _,
            plugins: _,
            canonical_style: _,
            extends: _,
            audit_log_limit: _,
            browser_engine: _,
            browser_wait_until: _,
            baseline_file: _,
            max_workers: _,
            crawl_headers: _,
            crawl_cookies: _,
            crawl_basic_auth: _,
            crawl_seeds: _,
            crawl_include_patterns: _,
            crawl_exclude_patterns: _,
            crawl_use_sitemap: _,
            crawl_ignore_robots: _,
            crawl_delay_ms: _,
            crawl_capture_trace: _,
            crawl_capture_screenshot: _,
            crawl_capture_console: _,
            crawl_capture_network: _,
            crawl_artifact_dir: _,
            crawl_allow_private_network: _,
            crawl_max_response_bytes: _,
            ignore_rules: _,
            ignore_paths: _,
            severity_overrides: _,
            suppressions: _,
            route_kinds: _,
            checks: _,
            orphan_exclude: _,
            repeatable_data_ui: _,
            utility_route_patterns: _,
            route_policy_overrides: _,
            editorial: _,
            min_inbound_links: _,
            link_suggestion_count: _,
            enable_link_autofix: _,
            related_links_heading: _,
            min_page_size: _,
            required_feature_markers: _,
            min_block_text_length: _,
            min_answer_blocks: _,
            require_fact_consistency: _,
            required_schema_types: _,
            required_schema_families: _,
            require_breadcrumb_schema: _,
            require_schema_title_alignment: _,
            require_html_lang: _,
            require_hreflang_self: _,
            require_meta_robots_consistency: _,
            require_open_graph: _,
            require_twitter_card: _,
            default_twitter_card: _,
            require_social_images: _,
            require_twitter_image: _,
            require_robots_sitemap: _,
            weak_anchor_text: _,
            accessibility: _,
            agent_discovery: _,
            plugin_settings: _,
            performance_budget_file: _,
        } = Config::default();
        // `checks` has its own reference section; every other field has a doc entry.
        const FIELDS_ABOVE: usize = 66;
        assert_eq!(
            config_field_docs().len() + 1,
            FIELDS_ABOVE,
            "a Config field has no entry in config_field_docs()"
        );
    }

    /// The reported failure: nesting under the flat name is rejected, and the
    /// nested name is accepted. The reference has to say which is which.
    #[test]
    fn the_reference_names_where_crawl_headers_goes() {
        assert_eq!(
            versioned_location("crawl_headers"),
            Some(&["runtime", "headers"][..])
        );
        let reference = crate::docs::render_config_reference();
        assert!(reference.contains("- Versioned location: `headers` under `[runtime]`"));

        let nested_flat_name: toml::Value =
            toml::from_str("version = 1\n[runtime]\ncrawl_headers = { A = \"1\" }\n").unwrap();
        let error = config_from_merged(nested_flat_name, "test").unwrap_err();
        assert!(format!("{error:#}").contains("unknown config key 'crawl_headers'"));
        let nested_name: toml::Value =
            toml::from_str("version = 1\n[runtime]\nheaders = { A = \"1\" }\n").unwrap();
        let config = config_from_merged(nested_name, "test").unwrap().config;
        assert_eq!(config.crawl_headers["A"], "1");
    }

    #[test]
    fn the_schema_accepts_the_tables_it_used_to_reject() {
        for toml_text in [
            "[accessibility]\nstrict = true\n",
            "[agent_discovery]\nenabled = true\n",
            "[route_kinds.a]\nmatch = [\"/a\"]\nskip_rules = [\"GEO007\"]\nnoindex = true\n",
            "version = 1\n[rules.surfaces]\nenabled = false\n",
        ] {
            let value: toml::Value = toml::from_str(toml_text).unwrap();
            let json = serde_json::to_value(&value).unwrap();
            let errors = schema_errors(&json, &schema(), "$");
            assert!(errors.is_empty(), "{toml_text}: {errors:?}");
            config_from_merged(value, "test").unwrap();
        }
    }

    #[test]
    fn the_schema_check_rejects_what_it_should() {
        let schema = schema();
        for (value, expected) in [
            (json!({ "nope": true }), "$.nope: unknown key"),
            (
                json!({ "max_workers": "many" }),
                "$.max_workers: expected type",
            ),
            (
                json!({ "site": { "url": "https://a" } }),
                "missing required key 'version'",
            ),
            (
                json!({ "route_kinds": { "a": { "nope": 1 } } }),
                "$.route_kinds.a.nope: unknown key",
            ),
        ] {
            let errors = schema_errors(&value, &schema, "$").join("\n");
            assert!(errors.contains(expected), "{value}: {errors}");
        }
    }

    #[test]
    fn an_unsupported_schema_keyword_is_an_error() {
        let errors = schema_errors(&json!("a"), &json!({ "pattern": "a" }), "$");
        assert!(errors[0].contains("unsupported schema keyword 'pattern'"));
    }

    #[test]
    fn the_example_passes_and_a_missing_key_is_noticed() {
        assert!(example_sets(&example(), "crawl_headers"));
        let mut value = example();
        remove_path(&mut value, &["runtime", "headers"]);
        assert!(!example_sets(&value, "crawl_headers"));
    }
}

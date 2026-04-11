use serde_json::{Map, Value, json};

use super::ConfigFieldDoc;
use crate::plugin::registered_plugin_settings_schemas;

pub fn config_field_docs() -> &'static [ConfigFieldDoc] {
    &[
        ConfigFieldDoc {
            key: "site_url",
            default_value: "null",
            description: "Canonical base URL used for generation, canonical autofix, and robots output.",
        },
        ConfigFieldDoc {
            key: "source_dir",
            default_value: "`.`",
            description: "Optional build-output directory beneath the repository root.",
        },
        ConfigFieldDoc {
            key: "profile",
            default_value: "`generic`",
            description: "Built-in policy profile. `chau7` enables stricter feature-page expectations.",
        },
        ConfigFieldDoc {
            key: "adapter",
            default_value: "`auto`",
            description: "Adapter selection mode. `auto` chooses the highest-priority matching adapter.",
        },
        ConfigFieldDoc {
            key: "plugins",
            default_value: "(none)",
            description: "Modules that extend rules or adapters.",
        },
        ConfigFieldDoc {
            key: "canonical_style",
            default_value: "`extensionless`",
            description: "Preferred internal canonical style. `extensionless` expects clean routes.",
        },
        ConfigFieldDoc {
            key: "extends",
            default_value: "(none)",
            description: "Optional parent config file or files to merge before the current config.",
        },
        ConfigFieldDoc {
            key: "audit_log_limit",
            default_value: "`5`",
            description: "Number of audit artifacts retained per command, including history pruning behavior.",
        },
        ConfigFieldDoc {
            key: "browser_engine",
            default_value: "`http`",
            description: "Default crawl engine. `http` is the stable native runtime path. `auto` remains a compatibility alias for `http`, and `playwright` is reserved until implemented.",
        },
        ConfigFieldDoc {
            key: "browser_wait_until",
            default_value: "`networkidle`",
            description: "Playwright navigation wait strategy for browser-backed crawl mode.",
        },
        ConfigFieldDoc {
            key: "baseline_file",
            default_value: "`.seogeo-baseline.json`",
            description: "Default audit baseline path used for verification and diff workflows.",
        },
        ConfigFieldDoc {
            key: "max_workers",
            default_value: "`4`",
            description: "Worker count used for parallel file parsing and selected analysis tasks.",
        },
        ConfigFieldDoc {
            key: "enable_cache",
            default_value: "`true`",
            description: "Whether persistent parse and crawl caches may be used.",
        },
        ConfigFieldDoc {
            key: "cache_dir",
            default_value: "`.seogeo-cache`",
            description: "Directory for persistent seogeo caches.",
        },
        ConfigFieldDoc {
            key: "cache_ttl_seconds",
            default_value: "`3600`",
            description: "Maximum age for reusable crawl cache entries.",
        },
        ConfigFieldDoc {
            key: "crawl_headers",
            default_value: "(none)",
            description: "Extra HTTP headers applied to runtime crawl requests.",
        },
        ConfigFieldDoc {
            key: "crawl_cookies",
            default_value: "(none)",
            description: "Cookies injected into browser/runtime crawl sessions.",
        },
        ConfigFieldDoc {
            key: "crawl_basic_auth",
            default_value: "(none)",
            description: "Basic auth credentials for runtime crawl sessions.",
        },
        ConfigFieldDoc {
            key: "crawl_capture_trace",
            default_value: "`false`",
            description: "Whether browser crawl should capture a Playwright trace artifact.",
        },
        ConfigFieldDoc {
            key: "crawl_seeds",
            default_value: "(none)",
            description: "Additional route seeds for runtime crawl, such as `/pricing` or `/docs/getting-started`.",
        },
        ConfigFieldDoc {
            key: "crawl_include_patterns",
            default_value: "(none)",
            description: "Optional route substrings used to constrain runtime crawl scope.",
        },
        ConfigFieldDoc {
            key: "crawl_exclude_patterns",
            default_value: "(none)",
            description: "Route substrings excluded from runtime crawl.",
        },
        ConfigFieldDoc {
            key: "crawl_use_sitemap",
            default_value: "`true`",
            description: "Whether runtime crawl should seed additional routes from sitemap.xml when available.",
        },
        ConfigFieldDoc {
            key: "crawl_capture_screenshot",
            default_value: "`false`",
            description: "Whether browser crawl should save page screenshots.",
        },
        ConfigFieldDoc {
            key: "crawl_capture_console",
            default_value: "`false`",
            description: "Whether browser crawl should persist console output.",
        },
        ConfigFieldDoc {
            key: "crawl_capture_network",
            default_value: "`false`",
            description: "Whether browser crawl should persist network request logs.",
        },
        ConfigFieldDoc {
            key: "crawl_artifact_dir",
            default_value: "`.seogeo-reports/crawl-artifacts`",
            description: "Directory used for crawl artifacts such as traces, screenshots, console logs, and network logs.",
        },
        ConfigFieldDoc {
            key: "ignore_rules",
            default_value: "(none)",
            description: "Rule IDs to suppress after evaluation.",
        },
        ConfigFieldDoc {
            key: "ignore_paths",
            default_value: "(none)",
            description: "Glob-like path patterns to suppress after evaluation.",
        },
        ConfigFieldDoc {
            key: "severity_overrides",
            default_value: "(none)",
            description: "Per-rule severity overrides applied after rules run.",
        },
        ConfigFieldDoc {
            key: "suppressions",
            default_value: "(none)",
            description: "Explicit reviewable suppressions with rule, path pattern, reason, and optional expiry.",
        },
        ConfigFieldDoc {
            key: "orphan_exclude",
            default_value: "`404.html`",
            description: "Routes or filenames excluded from orphan detection.",
        },
        ConfigFieldDoc {
            key: "repeatable_data_ui",
            default_value: "`[\"card\", \"item\", \"entry\", ...]`",
            description: "data-ui labels treated as intentionally repeatable list-item markers rather than page-unique identifiers.",
        },
        ConfigFieldDoc {
            key: "utility_route_patterns",
            default_value: "`[\"search\", \"admin\", \"preview\", \"internal\"]`",
            description: "Route substrings used to classify utility pages for robots-policy nuance.",
        },
        ConfigFieldDoc {
            key: "route_policy_overrides",
            default_value: "(none)",
            description: "Per-route robots policy overrides such as allowing canonical+noindex or nofollow.",
        },
        ConfigFieldDoc {
            key: "min_inbound_links",
            default_value: "`1`",
            description: "Minimum inbound internal links before LNK004 triggers.",
        },
        ConfigFieldDoc {
            key: "link_suggestion_count",
            default_value: "`3`",
            description: "Number of candidate internal-link suggestions to produce per weakly linked page.",
        },
        ConfigFieldDoc {
            key: "enable_link_autofix",
            default_value: "`false`",
            description: "Whether safe fix mode may insert a generated related-links section into pages.",
        },
        ConfigFieldDoc {
            key: "related_links_heading",
            default_value: "`Related pages`",
            description: "Heading text used when link autofix inserts a related-links section.",
        },
        ConfigFieldDoc {
            key: "min_page_size",
            default_value: "`500`",
            description: "Minimum visible text length before CNT001 triggers.",
        },
        ConfigFieldDoc {
            key: "required_feature_markers",
            default_value: "`Related features`",
            description: "Literal strings required on feature-like pages before CNT002 triggers.",
        },
        ConfigFieldDoc {
            key: "min_block_text_length",
            default_value: "`120`",
            description: "Minimum visible text length for a semantic content block before GEO chunk-thinness triggers.",
        },
        ConfigFieldDoc {
            key: "min_answer_blocks",
            default_value: "`2`",
            description: "Minimum number of answer-oriented blocks before GEO answerability triggers.",
        },
        ConfigFieldDoc {
            key: "require_fact_consistency",
            default_value: "`true`",
            description: "Whether title/H1/OpenGraph/schema facts should align on a page.",
        },
        ConfigFieldDoc {
            key: "required_schema_types",
            default_value: "(none)",
            description: "JSON-LD @type values expected on route pages.",
        },
        ConfigFieldDoc {
            key: "required_schema_families",
            default_value: "(none)",
            description: "Schema families that should be validated when present or required by policy.",
        },
        ConfigFieldDoc {
            key: "require_breadcrumb_schema",
            default_value: "`false`",
            description: "Whether nested pages must emit BreadcrumbList schema.",
        },
        ConfigFieldDoc {
            key: "require_schema_title_alignment",
            default_value: "`true`",
            description: "Whether JSON-LD name/headline values must align with visible title or H1.",
        },
        ConfigFieldDoc {
            key: "require_html_lang",
            default_value: "`true`",
            description: "Whether indexable pages must declare a root html lang attribute.",
        },
        ConfigFieldDoc {
            key: "require_hreflang_self",
            default_value: "`false`",
            description: "Whether pages using hreflang alternates must include a self-referencing hreflang.",
        },
        ConfigFieldDoc {
            key: "require_meta_robots_consistency",
            default_value: "`true`",
            description: "Whether noindex pages in sitemap should be reported as inconsistent.",
        },
        ConfigFieldDoc {
            key: "require_open_graph",
            default_value: "`true`",
            description: "Whether og:title, og:description, and og:type are required.",
        },
        ConfigFieldDoc {
            key: "require_twitter_card",
            default_value: "`true`",
            description: "Whether twitter:card is required.",
        },
        ConfigFieldDoc {
            key: "default_twitter_card",
            default_value: "`summary`",
            description: "Fallback twitter:card value used by safe HTML autofix.",
        },
        ConfigFieldDoc {
            key: "require_social_images",
            default_value: "`false`",
            description: "Whether shared pages must provide og:image and optionally twitter:image.",
        },
        ConfigFieldDoc {
            key: "require_twitter_image",
            default_value: "`false`",
            description: "Whether twitter:image is required in addition to og:image.",
        },
        ConfigFieldDoc {
            key: "require_robots_sitemap",
            default_value: "`true`",
            description: "Whether robots.txt must declare the sitemap URL.",
        },
        ConfigFieldDoc {
            key: "weak_anchor_text",
            default_value: "`click here`, `here`, `learn more`, `more`, `read more`",
            description: "Anchor phrases treated as weak for internal-link quality checks.",
        },
        ConfigFieldDoc {
            key: "plugin_settings",
            default_value: "(none)",
            description: "Reserved plugin-specific configuration grouped by plugin namespace. Use quoted TOML tables such as `[plugin_settings.\"example.plugin\"]` only when a plugin publishes a registered settings schema. No built-in plugin settings schemas are currently shipped.",
        },
        ConfigFieldDoc {
            key: "typecheck_command",
            default_value: "`cargo check`",
            description: "Command used for static type checking in internal quality workflows.",
        },
        ConfigFieldDoc {
            key: "coverage_threshold",
            default_value: "`85`",
            description: "Minimum expected test coverage percentage for internal quality workflows.",
        },
        ConfigFieldDoc {
            key: "complexity_threshold",
            default_value: "`12`",
            description: "Maximum allowed AST branch complexity score per public function.",
        },
        ConfigFieldDoc {
            key: "performance_budget_file",
            default_value: "`performance-budget.json`",
            description: "Path to a JSON file describing runtime performance budgets.",
        },
    ]
}

fn string_schema(description: &str) -> Value {
    json!({
        "type": "string",
        "description": description,
    })
}

fn boolean_schema(description: &str) -> Value {
    json!({
        "type": "boolean",
        "description": description,
    })
}

fn integer_schema(description: &str) -> Value {
    json!({
        "type": "integer",
        "minimum": 0,
        "description": description,
    })
}

fn string_array_schema(description: &str) -> Value {
    json!({
        "type": "array",
        "items": { "type": "string" },
        "description": description,
    })
}

fn string_map_schema(description: &str) -> Value {
    json!({
        "type": "object",
        "additionalProperties": { "type": "string" },
        "description": description,
    })
}

fn suppression_rule_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["rule_id", "path_pattern", "reason"],
        "properties": {
            "rule_id": { "type": "string" },
            "path_pattern": { "type": "string" },
            "reason": { "type": "string" },
            "expires": { "type": ["string", "null"] }
        }
    })
}

fn route_policy_override_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["pattern"],
        "properties": {
            "pattern": { "type": "string" },
            "allow_canonical_noindex": { "type": "boolean" },
            "allow_nofollow": { "type": "boolean" }
        }
    })
}

fn rule_switch_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": { "type": "boolean" },
        "description": "Built-in rule-group toggles."
    })
}

fn plugin_settings_schema() -> Value {
    let schemas = registered_plugin_settings_schemas();
    if schemas.is_empty() {
        return json!({
            "type": "object",
            "additionalProperties": false,
            "maxProperties": 0,
            "description": "Reserved for future plugin-specific settings. No built-in plugins currently publish a registered settings schema."
        });
    }

    let mut properties = Map::new();
    for schema in schemas {
        let mut setting_properties = Map::new();
        for key in schema.allowed_keys {
            setting_properties.insert((*key).to_string(), json!({}));
        }
        properties.insert(
            schema.namespace.to_string(),
            json!({
                "type": "object",
                "additionalProperties": false,
                "properties": setting_properties,
                "description": format!("Settings accepted by plugin '{}'.", schema.namespace),
            }),
        );
    }

    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": properties,
        "description": "Plugin-specific settings keyed by plugin namespace. Each namespace must also appear in `plugins` and match a registered settings schema."
    })
}

fn flat_properties() -> Map<String, Value> {
    let mut properties = Map::new();
    properties.insert(
        "site_url".to_string(),
        string_schema("Canonical base URL used for generation and runtime verification."),
    );
    properties.insert(
        "source_dir".to_string(),
        string_schema("Optional build-output directory beneath the repository root."),
    );
    properties.insert(
        "profile".to_string(),
        string_schema("Built-in policy profile."),
    );
    properties.insert(
        "adapter".to_string(),
        string_schema("Adapter selection mode."),
    );
    properties.insert(
        "plugins".to_string(),
        string_array_schema("Plugins that extend rules or adapters."),
    );
    properties.insert(
        "canonical_style".to_string(),
        string_schema("Preferred internal canonical style."),
    );
    properties.insert(
        "extends".to_string(),
        json!({
            "oneOf": [
                { "type": "string" },
                { "type": "array", "items": { "type": "string" } }
            ],
            "description": "Parent config file or files merged before the current config."
        }),
    );
    properties.insert(
        "audit_log_limit".to_string(),
        integer_schema("Number of audit artifacts retained per command."),
    );
    properties.insert(
        "browser_engine".to_string(),
        string_schema("Default crawl engine."),
    );
    properties.insert(
        "browser_wait_until".to_string(),
        string_schema("Browser navigation wait strategy."),
    );
    properties.insert(
        "baseline_file".to_string(),
        string_schema("Default audit baseline path."),
    );
    properties.insert(
        "max_workers".to_string(),
        integer_schema("Worker count used for parallel analysis tasks."),
    );
    properties.insert(
        "enable_cache".to_string(),
        boolean_schema("Whether persistent caches may be used."),
    );
    properties.insert(
        "cache_dir".to_string(),
        string_schema("Directory for persistent caches."),
    );
    properties.insert(
        "cache_ttl_seconds".to_string(),
        integer_schema("Maximum age for reusable crawl cache entries."),
    );
    properties.insert(
        "crawl_headers".to_string(),
        string_map_schema("Extra HTTP headers applied to runtime crawl requests."),
    );
    properties.insert(
        "crawl_cookies".to_string(),
        json!({
            "type": "array",
            "items": {
                "type": "object",
                "additionalProperties": { "type": "string" }
            },
            "description": "Cookies injected into runtime crawl sessions."
        }),
    );
    properties.insert(
        "crawl_basic_auth".to_string(),
        json!({
            "type": "object",
            "additionalProperties": { "type": "string" },
            "description": "Basic auth credentials for runtime crawl sessions."
        }),
    );
    properties.insert(
        "crawl_seeds".to_string(),
        string_array_schema("Additional route seeds for runtime crawl."),
    );
    properties.insert(
        "crawl_include_patterns".to_string(),
        string_array_schema("Route substrings used to constrain runtime crawl scope."),
    );
    properties.insert(
        "crawl_exclude_patterns".to_string(),
        string_array_schema("Route substrings excluded from runtime crawl."),
    );
    properties.insert(
        "crawl_use_sitemap".to_string(),
        boolean_schema("Whether runtime crawl should seed routes from sitemap.xml."),
    );
    properties.insert(
        "crawl_capture_trace".to_string(),
        boolean_schema("Whether browser crawl should capture a trace artifact."),
    );
    properties.insert(
        "crawl_capture_screenshot".to_string(),
        boolean_schema("Whether browser crawl should save screenshots."),
    );
    properties.insert(
        "crawl_capture_console".to_string(),
        boolean_schema("Whether browser crawl should persist console output."),
    );
    properties.insert(
        "crawl_capture_network".to_string(),
        boolean_schema("Whether browser crawl should persist network logs."),
    );
    properties.insert(
        "crawl_artifact_dir".to_string(),
        string_schema("Directory used for crawl artifacts."),
    );
    properties.insert(
        "ignore_rules".to_string(),
        string_array_schema("Rule IDs to suppress after evaluation."),
    );
    properties.insert(
        "ignore_paths".to_string(),
        string_array_schema("Path patterns to suppress after evaluation."),
    );
    properties.insert(
        "severity_overrides".to_string(),
        string_map_schema("Per-rule severity overrides applied after rules run."),
    );
    properties.insert(
        "suppressions".to_string(),
        json!({
            "type": "array",
            "items": suppression_rule_schema(),
            "description": "Explicit reviewable suppressions."
        }),
    );
    properties.insert("checks".to_string(), rule_switch_schema());
    properties.insert(
        "orphan_exclude".to_string(),
        string_array_schema("Routes or filenames excluded from orphan detection."),
    );
    properties.insert(
        "repeatable_data_ui".to_string(),
        string_array_schema("data-ui labels treated as intentionally repeatable."),
    );
    properties.insert(
        "utility_route_patterns".to_string(),
        string_array_schema("Route substrings treated as utility routes."),
    );
    properties.insert(
        "route_policy_overrides".to_string(),
        json!({
            "type": "array",
            "items": route_policy_override_schema(),
            "description": "Per-route robots policy overrides."
        }),
    );
    properties.insert(
        "min_inbound_links".to_string(),
        integer_schema("Minimum inbound internal links before LNK004 triggers."),
    );
    properties.insert(
        "link_suggestion_count".to_string(),
        integer_schema("Candidate internal-link suggestions per weakly linked page."),
    );
    properties.insert(
        "enable_link_autofix".to_string(),
        boolean_schema("Whether safe fix mode may insert related-links sections."),
    );
    properties.insert(
        "related_links_heading".to_string(),
        string_schema("Heading text used when related-links autofix inserts content."),
    );
    properties.insert(
        "min_page_size".to_string(),
        integer_schema("Minimum visible text length before CNT001 triggers."),
    );
    properties.insert(
        "required_feature_markers".to_string(),
        string_array_schema("Literal strings required on feature-like pages."),
    );
    properties.insert(
        "min_block_text_length".to_string(),
        integer_schema("Minimum visible text length for a semantic content block."),
    );
    properties.insert(
        "min_answer_blocks".to_string(),
        integer_schema("Minimum answer-oriented blocks required for a page."),
    );
    properties.insert(
        "require_fact_consistency".to_string(),
        boolean_schema("Whether visible and structured page facts should align."),
    );
    properties.insert(
        "required_schema_types".to_string(),
        string_array_schema("JSON-LD @type values expected on route pages."),
    );
    properties.insert(
        "required_schema_families".to_string(),
        string_array_schema("Schema families that should be validated."),
    );
    properties.insert(
        "require_breadcrumb_schema".to_string(),
        boolean_schema("Whether nested pages must emit BreadcrumbList schema."),
    );
    properties.insert(
        "require_schema_title_alignment".to_string(),
        boolean_schema("Whether JSON-LD title fields must align with visible titles."),
    );
    properties.insert(
        "require_html_lang".to_string(),
        boolean_schema("Whether indexable pages must declare html lang."),
    );
    properties.insert(
        "require_hreflang_self".to_string(),
        boolean_schema("Whether pages with hreflang alternates require a self-reference."),
    );
    properties.insert(
        "require_meta_robots_consistency".to_string(),
        boolean_schema("Whether sitemap/indexability robots inconsistencies should be flagged."),
    );
    properties.insert(
        "require_open_graph".to_string(),
        boolean_schema("Whether core Open Graph tags are required."),
    );
    properties.insert(
        "require_twitter_card".to_string(),
        boolean_schema("Whether twitter:card is required."),
    );
    properties.insert(
        "default_twitter_card".to_string(),
        string_schema("Default twitter:card value used by safe fixes."),
    );
    properties.insert(
        "require_social_images".to_string(),
        boolean_schema("Whether og:image is required."),
    );
    properties.insert(
        "require_twitter_image".to_string(),
        boolean_schema("Whether twitter:image is required."),
    );
    properties.insert(
        "require_robots_sitemap".to_string(),
        boolean_schema("Whether robots.txt must declare a sitemap."),
    );
    properties.insert(
        "weak_anchor_text".to_string(),
        string_array_schema("Anchor texts treated as weak generic phrasing."),
    );
    properties.insert("plugin_settings".to_string(), plugin_settings_schema());
    properties.insert(
        "typecheck_command".to_string(),
        string_schema("Typecheck command used by repo quality checks."),
    );
    properties.insert(
        "coverage_threshold".to_string(),
        integer_schema("Minimum coverage percentage expected by quality checks."),
    );
    properties.insert(
        "complexity_threshold".to_string(),
        integer_schema("Maximum allowed complexity threshold."),
    );
    properties.insert(
        "performance_budget_file".to_string(),
        string_schema("Performance budget file checked by repo quality."),
    );
    properties
}

fn nested_properties() -> Map<String, Value> {
    let mut properties = Map::new();
    properties.insert(
        "site".to_string(),
        json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "url": string_schema("Canonical base URL."),
                "source_dir": string_schema("Build-output directory."),
                "adapter": string_schema("Adapter selection mode."),
                "canonical_style": string_schema("Preferred internal canonical style.")
            }
        }),
    );
    properties.insert(
        "runtime".to_string(),
        json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "engine": string_schema("Default crawl engine."),
                "wait_until": string_schema("Browser navigation wait strategy."),
                "headers": string_map_schema("Extra HTTP headers."),
                "cookies": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "additionalProperties": { "type": "string" }
                    }
                },
                "basic_auth": {
                    "type": "object",
                    "additionalProperties": { "type": "string" }
                },
                "seeds": { "type": "array", "items": { "type": "string" } },
                "include_patterns": { "type": "array", "items": { "type": "string" } },
                "exclude_patterns": { "type": "array", "items": { "type": "string" } },
                "use_sitemap": { "type": "boolean" },
                "capture_trace": { "type": "boolean" },
                "capture_screenshot": { "type": "boolean" },
                "capture_console": { "type": "boolean" },
                "capture_network": { "type": "boolean" },
                "artifact_dir": { "type": "string" }
            }
        }),
    );
    properties.insert(
        "policy".to_string(),
        json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "ignore_rules": { "type": "array", "items": { "type": "string" } },
                "ignore_paths": { "type": "array", "items": { "type": "string" } },
                "severity_overrides": {
                    "type": "object",
                    "additionalProperties": { "type": "string" }
                },
                "suppressions": {
                    "type": "array",
                    "items": suppression_rule_schema()
                },
                "route_policy_overrides": {
                    "type": "array",
                    "items": route_policy_override_schema()
                }
            }
        }),
    );
    properties.insert(
        "rules".to_string(),
        json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "html": { "oneOf": [{ "type": "boolean" }, {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "enabled": { "type": "boolean" },
                        "require_html_lang": { "type": "boolean" },
                        "require_hreflang_self": { "type": "boolean" }
                    }
                }]},
                "links": { "oneOf": [{ "type": "boolean" }, {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "enabled": { "type": "boolean" },
                        "min_inbound_links": { "type": "integer", "minimum": 0 },
                        "link_suggestion_count": { "type": "integer", "minimum": 0 },
                        "enable_link_autofix": { "type": "boolean" },
                        "related_links_heading": { "type": "string" },
                        "weak_anchor_text": { "type": "array", "items": { "type": "string" } }
                    }
                }]},
                "sitemap": { "oneOf": [{ "type": "boolean" }, {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "enabled": { "type": "boolean" }
                    }
                }]},
                "robots": { "oneOf": [{ "type": "boolean" }, {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "enabled": { "type": "boolean" },
                        "require_sitemap": { "type": "boolean" },
                        "require_meta_consistency": { "type": "boolean" }
                    }
                }]},
                "social": { "oneOf": [{ "type": "boolean" }, {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "enabled": { "type": "boolean" },
                        "require_open_graph": { "type": "boolean" },
                        "require_twitter_card": { "type": "boolean" },
                        "default_twitter_card": { "type": "string" },
                        "require_social_images": { "type": "boolean" },
                        "require_twitter_image": { "type": "boolean" }
                    }
                }]},
                "schema": { "oneOf": [{ "type": "boolean" }, {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "enabled": { "type": "boolean" },
                        "required_types": { "type": "array", "items": { "type": "string" } },
                        "required_families": { "type": "array", "items": { "type": "string" } },
                        "require_breadcrumb_schema": { "type": "boolean" },
                        "require_title_alignment": { "type": "boolean" }
                    }
                }]},
                "llm": { "oneOf": [{ "type": "boolean" }, {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "enabled": { "type": "boolean" }
                    }
                }]},
                "content": { "oneOf": [{ "type": "boolean" }, {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "enabled": { "type": "boolean" },
                        "min_page_size": { "type": "integer", "minimum": 0 },
                        "required_feature_markers": { "type": "array", "items": { "type": "string" } }
                    }
                }]},
                "structure": { "oneOf": [{ "type": "boolean" }, {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "enabled": { "type": "boolean" },
                        "repeatable_data_ui": { "type": "array", "items": { "type": "string" } },
                        "utility_route_patterns": { "type": "array", "items": { "type": "string" } },
                        "min_block_text_length": { "type": "integer", "minimum": 0 },
                        "min_answer_blocks": { "type": "integer", "minimum": 0 },
                        "require_fact_consistency": { "type": "boolean" }
                    }
                }]}
            }
        }),
    );
    properties.insert(
        "output".to_string(),
        json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "baseline_file": { "type": "string" },
                "audit_log_limit": { "type": "integer", "minimum": 0 }
            }
        }),
    );
    properties.insert(
        "quality".to_string(),
        json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "typecheck_command": { "type": "string" },
                "coverage_threshold": { "type": "integer", "minimum": 0 },
                "complexity_threshold": { "type": "integer", "minimum": 0 },
                "performance_budget_file": { "type": "string" }
            }
        }),
    );
    properties
}

pub fn render_config_schema() -> String {
    let mut properties = flat_properties();
    properties.insert(
        "version".to_string(),
        json!({
            "type": "integer",
            "const": 1,
            "description": "Canonical nested config surface version."
        }),
    );
    properties.extend(nested_properties());

    serde_json::to_string_pretty(&json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "title": "seogeo configuration",
        "type": "object",
        "additionalProperties": false,
        "properties": properties,
        "allOf": [
            {
                "if": {
                    "anyOf": [
                        { "required": ["site"] },
                        { "required": ["runtime"] },
                        { "required": ["policy"] },
                        { "required": ["rules"] },
                        { "required": ["output"] },
                        { "required": ["quality"] }
                    ]
                },
                "then": {
                    "required": ["version"],
                    "properties": {
                        "version": { "const": 1 }
                    }
                }
            }
        ]
    }))
    .expect("schema serialization should not fail")
}

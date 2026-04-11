use serde::Serialize;
use std::collections::BTreeMap;

use super::{Config, RoutePolicyOverride, SuppressionRule};

#[derive(Debug, Clone, Serialize)]
struct ResolvedConfigDocument {
    version: u8,
    profile: String,
    plugins: Vec<String>,
    extends: Vec<String>,
    max_workers: usize,
    enable_cache: bool,
    cache_dir: String,
    cache_ttl_seconds: usize,
    plugin_settings: BTreeMap<String, BTreeMap<String, toml::Value>>,
    site: SiteSection,
    runtime: RuntimeSection,
    policy: PolicySection,
    rules: RulesSection,
    output: OutputSection,
    quality: QualitySection,
}

#[derive(Debug, Clone, Serialize)]
struct SiteSection {
    url: Option<String>,
    source_dir: String,
    adapter: String,
    canonical_style: String,
}

#[derive(Debug, Clone, Serialize)]
struct RuntimeSection {
    engine: String,
    wait_until: String,
    headers: BTreeMap<String, String>,
    cookies: Vec<BTreeMap<String, String>>,
    basic_auth: BTreeMap<String, String>,
    seeds: Vec<String>,
    include_patterns: Vec<String>,
    exclude_patterns: Vec<String>,
    use_sitemap: bool,
    capture_trace: bool,
    capture_screenshot: bool,
    capture_console: bool,
    capture_network: bool,
    artifact_dir: String,
}

#[derive(Debug, Clone, Serialize)]
struct PolicySection {
    ignore_rules: Vec<String>,
    ignore_paths: Vec<String>,
    severity_overrides: BTreeMap<String, String>,
    suppressions: Vec<SuppressionRule>,
    route_policy_overrides: Vec<RoutePolicyOverride>,
}

#[derive(Debug, Clone, Serialize)]
struct RulesSection {
    html: HtmlRulesSection,
    links: LinkRulesSection,
    sitemap: EnabledRuleSection,
    robots: RobotsRulesSection,
    social: SocialRulesSection,
    schema: SchemaRulesSection,
    llm: EnabledRuleSection,
    content: ContentRulesSection,
    structure: StructureRulesSection,
}

#[derive(Debug, Clone, Serialize)]
struct EnabledRuleSection {
    enabled: bool,
}

#[derive(Debug, Clone, Serialize)]
struct HtmlRulesSection {
    enabled: bool,
    require_html_lang: bool,
    require_hreflang_self: bool,
}

#[derive(Debug, Clone, Serialize)]
struct LinkRulesSection {
    enabled: bool,
    min_inbound_links: usize,
    link_suggestion_count: usize,
    enable_link_autofix: bool,
    related_links_heading: String,
    weak_anchor_text: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
struct RobotsRulesSection {
    enabled: bool,
    require_sitemap: bool,
    require_meta_consistency: bool,
}

#[derive(Debug, Clone, Serialize)]
struct SocialRulesSection {
    enabled: bool,
    require_open_graph: bool,
    require_twitter_card: bool,
    default_twitter_card: String,
    require_social_images: bool,
    require_twitter_image: bool,
}

#[derive(Debug, Clone, Serialize)]
struct SchemaRulesSection {
    enabled: bool,
    required_types: Vec<String>,
    required_families: Vec<String>,
    require_breadcrumb_schema: bool,
    require_title_alignment: bool,
}

#[derive(Debug, Clone, Serialize)]
struct ContentRulesSection {
    enabled: bool,
    min_page_size: usize,
    required_feature_markers: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
struct StructureRulesSection {
    enabled: bool,
    repeatable_data_ui: Vec<String>,
    utility_route_patterns: Vec<String>,
    min_block_text_length: usize,
    min_answer_blocks: usize,
    require_fact_consistency: bool,
}

#[derive(Debug, Clone, Serialize)]
struct OutputSection {
    baseline_file: String,
    audit_log_limit: usize,
}

#[derive(Debug, Clone, Serialize)]
struct QualitySection {
    typecheck_command: String,
    coverage_threshold: usize,
    complexity_threshold: usize,
    performance_budget_file: String,
}

fn is_enabled(config: &Config, key: &str) -> bool {
    config.checks.get(key).copied().unwrap_or(true)
}

fn resolved_config_document(config: &Config) -> ResolvedConfigDocument {
    ResolvedConfigDocument {
        version: 1,
        profile: config.profile.clone(),
        plugins: config.plugins.clone(),
        extends: config.extends.clone(),
        max_workers: config.max_workers,
        enable_cache: config.enable_cache,
        cache_dir: config.cache_dir.clone(),
        cache_ttl_seconds: config.cache_ttl_seconds,
        plugin_settings: config.plugin_settings.clone(),
        site: SiteSection {
            url: config.site_url.clone(),
            source_dir: config.source_dir.clone(),
            adapter: config.adapter.clone(),
            canonical_style: config.canonical_style.clone(),
        },
        runtime: RuntimeSection {
            engine: config.browser_engine.clone(),
            wait_until: config.browser_wait_until.clone(),
            headers: config.crawl_headers.clone(),
            cookies: config.crawl_cookies.clone(),
            basic_auth: config.crawl_basic_auth.clone(),
            seeds: config.crawl_seeds.clone(),
            include_patterns: config.crawl_include_patterns.clone(),
            exclude_patterns: config.crawl_exclude_patterns.clone(),
            use_sitemap: config.crawl_use_sitemap,
            capture_trace: config.crawl_capture_trace,
            capture_screenshot: config.crawl_capture_screenshot,
            capture_console: config.crawl_capture_console,
            capture_network: config.crawl_capture_network,
            artifact_dir: config.crawl_artifact_dir.clone(),
        },
        policy: PolicySection {
            ignore_rules: config.ignore_rules.clone(),
            ignore_paths: config.ignore_paths.clone(),
            severity_overrides: config.severity_overrides.clone(),
            suppressions: config.suppressions.clone(),
            route_policy_overrides: config.route_policy_overrides.clone(),
        },
        rules: RulesSection {
            html: HtmlRulesSection {
                enabled: is_enabled(config, "html"),
                require_html_lang: config.require_html_lang,
                require_hreflang_self: config.require_hreflang_self,
            },
            links: LinkRulesSection {
                enabled: is_enabled(config, "links"),
                min_inbound_links: config.min_inbound_links,
                link_suggestion_count: config.link_suggestion_count,
                enable_link_autofix: config.enable_link_autofix,
                related_links_heading: config.related_links_heading.clone(),
                weak_anchor_text: config.weak_anchor_text.clone(),
            },
            sitemap: EnabledRuleSection {
                enabled: is_enabled(config, "sitemap"),
            },
            robots: RobotsRulesSection {
                enabled: is_enabled(config, "robots"),
                require_sitemap: config.require_robots_sitemap,
                require_meta_consistency: config.require_meta_robots_consistency,
            },
            social: SocialRulesSection {
                enabled: is_enabled(config, "social"),
                require_open_graph: config.require_open_graph,
                require_twitter_card: config.require_twitter_card,
                default_twitter_card: config.default_twitter_card.clone(),
                require_social_images: config.require_social_images,
                require_twitter_image: config.require_twitter_image,
            },
            schema: SchemaRulesSection {
                enabled: is_enabled(config, "schema"),
                required_types: config.required_schema_types.clone(),
                required_families: config.required_schema_families.clone(),
                require_breadcrumb_schema: config.require_breadcrumb_schema,
                require_title_alignment: config.require_schema_title_alignment,
            },
            llm: EnabledRuleSection {
                enabled: is_enabled(config, "llm"),
            },
            content: ContentRulesSection {
                enabled: is_enabled(config, "content"),
                min_page_size: config.min_page_size,
                required_feature_markers: config.required_feature_markers.clone(),
            },
            structure: StructureRulesSection {
                enabled: is_enabled(config, "structure"),
                repeatable_data_ui: config.repeatable_data_ui.clone(),
                utility_route_patterns: config.utility_route_patterns.clone(),
                min_block_text_length: config.min_block_text_length,
                min_answer_blocks: config.min_answer_blocks,
                require_fact_consistency: config.require_fact_consistency,
            },
        },
        output: OutputSection {
            baseline_file: config.baseline_file.clone(),
            audit_log_limit: config.audit_log_limit,
        },
        quality: QualitySection {
            typecheck_command: config.typecheck_command.clone(),
            coverage_threshold: config.coverage_threshold,
            complexity_threshold: config.complexity_threshold,
            performance_budget_file: config.performance_budget_file.clone(),
        },
    }
}

pub fn render_resolved_config_json(config: &Config) -> anyhow::Result<String> {
    Ok(serde_json::to_string_pretty(&resolved_config_document(
        config,
    ))?)
}

pub fn render_resolved_config_toml(config: &Config) -> anyhow::Result<String> {
    Ok(toml::to_string_pretty(&resolved_config_document(config))?)
}

#[cfg(test)]
mod tests {
    use super::{render_resolved_config_json, render_resolved_config_toml};
    use crate::config::Config;

    #[test]
    fn renders_canonical_resolved_config() {
        let mut config = Config {
            browser_engine: "http".to_string(),
            ..Config::default()
        };
        config.checks.insert("links".to_string(), false);
        let toml = render_resolved_config_toml(&config).unwrap();
        let json = render_resolved_config_json(&config).unwrap();

        assert!(toml.contains("version = 1"));
        assert!(toml.contains("[rules.links]"));
        assert!(toml.contains("enabled = false"));
        assert!(json.contains("\"version\": 1"));
        assert!(json.contains("\"links\""));
    }
}

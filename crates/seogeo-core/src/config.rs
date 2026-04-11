mod defaults;
mod docs;
mod load;
mod render;
mod views;

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use self::defaults::{
    default_adapter, default_audit_log_limit, default_baseline_file, default_browser_engine,
    default_browser_wait_until, default_cache_dir, default_cache_ttl_seconds,
    default_canonical_style, default_checks, default_complexity_threshold,
    default_coverage_threshold, default_crawl_artifact_dir, default_crawl_use_sitemap,
    default_default_twitter_card, default_enable_cache, default_link_suggestion_count,
    default_max_workers, default_min_answer_blocks, default_min_block_text_length,
    default_min_inbound_links, default_min_page_size, default_orphan_exclude,
    default_performance_budget_file, default_profile, default_related_links_heading,
    default_repeatable_data_ui, default_require_fact_consistency, default_require_html_lang,
    default_require_meta_robots_consistency, default_require_open_graph,
    default_require_robots_sitemap, default_require_schema_title_alignment,
    default_require_twitter_card, default_required_feature_markers, default_source_dir,
    default_typecheck_command, default_utility_route_patterns, default_weak_anchor_text,
};

pub use self::defaults::default_rule_switches;
pub use self::docs::{config_field_docs, render_config_schema};
pub use self::load::load_config;
pub use self::render::{render_resolved_config_json, render_resolved_config_toml};
pub use self::views::{
    OutputConfig, PolicyConfig, QualityConfig, RulesConfig, RuntimeConfig, SiteConfig,
};

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub site_url: Option<String>,
    #[serde(default = "default_source_dir")]
    pub source_dir: String,
    #[serde(default = "default_profile")]
    pub profile: String,
    #[serde(default = "default_adapter")]
    pub adapter: String,
    #[serde(default)]
    pub plugins: Vec<String>,
    #[serde(default = "default_canonical_style")]
    pub canonical_style: String,
    #[serde(default)]
    pub extends: Vec<String>,
    #[serde(default = "default_audit_log_limit")]
    pub audit_log_limit: usize,
    #[serde(default = "default_browser_engine")]
    pub browser_engine: String,
    #[serde(default = "default_browser_wait_until")]
    pub browser_wait_until: String,
    #[serde(default = "default_baseline_file")]
    pub baseline_file: String,
    #[serde(default = "default_max_workers")]
    pub max_workers: usize,
    #[serde(default = "default_enable_cache")]
    pub enable_cache: bool,
    #[serde(default = "default_cache_dir")]
    pub cache_dir: String,
    #[serde(default = "default_cache_ttl_seconds")]
    pub cache_ttl_seconds: usize,
    #[serde(default)]
    pub crawl_headers: BTreeMap<String, String>,
    #[serde(default)]
    pub crawl_cookies: Vec<BTreeMap<String, String>>,
    #[serde(default)]
    pub crawl_basic_auth: BTreeMap<String, String>,
    #[serde(default)]
    pub crawl_seeds: Vec<String>,
    #[serde(default)]
    pub crawl_include_patterns: Vec<String>,
    #[serde(default)]
    pub crawl_exclude_patterns: Vec<String>,
    #[serde(default = "default_crawl_use_sitemap")]
    pub crawl_use_sitemap: bool,
    #[serde(default)]
    pub crawl_capture_trace: bool,
    #[serde(default)]
    pub crawl_capture_screenshot: bool,
    #[serde(default)]
    pub crawl_capture_console: bool,
    #[serde(default)]
    pub crawl_capture_network: bool,
    #[serde(default = "default_crawl_artifact_dir")]
    pub crawl_artifact_dir: String,
    #[serde(default)]
    pub ignore_rules: Vec<String>,
    #[serde(default)]
    pub ignore_paths: Vec<String>,
    #[serde(default)]
    pub severity_overrides: BTreeMap<String, String>,
    #[serde(default)]
    pub suppressions: Vec<SuppressionRule>,
    #[serde(default = "default_checks")]
    pub checks: BTreeMap<String, bool>,
    #[serde(default = "default_orphan_exclude")]
    pub orphan_exclude: Vec<String>,
    #[serde(default = "default_repeatable_data_ui")]
    pub repeatable_data_ui: Vec<String>,
    #[serde(default = "default_utility_route_patterns")]
    pub utility_route_patterns: Vec<String>,
    #[serde(default)]
    pub route_policy_overrides: Vec<RoutePolicyOverride>,
    #[serde(default = "default_min_inbound_links")]
    pub min_inbound_links: usize,
    #[serde(default = "default_link_suggestion_count")]
    pub link_suggestion_count: usize,
    #[serde(default)]
    pub enable_link_autofix: bool,
    #[serde(default = "default_related_links_heading")]
    pub related_links_heading: String,
    #[serde(default = "default_min_page_size")]
    pub min_page_size: usize,
    #[serde(default = "default_required_feature_markers")]
    pub required_feature_markers: Vec<String>,
    #[serde(default = "default_min_block_text_length")]
    pub min_block_text_length: usize,
    #[serde(default = "default_min_answer_blocks")]
    pub min_answer_blocks: usize,
    #[serde(default = "default_require_fact_consistency")]
    pub require_fact_consistency: bool,
    #[serde(default)]
    pub required_schema_types: Vec<String>,
    #[serde(default)]
    pub required_schema_families: Vec<String>,
    #[serde(default)]
    pub require_breadcrumb_schema: bool,
    #[serde(default = "default_require_schema_title_alignment")]
    pub require_schema_title_alignment: bool,
    #[serde(default = "default_require_html_lang")]
    pub require_html_lang: bool,
    #[serde(default)]
    pub require_hreflang_self: bool,
    #[serde(default = "default_require_meta_robots_consistency")]
    pub require_meta_robots_consistency: bool,
    #[serde(default = "default_require_open_graph")]
    pub require_open_graph: bool,
    #[serde(default = "default_require_twitter_card")]
    pub require_twitter_card: bool,
    #[serde(default = "default_default_twitter_card")]
    pub default_twitter_card: String,
    #[serde(default)]
    pub require_social_images: bool,
    #[serde(default)]
    pub require_twitter_image: bool,
    #[serde(default = "default_require_robots_sitemap")]
    pub require_robots_sitemap: bool,
    #[serde(default = "default_weak_anchor_text")]
    pub weak_anchor_text: Vec<String>,
    #[serde(default)]
    pub plugin_settings: BTreeMap<String, BTreeMap<String, toml::Value>>,
    #[serde(default = "default_typecheck_command")]
    pub typecheck_command: String,
    #[serde(default = "default_coverage_threshold")]
    pub coverage_threshold: usize,
    #[serde(default = "default_complexity_threshold")]
    pub complexity_threshold: usize,
    #[serde(default = "default_performance_budget_file")]
    pub performance_budget_file: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct RoutePolicyOverride {
    pub pattern: String,
    #[serde(default)]
    pub allow_canonical_noindex: bool,
    #[serde(default)]
    pub allow_nofollow: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct SuppressionRule {
    pub rule_id: String,
    pub path_pattern: String,
    pub reason: String,
    #[serde(default)]
    pub expires: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigFieldDoc {
    pub key: &'static str,
    pub default_value: &'static str,
    pub description: &'static str,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            site_url: None,
            source_dir: default_source_dir(),
            profile: default_profile(),
            adapter: default_adapter(),
            plugins: Vec::new(),
            canonical_style: default_canonical_style(),
            extends: Vec::new(),
            audit_log_limit: default_audit_log_limit(),
            browser_engine: default_browser_engine(),
            browser_wait_until: default_browser_wait_until(),
            baseline_file: default_baseline_file(),
            max_workers: default_max_workers(),
            enable_cache: default_enable_cache(),
            cache_dir: default_cache_dir(),
            cache_ttl_seconds: default_cache_ttl_seconds(),
            crawl_headers: BTreeMap::new(),
            crawl_cookies: Vec::new(),
            crawl_basic_auth: BTreeMap::new(),
            crawl_seeds: Vec::new(),
            crawl_include_patterns: Vec::new(),
            crawl_exclude_patterns: Vec::new(),
            crawl_use_sitemap: default_crawl_use_sitemap(),
            crawl_capture_trace: false,
            crawl_capture_screenshot: false,
            crawl_capture_console: false,
            crawl_capture_network: false,
            crawl_artifact_dir: default_crawl_artifact_dir(),
            ignore_rules: Vec::new(),
            ignore_paths: Vec::new(),
            severity_overrides: BTreeMap::new(),
            suppressions: Vec::new(),
            checks: default_checks(),
            orphan_exclude: default_orphan_exclude(),
            repeatable_data_ui: default_repeatable_data_ui(),
            utility_route_patterns: default_utility_route_patterns(),
            route_policy_overrides: Vec::new(),
            min_inbound_links: default_min_inbound_links(),
            link_suggestion_count: default_link_suggestion_count(),
            enable_link_autofix: false,
            related_links_heading: default_related_links_heading(),
            min_page_size: default_min_page_size(),
            required_feature_markers: default_required_feature_markers(),
            min_block_text_length: default_min_block_text_length(),
            min_answer_blocks: default_min_answer_blocks(),
            require_fact_consistency: default_require_fact_consistency(),
            required_schema_types: Vec::new(),
            required_schema_families: Vec::new(),
            require_breadcrumb_schema: false,
            require_schema_title_alignment: default_require_schema_title_alignment(),
            require_html_lang: default_require_html_lang(),
            require_hreflang_self: false,
            require_meta_robots_consistency: default_require_meta_robots_consistency(),
            require_open_graph: default_require_open_graph(),
            require_twitter_card: default_require_twitter_card(),
            default_twitter_card: default_default_twitter_card(),
            require_social_images: false,
            require_twitter_image: false,
            require_robots_sitemap: default_require_robots_sitemap(),
            weak_anchor_text: default_weak_anchor_text(),
            plugin_settings: BTreeMap::new(),
            typecheck_command: default_typecheck_command(),
            coverage_threshold: default_coverage_threshold(),
            complexity_threshold: default_complexity_threshold(),
            performance_budget_file: default_performance_budget_file(),
        }
    }
}

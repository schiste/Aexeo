use std::collections::BTreeMap;

use super::{Config, RoutePolicyOverride, SuppressionRule};

#[derive(Debug, Clone, Copy)]
pub struct SiteConfig<'a> {
    pub site_url: Option<&'a str>,
    pub source_dir: &'a str,
    pub adapter: &'a str,
    pub canonical_style: &'a str,
    pub plugins: &'a [String],
}

#[derive(Debug, Clone, Copy)]
pub struct RuntimeConfig<'a> {
    pub browser_engine: &'a str,
    pub browser_wait_until: &'a str,
    pub max_workers: usize,
    pub enable_cache: bool,
    pub cache_dir: &'a str,
    pub cache_ttl_seconds: usize,
    pub crawl_headers: &'a BTreeMap<String, String>,
    pub crawl_cookies: &'a [BTreeMap<String, String>],
    pub crawl_basic_auth: &'a BTreeMap<String, String>,
    pub crawl_seeds: &'a [String],
    pub crawl_include_patterns: &'a [String],
    pub crawl_exclude_patterns: &'a [String],
    pub crawl_use_sitemap: bool,
    pub crawl_capture_trace: bool,
    pub crawl_capture_screenshot: bool,
    pub crawl_capture_console: bool,
    pub crawl_capture_network: bool,
    pub crawl_artifact_dir: &'a str,
}

#[derive(Debug, Clone, Copy)]
pub struct PolicyConfig<'a> {
    pub ignore_rules: &'a [String],
    pub ignore_paths: &'a [String],
    pub severity_overrides: &'a BTreeMap<String, String>,
    pub suppressions: &'a [SuppressionRule],
    pub route_policy_overrides: &'a [RoutePolicyOverride],
}

#[derive(Debug, Clone, Copy)]
pub struct RulesConfig<'a> {
    pub checks: &'a BTreeMap<String, bool>,
    pub orphan_exclude: &'a [String],
    pub repeatable_data_ui: &'a [String],
    pub utility_route_patterns: &'a [String],
    pub min_inbound_links: usize,
    pub link_suggestion_count: usize,
    pub enable_link_autofix: bool,
    pub related_links_heading: &'a str,
    pub min_page_size: usize,
    pub required_feature_markers: &'a [String],
    pub min_block_text_length: usize,
    pub min_answer_blocks: usize,
    pub require_fact_consistency: bool,
    pub required_schema_types: &'a [String],
    pub required_schema_families: &'a [String],
    pub require_breadcrumb_schema: bool,
    pub require_schema_title_alignment: bool,
    pub require_html_lang: bool,
    pub require_hreflang_self: bool,
    pub require_meta_robots_consistency: bool,
    pub require_open_graph: bool,
    pub require_twitter_card: bool,
    pub default_twitter_card: &'a str,
    pub require_social_images: bool,
    pub require_twitter_image: bool,
    pub require_robots_sitemap: bool,
    pub weak_anchor_text: &'a [String],
}

#[derive(Debug, Clone, Copy)]
pub struct OutputConfig<'a> {
    pub baseline_file: &'a str,
    pub audit_log_limit: usize,
}

#[derive(Debug, Clone, Copy)]
pub struct QualityConfig<'a> {
    pub typecheck_command: &'a str,
    pub coverage_threshold: usize,
    pub complexity_threshold: usize,
    pub performance_budget_file: &'a str,
}

impl Config {
    pub fn site(&self) -> SiteConfig<'_> {
        SiteConfig {
            site_url: self.site_url.as_deref(),
            source_dir: &self.source_dir,
            adapter: &self.adapter,
            canonical_style: &self.canonical_style,
            plugins: &self.plugins,
        }
    }

    pub fn runtime(&self) -> RuntimeConfig<'_> {
        RuntimeConfig {
            browser_engine: &self.browser_engine,
            browser_wait_until: &self.browser_wait_until,
            max_workers: self.max_workers,
            enable_cache: self.enable_cache,
            cache_dir: &self.cache_dir,
            cache_ttl_seconds: self.cache_ttl_seconds,
            crawl_headers: &self.crawl_headers,
            crawl_cookies: &self.crawl_cookies,
            crawl_basic_auth: &self.crawl_basic_auth,
            crawl_seeds: &self.crawl_seeds,
            crawl_include_patterns: &self.crawl_include_patterns,
            crawl_exclude_patterns: &self.crawl_exclude_patterns,
            crawl_use_sitemap: self.crawl_use_sitemap,
            crawl_capture_trace: self.crawl_capture_trace,
            crawl_capture_screenshot: self.crawl_capture_screenshot,
            crawl_capture_console: self.crawl_capture_console,
            crawl_capture_network: self.crawl_capture_network,
            crawl_artifact_dir: &self.crawl_artifact_dir,
        }
    }

    pub fn policy(&self) -> PolicyConfig<'_> {
        PolicyConfig {
            ignore_rules: &self.ignore_rules,
            ignore_paths: &self.ignore_paths,
            severity_overrides: &self.severity_overrides,
            suppressions: &self.suppressions,
            route_policy_overrides: &self.route_policy_overrides,
        }
    }

    pub fn rules(&self) -> RulesConfig<'_> {
        RulesConfig {
            checks: &self.checks,
            orphan_exclude: &self.orphan_exclude,
            repeatable_data_ui: &self.repeatable_data_ui,
            utility_route_patterns: &self.utility_route_patterns,
            min_inbound_links: self.min_inbound_links,
            link_suggestion_count: self.link_suggestion_count,
            enable_link_autofix: self.enable_link_autofix,
            related_links_heading: &self.related_links_heading,
            min_page_size: self.min_page_size,
            required_feature_markers: &self.required_feature_markers,
            min_block_text_length: self.min_block_text_length,
            min_answer_blocks: self.min_answer_blocks,
            require_fact_consistency: self.require_fact_consistency,
            required_schema_types: &self.required_schema_types,
            required_schema_families: &self.required_schema_families,
            require_breadcrumb_schema: self.require_breadcrumb_schema,
            require_schema_title_alignment: self.require_schema_title_alignment,
            require_html_lang: self.require_html_lang,
            require_hreflang_self: self.require_hreflang_self,
            require_meta_robots_consistency: self.require_meta_robots_consistency,
            require_open_graph: self.require_open_graph,
            require_twitter_card: self.require_twitter_card,
            default_twitter_card: &self.default_twitter_card,
            require_social_images: self.require_social_images,
            require_twitter_image: self.require_twitter_image,
            require_robots_sitemap: self.require_robots_sitemap,
            weak_anchor_text: &self.weak_anchor_text,
        }
    }

    pub fn output(&self) -> OutputConfig<'_> {
        OutputConfig {
            baseline_file: &self.baseline_file,
            audit_log_limit: self.audit_log_limit,
        }
    }

    pub fn quality(&self) -> QualityConfig<'_> {
        QualityConfig {
            typecheck_command: &self.typecheck_command,
            coverage_threshold: self.coverage_threshold,
            complexity_threshold: self.complexity_threshold,
            performance_budget_file: &self.performance_budget_file,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Config;

    #[test]
    fn exposes_domain_config_views() {
        let config = Config::default();
        assert_eq!(config.site().adapter, "auto");
        assert_eq!(config.runtime().browser_engine, "http");
        assert!(config.rules().checks.get("html").copied().unwrap_or(false));
        assert_eq!(config.output().baseline_file, ".seogeo-baseline.json");
        assert_eq!(config.quality().coverage_threshold, 85);
    }
}

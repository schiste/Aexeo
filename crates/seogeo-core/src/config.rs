use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

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

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct RoutePolicyOverride {
    pub pattern: String,
    #[serde(default)]
    pub allow_canonical_noindex: bool,
    #[serde(default)]
    pub allow_nofollow: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
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

fn default_source_dir() -> String {
    ".".to_string()
}
fn default_profile() -> String {
    "generic".to_string()
}
fn default_adapter() -> String {
    "auto".to_string()
}
fn default_canonical_style() -> String {
    "extensionless".to_string()
}
fn default_audit_log_limit() -> usize {
    5
}
fn default_browser_engine() -> String {
    "auto".to_string()
}
fn default_browser_wait_until() -> String {
    "networkidle".to_string()
}
fn default_baseline_file() -> String {
    ".seogeo-baseline.json".to_string()
}
fn default_max_workers() -> usize {
    4
}
fn default_enable_cache() -> bool {
    true
}
fn default_cache_dir() -> String {
    ".seogeo-cache".to_string()
}
fn default_cache_ttl_seconds() -> usize {
    3600
}
fn default_crawl_artifact_dir() -> String {
    ".seogeo-reports/crawl-artifacts".to_string()
}
fn default_crawl_use_sitemap() -> bool {
    true
}
fn default_orphan_exclude() -> Vec<String> {
    vec!["404.html".to_string()]
}
fn default_repeatable_data_ui() -> Vec<String> {
    vec![
        "card".to_string(),
        "item".to_string(),
        "entry".to_string(),
        "result".to_string(),
        "tile".to_string(),
        "row".to_string(),
        "skill-card".to_string(),
    ]
}
fn default_utility_route_patterns() -> Vec<String> {
    vec![
        "search".to_string(),
        "admin".to_string(),
        "preview".to_string(),
        "internal".to_string(),
    ]
}
fn default_min_inbound_links() -> usize {
    1
}
fn default_link_suggestion_count() -> usize {
    3
}
fn default_related_links_heading() -> String {
    "Related pages".to_string()
}
fn default_min_page_size() -> usize {
    500
}
fn default_required_feature_markers() -> Vec<String> {
    vec!["Related features".to_string()]
}
fn default_min_block_text_length() -> usize {
    120
}
fn default_min_answer_blocks() -> usize {
    2
}
fn default_require_fact_consistency() -> bool {
    true
}
fn default_require_schema_title_alignment() -> bool {
    true
}
fn default_require_html_lang() -> bool {
    true
}
fn default_require_meta_robots_consistency() -> bool {
    true
}
fn default_require_open_graph() -> bool {
    true
}
fn default_require_twitter_card() -> bool {
    true
}
fn default_default_twitter_card() -> String {
    "summary".to_string()
}
fn default_require_robots_sitemap() -> bool {
    true
}
fn default_weak_anchor_text() -> Vec<String> {
    vec![
        "click here".to_string(),
        "here".to_string(),
        "learn more".to_string(),
        "more".to_string(),
        "read more".to_string(),
    ]
}
fn default_typecheck_command() -> String {
    "cargo check".to_string()
}
fn default_coverage_threshold() -> usize {
    85
}
fn default_complexity_threshold() -> usize {
    12
}
fn default_performance_budget_file() -> String {
    "performance-budget.json".to_string()
}

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
            default_value: "`auto`",
            description: "Default crawl engine. `auto` prefers Playwright when available, otherwise HTTP fetch.",
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
            description: "Plugin-specific configuration grouped by plugin namespace.",
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

pub fn default_rule_switches() -> BTreeMap<&'static str, bool> {
    BTreeMap::from([
        ("html", true),
        ("links", true),
        ("sitemap", true),
        ("robots", true),
        ("social", true),
        ("schema", true),
        ("llm", true),
        ("content", true),
        ("structure", true),
    ])
}

fn default_checks() -> BTreeMap<String, bool> {
    default_rule_switches()
        .into_iter()
        .map(|(key, value)| (key.to_string(), value))
        .collect()
}

pub fn load_config(root: &Path, explicit_path: Option<&Path>) -> Result<Config> {
    let config_path = explicit_path
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("seogeo.toml"));
    if !config_path.exists() {
        return Ok(Config::default());
    }
    let text = fs::read_to_string(&config_path)
        .with_context(|| format!("failed to read config at {}", config_path.display()))?;
    let config = toml::from_str::<Config>(&text)
        .with_context(|| format!("failed to parse TOML config at {}", config_path.display()))?;
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::{default_rule_switches, load_config};
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
}

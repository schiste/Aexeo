use std::collections::BTreeMap;

pub(super) fn default_source_dir() -> String {
    ".".to_string()
}
pub(super) fn default_profile() -> String {
    "generic".to_string()
}
pub(super) fn default_adapter() -> String {
    "auto".to_string()
}
pub(super) fn default_canonical_style() -> String {
    "extensionless".to_string()
}
pub(super) fn default_audit_log_limit() -> usize {
    5
}
pub(super) fn default_browser_engine() -> String {
    "http".to_string()
}
pub(super) fn default_browser_wait_until() -> String {
    "networkidle".to_string()
}
pub(super) fn default_baseline_file() -> String {
    ".aexeo-baseline.json".to_string()
}
pub(super) fn default_max_workers() -> usize {
    4
}
pub(super) fn default_enable_cache() -> bool {
    true
}
pub(super) fn default_cache_dir() -> String {
    ".aexeo-cache".to_string()
}
pub(super) fn default_cache_ttl_seconds() -> usize {
    3600
}
pub(super) fn default_crawl_artifact_dir() -> String {
    ".aexeo-reports/crawl-artifacts".to_string()
}
pub(super) fn default_crawl_use_sitemap() -> bool {
    true
}
pub(super) fn default_orphan_exclude() -> Vec<String> {
    vec!["404.html".to_string()]
}
pub(super) fn default_repeatable_data_ui() -> Vec<String> {
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
pub(super) fn default_utility_route_patterns() -> Vec<String> {
    vec![
        "search".to_string(),
        "admin".to_string(),
        "preview".to_string(),
        "internal".to_string(),
    ]
}
pub(super) fn default_min_inbound_links() -> usize {
    1
}
pub(super) fn default_link_suggestion_count() -> usize {
    3
}
pub(super) fn default_related_links_heading() -> String {
    "Related pages".to_string()
}
pub(super) fn default_min_page_size() -> usize {
    500
}
pub(super) fn default_required_feature_markers() -> Vec<String> {
    vec!["Related features".to_string()]
}
pub(super) fn default_min_block_text_length() -> usize {
    120
}
pub(super) fn default_min_answer_blocks() -> usize {
    2
}
pub(super) fn default_require_fact_consistency() -> bool {
    true
}
pub(super) fn default_require_schema_title_alignment() -> bool {
    true
}
pub(super) fn default_require_html_lang() -> bool {
    true
}
pub(super) fn default_require_meta_robots_consistency() -> bool {
    true
}
pub(super) fn default_require_open_graph() -> bool {
    true
}
pub(super) fn default_require_twitter_card() -> bool {
    true
}
pub(super) fn default_default_twitter_card() -> String {
    "summary".to_string()
}
pub(super) fn default_require_social_images() -> bool {
    true
}
pub(super) fn default_require_robots_sitemap() -> bool {
    true
}
pub(super) fn default_weak_anchor_text() -> Vec<String> {
    vec![
        "click here".to_string(),
        "here".to_string(),
        "learn more".to_string(),
        "more".to_string(),
        "read more".to_string(),
    ]
}
pub(super) fn default_typecheck_command() -> String {
    "cargo check".to_string()
}
pub(super) fn default_coverage_threshold() -> usize {
    85
}
pub(super) fn default_complexity_threshold() -> usize {
    12
}
pub(super) fn default_performance_budget_file() -> String {
    "performance-budget.json".to_string()
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
        ("surfaces", true),
        ("content", true),
        ("structure", true),
    ])
}

pub(super) fn default_checks() -> BTreeMap<String, bool> {
    default_rule_switches()
        .into_iter()
        .map(|(key, value)| (key.to_string(), value))
        .collect()
}

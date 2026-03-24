use seogeo_contracts::Finding;
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::Path;

use crate::config::Config;
use crate::schema_rules::iter_schema_field_values;
use crate::site::{PageKind, Site, collapse_whitespace};

fn finding(
    rule_id: &str,
    message: impl Into<String>,
    path: &Path,
    line: usize,
    column: usize,
    severity: &str,
) -> Finding {
    Finding {
        rule_id: rule_id.to_string(),
        message: message.into(),
        path: path.to_string_lossy().into_owned(),
        line,
        column,
        severity: severity.to_string(),
        suggestion: None,
    }
}

fn apply_config_overrides(findings: Vec<Finding>, config: &Config) -> Vec<Finding> {
    findings
        .into_iter()
        .filter(|finding| {
            !config
                .ignore_rules
                .iter()
                .any(|rule| rule == &finding.rule_id)
        })
        .filter(|finding| {
            !config
                .ignore_paths
                .iter()
                .any(|pattern| finding.path.contains(pattern))
        })
        .map(|mut finding| {
            if let Some(severity) = config.severity_overrides.get(&finding.rule_id) {
                finding.severity = severity.clone();
            }
            finding
        })
        .collect()
}

fn normalize_fact_text(text: &str) -> String {
    collapse_whitespace(&text.to_ascii_lowercase())
}

fn block_visible_length(text: &str) -> usize {
    collapse_whitespace(text).len()
}

fn first_words(text: &str, count: usize) -> String {
    collapse_whitespace(text)
        .split_whitespace()
        .take(count)
        .map(|word| word.to_ascii_lowercase())
        .collect::<Vec<_>>()
        .join(" ")
}

fn looks_like_question(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.ends_with('?') {
        return true;
    }
    let lower = trimmed.to_ascii_lowercase();
    ["what", "why", "how", "when", "where", "who"]
        .iter()
        .any(|prefix| lower.starts_with(&format!("{} ", prefix)) || lower == *prefix)
}

fn is_repeatable_data_ui(data_ui: &str, config: &Config) -> bool {
    let normalized = data_ui.trim().to_ascii_lowercase();
    config.repeatable_data_ui.iter().any(|candidate| {
        let candidate = candidate.trim().to_ascii_lowercase();
        normalized == candidate
            || normalized.ends_with(&format!("-{}", candidate))
            || normalized.starts_with(&format!("{}-", candidate))
    })
}

fn is_supporting_data_ui(data_ui: &str) -> bool {
    let normalized = data_ui.trim().to_ascii_lowercase();
    [
        "hero",
        "showcase",
        "cta",
        "step",
        "proof",
        "screenshot",
        "banner",
        "stat",
        "stats",
        "metric",
        "metrics",
        "counter",
        "summary",
        "callout",
        "highlight",
        "badge",
        "promo",
        "overview",
    ]
    .iter()
    .any(|needle| normalized.contains(needle))
}

fn is_supporting_block(block: &crate::site::Block) -> bool {
    block
        .data_ui
        .as_deref()
        .map(is_supporting_data_ui)
        .unwrap_or(false)
}

fn has_source_cue(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    [
        "source",
        "sources",
        "reference",
        "references",
        "according to",
        "citation",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

fn looks_like_decorative_counter_block(block: &crate::site::Block) -> bool {
    let text = collapse_whitespace(&block.text);
    if text.len() > 120 || !contains_number(&text) {
        return false;
    }
    let lower = text.to_ascii_lowercase();
    [
        "feature",
        "features",
        "category",
        "categories",
        "tool",
        "tools",
        "tab",
        "tabs",
        "agent",
        "agents",
        "session",
        "sessions",
        "mcp",
        "count",
        "counts",
        "stat",
        "stats",
        "metric",
        "metrics",
        "status",
        "active",
        "inactive",
        "cost",
        "latency",
        "users",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

fn contains_number(text: &str) -> bool {
    text.chars().any(|ch| ch.is_ascii_digit())
}

fn primary_entity_names(page: &crate::site::Page) -> Vec<String> {
    let mut names = Vec::new();
    for block in &page.json_ld_blocks {
        let Ok(payload) = serde_json::from_str::<Value>(&block.raw) else {
            continue;
        };
        let top_level_objects: Vec<&Value> = match &payload {
            Value::Array(items) => items.iter().collect(),
            _ => vec![&payload],
        };
        for object in top_level_objects {
            let Value::Object(map) = object else {
                continue;
            };
            let object_types: Vec<&str> = match map.get("@type") {
                Some(Value::String(value)) => vec![value.as_str()],
                Some(Value::Array(items)) => items
                    .iter()
                    .filter_map(|item| match item {
                        Value::String(text) => Some(text.as_str()),
                        _ => None,
                    })
                    .collect(),
                _ => Vec::new(),
            };
            if !object_types.iter().any(|object_type| {
                matches!(
                    *object_type,
                    "SoftwareApplication" | "Product" | "Organization" | "WebSite"
                )
            }) {
                continue;
            }
            if let Some(Value::String(name)) = map.get("name") {
                let normalized = normalize_fact_text(name);
                if !normalized.is_empty() && !names.contains(&normalized) {
                    names.push(normalized);
                }
            }
        }
    }
    names
}

fn strip_primary_entities(text: &str, entities: &[String]) -> String {
    let mut normalized = normalize_fact_text(text);
    let mut ordered_entities = entities.to_vec();
    ordered_entities.sort_by_key(|entity| std::cmp::Reverse(entity.len()));
    for entity in ordered_entities {
        if entity.len() < 3 {
            continue;
        }
        normalized = collapse_whitespace(&normalized.replace(&entity, " "));
    }
    normalized
}

fn fact_values_conflict(left: &str, right: &str) -> bool {
    if left.is_empty()
        || right.is_empty()
        || left == right
        || left.contains(right)
        || right.contains(left)
    {
        return false;
    }
    let left_tokens: BTreeSet<&str> = left.split_whitespace().collect();
    let right_tokens: BTreeSet<&str> = right.split_whitespace().collect();
    let overlap = left_tokens.intersection(&right_tokens).count();
    let minimum_overlap = left_tokens.len().min(right_tokens.len()).min(2);
    overlap < minimum_overlap
}

fn strong_answer_block_count(page: &crate::site::Page, config: &Config) -> usize {
    page.blocks
        .iter()
        .filter(|block| {
            block.has_heading
                && block_visible_length(&block.text) >= config.min_block_text_length
                && !is_supporting_block(block)
        })
        .count()
}

fn collect_semantic_block_findings(
    page: &crate::site::Page,
    block: &crate::site::Block,
    config: &Config,
    seen_data_ui: &mut BTreeSet<String>,
    strong_answer_blocks: usize,
) -> (Vec<Finding>, Option<String>) {
    let mut findings = Vec::new();
    let mut answer_like = None;
    if block.tag == "section" && block.data_ui.is_none() {
        findings.push(finding(
            "GEO001",
            "<section> is missing data-ui",
            &page.path,
            block.line,
            block.column,
            "warning",
        ));
    }
    if block.tag == "article" && block.data_ui.is_none() {
        findings.push(finding(
            "GEO002",
            "<article> is missing data-ui",
            &page.path,
            block.line,
            block.column,
            "warning",
        ));
    }
    if block.tag == "section" && !block.has_heading {
        findings.push(finding(
            "GEO004",
            "<section> is missing a heading",
            &page.path,
            block.line,
            block.column,
            "warning",
        ));
    }
    if let Some(data_ui) = &block.data_ui {
        if is_repeatable_data_ui(data_ui, config) {
            let _ = seen_data_ui.insert(data_ui.clone());
        } else if !seen_data_ui.insert(data_ui.clone()) {
            findings.push(finding(
                "GEO003",
                format!("duplicate data-ui '{}' on page", data_ui),
                &page.path,
                block.line,
                block.column,
                "warning",
            ));
        }
    }
    if block.has_heading && block_visible_length(&block.text) < config.min_block_text_length {
        let is_supporting = is_supporting_block(block);
        let should_suppress = is_supporting && strong_answer_blocks > 0;
        if !should_suppress {
            findings.push(finding(
                "GEO007",
                "semantic block is too thin to answer a focused query",
                &page.path,
                block.line,
                block.column,
                "warning",
            ));
        }
    }
    if block.has_heading {
        answer_like = Some(block.text.clone());
    }
    (findings, answer_like)
}

fn collect_details_and_pre_findings(page: &crate::site::Page) -> Vec<Finding> {
    let mut findings = Vec::new();
    for details in &page.details_blocks {
        if !details.has_summary {
            findings.push(finding(
                "GEO005",
                "<details> is missing a <summary>",
                &page.path,
                details.line,
                details.column,
                "warning",
            ));
        }
    }
    for pre in &page.pre_blocks {
        if !pre.has_code {
            findings.push(finding(
                "GEO006",
                "<pre> is missing nested <code>",
                &page.path,
                pre.line,
                pre.column,
                "warning",
            ));
        }
    }
    findings
}

fn count_answer_blocks(page: &crate::site::Page, config: &Config) -> usize {
    let semantic_blocks = page
        .blocks
        .iter()
        .filter(|block| {
            block.has_heading && block_visible_length(&block.text) >= config.min_block_text_length
        })
        .count();
    semantic_blocks + page.details_blocks.len() + page.pre_blocks.len()
}

fn collect_answerability_findings(page: &crate::site::Page, config: &Config) -> Vec<Finding> {
    if matches!(
        page.page_kind,
        PageKind::Search
            | PageKind::Admin
            | PageKind::Legal
            | PageKind::Utility
            | PageKind::NotFound
            | PageKind::Feed
    ) {
        return Vec::new();
    }
    let answer_blocks = count_answer_blocks(page, config);
    let minimum = if page.page_kind == PageKind::Listing {
        1
    } else {
        config.min_answer_blocks
    };
    if answer_blocks < minimum {
        return vec![finding(
            "GEO008",
            format!(
                "page has only {} answer-oriented blocks; expected at least {}",
                answer_blocks, minimum
            ),
            &page.path,
            1,
            1,
            "warning",
        )];
    }
    Vec::new()
}

fn collect_citation_and_title_findings(page: &crate::site::Page) -> Vec<Finding> {
    let mut findings = Vec::new();
    if !matches!(
        page.page_kind,
        PageKind::Search
            | PageKind::Admin
            | PageKind::Utility
            | PageKind::Feed
            | PageKind::NotFound
            | PageKind::Home
            | PageKind::Listing
    ) && let Some(block) = page.blocks.iter().find(|block| {
        let visible_length = block_visible_length(&block.text);
        visible_length >= 80
            && !is_supporting_block(block)
            && !looks_like_decorative_counter_block(block)
            && contains_number(&block.text)
            && !has_source_cue(&block.text)
    }) {
        findings.push(finding(
            "GEO010",
            "page contains factual numeric claims without visible source or citation cues",
            &page.path,
            block.line,
            block.column,
            "warning",
        ));
    }
    if let Some(title) = &page.title
        && title.split_whitespace().count() < 2
        && !page.route.is_empty()
    {
        findings.push(finding(
            "GEO011",
            "page title is weakly disambiguated for retrieval",
            &page.path,
            1,
            1,
            "warning",
        ));
    }
    findings
}

fn collect_question_block_findings(page: &crate::site::Page, config: &Config) -> Vec<Finding> {
    if matches!(
        page.page_kind,
        PageKind::Search | PageKind::Admin | PageKind::Utility | PageKind::Feed
    ) {
        return Vec::new();
    }
    for block in &page.blocks {
        if looks_like_question(&block.text)
            && block_visible_length(&block.text) < config.min_block_text_length.max(80)
        {
            return vec![finding(
                "GEO012",
                "question-like block appears under-explained",
                &page.path,
                block.line,
                block.column,
                "warning",
            )];
        }
    }
    Vec::new()
}

fn collect_overlap_findings(
    page: &crate::site::Page,
    answer_like_blocks: &[String],
) -> Vec<Finding> {
    let mut prefixes = BTreeSet::new();
    for text in answer_like_blocks {
        let prefix = first_words(text, 8);
        if prefix.is_empty() {
            continue;
        }
        if !prefixes.insert(prefix) {
            return vec![finding(
                "GEO013",
                "page contains overlapping answer chunks that may reduce retrieval quality",
                &page.path,
                1,
                1,
                "warning",
            )];
        }
    }
    Vec::new()
}

fn collect_fact_consistency_findings(page: &crate::site::Page, config: &Config) -> Vec<Finding> {
    if !config.require_fact_consistency {
        return Vec::new();
    }
    if matches!(
        page.page_kind,
        PageKind::Search | PageKind::Admin | PageKind::Utility | PageKind::Feed
    ) {
        return Vec::new();
    }
    let mut fact_values = Vec::new();
    if let Some(title) = &page.title {
        fact_values.push(title.clone());
    }
    if let Some(first_h1) = page.h1_texts.first() {
        fact_values.push(first_h1.clone());
    }
    if let Some(og_title) = page.metadata("og:title") {
        fact_values.push(og_title.to_string());
    }
    for block in &page.json_ld_blocks {
        let Ok(payload) = serde_json::from_str::<Value>(&block.raw) else {
            continue;
        };
        if let Some(name) = iter_schema_field_values(&payload, "name")
            .into_iter()
            .next()
        {
            fact_values.push(name);
        }
        if let Some(headline) = iter_schema_field_values(&payload, "headline")
            .into_iter()
            .next()
        {
            fact_values.push(headline);
        }
    }
    let mut normalized_facts = Vec::new();
    let primary_entities = primary_entity_names(page);
    for value in fact_values {
        let normalized = normalize_fact_text(&value);
        let stripped = strip_primary_entities(&value, &primary_entities);
        if stripped.is_empty() && primary_entities.contains(&normalized) {
            continue;
        }
        let normalized = if stripped.is_empty() {
            normalized
        } else {
            stripped
        };
        if !normalized.is_empty() && !normalized_facts.contains(&normalized) {
            normalized_facts.push(normalized);
        }
    }
    if normalized_facts.len() >= 2 {
        let base = normalized_facts[0].clone();
        if normalized_facts[1..]
            .iter()
            .any(|value| fact_values_conflict(&base, value))
        {
            return vec![finding(
                "GEO009",
                "core page facts do not align across title, H1, OpenGraph, and schema",
                &page.path,
                1,
                1,
                "warning",
            )];
        }
    }
    Vec::new()
}

pub fn run_structure_rules(site: &Site, config: &Config) -> Vec<Finding> {
    let mut findings = Vec::new();
    for page in site.route_pages.values() {
        let mut seen_data_ui = BTreeSet::new();
        let mut answer_like_blocks = Vec::new();
        let strong_answer_blocks = strong_answer_block_count(page, config);
        for block in &page.blocks {
            let (block_findings, answer_like) = collect_semantic_block_findings(
                page,
                block,
                config,
                &mut seen_data_ui,
                strong_answer_blocks,
            );
            findings.extend(block_findings);
            if let Some(answer_like) = answer_like {
                answer_like_blocks.push(answer_like);
            }
        }
        findings.extend(collect_details_and_pre_findings(page));
        findings.extend(collect_answerability_findings(page, config));
        findings.extend(collect_citation_and_title_findings(page));
        findings.extend(collect_question_block_findings(page, config));
        findings.extend(collect_overlap_findings(page, &answer_like_blocks));
        findings.extend(collect_fact_consistency_findings(page, config));
    }
    apply_config_overrides(findings, config)
}

#[cfg(test)]
mod tests {
    use super::run_structure_rules;
    use crate::config::Config;
    use crate::site::load_site;
    use std::collections::BTreeSet;
    use std::fs;

    fn write(path: &std::path::Path, text: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, text).unwrap();
    }

    #[test]
    fn covers_structure_basics() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        write(
            &root.join("index.html"),
            r#"<html><head><title>x</title><meta name="description" content="y"><link rel="canonical" href="https://example.com/"></head><body><h1>x</h1><section><p>missing data-ui and heading</p></section><article><h2>article</h2></article><section data-ui="dup"><h2>first</h2></section><section data-ui="dup"><h2>second</h2></section><details><p>missing summary</p></details><pre>raw output</pre></body></html>"#,
        );
        let ids = run_structure_rules(&load_site(root).unwrap(), &Config::default())
            .into_iter()
            .map(|finding| finding.rule_id)
            .collect::<BTreeSet<_>>();
        assert!(ids.contains("GEO001"));
        assert!(ids.contains("GEO002"));
        assert!(ids.contains("GEO003"));
        assert!(ids.contains("GEO004"));
        assert!(ids.contains("GEO005"));
        assert!(ids.contains("GEO006"));
    }

    #[test]
    fn allows_repeatable_list_item_data_ui() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        write(
            &root.join("index.html"),
            r#"<html><head><title>x tool</title><meta name="description" content="y"><link rel="canonical" href="https://example.com/"></head><body><h1>x tool</h1><section data-ui="skill-card"><h2>One</h2><p>Long enough body text to satisfy the semantic block threshold for one repeated card component in a list view.</p></section><section data-ui="skill-card"><h2>Two</h2><p>Long enough body text to satisfy the semantic block threshold for another repeated card component in the same list view.</p></section></body></html>"#,
        );
        let ids = run_structure_rules(&load_site(root).unwrap(), &Config::default())
            .into_iter()
            .map(|finding| finding.rule_id)
            .collect::<BTreeSet<_>>();
        assert!(!ids.contains("GEO003"));
    }

    #[test]
    fn softens_geo007_on_supporting_blocks_when_answer_blocks_exist() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        write(
            &root.join("index.html"),
            r#"<html><head><title>x tool</title><meta name="description" content="y"><link rel="canonical" href="https://example.com/"></head><body><h1>x tool</h1><section data-ui="hero"><h2>Hero</h2><p>Short supporting intro.</p></section><section data-ui="showcase"><h2>Showcase</h2><p>Short supporting proof.</p></section><section data-ui="answer"><h2>Answer</h2><p>This block has enough body text to count as a real answer block for the page and should allow the supporting sections to stay compact without being flagged.</p></section><section data-ui="cta"><h2>Call to action</h2><p>Short supporting CTA.</p></section></body></html>"#,
        );
        let findings = run_structure_rules(&load_site(root).unwrap(), &Config::default());
        assert!(!findings.iter().any(|finding| finding.rule_id == "GEO007"));
    }

    #[test]
    fn still_flags_geo007_for_non_supporting_thin_blocks() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        write(
            &root.join("index.html"),
            r#"<html><head><title>x tool</title><meta name="description" content="y"><link rel="canonical" href="https://example.com/"></head><body><h1>x tool</h1><section data-ui="answer"><h2>Answer</h2><p>This block has enough body text to count as a real answer block for the page.</p></section><section data-ui="details"><h2>Details</h2><p>Short text.</p></section></body></html>"#,
        );
        let findings = run_structure_rules(&load_site(root).unwrap(), &Config::default());
        assert!(findings.iter().any(|finding| finding.rule_id == "GEO007"));
    }

    #[test]
    fn suppresses_geo010_on_decorative_counter_blocks() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        write(
            &root.join("features/test/index.html"),
            r#"<html><head><title>x tool</title><meta name="description" content="y"><link rel="canonical" href="https://example.com/features/test"></head><body><h1>x tool</h1><section data-ui="hero-stats"><h2>Numbers</h2><p>26 MCP tools, 4 resources, 8 telemetry surfaces, and 3 sessions monitored globally.</p></section><section data-ui="evidence"><h2>Evidence</h2><p>The benchmark showed 12% lower latency and 40% fewer retries across a long sustained load test window, but the page gives readers no linked study, benchmark record, or dataset to validate those numbers.</p></section></body></html>"#,
        );
        let findings = run_structure_rules(&load_site(root).unwrap(), &Config::default());
        let geo010_count = findings
            .iter()
            .filter(|finding| finding.rule_id == "GEO010")
            .count();
        assert_eq!(geo010_count, 1);
    }

    #[test]
    fn ignores_brand_layering_in_fact_consistency_checks() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        write(
            &root.join("features/terminal-search/index.html"),
            r#"<html><head><title>Terminal Search | Chau7 Terminal</title><meta name="description" content="Find text in terminal output"><meta property="og:title" content="Terminal Search | Chau7"><link rel="canonical" href="https://example.com/features/terminal-search"><script type="application/ld+json">[{"@context":"https://schema.org","@type":"SoftwareApplication","name":"Chau7","operatingSystem":"macOS"},{"@context":"https://schema.org","@type":"FAQPage","mainEntity":[{"@type":"Question","name":"What is Terminal Search?"}]}]</script></head><body><h1>Terminal Search</h1><section data-ui="answer"><h2>Overview</h2><p>This block has enough body text to count as a real answer block for the page while the schema keeps the product name at the application layer.</p></section></body></html>"#,
        );
        let findings = run_structure_rules(&load_site(root).unwrap(), &Config::default());
        assert!(!findings.iter().any(|finding| finding.rule_id == "GEO009"));
    }
}

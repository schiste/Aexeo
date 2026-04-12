use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

use crate::schema_rules::iter_schema_types;
use crate::site::{Page, PageKind, Site};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GroundingIntentFamily {
    Definition,
    Procedural,
    Comparison,
    Pricing,
    Troubleshooting,
    Asset,
    Feature,
    Reference,
    Generic,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GroundingIntentConfidence {
    Strong,
    Medium,
    Weak,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroundingIntentMatch {
    pub intent: GroundingIntentFamily,
    pub confidence: GroundingIntentConfidence,
    pub score: u8,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GroundingCoverageGap {
    MissingDirectAnswer,
    ThinAnswerCoverage,
    WeakComparisonStructure,
    MissingPricingSignals,
    MissingProceduralSignals,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroundingRouteAnalysis {
    pub route: String,
    pub page_kind: String,
    pub primary_topic: String,
    pub secondary_topics: Vec<String>,
    pub primary_intent: GroundingIntentFamily,
    pub secondary_intents: Vec<GroundingIntentFamily>,
    pub intents: Vec<GroundingIntentFamily>,
    pub intent_matches: Vec<GroundingIntentMatch>,
    pub schema_types: Vec<String>,
    pub signals: Vec<String>,
    pub coverage_gaps: Vec<GroundingCoverageGap>,
    pub answer_blocks: usize,
    pub heading_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroundingSiteAnalysis {
    pub pages_analyzed: usize,
    pub routes_with_topics: usize,
    pub intent_distribution: BTreeMap<String, usize>,
    pub primary_intent_distribution: BTreeMap<String, usize>,
    pub topic_clusters: BTreeMap<String, Vec<String>>,
    pub routes: Vec<GroundingRouteAnalysis>,
    pub elapsed_us: u64,
}

pub fn map_grounding_queries(site: &Site) -> GroundingSiteAnalysis {
    let started = Instant::now();
    let mut routes = site
        .route_pages()
        .map(analyze_page_grounding)
        .collect::<Vec<_>>();
    routes.sort_by(|left, right| left.route.cmp(&right.route));

    let mut intent_distribution = BTreeMap::new();
    let mut primary_intent_distribution = BTreeMap::new();
    let mut topic_clusters: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for route in &routes {
        for intent in &route.intents {
            *intent_distribution
                .entry(intent_label(intent).to_string())
                .or_insert(0) += 1;
        }
        *primary_intent_distribution
            .entry(intent_label(&route.primary_intent).to_string())
            .or_insert(0) += 1;
        topic_clusters
            .entry(route.primary_topic.clone())
            .or_default()
            .push(route.route.clone());
    }
    for values in topic_clusters.values_mut() {
        values.sort();
    }

    GroundingSiteAnalysis {
        pages_analyzed: routes.len(),
        routes_with_topics: routes
            .iter()
            .filter(|route| route.primary_topic != "(unclassified)")
            .count(),
        intent_distribution,
        primary_intent_distribution,
        topic_clusters,
        routes,
        elapsed_us: started.elapsed().as_micros() as u64,
    }
}

fn analyze_page_grounding(page: &Page) -> GroundingRouteAnalysis {
    let title = page.title.clone().unwrap_or_default();
    let h1 = page.h1_texts.first().cloned().unwrap_or_default();
    let h2s = capture_tag_texts(&page.raw_text, "h2");
    let h3s = capture_tag_texts(&page.raw_text, "h3");
    let schema_types = schema_types(page);

    let topics = infer_topics(page, &title, &h1, &h2s);
    let primary_topic = topics
        .first()
        .cloned()
        .unwrap_or_else(|| "(unclassified)".to_string());
    let secondary_topics = topics.iter().skip(1).take(4).cloned().collect::<Vec<_>>();

    let intent_matches = infer_intent_matches(page, &title, &h1, &h2s, &h3s, &schema_types);
    let intents = selected_intents(&intent_matches);
    let primary_intent = intents
        .first()
        .cloned()
        .unwrap_or(GroundingIntentFamily::Generic);
    let secondary_intents = intents.iter().skip(1).cloned().collect::<Vec<_>>();
    let coverage_gaps = infer_coverage_gaps(page, &intent_matches, &intents);

    GroundingRouteAnalysis {
        route: page.route.clone(),
        page_kind: format!("{:?}", page.page_kind).to_ascii_lowercase(),
        primary_topic,
        secondary_topics,
        primary_intent,
        secondary_intents,
        intents,
        intent_matches: intent_matches.clone(),
        schema_types,
        signals: intent_matches
            .iter()
            .flat_map(|item| item.reasons.iter().cloned())
            .collect(),
        coverage_gaps,
        answer_blocks: answer_block_count(page),
        heading_count: 1 + h2s.len() + h3s.len(),
    }
}

fn schema_types(page: &Page) -> Vec<String> {
    let mut types = BTreeSet::new();
    for block in &page.json_ld_blocks {
        if let Ok(payload) = serde_json::from_str::<serde_json::Value>(&block.raw) {
            for item in iter_schema_types(&payload) {
                if !item.is_empty() {
                    types.insert(item);
                }
            }
        }
    }
    types.into_iter().collect()
}

fn answer_block_count(page: &Page) -> usize {
    page.blocks
        .iter()
        .filter(|block| block.text.split_whitespace().count() >= 30)
        .count()
}

fn infer_intent_matches(
    page: &Page,
    title: &str,
    h1: &str,
    h2s: &[String],
    h3s: &[String],
    schema_types: &[String],
) -> Vec<GroundingIntentMatch> {
    let route = page.route.to_ascii_lowercase();
    let title_lower = title.to_ascii_lowercase();
    let h1_lower = h1.to_ascii_lowercase();
    let headings_lower = [h2s.join(" "), h3s.join(" ")]
        .join(" ")
        .to_ascii_lowercase();
    let body_lower = page.raw_text.to_ascii_lowercase();
    let answer_blocks = answer_block_count(page);
    let has_table = body_lower.contains("<table");
    let has_ordered_list = body_lower.contains("<ol");
    let has_faq_schema = schema_types.iter().any(|item| item == "FAQPage");
    let has_howto_schema = schema_types.iter().any(|item| item == "HowTo");
    let has_article_schema = schema_types
        .iter()
        .any(|item| matches!(item.as_str(), "TechArticle" | "Article"));

    let mut matches = Vec::new();

    let mut definition_score = 0_u8;
    let mut definition_reasons = Vec::new();
    if title_or_h1_contains(&title_lower, &h1_lower, &["what is", "overview"]) {
        definition_score += 5;
        definition_reasons.push("title_or_h1_definition_cue".to_string());
    }
    if headings_lower.contains("what is") {
        definition_score += 4;
        definition_reasons.push("heading_definition_cue".to_string());
    } else if headings_lower.contains("overview") {
        definition_score += 2;
        definition_reasons.push("heading_overview_cue".to_string());
    }
    if matches!(page.page_kind, PageKind::Docs | PageKind::Listing) {
        definition_score += 2;
        definition_reasons.push("page_kind_definition_bias".to_string());
    }
    if answer_blocks >= 2 {
        definition_score += 1;
        definition_reasons.push("sufficient_answer_blocks".to_string());
    }
    push_intent_match(
        &mut matches,
        GroundingIntentFamily::Definition,
        definition_score,
        definition_reasons,
    );

    let mut procedural_score = 0_u8;
    let mut procedural_reasons = Vec::new();
    if title_or_h1_contains(
        &title_lower,
        &h1_lower,
        &["how to", "setup", "configure", "install", "workflow"],
    ) {
        procedural_score += 5;
        procedural_reasons.push("title_or_h1_procedural_cue".to_string());
    }
    if headings_lower.contains("how to")
        || headings_lower.contains("step")
        || headings_lower.contains("setup")
        || headings_lower.contains("configure")
    {
        procedural_score += 2;
        procedural_reasons.push("heading_procedural_cue".to_string());
    }
    if body_lower.contains("step 1")
        || body_lower.contains("step one")
        || body_lower.contains("step-by-step")
    {
        procedural_score += 2;
        procedural_reasons.push("body_step_cue".to_string());
    }
    if has_ordered_list || has_howto_schema {
        procedural_score += 2;
        procedural_reasons.push("ordered_or_howto_structure".to_string());
    }
    push_intent_match(
        &mut matches,
        GroundingIntentFamily::Procedural,
        procedural_score,
        procedural_reasons,
    );

    let mut comparison_score = 0_u8;
    let mut comparison_reasons = Vec::new();
    if route.starts_with("compare/") || route.contains("/vs-") || route.contains("/vs/") {
        comparison_score += 5;
        comparison_reasons.push("route_comparison_cue".to_string());
    }
    if title_or_h1_contains(
        &title_lower,
        &h1_lower,
        &[" vs ", "versus", "compare", "alternative", "alternatives"],
    ) {
        comparison_score += 4;
        comparison_reasons.push("title_or_h1_comparison_cue".to_string());
    }
    if has_table {
        comparison_score += 2;
        comparison_reasons.push("table_structure".to_string());
    }
    push_intent_match(
        &mut matches,
        GroundingIntentFamily::Comparison,
        comparison_score,
        comparison_reasons,
    );

    let mut pricing_score = 0_u8;
    let mut pricing_reasons = Vec::new();
    if route.contains("pricing") || route.ends_with("/plans") {
        pricing_score += 5;
        pricing_reasons.push("route_pricing_cue".to_string());
    }
    if title_or_h1_contains(
        &title_lower,
        &h1_lower,
        &["pricing", "plans", "price", "quote"],
    ) {
        pricing_score += 4;
        pricing_reasons.push("title_or_h1_pricing_cue".to_string());
    }
    if headings_lower.contains("pricing")
        || headings_lower.contains("plans")
        || body_lower.contains("$")
        || body_lower.contains("€")
        || body_lower.contains(" per month")
        || body_lower.contains("/month")
    {
        pricing_score += 2;
        pricing_reasons.push("visible_pricing_signal".to_string());
    }
    push_intent_match(
        &mut matches,
        GroundingIntentFamily::Pricing,
        pricing_score,
        pricing_reasons,
    );

    let mut troubleshooting_score = 0_u8;
    let mut troubleshooting_reasons = Vec::new();
    if route.contains("troubleshoot") || route.contains("error") || route.contains("faq") {
        troubleshooting_score += 5;
        troubleshooting_reasons.push("route_support_cue".to_string());
    }
    if title_or_h1_contains(
        &title_lower,
        &h1_lower,
        &["troubleshoot", "fix", "error", "faq", "problem"],
    ) {
        troubleshooting_score += 4;
        troubleshooting_reasons.push("title_or_h1_support_cue".to_string());
    }
    if headings_lower.contains("faq") || headings_lower.contains("troubleshoot") {
        troubleshooting_score += 2;
        troubleshooting_reasons.push("heading_support_cue".to_string());
    }
    if has_faq_schema {
        troubleshooting_score += 1;
        troubleshooting_reasons.push("faq_schema".to_string());
    }
    push_intent_match(
        &mut matches,
        GroundingIntentFamily::Troubleshooting,
        troubleshooting_score,
        troubleshooting_reasons,
    );

    let mut asset_score = 0_u8;
    let mut asset_reasons = Vec::new();
    if title_or_h1_contains(
        &title_lower,
        &h1_lower,
        &[
            "template",
            "checklist",
            "worksheet",
            "download",
            "cheatsheet",
        ],
    ) {
        asset_score += 4;
        asset_reasons.push("title_or_h1_asset_cue".to_string());
    }
    if route.contains("template") || route.contains("checklist") || route.contains("download") {
        asset_score += 3;
        asset_reasons.push("route_asset_cue".to_string());
    }
    push_intent_match(
        &mut matches,
        GroundingIntentFamily::Asset,
        asset_score,
        asset_reasons,
    );

    let mut feature_score = 0_u8;
    let mut feature_reasons = Vec::new();
    if matches!(page.page_kind, PageKind::Detail) || route.starts_with("features/") {
        feature_score += 5;
        feature_reasons.push("feature_route_or_kind".to_string());
    }
    if title_or_h1_contains(&title_lower, &h1_lower, &["feature", "capability"]) {
        feature_score += 3;
        feature_reasons.push("title_or_h1_feature_cue".to_string());
    }
    push_intent_match(
        &mut matches,
        GroundingIntentFamily::Feature,
        feature_score,
        feature_reasons,
    );

    let mut reference_score = 0_u8;
    let mut reference_reasons = Vec::new();
    if matches!(page.page_kind, PageKind::Docs | PageKind::Legal) {
        reference_score += 4;
        reference_reasons.push("reference_page_kind".to_string());
    }
    if has_howto_schema || has_article_schema || has_faq_schema {
        reference_score += 2;
        reference_reasons.push("schema_reference_signal".to_string());
    }
    push_intent_match(
        &mut matches,
        GroundingIntentFamily::Reference,
        reference_score,
        reference_reasons,
    );

    if matches.is_empty() {
        matches.push(GroundingIntentMatch {
            intent: GroundingIntentFamily::Generic,
            confidence: GroundingIntentConfidence::Weak,
            score: 1,
            reasons: vec!["fallback_generic".to_string()],
        });
    }

    matches.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| intent_priority(&left.intent).cmp(&intent_priority(&right.intent)))
    });
    matches
}

fn infer_coverage_gaps(
    page: &Page,
    intent_matches: &[GroundingIntentMatch],
    intents: &[GroundingIntentFamily],
) -> Vec<GroundingCoverageGap> {
    let mut gaps = Vec::new();
    let text = page.raw_text.to_ascii_lowercase();
    let answer_blocks = answer_block_count(page);

    if answer_blocks < 2 {
        gaps.push(GroundingCoverageGap::ThinAnswerCoverage);
    }
    if intents.contains(&GroundingIntentFamily::Definition)
        && !text.contains("what is")
        && !text.contains("overview")
    {
        gaps.push(GroundingCoverageGap::MissingDirectAnswer);
    }
    if intents.contains(&GroundingIntentFamily::Procedural)
        && !text.contains("step")
        && !text.contains("<ol")
        && !text.contains("how to")
    {
        gaps.push(GroundingCoverageGap::MissingProceduralSignals);
    }
    if intents.contains(&GroundingIntentFamily::Comparison)
        && !text.contains("<table")
        && comparison_cue_count(&text) < 3
    {
        gaps.push(GroundingCoverageGap::WeakComparisonStructure);
    }
    if strongest_intent_score(intent_matches, GroundingIntentFamily::Pricing) >= 4
        && !has_visible_pricing_signals(&text)
    {
        gaps.push(GroundingCoverageGap::MissingPricingSignals);
    }

    gaps.sort_by_key(gap_order);
    gaps.dedup();
    gaps
}

fn selected_intents(matches: &[GroundingIntentMatch]) -> Vec<GroundingIntentFamily> {
    let mut selected = matches
        .iter()
        .filter(|item| item.score >= 4)
        .map(|item| item.intent.clone())
        .collect::<Vec<_>>();
    if selected.is_empty() {
        if let Some(best) = matches.first() {
            if best.score >= 3 {
                selected.push(best.intent.clone());
            } else {
                selected.push(GroundingIntentFamily::Generic);
            }
        } else {
            selected.push(GroundingIntentFamily::Generic);
        }
    }
    selected
}

fn push_intent_match(
    matches: &mut Vec<GroundingIntentMatch>,
    intent: GroundingIntentFamily,
    score: u8,
    reasons: Vec<String>,
) {
    if score == 0 {
        return;
    }
    let confidence = if score >= 5 {
        GroundingIntentConfidence::Strong
    } else if score >= 3 {
        GroundingIntentConfidence::Medium
    } else {
        GroundingIntentConfidence::Weak
    };
    matches.push(GroundingIntentMatch {
        intent,
        confidence,
        score,
        reasons,
    });
}

fn strongest_intent_score(matches: &[GroundingIntentMatch], intent: GroundingIntentFamily) -> u8 {
    matches
        .iter()
        .find(|item| item.intent == intent)
        .map(|item| item.score)
        .unwrap_or(0)
}

fn intent_priority(intent: &GroundingIntentFamily) -> u8 {
    match intent {
        GroundingIntentFamily::Comparison => 0,
        GroundingIntentFamily::Pricing => 1,
        GroundingIntentFamily::Procedural => 2,
        GroundingIntentFamily::Feature => 3,
        GroundingIntentFamily::Asset => 4,
        GroundingIntentFamily::Troubleshooting => 5,
        GroundingIntentFamily::Reference => 6,
        GroundingIntentFamily::Definition => 7,
        GroundingIntentFamily::Generic => 8,
    }
}

fn has_visible_pricing_signals(text: &str) -> bool {
    text.contains("$")
        || text.contains("€")
        || text.contains("pricing")
        || text.contains("plans")
        || text.contains("per month")
        || text.contains("/month")
}

fn title_or_h1_contains(title: &str, h1: &str, needles: &[&str]) -> bool {
    needles
        .iter()
        .any(|needle| title.contains(needle) || h1.contains(needle))
}

fn comparison_cue_count(text: &str) -> usize {
    [
        " versus ",
        " vs ",
        "alternative",
        "compare",
        "comparison",
        "<table",
    ]
    .into_iter()
    .filter(|cue| text.contains(cue))
    .count()
}

fn gap_order(gap: &GroundingCoverageGap) -> usize {
    match gap {
        GroundingCoverageGap::MissingDirectAnswer => 0,
        GroundingCoverageGap::ThinAnswerCoverage => 1,
        GroundingCoverageGap::WeakComparisonStructure => 2,
        GroundingCoverageGap::MissingPricingSignals => 3,
        GroundingCoverageGap::MissingProceduralSignals => 4,
    }
}

fn infer_topics(page: &Page, title: &str, h1: &str, h2s: &[String]) -> Vec<String> {
    let mut scores: BTreeMap<String, usize> = BTreeMap::new();
    for candidate in split_topic_candidates(title) {
        *scores.entry(candidate).or_default() += 5;
    }
    for candidate in split_topic_candidates(h1) {
        *scores.entry(candidate).or_default() += 4;
    }
    for candidate in split_topic_candidates(&page.route.replace(['/', '-'], " ")) {
        *scores.entry(candidate).or_default() += 3;
    }
    for heading in h2s {
        for candidate in split_topic_candidates(heading) {
            *scores.entry(candidate).or_default() += 2;
        }
    }

    let mut ranked = scores.into_iter().collect::<Vec<_>>();
    ranked.sort_by(|left, right| {
        right
            .1
            .cmp(&left.1)
            .then_with(|| {
                right
                    .0
                    .split_whitespace()
                    .count()
                    .cmp(&left.0.split_whitespace().count())
            })
            .then_with(|| left.0.cmp(&right.0))
    });
    ranked.into_iter().map(|(topic, _)| topic).take(5).collect()
}

fn split_topic_candidates(text: &str) -> Vec<String> {
    text.split(['|', ':', '•', '—', '-', ',', '(', ')'])
        .map(normalize_phrase)
        .filter(|candidate| {
            let words = candidate.split_whitespace().count();
            (2..=8).contains(&words) && !is_low_signal_phrase(candidate)
        })
        .collect()
}

fn normalize_phrase(text: &str) -> String {
    text.chars()
        .map(|ch| {
            if ch.is_alphanumeric() || ch.is_whitespace() {
                ch.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn is_low_signal_phrase(candidate: &str) -> bool {
    matches!(
        candidate,
        "" | "home" | "blog" | "docs" | "documentation" | "features" | "learn more" | "read more"
    )
}

fn capture_tag_texts(raw: &str, tag: &str) -> Vec<String> {
    let mut texts = Vec::new();
    let start_marker = format!("<{}", tag);
    let end_marker = format!("</{}>", tag);
    let mut offset = 0;
    while let Some(index) = raw[offset..].find(&start_marker) {
        let start = offset + index;
        let Some(open_end_rel) = raw[start..].find('>') else {
            break;
        };
        let open_end = start + open_end_rel;
        let Some(close_rel) = raw[open_end + 1..].find(&end_marker) else {
            break;
        };
        let close_start = open_end + 1 + close_rel;
        let text = raw[open_end + 1..close_start]
            .replace(['\n', '\r'], " ")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        let normalized = normalize_phrase(&text);
        if !normalized.is_empty() {
            texts.push(normalized);
        }
        offset = close_start + end_marker.len();
    }
    texts
}

fn intent_label(intent: &GroundingIntentFamily) -> &'static str {
    match intent {
        GroundingIntentFamily::Definition => "definition",
        GroundingIntentFamily::Procedural => "procedural",
        GroundingIntentFamily::Comparison => "comparison",
        GroundingIntentFamily::Pricing => "pricing",
        GroundingIntentFamily::Troubleshooting => "troubleshooting",
        GroundingIntentFamily::Asset => "asset",
        GroundingIntentFamily::Feature => "feature",
        GroundingIntentFamily::Reference => "reference",
        GroundingIntentFamily::Generic => "generic",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::site::load_site;
    use tempfile::tempdir;

    #[test]
    fn maps_grounding_topics_and_intents() {
        let temp = tempdir().unwrap();
        std::fs::write(
            temp.path().join("index.html"),
            r#"<html><head><title>Contract Lifecycle Management Software | Aexeo</title></head><body><h1>Contract lifecycle management software</h1><section data-ui="hero"><h2>What is contract lifecycle management</h2><p>Contract lifecycle management software helps legal teams control authoring, review, approval, and renewal.</p></section><section data-ui="steps"><h2>How to implement CLM</h2><p>Step 1 audit contracts. Step 2 configure workflows.</p></section></body></html>"#,
        )
        .unwrap();
        let site = load_site(temp.path()).unwrap();
        let report = map_grounding_queries(&site);
        assert_eq!(report.pages_analyzed, 1);
        let route = &report.routes[0];
        assert_eq!(
            route.primary_topic,
            "contract lifecycle management software"
        );
        assert!(matches!(
            route.primary_intent,
            GroundingIntentFamily::Definition | GroundingIntentFamily::Procedural
        ));
        assert!(route.intents.contains(&GroundingIntentFamily::Definition));
        assert!(route.intents.contains(&GroundingIntentFamily::Procedural));
    }

    #[test]
    fn flags_weak_comparison_structure_when_missing_comparison_signals() {
        let temp = tempdir().unwrap();
        std::fs::write(
            temp.path().join("compare.html"),
            r#"<html><head><title>Aexeo vs other tools</title></head><body><h1>Aexeo vs other tools</h1><section><h2>Overview</h2><p>Some text.</p></section></body></html>"#,
        )
        .unwrap();
        let site = load_site(temp.path()).unwrap();
        let route = &map_grounding_queries(&site).routes[0];
        assert_eq!(route.primary_intent, GroundingIntentFamily::Comparison);
        assert!(
            route
                .coverage_gaps
                .contains(&GroundingCoverageGap::WeakComparisonStructure)
        );
    }

    #[test]
    fn does_not_overclassify_pricing_from_generic_cost_language() {
        let temp = tempdir().unwrap();
        std::fs::write(
            temp.path().join("index.html"),
            r#"<html><head><title>Observe your agents</title></head><body><h1>Observe your agents</h1><section><h2>Cost visibility</h2><p>Understand agent cost drift across sessions.</p></section><section><h2>Analytics</h2><p>Operational telemetry for agents.</p></section></body></html>"#,
        )
        .unwrap();
        let site = load_site(temp.path()).unwrap();
        let route = &map_grounding_queries(&site).routes[0];
        assert!(!route.intents.contains(&GroundingIntentFamily::Pricing));
    }
}

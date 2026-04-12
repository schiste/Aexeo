use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

use crate::schema_rules::iter_schema_types;
use crate::site::{Page, PageKind, Site};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    pub intents: Vec<GroundingIntentFamily>,
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
    let mut topic_clusters: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for route in &routes {
        for intent in &route.intents {
            *intent_distribution
                .entry(intent_label(intent).to_string())
                .or_insert(0) += 1;
        }
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

    let intent_signals = build_intent_signals(page, &title, &h1, &h2s, &h3s, &schema_types);
    let intents = infer_intents(page, &intent_signals);
    let coverage_gaps = infer_coverage_gaps(page, &intents, &intent_signals);

    GroundingRouteAnalysis {
        route: page.route.clone(),
        page_kind: format!("{:?}", page.page_kind).to_ascii_lowercase(),
        primary_topic,
        secondary_topics,
        intents,
        schema_types,
        signals: intent_signals.into_iter().collect(),
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

fn build_intent_signals(
    page: &Page,
    title: &str,
    h1: &str,
    h2s: &[String],
    h3s: &[String],
    schema_types: &[String],
) -> BTreeSet<String> {
    let mut signals = BTreeSet::new();
    let route = page.route.to_ascii_lowercase();
    let combined = [title, h1, &h2s.join(" "), &h3s.join(" "), &page.raw_text]
        .join(" ")
        .to_ascii_lowercase();

    if combined.contains("what is ")
        || combined.contains("what’s ")
        || combined.contains("what is")
        || combined.contains("overview")
        || matches!(page.page_kind, PageKind::Docs | PageKind::Listing)
    {
        signals.insert("definition".to_string());
    }
    if combined.contains("how to")
        || combined.contains(" step ")
        || combined.contains(" tutorial")
        || combined.contains(" install")
        || combined.contains(" setup")
        || combined.contains(" configure")
    {
        signals.insert("procedural".to_string());
    }
    if route.contains("/vs")
        || combined.contains(" versus ")
        || combined.contains(" vs ")
        || combined.contains("alternative")
        || combined.contains("compare")
    {
        signals.insert("comparison".to_string());
    }
    if combined.contains("pricing")
        || combined.contains("price")
        || combined.contains("cost")
        || combined.contains("quote")
        || combined.contains("plan")
    {
        signals.insert("pricing".to_string());
    }
    if combined.contains("error")
        || combined.contains("issue")
        || combined.contains("fix")
        || combined.contains("troubleshoot")
        || combined.contains("faq")
    {
        signals.insert("troubleshooting".to_string());
    }
    if combined.contains("template")
        || combined.contains("checklist")
        || combined.contains("worksheet")
        || combined.contains("download")
    {
        signals.insert("asset".to_string());
    }
    if combined.contains("feature")
        || combined.contains("capability")
        || route.starts_with("features/")
    {
        signals.insert("feature".to_string());
    }
    if schema_types
        .iter()
        .any(|item| matches!(item.as_str(), "HowTo" | "TechArticle" | "FAQPage"))
    {
        signals.insert("reference".to_string());
    }
    if signals.is_empty() {
        signals.insert("generic".to_string());
    }
    signals
}

fn infer_intents(page: &Page, signals: &BTreeSet<String>) -> Vec<GroundingIntentFamily> {
    let mut intents = Vec::new();
    for signal in signals {
        match signal.as_str() {
            "definition" => intents.push(GroundingIntentFamily::Definition),
            "procedural" => intents.push(GroundingIntentFamily::Procedural),
            "comparison" => intents.push(GroundingIntentFamily::Comparison),
            "pricing" => intents.push(GroundingIntentFamily::Pricing),
            "troubleshooting" => intents.push(GroundingIntentFamily::Troubleshooting),
            "asset" => intents.push(GroundingIntentFamily::Asset),
            "feature" => intents.push(GroundingIntentFamily::Feature),
            "reference" => intents.push(GroundingIntentFamily::Reference),
            "generic" => intents.push(GroundingIntentFamily::Generic),
            _ => {}
        }
    }
    if intents.is_empty() {
        intents.push(match page.page_kind {
            PageKind::Docs => GroundingIntentFamily::Reference,
            PageKind::Listing => GroundingIntentFamily::Definition,
            PageKind::Detail => GroundingIntentFamily::Feature,
            _ => GroundingIntentFamily::Generic,
        });
    }
    intents
}

fn infer_coverage_gaps(
    page: &Page,
    intents: &[GroundingIntentFamily],
    signals: &BTreeSet<String>,
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
    if intents.contains(&GroundingIntentFamily::Pricing) && !signals.contains("pricing") {
        gaps.push(GroundingCoverageGap::MissingPricingSignals);
    }

    gaps.sort_by_key(gap_order);
    gaps.dedup();
    gaps
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
        assert!(
            route
                .coverage_gaps
                .contains(&GroundingCoverageGap::WeakComparisonStructure)
        );
    }
}

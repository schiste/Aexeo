use crate::time_shim::Instant;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

use super::grounding::{GroundingIntentFamily, GroundingRouteAnalysis, map_grounding_queries};
use crate::site::Site;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnswerFanoutRouteMatch {
    pub route: String,
    pub score: u8,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnswerFanoutQuery {
    pub query: String,
    pub family: String,
    pub topic: String,
    pub expected_surface: String,
    pub coverage_score: u8,
    pub matched_routes: Vec<AnswerFanoutRouteMatch>,
    pub gaps: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnswerFanoutReport {
    pub routes_analyzed: usize,
    pub query_count: usize,
    pub covered_queries: usize,
    pub coverage_score: u8,
    pub queries: Vec<AnswerFanoutQuery>,
    pub elapsed_us: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FanoutSeed {
    family: &'static str,
    topic: String,
    query: String,
    expected_surface: &'static str,
    preferred_intents: Vec<GroundingIntentFamily>,
}

fn normalized_topic(topic: &str) -> String {
    topic
        .trim()
        .trim_matches(|ch: char| !ch.is_ascii_alphanumeric() && ch != ' ')
        .to_ascii_lowercase()
}

fn topic_tokens(topic: &str) -> BTreeSet<String> {
    normalized_topic(topic)
        .split_whitespace()
        .filter(|part| part.len() > 2)
        .map(str::to_string)
        .collect()
}

fn route_topic_tokens(route: &GroundingRouteAnalysis) -> BTreeSet<String> {
    let mut tokens = topic_tokens(&route.primary_topic);
    for topic in &route.secondary_topics {
        tokens.extend(topic_tokens(topic));
    }
    tokens.extend(
        route
            .route
            .split(['/', '-', '_'])
            .filter(|part| part.len() > 2)
            .map(|part| part.to_ascii_lowercase()),
    );
    tokens
}

fn push_seed(
    seeds: &mut Vec<FanoutSeed>,
    seen: &mut BTreeSet<(String, String)>,
    family: &'static str,
    topic: &str,
    query: String,
    expected_surface: &'static str,
    preferred_intents: Vec<GroundingIntentFamily>,
) {
    let topic = normalized_topic(topic);
    if topic.is_empty() || topic == "(unclassified)" {
        return;
    }
    if seen.insert((family.to_string(), query.clone())) {
        seeds.push(FanoutSeed {
            family,
            topic,
            query,
            expected_surface,
            preferred_intents,
        });
    }
}

fn build_fanout_seeds(routes: &[GroundingRouteAnalysis]) -> Vec<FanoutSeed> {
    let mut seeds = Vec::new();
    let mut seen = BTreeSet::new();
    for route in routes {
        let topic = route.primary_topic.as_str();
        push_seed(
            &mut seeds,
            &mut seen,
            "definition",
            topic,
            format!("what is {}", normalized_topic(topic)),
            "direct answer block with schema-backed definition",
            vec![
                GroundingIntentFamily::Definition,
                GroundingIntentFamily::Reference,
            ],
        );
        push_seed(
            &mut seeds,
            &mut seen,
            "setup",
            topic,
            format!("how to set up {}", normalized_topic(topic)),
            "procedural page, ordered steps, or HowTo-like structure",
            vec![GroundingIntentFamily::Procedural],
        );
        push_seed(
            &mut seeds,
            &mut seen,
            "comparison",
            topic,
            format!("{} alternatives", normalized_topic(topic)),
            "comparison page, table, or alternatives section",
            vec![GroundingIntentFamily::Comparison],
        );
        push_seed(
            &mut seeds,
            &mut seen,
            "pricing",
            topic,
            format!("{} pricing", normalized_topic(topic)),
            "pricing page, offer schema, or clear plan/cost section",
            vec![GroundingIntentFamily::Pricing],
        );
        push_seed(
            &mut seeds,
            &mut seen,
            "trust",
            topic,
            format!("is {} secure", normalized_topic(topic)),
            "trust, security, evidence, or external validation page",
            vec![
                GroundingIntentFamily::Reference,
                GroundingIntentFamily::Feature,
            ],
        );
        push_seed(
            &mut seeds,
            &mut seen,
            "support",
            topic,
            format!("{} troubleshooting", normalized_topic(topic)),
            "support, FAQ, troubleshooting, or docs page",
            vec![
                GroundingIntentFamily::Troubleshooting,
                GroundingIntentFamily::Reference,
            ],
        );
    }
    seeds
}

fn score_route(
    seed: &FanoutSeed,
    route: &GroundingRouteAnalysis,
) -> Option<AnswerFanoutRouteMatch> {
    let seed_tokens = topic_tokens(&seed.topic);
    let route_tokens = route_topic_tokens(route);
    let shared = seed_tokens.intersection(&route_tokens).count();
    let mut score = 0_u8;
    let mut reasons = Vec::new();
    if normalized_topic(&route.primary_topic) == seed.topic {
        score = score.saturating_add(35);
        reasons.push("primary_topic_match".to_string());
    } else if shared > 0 {
        score = score.saturating_add((shared.min(3) as u8) * 10);
        reasons.push("topic_token_overlap".to_string());
    }
    if route
        .intents
        .iter()
        .any(|intent| seed.preferred_intents.contains(intent))
    {
        score = score.saturating_add(30);
        reasons.push("intent_match".to_string());
    }
    if route.answer_blocks >= 2 {
        score = score.saturating_add(15);
        reasons.push("answer_block_depth".to_string());
    } else if route.answer_blocks == 1 {
        score = score.saturating_add(8);
        reasons.push("single_answer_block".to_string());
    }
    if !route.schema_types.is_empty() {
        score = score.saturating_add(10);
        reasons.push("schema_context".to_string());
    }
    if route.heading_count >= 3 {
        score = score.saturating_add(5);
        reasons.push("section_structure".to_string());
    }
    if score == 0 {
        return None;
    }
    Some(AnswerFanoutRouteMatch {
        route: route.route.clone(),
        score: score.min(100),
        reasons,
    })
}

fn query_gaps(seed: &FanoutSeed, matched_routes: &[AnswerFanoutRouteMatch]) -> Vec<String> {
    if matched_routes.is_empty() {
        return vec![format!(
            "no route appears to answer the '{}' fan-out query",
            seed.family
        )];
    }
    let best = matched_routes[0].score;
    let mut gaps = Vec::new();
    if best < 60 {
        gaps.push(format!(
            "best route score is {}; add a stronger {} surface",
            best, seed.expected_surface
        ));
    }
    if !matched_routes[0]
        .reasons
        .iter()
        .any(|reason| reason == "intent_match")
    {
        gaps.push("matched route shares topic but lacks the expected search intent".to_string());
    }
    gaps
}

pub fn assess_answer_fanout(site: &Site) -> AnswerFanoutReport {
    let started = Instant::now();
    let grounding = map_grounding_queries(site);
    let seeds = build_fanout_seeds(&grounding.routes);
    let mut queries = Vec::new();
    for seed in seeds {
        let mut matched_routes = grounding
            .routes
            .iter()
            .filter_map(|route| score_route(&seed, route))
            .collect::<Vec<_>>();
        matched_routes.sort_by(|left, right| {
            right
                .score
                .cmp(&left.score)
                .then_with(|| left.route.cmp(&right.route))
        });
        matched_routes.truncate(5);
        let coverage_score = matched_routes.first().map(|item| item.score).unwrap_or(0);
        let gaps = query_gaps(&seed, &matched_routes);
        queries.push(AnswerFanoutQuery {
            query: seed.query,
            family: seed.family.to_string(),
            topic: seed.topic,
            expected_surface: seed.expected_surface.to_string(),
            coverage_score,
            matched_routes,
            gaps,
        });
    }
    queries.sort_by(|left, right| {
        left.coverage_score
            .cmp(&right.coverage_score)
            .then_with(|| left.family.cmp(&right.family))
            .then_with(|| left.query.cmp(&right.query))
    });
    let covered_queries = queries
        .iter()
        .filter(|query| query.coverage_score >= 60 && query.gaps.is_empty())
        .count();
    let coverage_score = if queries.is_empty() {
        0
    } else {
        ((covered_queries * 100) / queries.len()) as u8
    };
    AnswerFanoutReport {
        routes_analyzed: grounding.routes.len(),
        query_count: queries.len(),
        covered_queries,
        coverage_score,
        queries,
        elapsed_us: started.elapsed().as_micros() as u64,
    }
}

#[cfg(test)]
mod tests {
    use super::assess_answer_fanout;
    use crate::site::load_site;
    use std::fs;

    fn write(path: &std::path::Path, text: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, text).unwrap();
    }

    #[test]
    fn builds_deterministic_fanout_queries_from_site_topics() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        write(
            &root.join("index.html"),
            r#"<html><head><title>Acme Terminal Overview</title><meta name="description" content="Terminal overview"><script type="application/ld+json">{"@type":"SoftwareApplication","name":"Acme Terminal"}</script></head><body><h1>Acme Terminal</h1><section data-ui="overview"><h2>What is Acme Terminal?</h2><p>Acme Terminal is a secure terminal for teams with fast command workflows and deep automation. It helps engineering organizations run repeatable command operations with clear visibility and strong governance.</p></section><section data-ui="security"><h2>Security</h2><p>Acme Terminal protects sessions with auditable controls, secure team access, and enterprise governance for sensitive operational environments.</p></section></body></html>"#,
        );
        write(
            &root.join("compare/vs-legacy/index.html"),
            r#"<html><head><title>Acme Terminal alternatives</title><meta name="description" content="Compare terminal alternatives"></head><body><h1>Acme Terminal alternatives</h1><section data-ui="comparison"><h2>Comparison</h2><p>Compare Acme Terminal with legacy terminals across automation, collaboration, security, and governance for enterprise engineering teams.</p></section></body></html>"#,
        );
        let site = load_site(root).unwrap();
        let report = assess_answer_fanout(&site);
        assert!(report.query_count >= 6);
        assert!(
            report
                .queries
                .iter()
                .any(|query| query.family == "comparison")
        );
        assert!(
            report
                .queries
                .iter()
                .any(|query| query.query.contains("acme terminal"))
        );
    }
}

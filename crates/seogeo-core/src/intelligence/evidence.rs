use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::time::Instant;

use crate::site::Site;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceClaimKind {
    Numeric,
    Comparative,
    Temporal,
    Superlative,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceClaim {
    pub excerpt: String,
    pub kinds: Vec<EvidenceClaimKind>,
    pub supported: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceSectionAssessment {
    pub section_key: String,
    pub claim_count: usize,
    pub supported_claims: usize,
    pub unsupported_claims: usize,
    pub evidence_signals: Vec<String>,
    pub fidelity_risk_score: u8,
    pub claims: Vec<EvidenceClaim>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceRouteAssessment {
    pub route: String,
    pub sections: Vec<EvidenceSectionAssessment>,
    pub claim_count: usize,
    pub unsupported_claims: usize,
    pub evidence_density_score: u8,
    pub citation_readiness_score: u8,
    pub fidelity_risk_score: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceSiteAssessment {
    pub pages_analyzed: usize,
    pub routes_with_claims: usize,
    pub claim_count: usize,
    pub unsupported_claims: usize,
    pub claim_kind_distribution: BTreeMap<String, usize>,
    pub evidence_density_score: u8,
    pub citation_readiness_score: u8,
    pub average_fidelity_risk_score: u8,
    pub routes: Vec<EvidenceRouteAssessment>,
    pub elapsed_us: u64,
}

pub fn assess_evidence_coverage(site: &Site) -> EvidenceSiteAssessment {
    let started = Instant::now();
    let mut routes = site
        .route_pages()
        .map(assess_route)
        .collect::<Vec<EvidenceRouteAssessment>>();
    routes.sort_by(|left, right| {
        right
            .fidelity_risk_score
            .cmp(&left.fidelity_risk_score)
            .then_with(|| right.unsupported_claims.cmp(&left.unsupported_claims))
            .then_with(|| left.route.cmp(&right.route))
    });

    let mut kind_distribution = BTreeMap::new();
    let mut claim_count = 0;
    let mut unsupported_claims = 0;
    let mut evidence_density_total = 0_u64;
    let mut citation_readiness_total = 0_u64;
    let mut fidelity_total = 0_u64;
    let mut routes_with_claims = 0;

    for route in &routes {
        if route.claim_count > 0 {
            routes_with_claims += 1;
        }
        claim_count += route.claim_count;
        unsupported_claims += route.unsupported_claims;
        evidence_density_total += u64::from(route.evidence_density_score);
        citation_readiness_total += u64::from(route.citation_readiness_score);
        fidelity_total += u64::from(route.fidelity_risk_score);
        for section in &route.sections {
            for claim in &section.claims {
                for kind in &claim.kinds {
                    *kind_distribution
                        .entry(kind_label(kind).to_string())
                        .or_insert(0) += 1;
                }
            }
        }
    }

    let count = routes.len().max(1) as u64;
    EvidenceSiteAssessment {
        pages_analyzed: routes.len(),
        routes_with_claims,
        claim_count,
        unsupported_claims,
        claim_kind_distribution: kind_distribution,
        evidence_density_score: (evidence_density_total / count) as u8,
        citation_readiness_score: (citation_readiness_total / count) as u8,
        average_fidelity_risk_score: (fidelity_total / count) as u8,
        routes,
        elapsed_us: started.elapsed().as_micros() as u64,
    }
}

fn assess_route(page: &crate::site::Page) -> EvidenceRouteAssessment {
    let external_link_count = page
        .links
        .iter()
        .filter(|link| link.href.starts_with("http://") || link.href.starts_with("https://"))
        .filter(|link| link.target.is_none())
        .count();

    let sections = if page.blocks.is_empty() {
        vec![assess_section(
            "page".to_string(),
            &plain_text(&page.raw_text),
            external_link_count,
        )]
    } else {
        page.blocks
            .iter()
            .enumerate()
            .map(|(index, block)| {
                let key = block
                    .data_ui
                    .clone()
                    .filter(|value| !value.is_empty())
                    .unwrap_or_else(|| format!("{}:{}", block.tag, index + 1));
                assess_section(key, &block.text, external_link_count)
            })
            .collect::<Vec<_>>()
    };

    let claim_count = sections
        .iter()
        .map(|section| section.claim_count)
        .sum::<usize>();
    let unsupported_claims = sections
        .iter()
        .map(|section| section.unsupported_claims)
        .sum::<usize>();
    let evidence_density_score = if claim_count == 0 {
        if page.blocks.is_empty() { 75 } else { 90 }
    } else {
        (((claim_count.saturating_sub(unsupported_claims)) * 100) / claim_count) as u8
    };
    let citation_readiness_score = evidence_density_score
        .saturating_sub((page.blocks.len() < 2) as u8 * 10)
        .saturating_add((external_link_count > 0) as u8 * 5)
        .min(100);
    let fidelity_risk_score = sections
        .iter()
        .map(|section| section.fidelity_risk_score)
        .max()
        .unwrap_or(0);

    EvidenceRouteAssessment {
        route: page.route.clone(),
        sections,
        claim_count,
        unsupported_claims,
        evidence_density_score,
        citation_readiness_score,
        fidelity_risk_score,
    }
}

fn assess_section(
    section_key: String,
    text: &str,
    external_link_count: usize,
) -> EvidenceSectionAssessment {
    let evidence_signals = evidence_signals(text, external_link_count);
    let sentences = split_sentences(text);
    let mut claims = Vec::new();
    for sentence in sentences {
        let kinds = detect_claim_kinds(&sentence);
        if kinds.is_empty() {
            continue;
        }
        let supported = !evidence_signals.is_empty();
        claims.push(EvidenceClaim {
            excerpt: truncate_excerpt(&sentence),
            kinds,
            supported,
        });
    }
    let claim_count = claims.len();
    let supported_claims = claims.iter().filter(|claim| claim.supported).count();
    let unsupported_claims = claim_count.saturating_sub(supported_claims);
    let fidelity_risk_score = compute_fidelity_risk(text, &claims, evidence_signals.is_empty());

    EvidenceSectionAssessment {
        section_key,
        claim_count,
        supported_claims,
        unsupported_claims,
        evidence_signals,
        fidelity_risk_score,
        claims,
    }
}

fn evidence_signals(text: &str, external_link_count: usize) -> Vec<String> {
    let normalized = text.to_ascii_lowercase();
    let mut signals = Vec::new();
    for (needle, label) in [
        ("according to", "attribution_phrase"),
        ("source:", "explicit_source_label"),
        ("study", "study_reference"),
        ("survey", "survey_reference"),
        ("benchmark", "benchmark_reference"),
        ("report", "report_reference"),
        ("research", "research_reference"),
        ("data from", "data_source_phrase"),
        ("as of ", "dated_qualifier"),
    ] {
        if normalized.contains(needle) {
            signals.push(label.to_string());
        }
    }
    if external_link_count > 0 {
        signals.push("external_reference_links".to_string());
    }
    signals.sort();
    signals.dedup();
    signals
}

fn detect_claim_kinds(sentence: &str) -> Vec<EvidenceClaimKind> {
    let normalized = sentence.to_ascii_lowercase();
    let mut kinds = Vec::new();
    if sentence.chars().any(|ch| ch.is_ascii_digit()) || normalized.contains('%') {
        kinds.push(EvidenceClaimKind::Numeric);
    }
    if [
        "faster", "better", "lower", "higher", "more ", "less ", " than ", " vs ",
    ]
    .iter()
    .any(|needle| normalized.contains(needle))
    {
        kinds.push(EvidenceClaimKind::Comparative);
    }
    if contains_year(&normalized)
        || ["today", "currently", "now", "as of"]
            .iter()
            .any(|needle| normalized.contains(needle))
    {
        kinds.push(EvidenceClaimKind::Temporal);
    }
    if [
        "best", "fastest", "most", "least", "only", "leading", "first",
    ]
    .iter()
    .any(|needle| normalized.contains(needle))
    {
        kinds.push(EvidenceClaimKind::Superlative);
    }
    kinds
}

fn compute_fidelity_risk(text: &str, claims: &[EvidenceClaim], no_evidence_signals: bool) -> u8 {
    if claims.is_empty() {
        return 0;
    }
    let mut risk = 0_u8;
    let words = text.split_whitespace().count();
    let unsupported_claims = claims.iter().filter(|claim| !claim.supported).count() as u8;
    risk = risk.saturating_add(unsupported_claims.saturating_mul(18));
    if no_evidence_signals {
        risk = risk.saturating_add(12);
    }
    if words > 120 {
        risk = risk.saturating_add(10);
    }
    if claims.len() > 3 {
        risk = risk.saturating_add(10);
    }
    if claims.iter().any(|claim| {
        claim.kinds.iter().any(|kind| {
            matches!(
                kind,
                EvidenceClaimKind::Comparative | EvidenceClaimKind::Superlative
            )
        }) && !claim.supported
    }) {
        risk = risk.saturating_add(20);
    }
    risk.min(100)
}

fn contains_year(text: &str) -> bool {
    for token in text.split(|ch: char| !ch.is_ascii_alphanumeric()) {
        if token.len() == 4
            && token.starts_with("20")
            && token.chars().all(|ch| ch.is_ascii_digit())
        {
            return true;
        }
    }
    false
}

fn split_sentences(text: &str) -> Vec<String> {
    text.split(['.', '!', '?', '\n'])
        .map(str::trim)
        .filter(|item| item.split_whitespace().count() >= 5)
        .map(ToString::to_string)
        .collect()
}

fn truncate_excerpt(text: &str) -> String {
    const LIMIT: usize = 120;
    if text.chars().count() <= LIMIT {
        return text.to_string();
    }
    let truncated = text.chars().take(LIMIT).collect::<String>();
    format!("{truncated}...")
}

fn plain_text(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    for ch in html.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => {
                in_tag = false;
                out.push(' ');
            }
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn kind_label(kind: &EvidenceClaimKind) -> &'static str {
    match kind {
        EvidenceClaimKind::Numeric => "numeric",
        EvidenceClaimKind::Comparative => "comparative",
        EvidenceClaimKind::Temporal => "temporal",
        EvidenceClaimKind::Superlative => "superlative",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::site::load_site;
    use tempfile::tempdir;

    #[test]
    fn evidence_assessment_detects_supported_and_unsupported_claims() {
        let temp = tempdir().unwrap();
        std::fs::write(
            temp.path().join("index.html"),
            r#"<html><head><title>Aexeo benchmarks</title></head><body><section data-ui="hero"><h2>Results</h2><p>Aexeo reduced audit time by 42% in 2026 according to our benchmark report.</p><a href="https://example.com/report">report</a></section><section data-ui="claim"><p>Aexeo is the fastest platform for GEO teams.</p></section></body></html>"#,
        )
        .unwrap();
        let site = load_site(temp.path()).unwrap();
        let report = assess_evidence_coverage(&site);
        assert_eq!(report.pages_analyzed, 1);
        assert_eq!(report.claim_count, 2);
        assert_eq!(report.unsupported_claims, 0);
        assert!(report.citation_readiness_score > 50);
    }

    #[test]
    fn evidence_assessment_flags_unattributed_claim_risk() {
        let temp = tempdir().unwrap();
        std::fs::write(
            temp.path().join("index.html"),
            r#"<html><head><title>Claims</title></head><body><section><p>The best GEO platform improves results by 80% and is faster than every alternative.</p></section></body></html>"#,
        )
        .unwrap();
        let site = load_site(temp.path()).unwrap();
        let report = assess_evidence_coverage(&site);
        assert_eq!(report.unsupported_claims, 1);
        assert!(report.routes[0].fidelity_risk_score >= 40);
    }
}

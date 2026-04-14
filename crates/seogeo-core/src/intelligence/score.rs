use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::time::Instant;

use super::{
    EvidenceSiteAssessment, GroundingCoverageGap, GroundingSiteAnalysis, TrustSurfaceIssueKind,
    TrustSurfaceReconciliation, TruthAssessment,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntelligenceBlocker {
    pub category: String,
    pub route: Option<String>,
    pub severity: u8,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteIntelligenceScore {
    pub route: String,
    pub citation_readiness_score: u8,
    pub truth_consistency_score: u8,
    pub answer_pack_score: u8,
    pub external_trust_alignment_score: Option<u8>,
    pub overall_score: u8,
    pub blockers: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SiteIntelligenceScore {
    pub citation_readiness_score: u8,
    pub truth_consistency_score: u8,
    pub answer_pack_score: u8,
    pub external_trust_alignment_score: Option<u8>,
    pub overall_score: u8,
    pub route_scores: Vec<RouteIntelligenceScore>,
    pub blockers: Vec<IntelligenceBlocker>,
    pub elapsed_us: u64,
}

pub fn score_intelligence(
    grounding: &GroundingSiteAnalysis,
    truth: &TruthAssessment,
    evidence: &EvidenceSiteAssessment,
    trust: Option<&TrustSurfaceReconciliation>,
) -> SiteIntelligenceScore {
    let started = Instant::now();
    let citation_readiness_score = evidence.citation_readiness_score;
    let truth_consistency_score = truth.score;
    let answer_pack_score = answer_pack_score(grounding);
    let external_trust_alignment_score = trust.map(trust_alignment_score);

    let route_scores = build_route_scores(grounding, truth, evidence, trust);
    let blockers = collect_blockers(grounding, truth, evidence, trust);
    let overall_score = weighted_overall_score(
        citation_readiness_score,
        truth_consistency_score,
        answer_pack_score,
        external_trust_alignment_score,
    );

    SiteIntelligenceScore {
        citation_readiness_score,
        truth_consistency_score,
        answer_pack_score,
        external_trust_alignment_score,
        overall_score,
        route_scores,
        blockers,
        elapsed_us: started.elapsed().as_micros() as u64,
    }
}

fn build_route_scores(
    grounding: &GroundingSiteAnalysis,
    truth: &TruthAssessment,
    evidence: &EvidenceSiteAssessment,
    trust: Option<&TrustSurfaceReconciliation>,
) -> Vec<RouteIntelligenceScore> {
    let grounding_by_route = grounding
        .routes
        .iter()
        .map(|route| (route.route.as_str(), route))
        .collect::<BTreeMap<_, _>>();
    let evidence_by_route = evidence
        .routes
        .iter()
        .map(|route| (route.route.as_str(), route))
        .collect::<BTreeMap<_, _>>();
    let mut trust_penalties = BTreeMap::<String, u8>::new();
    if let Some(trust) = trust {
        for issue in &trust.issues {
            if let Some(route) = &issue.route {
                *trust_penalties.entry(route.clone()).or_insert(100) = trust_penalty(issue.kind());
            }
        }
    }

    let mut routes = grounding
        .routes
        .iter()
        .map(|grounding_route| {
            let evidence_route = evidence_by_route
                .get(grounding_route.route.as_str())
                .copied();
            let citation_readiness_score = evidence_route
                .map(|route| route.citation_readiness_score)
                .unwrap_or(75);
            let answer_pack_score = route_answer_pack_score(grounding_route);
            let truth_consistency_score = route_truth_score(&grounding_route.route, truth);
            let external_trust_alignment_score =
                trust.map(|_| *trust_penalties.get(&grounding_route.route).unwrap_or(&100));
            let overall_score = weighted_overall_score(
                citation_readiness_score,
                truth_consistency_score,
                answer_pack_score,
                external_trust_alignment_score,
            );
            let mut blockers = Vec::new();
            if let Some(evidence_route) = evidence_route
                && evidence_route.unsupported_claims > 0
            {
                blockers.push(format!(
                    "{} unsupported claims",
                    evidence_route.unsupported_claims
                ));
            }
            for gap in &grounding_route.coverage_gaps {
                blockers.push(gap_label(gap).to_string());
            }
            if route_truth_score(&grounding_route.route, truth) < 80 {
                blockers.push("truth_consistency_gap".to_string());
            }
            if let Some(score) = external_trust_alignment_score
                && score < 100
            {
                blockers.push("external_trust_alignment_gap".to_string());
            }
            blockers.sort();
            blockers.dedup();
            RouteIntelligenceScore {
                route: grounding_route.route.clone(),
                citation_readiness_score,
                truth_consistency_score,
                answer_pack_score,
                external_trust_alignment_score,
                overall_score,
                blockers,
            }
        })
        .collect::<Vec<_>>();

    for route in evidence
        .routes
        .iter()
        .filter(|route| !grounding_by_route.contains_key(route.route.as_str()))
    {
        routes.push(RouteIntelligenceScore {
            route: route.route.clone(),
            citation_readiness_score: route.citation_readiness_score,
            truth_consistency_score: truth.score,
            answer_pack_score: 75,
            external_trust_alignment_score: trust.map(|_| 100),
            overall_score: weighted_overall_score(
                route.citation_readiness_score,
                truth.score,
                75,
                trust.map(|_| 100),
            ),
            blockers: if route.unsupported_claims > 0 {
                vec![format!("{} unsupported claims", route.unsupported_claims)]
            } else {
                Vec::new()
            },
        });
    }

    routes.sort_by(|left, right| {
        left.overall_score
            .cmp(&right.overall_score)
            .then_with(|| right.blockers.len().cmp(&left.blockers.len()))
            .then_with(|| left.route.cmp(&right.route))
    });
    routes
}

fn collect_blockers(
    grounding: &GroundingSiteAnalysis,
    truth: &TruthAssessment,
    evidence: &EvidenceSiteAssessment,
    trust: Option<&TrustSurfaceReconciliation>,
) -> Vec<IntelligenceBlocker> {
    let mut blockers = Vec::new();
    for mismatch in &truth.mismatches {
        blockers.push(IntelligenceBlocker {
            category: "truth".to_string(),
            route: Some(mismatch.route.clone()),
            severity: match mismatch.severity {
                super::TruthMismatchSeverity::Error => 90,
                super::TruthMismatchSeverity::Warning => 65,
            },
            message: format!(
                "{} expected '{}' but observed '{}'",
                mismatch.field, mismatch.expected, mismatch.observed
            ),
        });
    }
    for route in &evidence.routes {
        if route.unsupported_claims > 0 {
            blockers.push(IntelligenceBlocker {
                category: "evidence".to_string(),
                route: Some(route.route.clone()),
                severity: 80,
                message: format!(
                    "{} unsupported claims and fidelity risk {}",
                    route.unsupported_claims, route.fidelity_risk_score
                ),
            });
        }
    }
    for route in &grounding.routes {
        for gap in &route.coverage_gaps {
            blockers.push(IntelligenceBlocker {
                category: "answer_pack".to_string(),
                route: Some(route.route.clone()),
                severity: gap_severity(gap),
                message: gap_label(gap).to_string(),
            });
        }
    }
    if let Some(trust) = trust {
        for issue in &trust.issues {
            blockers.push(IntelligenceBlocker {
                category: "trust".to_string(),
                route: issue.route.clone(),
                severity: trust_penalty(issue.kind()).saturating_sub(10),
                message: issue.message.clone(),
            });
        }
    }
    blockers.sort_by(|left, right| {
        right
            .severity
            .cmp(&left.severity)
            .then_with(|| left.category.cmp(&right.category))
            .then_with(|| left.route.cmp(&right.route))
    });
    blockers.truncate(25);
    blockers
}

fn answer_pack_score(grounding: &GroundingSiteAnalysis) -> u8 {
    let total = grounding
        .routes
        .iter()
        .map(|route| u64::from(route_answer_pack_score(route)))
        .sum::<u64>();
    (total / grounding.routes.len().max(1) as u64) as u8
}

fn route_answer_pack_score(route: &super::GroundingRouteAnalysis) -> u8 {
    let mut score = route_answer_pack_baseline(route.primary_intent.clone());

    score = score.saturating_add(match route.answer_blocks {
        0 => 0,
        1 => 4,
        2..=3 => 9,
        _ => 12,
    });

    score = score.saturating_add(match route.heading_count {
        0..=1 => 0,
        2 => 3,
        3..=4 => 6,
        _ => 8,
    });

    score = score.saturating_add(match route.secondary_intents.len() {
        0 => 0,
        1 => 2,
        _ => 4,
    });

    if route.page_kind == "detail" && route.answer_blocks >= 2 {
        score = score.saturating_add(4);
    }

    for gap in &route.coverage_gaps {
        score = score.saturating_sub(match gap {
            GroundingCoverageGap::MissingDirectAnswer => 20,
            GroundingCoverageGap::ThinAnswerCoverage => 15,
            GroundingCoverageGap::WeakComparisonStructure => 15,
            GroundingCoverageGap::MissingPricingSignals => 10,
            GroundingCoverageGap::MissingProceduralSignals => 10,
        });
    }

    score.min(100)
}

fn route_answer_pack_baseline(intent: super::GroundingIntentFamily) -> u8 {
    match intent {
        super::GroundingIntentFamily::Feature => 78,
        super::GroundingIntentFamily::Comparison => 76,
        super::GroundingIntentFamily::Definition => 74,
        super::GroundingIntentFamily::Procedural => 74,
        super::GroundingIntentFamily::Pricing => 74,
        super::GroundingIntentFamily::Troubleshooting => 72,
        super::GroundingIntentFamily::Asset => 72,
        super::GroundingIntentFamily::Reference => 68,
        super::GroundingIntentFamily::Generic => 64,
    }
}

fn route_truth_score(route: &str, truth: &TruthAssessment) -> u8 {
    let penalties = truth
        .mismatches
        .iter()
        .filter(|mismatch| mismatch.route == route)
        .count()
        .min(4) as u8;
    truth.score.saturating_sub(penalties * 10)
}

fn trust_alignment_score(report: &TrustSurfaceReconciliation) -> u8 {
    if report.rows_read == 0 {
        return 100;
    }
    let match_ratio = ((report.matched_first_party_routes * 100) / report.rows_read) as u8;
    let issue_penalty = (report.issues.len().min(10) * 7) as u8;
    match_ratio.saturating_sub(issue_penalty).max(40)
}

fn weighted_overall_score(
    citation_readiness_score: u8,
    truth_consistency_score: u8,
    answer_pack_score: u8,
    external_trust_alignment_score: Option<u8>,
) -> u8 {
    match external_trust_alignment_score {
        Some(trust) => {
            ((u32::from(citation_readiness_score) * 35
                + u32::from(truth_consistency_score) * 30
                + u32::from(answer_pack_score) * 25
                + u32::from(trust) * 10)
                / 100) as u8
        }
        None => {
            ((u32::from(citation_readiness_score) * 40
                + u32::from(truth_consistency_score) * 35
                + u32::from(answer_pack_score) * 25)
                / 100) as u8
        }
    }
}

fn gap_label(gap: &GroundingCoverageGap) -> &'static str {
    match gap {
        GroundingCoverageGap::MissingDirectAnswer => "missing_direct_answer",
        GroundingCoverageGap::ThinAnswerCoverage => "thin_answer_coverage",
        GroundingCoverageGap::WeakComparisonStructure => "weak_comparison_structure",
        GroundingCoverageGap::MissingPricingSignals => "missing_pricing_signals",
        GroundingCoverageGap::MissingProceduralSignals => "missing_procedural_signals",
    }
}

fn gap_severity(gap: &GroundingCoverageGap) -> u8 {
    match gap {
        GroundingCoverageGap::MissingDirectAnswer => 75,
        GroundingCoverageGap::ThinAnswerCoverage => 65,
        GroundingCoverageGap::WeakComparisonStructure => 70,
        GroundingCoverageGap::MissingPricingSignals => 55,
        GroundingCoverageGap::MissingProceduralSignals => 55,
    }
}

trait TrustIssueKindExt {
    fn kind(&self) -> TrustSurfaceIssueKind;
}

impl TrustIssueKindExt for super::TrustSurfaceIssue {
    fn kind(&self) -> TrustSurfaceIssueKind {
        self.kind.clone()
    }
}

fn trust_penalty(kind: TrustSurfaceIssueKind) -> u8 {
    match kind {
        TrustSurfaceIssueKind::RouteNotInSite => 45,
        TrustSurfaceIssueKind::MissingCanonicalEntity => 55,
        TrustSurfaceIssueKind::ForbiddenTerminology => 35,
        TrustSurfaceIssueKind::DescriptorGap => 70,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intelligence::{
        GroundingIntentFamily, GroundingRouteAnalysis, TruthMismatch, TruthMismatchSeverity,
        TruthStructuredSource,
    };

    #[test]
    fn computes_route_and_site_scores() {
        let grounding = GroundingSiteAnalysis {
            pages_analyzed: 1,
            routes_with_topics: 1,
            intent_distribution: BTreeMap::new(),
            primary_intent_distribution: BTreeMap::new(),
            topic_clusters: BTreeMap::new(),
            routes: vec![GroundingRouteAnalysis {
                route: "pricing".to_string(),
                page_kind: "detail".to_string(),
                primary_topic: "pricing".to_string(),
                secondary_topics: Vec::new(),
                primary_intent: GroundingIntentFamily::Pricing,
                secondary_intents: Vec::new(),
                intents: vec![GroundingIntentFamily::Pricing],
                intent_matches: Vec::new(),
                schema_types: Vec::new(),
                signals: Vec::new(),
                coverage_gaps: vec![GroundingCoverageGap::MissingPricingSignals],
                answer_blocks: 1,
                heading_count: 1,
            }],
            elapsed_us: 0,
        };
        let truth = TruthAssessment {
            structured_truth_source: TruthStructuredSource::Schema,
            structured_truth_prerequisite_met: true,
            score: 82,
            score_ceiling: 79,
            pages_analyzed: 1,
            pages_with_schema: 1,
            manifest_present: false,
            organization_schema_pages: 1,
            website_schema_pages: 1,
            preferred_term_hits: 0,
            forbidden_term_hits: 0,
            mismatches: vec![TruthMismatch {
                route: "pricing".to_string(),
                field: "title".to_string(),
                expected: "Aexeo".to_string(),
                observed: "Pricing".to_string(),
                source: "manifest".to_string(),
                severity: TruthMismatchSeverity::Warning,
            }],
            elapsed_us: 0,
        };
        let evidence = EvidenceSiteAssessment {
            pages_analyzed: 1,
            routes_with_claims: 1,
            claim_count: 1,
            unsupported_claims: 1,
            claim_kind_distribution: BTreeMap::new(),
            evidence_density_score: 40,
            citation_readiness_score: 55,
            average_fidelity_risk_score: 60,
            routes: vec![super::super::EvidenceRouteAssessment {
                route: "pricing".to_string(),
                sections: Vec::new(),
                claim_count: 1,
                unsupported_claims: 1,
                evidence_density_score: 40,
                citation_readiness_score: 55,
                fidelity_risk_score: 60,
            }],
            elapsed_us: 0,
        };
        let report = score_intelligence(&grounding, &truth, &evidence, None);
        assert_eq!(report.route_scores.len(), 1);
        assert!(report.overall_score < 80);
        assert!(!report.blockers.is_empty());
    }

    #[test]
    fn answer_pack_scoring_requires_real_structure_for_perfect_scores() {
        let generic_route = GroundingRouteAnalysis {
            route: String::new(),
            page_kind: "landing".to_string(),
            primary_topic: "home".to_string(),
            secondary_topics: Vec::new(),
            primary_intent: GroundingIntentFamily::Generic,
            secondary_intents: Vec::new(),
            intents: vec![GroundingIntentFamily::Generic],
            intent_matches: Vec::new(),
            schema_types: Vec::new(),
            signals: Vec::new(),
            coverage_gaps: Vec::new(),
            answer_blocks: 1,
            heading_count: 1,
        };
        let rich_feature_route = GroundingRouteAnalysis {
            route: "features/hyperlinks".to_string(),
            page_kind: "detail".to_string(),
            primary_topic: "hyperlinks".to_string(),
            secondary_topics: vec!["terminal hyperlinks".to_string()],
            primary_intent: GroundingIntentFamily::Feature,
            secondary_intents: vec![GroundingIntentFamily::Definition],
            intents: vec![
                GroundingIntentFamily::Feature,
                GroundingIntentFamily::Definition,
            ],
            intent_matches: Vec::new(),
            schema_types: Vec::new(),
            signals: Vec::new(),
            coverage_gaps: Vec::new(),
            answer_blocks: 4,
            heading_count: 5,
        };

        assert_eq!(route_answer_pack_score(&generic_route), 68);
        assert_eq!(route_answer_pack_score(&rich_feature_route), 100);
    }
}

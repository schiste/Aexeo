use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use super::{
    EvidenceSiteAssessment, GroundingCoverageGap, GroundingSiteAnalysis, TrustSurfaceIssueKind,
    TrustSurfaceReconciliation, TruthAssessment, TruthStructuredSource,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntelligenceBlocker {
    pub category: String,
    pub route: Option<String>,
    pub severity: u8,
    pub message: String,
}

/// Per-route facts. A `None` component means "unknown": nothing was measured
/// for this route, so no number is reported.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteIntelligenceScore {
    pub route: String,
    /// `None` when the route makes no assessable claims.
    pub citation_readiness_score: Option<u8>,
    /// `None` when the site exposes no structured facts to compare against.
    pub truth_consistency_score: Option<u8>,
    /// `None` when the route was not part of the grounding analysis.
    pub answer_blocks: Option<usize>,
    /// `None` when the route was not part of the grounding analysis.
    pub heading_count: Option<usize>,
    /// The worst issue's value on this route. `None` when no imported trust
    /// row reports an issue here: the reconciliation does not record which
    /// routes were covered, so an issue-free route is unknown, not perfect.
    pub external_trust_alignment_score: Option<u8>,
    pub external_trust_issue_count: usize,
    pub blockers: Vec<String>,
}

/// Measured answer structure across the routes in the grounding analysis.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnswerPackFacts {
    pub routes_analyzed: usize,
    pub routes_with_answer_blocks: usize,
    pub routes_with_coverage_gaps: usize,
    pub answer_blocks: usize,
}

/// Site-level facts and blockers. There is deliberately no composite: see
/// CONSTITUTION.md, "No vanity scoring". This is persisted as
/// `intelligence-score-latest.json`, so it holds no wall-clock data and
/// identical inputs produce identical bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SiteIntelligenceScore {
    /// `None` when no route makes an assessable claim.
    pub citation_readiness_score: Option<u8>,
    /// `None` when the site exposes no structured facts to compare against.
    pub truth_consistency_score: Option<u8>,
    pub answer_pack: AnswerPackFacts,
    /// `None` when no trust surface was imported, or the import has no rows.
    pub external_trust_alignment_score: Option<u8>,
    pub route_scores: Vec<RouteIntelligenceScore>,
    pub blockers: Vec<IntelligenceBlocker>,
}

pub fn score_intelligence(
    grounding: &GroundingSiteAnalysis,
    truth: &TruthAssessment,
    evidence: &EvidenceSiteAssessment,
    trust: Option<&TrustSurfaceReconciliation>,
) -> SiteIntelligenceScore {
    SiteIntelligenceScore {
        citation_readiness_score: site_citation_readiness(evidence),
        truth_consistency_score: site_truth_score(truth),
        answer_pack: answer_pack_facts(grounding),
        external_trust_alignment_score: trust.and_then(trust_alignment_score),
        route_scores: build_route_scores(grounding, truth, evidence, trust),
        blockers: collect_blockers(grounding, truth, evidence, trust),
    }
}

/// What the trust import says about one route: the worst issue wins, and every
/// issue is counted.
#[derive(Clone, Copy)]
struct RouteTrustIssues {
    worst: u8,
    count: usize,
}

fn trust_issues_by_route(
    trust: Option<&TrustSurfaceReconciliation>,
) -> BTreeMap<&str, RouteTrustIssues> {
    let mut by_route = BTreeMap::<&str, RouteTrustIssues>::new();
    for issue in trust.iter().flat_map(|report| &report.issues) {
        let Some(route) = issue.route.as_deref() else {
            continue;
        };
        let penalty = trust_penalty(issue.kind());
        by_route
            .entry(route)
            .and_modify(|merged| {
                merged.worst = merged.worst.min(penalty);
                merged.count += 1;
            })
            .or_insert(RouteTrustIssues {
                worst: penalty,
                count: 1,
            });
    }
    by_route
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
    let trust_issues = trust_issues_by_route(trust);

    let mut routes = grounding
        .routes
        .iter()
        .map(|grounding_route| {
            let evidence_route = evidence_by_route
                .get(grounding_route.route.as_str())
                .copied();
            let mut route = route_facts(
                &grounding_route.route,
                evidence_route,
                truth,
                trust_issues.get(grounding_route.route.as_str()).copied(),
            );
            route.answer_blocks = Some(grounding_route.answer_blocks);
            route.heading_count = Some(grounding_route.heading_count);
            route.blockers.extend(
                grounding_route
                    .coverage_gaps
                    .iter()
                    .map(|gap| gap_label(gap).to_string()),
            );
            route.blockers.sort();
            route.blockers.dedup();
            route
        })
        .collect::<Vec<_>>();

    for route in evidence
        .routes
        .iter()
        .filter(|route| !grounding_by_route.contains_key(route.route.as_str()))
    {
        routes.push(route_facts(
            &route.route,
            Some(route),
            truth,
            trust_issues.get(route.route.as_str()).copied(),
        ));
    }

    routes.sort_by(|left, right| {
        right
            .blockers
            .len()
            .cmp(&left.blockers.len())
            .then_with(|| left.route.cmp(&right.route))
    });
    routes
}

/// The facts every route has, whichever analysis it came from. Grounding facts
/// start unknown and are filled in by the caller when the route was analyzed.
fn route_facts(
    route: &str,
    evidence_route: Option<&super::EvidenceRouteAssessment>,
    truth: &TruthAssessment,
    trust_issues: Option<RouteTrustIssues>,
) -> RouteIntelligenceScore {
    let truth_consistency_score = route_truth_score(route, truth);
    let mut blockers = Vec::new();
    if let Some(evidence_route) = evidence_route
        && evidence_route.unsupported_claims > 0
    {
        blockers.push(format!(
            "{} unsupported claims",
            evidence_route.unsupported_claims
        ));
    }
    if truth_consistency_score.is_some_and(|score| score < 80) {
        blockers.push("truth_consistency_gap".to_string());
    }
    if trust_issues.is_some() {
        blockers.push("external_trust_alignment_gap".to_string());
    }
    RouteIntelligenceScore {
        route: route.to_string(),
        citation_readiness_score: evidence_route
            .filter(|route| route.claim_count > 0)
            .map(|route| route.citation_readiness_score),
        truth_consistency_score,
        answer_blocks: None,
        heading_count: None,
        external_trust_alignment_score: trust_issues.map(|issues| issues.worst),
        external_trust_issue_count: trust_issues.map_or(0, |issues| issues.count),
        blockers,
    }
}

fn collect_blockers(
    grounding: &GroundingSiteAnalysis,
    truth: &TruthAssessment,
    evidence: &EvidenceSiteAssessment,
    trust: Option<&TrustSurfaceReconciliation>,
) -> Vec<IntelligenceBlocker> {
    let mut blockers = Vec::new();
    if matches!(truth.structured_truth_source, TruthStructuredSource::None) {
        blockers.push(IntelligenceBlocker {
            category: "truth".to_string(),
            route: None,
            severity: 90,
            message: "no structured facts to check: add schema.org JSON-LD or a facts manifest"
                .to_string(),
        });
    }
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

fn answer_pack_facts(grounding: &GroundingSiteAnalysis) -> AnswerPackFacts {
    AnswerPackFacts {
        routes_analyzed: grounding.routes.len(),
        routes_with_answer_blocks: grounding
            .routes
            .iter()
            .filter(|route| route.answer_blocks > 0)
            .count(),
        routes_with_coverage_gaps: grounding
            .routes
            .iter()
            .filter(|route| !route.coverage_gaps.is_empty())
            .count(),
        answer_blocks: grounding
            .routes
            .iter()
            .map(|route| route.answer_blocks)
            .sum(),
    }
}

/// Mean readiness over the routes that make claims. The evidence report gives
/// claim-free pages a default density, so including them would average that
/// default into a number that looks measured.
fn site_citation_readiness(evidence: &EvidenceSiteAssessment) -> Option<u8> {
    let scores = evidence
        .routes
        .iter()
        .filter(|route| route.claim_count > 0)
        .map(|route| u64::from(route.citation_readiness_score))
        .collect::<Vec<_>>();
    let mean = scores
        .iter()
        .sum::<u64>()
        .checked_div(scores.len() as u64)?;
    Some(mean as u8)
}

/// Without schema or a manifest, `truth.score` is its ceiling less penalties
/// for the absent schema: there was nothing to be consistent with.
fn site_truth_score(truth: &TruthAssessment) -> Option<u8> {
    (!matches!(truth.structured_truth_source, TruthStructuredSource::None)).then_some(truth.score)
}

fn route_truth_score(route: &str, truth: &TruthAssessment) -> Option<u8> {
    let penalties = truth
        .mismatches
        .iter()
        .filter(|mismatch| mismatch.route == route)
        .count()
        .min(4) as u8;
    site_truth_score(truth).map(|score| score.saturating_sub(penalties * 10))
}

fn trust_alignment_score(report: &TrustSurfaceReconciliation) -> Option<u8> {
    let match_ratio =
        (report.matched_first_party_routes * 100).checked_div(report.rows_read)? as u8;
    let issue_penalty = (report.issues.len().min(10) * 7) as u8;
    Some(match_ratio.saturating_sub(issue_penalty))
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

    fn truth_assessment_base() -> crate::intelligence::truth::TruthAssessment {
        crate::intelligence::truth::TruthAssessment {
            structured_truth_source: crate::intelligence::truth::TruthStructuredSource::None,
            structured_truth_prerequisite_met: false,
            score: 90,
            score_ceiling: 100,
            pages_analyzed: 0,
            pages_with_schema: 0,
            manifest_present: false,
            organization_schema_pages: 0,
            website_schema_pages: 0,
            preferred_term_hits: 0,
            forbidden_term_hits: 0,
            mismatches: vec![],
            elapsed_us: 0,
        }
    }

    fn mismatch(route: &str) -> TruthMismatch {
        TruthMismatch {
            route: route.to_string(),
            field: "name".to_string(),
            expected: "Aeptus".to_string(),
            observed: "Acme".to_string(),
            source: "manifest".to_string(),
            severity: TruthMismatchSeverity::Warning,
        }
    }

    fn trust_report_base() -> TrustSurfaceReconciliation {
        TrustSurfaceReconciliation {
            rows_read: 0,
            matched_first_party_routes: 0,
            offsite_mentions: 0,
            source_summaries: vec![],
            issues: vec![],
            elapsed_us: 0,
        }
    }
    use crate::intelligence::{
        EvidenceRouteAssessment, GroundingIntentFamily, GroundingRouteAnalysis, TrustSurfaceIssue,
        TruthMismatch, TruthMismatchSeverity,
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
            evidence_quality_score: 40,
            citation_readiness_score: 55,
            average_fidelity_risk_score: 60,
            routes: vec![EvidenceRouteAssessment {
                route: "pricing".to_string(),
                sections: Vec::new(),
                claim_count: 1,
                unsupported_claims: 1,
                evidence_density_score: 40,
                evidence_quality_score: 40,
                citation_readiness_score: 55,
                fidelity_risk_score: 60,
            }],
            elapsed_us: 0,
        };
        let report = score_intelligence(&grounding, &truth, &evidence, None);
        assert_eq!(report.route_scores.len(), 1);
        assert_eq!(report.citation_readiness_score, Some(55));
        assert_eq!(report.truth_consistency_score, Some(82));
        assert_eq!(
            report.answer_pack,
            AnswerPackFacts {
                routes_analyzed: 1,
                routes_with_answer_blocks: 1,
                routes_with_coverage_gaps: 1,
                answer_blocks: 1,
            }
        );
        assert_eq!(report.external_trust_alignment_score, None);
        assert_eq!(report.route_scores[0].answer_blocks, Some(1));
        assert!(!report.blockers.is_empty());
    }

    fn grounding_base(routes: Vec<GroundingRouteAnalysis>) -> GroundingSiteAnalysis {
        GroundingSiteAnalysis {
            pages_analyzed: routes.len(),
            routes_with_topics: routes.len(),
            intent_distribution: BTreeMap::new(),
            primary_intent_distribution: BTreeMap::new(),
            topic_clusters: BTreeMap::new(),
            routes,
            elapsed_us: 0,
        }
    }

    fn grounding_route(route: &str) -> GroundingRouteAnalysis {
        GroundingRouteAnalysis {
            route: route.to_string(),
            page_kind: "detail".to_string(),
            primary_topic: "topic".to_string(),
            secondary_topics: Vec::new(),
            primary_intent: GroundingIntentFamily::Feature,
            secondary_intents: Vec::new(),
            intents: vec![GroundingIntentFamily::Feature],
            intent_matches: Vec::new(),
            schema_types: Vec::new(),
            signals: Vec::new(),
            coverage_gaps: Vec::new(),
            answer_blocks: 0,
            heading_count: 1,
        }
    }

    fn evidence_route(route: &str, claim_count: usize) -> EvidenceRouteAssessment {
        EvidenceRouteAssessment {
            route: route.to_string(),
            sections: Vec::new(),
            claim_count,
            unsupported_claims: 0,
            evidence_density_score: 90,
            evidence_quality_score: 0,
            citation_readiness_score: 58,
            fidelity_risk_score: 0,
        }
    }

    fn evidence_base(routes: Vec<EvidenceRouteAssessment>) -> EvidenceSiteAssessment {
        EvidenceSiteAssessment {
            pages_analyzed: routes.len(),
            routes_with_claims: 0,
            claim_count: 0,
            unsupported_claims: 0,
            claim_kind_distribution: BTreeMap::new(),
            evidence_density_score: 0,
            evidence_quality_score: 0,
            citation_readiness_score: 58,
            average_fidelity_risk_score: 0,
            routes,
            elapsed_us: 0,
        }
    }

    fn trust_issue(route: &str, kind: TrustSurfaceIssueKind) -> TrustSurfaceIssue {
        TrustSurfaceIssue {
            source_type: "search_console".to_string(),
            url: format!("https://example.com/{route}"),
            route: Some(route.to_string()),
            kind,
            message: "issue".to_string(),
        }
    }

    /// The composite and the wall clock are gone from the persisted artifact:
    /// consumers must not find either, and identical runs serialize alike.
    #[test]
    fn serialized_report_has_no_composite_or_timing_and_is_stable() {
        let grounding = grounding_base(vec![grounding_route("a")]);
        let evidence = evidence_base(vec![evidence_route("a", 1)]);
        let truth = truth_assessment_base();
        let first = serde_json::to_value(score_intelligence(&grounding, &truth, &evidence, None))
            .expect("serialize");
        let second = serde_json::to_value(score_intelligence(&grounding, &truth, &evidence, None))
            .expect("serialize");
        assert_eq!(first, second);
        let text = first.to_string();
        for removed in ["overall_score", "elapsed_us", "answer_pack_score"] {
            assert!(!text.contains(removed), "{removed} in {text}");
        }
    }

    /// A claim-free route used to report 75 or the evidence report's default;
    /// nothing was measured, so its readiness is unknown. The same default
    /// must not leak into the site mean.
    #[test]
    fn citation_readiness_is_unknown_without_claims() {
        let grounding = grounding_base(vec![
            grounding_route("no-evidence"),
            grounding_route("no-claims"),
            grounding_route("claims"),
        ]);
        let mut with_claims = evidence_route("claims", 2);
        with_claims.citation_readiness_score = 40;
        let evidence = evidence_base(vec![evidence_route("no-claims", 0), with_claims]);
        let report = score_intelligence(&grounding, &truth_assessment_base(), &evidence, None);

        let readiness = |route: &str| {
            report
                .route_scores
                .iter()
                .find(|item| item.route == route)
                .expect("route")
                .citation_readiness_score
        };
        assert_eq!(readiness("no-evidence"), None);
        assert_eq!(readiness("no-claims"), None);
        assert_eq!(readiness("claims"), Some(40));
        assert_eq!(report.citation_readiness_score, Some(40));

        let no_claims = evidence_base(vec![evidence_route("no-claims", 0)]);
        let report = score_intelligence(&grounding, &truth_assessment_base(), &no_claims, None);
        assert_eq!(report.citation_readiness_score, None);
    }

    /// A route known only to the evidence report has unknown answer structure
    /// (it used to report a flat 75), and trust issues still apply to it.
    #[test]
    fn evidence_only_route_has_unknown_answer_structure_and_keeps_trust_issues() {
        let report = score_intelligence(
            &grounding_base(Vec::new()),
            &truth_assessment_base(),
            &evidence_base(vec![evidence_route("only-evidence", 1)]),
            Some(&TrustSurfaceReconciliation {
                rows_read: 1,
                issues: vec![trust_issue(
                    "only-evidence",
                    TrustSurfaceIssueKind::RouteNotInSite,
                )],
                ..trust_report_base()
            }),
        );
        let route = &report.route_scores[0];
        assert_eq!(route.answer_blocks, None);
        assert_eq!(route.heading_count, None);
        assert_eq!(route.external_trust_alignment_score, Some(45));
        assert_eq!(route.external_trust_issue_count, 1);
    }

    /// Without schema or a manifest, there is nothing to be consistent with:
    /// the facts dimension is unknown and the missing input is the blocker.
    #[test]
    fn truth_consistency_is_unknown_without_structured_facts() {
        let truth = TruthAssessment {
            score: 43,
            ..truth_assessment_base()
        };
        let report = score_intelligence(
            &grounding_base(vec![grounding_route("a")]),
            &truth,
            &evidence_base(Vec::new()),
            None,
        );
        assert_eq!(report.truth_consistency_score, None);
        let route = &report.route_scores[0];
        assert_eq!(route.truth_consistency_score, None);
        assert!(!route.blockers.iter().any(|b| b == "truth_consistency_gap"));
        assert!(
            report
                .blockers
                .iter()
                .any(|b| b.category == "truth" && b.route.is_none()),
            "{:?}",
            report.blockers
        );
    }

    /// The worst issue wins whatever order the import lists them in: a later
    /// DescriptorGap (70) must not raise a RouteNotInSite (45), and both count.
    #[test]
    fn route_trust_merge_keeps_the_worst_issue_and_counts_all() {
        for issues in [
            vec![
                trust_issue("a", TrustSurfaceIssueKind::RouteNotInSite),
                trust_issue("a", TrustSurfaceIssueKind::DescriptorGap),
            ],
            vec![
                trust_issue("a", TrustSurfaceIssueKind::DescriptorGap),
                trust_issue("a", TrustSurfaceIssueKind::RouteNotInSite),
            ],
        ] {
            let report = score_intelligence(
                &grounding_base(vec![grounding_route("a"), grounding_route("clean")]),
                &truth_assessment_base(),
                &evidence_base(Vec::new()),
                Some(&TrustSurfaceReconciliation {
                    rows_read: 2,
                    issues,
                    ..trust_report_base()
                }),
            );
            let route = |name: &str| {
                report
                    .route_scores
                    .iter()
                    .find(|item| item.route == name)
                    .expect("route")
            };
            assert_eq!(route("a").external_trust_alignment_score, Some(45));
            assert_eq!(route("a").external_trust_issue_count, 2);
            assert_eq!(route("clean").external_trust_alignment_score, None);
            assert_eq!(route("clean").external_trust_issue_count, 0);
        }
    }

    /// Routes come back most-blocked first, then by name, with no score to
    /// sort on.
    #[test]
    fn routes_sort_by_blocker_count_then_name() {
        let mut blocked = grounding_route("z-blocked");
        blocked.coverage_gaps = vec![GroundingCoverageGap::MissingDirectAnswer];
        let report = score_intelligence(
            &grounding_base(vec![grounding_route("b"), grounding_route("a"), blocked]),
            &TruthAssessment {
                structured_truth_source: TruthStructuredSource::SchemaAndManifest,
                ..truth_assessment_base()
            },
            &evidence_base(Vec::new()),
            None,
        );
        let order = report
            .route_scores
            .iter()
            .map(|route| route.route.as_str())
            .collect::<Vec<_>>();
        assert_eq!(order, ["z-blocked", "a", "b"]);
    }

    /// Every grounding gap needs a stable label (it appears in report output
    /// and is compared across runs) and a severity in range.
    #[test]
    fn every_grounding_gap_has_a_label_and_a_sane_severity() {
        for gap in [
            GroundingCoverageGap::MissingDirectAnswer,
            GroundingCoverageGap::ThinAnswerCoverage,
            GroundingCoverageGap::WeakComparisonStructure,
            GroundingCoverageGap::MissingPricingSignals,
            GroundingCoverageGap::MissingProceduralSignals,
        ] {
            let label = gap_label(&gap);
            assert!(!label.is_empty(), "{gap:?} has no label");
            assert!(
                label.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
                "{gap:?} label {label} is not snake_case"
            );
            let severity = gap_severity(&gap);
            assert!(
                (1..=100).contains(&severity),
                "{gap:?} severity {severity} is out of range"
            );
        }
        // The most serious gap must outrank the least.
        assert!(
            gap_severity(&GroundingCoverageGap::MissingDirectAnswer)
                > gap_severity(&GroundingCoverageGap::MissingPricingSignals)
        );
    }

    /// Trust penalties stay in the range the route value is reported in.
    #[test]
    fn every_trust_issue_kind_has_a_bounded_penalty() {
        for kind in [
            TrustSurfaceIssueKind::RouteNotInSite,
            TrustSurfaceIssueKind::MissingCanonicalEntity,
            TrustSurfaceIssueKind::ForbiddenTerminology,
            TrustSurfaceIssueKind::DescriptorGap,
        ] {
            let penalty = trust_penalty(kind.clone());
            assert!((1..=100).contains(&penalty), "{kind:?} penalty {penalty}");
        }
    }

    /// A truth mismatch on a route degrades that route, capped at four so a
    /// pathologically noisy route cannot zero out the value, and other routes
    /// are unaffected.
    #[test]
    fn truth_mismatches_penalise_only_their_own_route_and_are_capped() {
        let structured = || TruthAssessment {
            structured_truth_source: TruthStructuredSource::Schema,
            ..truth_assessment_base()
        };
        assert_eq!(route_truth_score("/a", &structured()), Some(90));

        let one = TruthAssessment {
            mismatches: vec![mismatch("/a")],
            ..structured()
        };
        assert_eq!(
            route_truth_score("/a", &one),
            Some(80),
            "one mismatch costs 10"
        );
        assert_eq!(
            route_truth_score("/b", &one),
            Some(90),
            "other routes untouched"
        );

        // Five mismatches still cost 40, not 50: the cap is at four.
        let many = TruthAssessment {
            mismatches: (0..5).map(|_| mismatch("/a")).collect(),
            ..structured()
        };
        assert_eq!(route_truth_score("/a", &many), Some(50));
    }

    /// An import with no rows measured nothing: unknown, not a perfect 100.
    /// A real import reports what it found, with no floor holding it at 40.
    #[test]
    fn trust_alignment_is_unknown_when_empty_and_has_no_floor() {
        let empty = TrustSurfaceReconciliation {
            rows_read: 0,
            ..trust_report_base()
        };
        assert_eq!(trust_alignment_score(&empty), None);

        let all_matched = TrustSurfaceReconciliation {
            rows_read: 4,
            matched_first_party_routes: 4,
            ..trust_report_base()
        };
        assert_eq!(trust_alignment_score(&all_matched), Some(100));

        let none_matched = TrustSurfaceReconciliation {
            rows_read: 4,
            matched_first_party_routes: 0,
            ..trust_report_base()
        };
        assert_eq!(trust_alignment_score(&none_matched), Some(0));
    }
}

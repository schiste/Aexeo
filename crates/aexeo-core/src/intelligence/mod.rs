mod evidence;
mod facts_prompt;
mod fanout;
mod grounding;
mod identity;
mod schema_gen;
mod score;
mod trust;
mod truth;

pub use facts_prompt::render_facts_prompt;
pub use identity::{IdentityDrift, PageIdentity, PageIdentitySources, compute_page_identity};
pub use schema_gen::{SchemaSuggestion, generate_schema_suggestions};

pub use evidence::{
    EvidenceClaim, EvidenceClaimKind, EvidenceRouteAssessment, EvidenceSectionAssessment,
    EvidenceSiteAssessment, assess_evidence_coverage,
};
pub use fanout::{
    AnswerFanoutQuery, AnswerFanoutReport, AnswerFanoutRouteMatch, assess_answer_fanout,
};
pub use grounding::{
    GroundingCoverageGap, GroundingIntentFamily, GroundingRouteAnalysis, GroundingSiteAnalysis,
    map_grounding_queries,
};
pub use score::{
    IntelligenceBlocker, RouteIntelligenceScore, SiteIntelligenceScore, score_intelligence,
};
pub use trust::{
    TrustSurfaceIssue, TrustSurfaceIssueKind, TrustSurfaceReconciliation, TrustSurfaceRecord,
    TrustSurfaceSourceSummary, import_trust_surface_records, reconcile_trust_surfaces,
};
pub use truth::{
    TruthAssessment, TruthEntity, TruthManifest, TruthManifestGeneration, TruthManifestValidation,
    TruthMismatch, TruthMismatchSeverity, TruthStructuredSource, TruthTerminology,
    assess_truth_layer, default_truth_manifest_version, discover_truth_manifest,
    generate_truth_manifest, generate_truth_manifest_with_options, load_truth_manifest,
    validate_truth_manifest,
};

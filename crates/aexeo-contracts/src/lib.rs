#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RuleClass {
    Hard,
    Policy,
    Heuristic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConfidenceLevel {
    High,
    Medium,
    Low,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuleMetadata {
    pub class: RuleClass,
    pub confidence: ConfidenceLevel,
}

/// The four-layer GEO model from the May 2026 literature synthesis,
/// extended with a fifth axis for accessibility added in 0.0.17 /
/// 0.8.12 per Aeptus's third-axis proposal.
/// Each rule is tagged with one primary layer (the one it most directly
/// serves) plus zero or more secondary layers (the ones it also affects).
///
/// - `Retrievability`: can the engine find the page at all? (robots,
///   sitemap, internal links, machine-readable surfaces)
/// - `Citability`: once retrieved, does it look worth citing? (structure,
///   schema, evidence density, scannable content)
/// - `Absorbability`: does the answer actually use this content? (cite-
///   ready evidence units, markdown mirrors, llms.txt)
/// - `EntityLegitimacy`: does the entity exist strongly enough to be
///   selected at all? (truth manifest, external presence — Aexeo
///   surfaces but does not fix this layer)
/// - `Accessibility`: can humans use the page? (semantic markup,
///   keyboard access, focus, contrast, labels, landmarks). Distinct
///   from the GEO axes because it serves human users directly, not
///   answer-engine pipelines — but A11Y signals frequently feed
///   the GEO layers as secondaries (alt text → retrievability,
///   landmarks → citability, etc.).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Layer {
    Retrievability,
    Citability,
    Absorbability,
    EntityLegitimacy,
    Accessibility,
}

impl Layer {
    /// Stable display order for grouping output. The four GEO axes
    /// stay in their original upstream-to-downstream order;
    /// Accessibility lands last as a separate axis rather than
    /// being interleaved into the GEO ordering.
    pub fn ordered() -> [Layer; 5] {
        [
            Layer::Retrievability,
            Layer::Citability,
            Layer::Absorbability,
            Layer::EntityLegitimacy,
            Layer::Accessibility,
        ]
    }

    pub fn label(self) -> &'static str {
        match self {
            Layer::Retrievability => "retrievability",
            Layer::Citability => "citability",
            Layer::Absorbability => "absorbability",
            Layer::EntityLegitimacy => "entity_legitimacy",
            Layer::Accessibility => "accessibility",
        }
    }

    pub fn human_label(self) -> &'static str {
        match self {
            Layer::Retrievability => "Retrievability",
            Layer::Citability => "Citability",
            Layer::Absorbability => "Absorbability",
            Layer::EntityLegitimacy => "Entity legitimacy",
            Layer::Accessibility => "Accessibility",
        }
    }
}

/// Layer assignment for a single rule. Always has a primary layer;
/// `secondaries` lists other layers the rule meaningfully affects.
/// Most rules have an empty secondaries list — only cross-cutting rules
/// (e.g. a rule about meta descriptions affects both retrievability
/// search-result snippets and citability AI-engine reuse) carry one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuleLayers {
    pub primary: Layer,
    pub secondaries: Vec<Layer>,
}

impl RuleLayers {
    pub fn primary_only(layer: Layer) -> Self {
        Self {
            primary: layer,
            secondaries: Vec::new(),
        }
    }

    pub fn with_secondaries(primary: Layer, secondaries: Vec<Layer>) -> Self {
        Self {
            primary,
            secondaries,
        }
    }

    /// True if any of the rule's layer assignments matches `layer`.
    /// Useful for `--layer X` filtering at call sites.
    pub fn touches(&self, layer: Layer) -> bool {
        self.primary == layer || self.secondaries.contains(&layer)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum FindingScope {
    #[default]
    Page,
    Template,
    Sitewide,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finding {
    pub rule_id: String,
    pub message: String,
    pub path: String,
    #[serde(default = "default_line")]
    pub line: usize,
    #[serde(default = "default_column")]
    pub column: usize,
    #[serde(default = "default_severity")]
    pub severity: String,
    #[serde(default)]
    pub suggestion: Option<String>,
    #[serde(default)]
    pub scope: FindingScope,
    /// Stable key telling apart findings of one rule on one path, such as a
    /// link target. Part of the fingerprint, unlike `message`. Rules that can
    /// report several distinct problems per path must set it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AuditStatus {
    #[default]
    Complete,
    Partial,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AuditSummary {
    pub total: usize,
    pub errors: usize,
    pub warnings: usize,
    pub actionable: usize,
    pub heuristic: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SlowCrawlPath {
    pub url: String,
    #[serde(default)]
    pub fetch_us: u64,
    #[serde(default)]
    pub process_us: u64,
    pub fetch_ms: u64,
    pub process_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PhaseTiming {
    pub name: String,
    pub elapsed_us: u64,
    #[serde(default)]
    pub basis: String,
    #[serde(default)]
    pub sample_count: usize,
    #[serde(default)]
    pub min_us: u64,
    #[serde(default)]
    pub max_us: u64,
    #[serde(default)]
    pub p50_us: u64,
    #[serde(default)]
    pub p75_us: u64,
    #[serde(default)]
    pub p95_us: u64,
    #[serde(default)]
    pub p99_us: u64,
    #[serde(default)]
    pub wall_share_basis_points: u32,
    #[serde(default)]
    pub cumulative_share_basis_points: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct RuleTiming {
    pub group: String,
    pub elapsed_us: u64,
    pub findings: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PerformanceBottleneck {
    pub kind: String,
    pub name: String,
    pub elapsed_us: u64,
    #[serde(default)]
    pub share_basis_points: u32,
    #[serde(default)]
    pub wall_share_basis_points: u32,
    #[serde(default)]
    pub cumulative_share_basis_points: u32,
    #[serde(default)]
    pub findings: Option<usize>,
    #[serde(default)]
    pub recommendation: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PerformanceBudget {
    #[serde(default)]
    pub max_elapsed_ms: Option<u64>,
    #[serde(default)]
    pub max_fetch_average_ms: Option<u64>,
    #[serde(default)]
    pub max_fetch_p95_ms: Option<u64>,
    #[serde(default)]
    pub max_rule_evaluation_ms: Option<u64>,
    #[serde(default)]
    pub max_final_audit_ms: Option<u64>,
    #[serde(default)]
    pub max_total_findings: Option<usize>,
    #[serde(default)]
    pub max_errors: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PerformanceBudgetViolation {
    pub metric: String,
    pub actual: u64,
    pub budget: u64,
    pub unit: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PerformanceBudgetReport {
    pub passed: bool,
    #[serde(default)]
    pub budget_path: Option<String>,
    pub budget: PerformanceBudget,
    #[serde(default)]
    pub violations: Vec<PerformanceBudgetViolation>,
    #[serde(default)]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PerformanceDiffThresholds {
    pub relative_threshold_basis_points: u32,
    pub absolute_threshold: u64,
}

impl Default for PerformanceDiffThresholds {
    fn default() -> Self {
        Self {
            relative_threshold_basis_points: 1_000,
            absolute_threshold: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PerformanceMetricDelta {
    pub metric: String,
    pub label: String,
    pub unit: String,
    pub direction: String,
    #[serde(default)]
    pub baseline: Option<u64>,
    #[serde(default)]
    pub current: Option<u64>,
    #[serde(default)]
    pub delta: Option<i64>,
    #[serde(default)]
    pub relative_delta_basis_points: Option<i64>,
    pub status: String,
    pub regressed: bool,
    pub improved: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PerformanceDiffSummary {
    pub metrics_compared: usize,
    pub regressions: usize,
    pub improvements: usize,
    pub unchanged: usize,
    pub missing: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PerformanceDiffReport {
    #[serde(default)]
    pub baseline_path: Option<String>,
    #[serde(default)]
    pub current_path: Option<String>,
    pub thresholds: PerformanceDiffThresholds,
    pub summary: PerformanceDiffSummary,
    #[serde(default)]
    pub metrics: Vec<PerformanceMetricDelta>,
    #[serde(default)]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AuditPerformance {
    #[serde(default)]
    pub elapsed_us: u64,
    #[serde(default)]
    pub wall_clock_us: u64,
    #[serde(default)]
    pub cumulative_tracked_us: u64,
    #[serde(default)]
    pub phases: Vec<PhaseTiming>,
    #[serde(default)]
    pub rule_groups: Vec<RuleTiming>,
    #[serde(default)]
    pub bottlenecks: Vec<PerformanceBottleneck>,
    #[serde(default)]
    pub observations: Vec<String>,
    #[serde(default)]
    pub budget: Option<PerformanceBudgetReport>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AuditCrawlMeta {
    pub visited_pages: usize,
    pub max_pages: usize,
    pub discovered_internal_routes: usize,
    pub truncated: bool,
    /// Paths of well-known machine-readable artifacts that were
    /// HEAD-probed during the live crawl and returned 2xx. Populated
    /// by `probe_well_known_artifacts` after the page-crawl loop.
    /// `discover_machine_surface_graph` consults these to mark
    /// `facts_present` / `llms_full_present` / etc. for sites where
    /// the artifacts aren't reachable via the HTML link graph
    /// (manifest.json-listed only, robots-Sitemap-only, etc).
    /// `#[serde(default)]` so old artifacts deserialize cleanly.
    #[serde(default)]
    pub probed_artifact_paths: Vec<String>,
    /// Requests answered with a non-2xx status during the live crawl.
    /// They are not pages. Their routes count as broken link targets, except
    /// after a 401 or 403, which says nothing about whether the page exists.
    #[serde(default)]
    pub failed_responses: Vec<CrawlResponseProvenance>,
    /// URLs the crawler did not request because robots.txt disallows them.
    #[serde(default)]
    pub robots_skipped_urls: Vec<String>,
}

/// Response provenance recorded by a live crawl. Missing fields are unknown, not inferred.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct CrawlResponseProvenance {
    pub requested_url: String,
    #[serde(default)]
    pub observed_url: Option<String>,
    #[serde(default)]
    pub status_code: Option<u16>,
    #[serde(default)]
    pub headers_observed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AuditPageSnapshot {
    pub path: String,
    pub relative_path: String,
    pub route: String,
    /// Kept in the saved artifact, which `verify` replays; left out of command
    /// output, where it was most of the payload.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub raw_html: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crawl_response: Option<CrawlResponseProvenance>,
    #[serde(default)]
    pub response_headers: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AuditSiteSnapshot {
    #[serde(default = "default_audit_site_snapshot_version")]
    pub version: u32,
    #[serde(default)]
    pub site_url: Option<String>,
    pub root: String,
    pub deployment_model: String,
    #[serde(default)]
    pub deployment_markers: Vec<String>,
    #[serde(default)]
    pub crawl_meta: Option<AuditCrawlMeta>,
    #[serde(default)]
    pub llms_text: Option<String>,
    #[serde(default)]
    pub robots_text: Option<String>,
    #[serde(default)]
    pub sitemap_text: Option<String>,
    #[serde(default)]
    pub pages: Vec<AuditPageSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct CrawlStats {
    pub engine: String,
    pub visited_pages: usize,
    pub discovered_internal_routes: usize,
    pub queued_routes_remaining: usize,
    pub max_pages: usize,
    pub fetch_failures: usize,
    pub fetch_retries: usize,
    pub skipped_non_html: usize,
    #[serde(default)]
    pub skipped_by_robots: usize,
    /// Non-2xx answers that are defects of the site, such as a 404. These do not
    /// make a crawl partial; `fetch_failures` and `access_denied` do.
    #[serde(default)]
    pub observed_error_responses: usize,
    /// URLs answered with HTTP 401 or 403.
    #[serde(default)]
    pub access_denied: usize,
    pub truncated: bool,
    #[serde(default)]
    pub elapsed_us: u64,
    pub elapsed_ms: u64,
    pub pages_per_minute: usize,
    pub checkpoints_written: usize,
    #[serde(default)]
    pub progress_artifacts_written: usize,
    pub partial_artifacts_written: usize,
    #[serde(default)]
    pub total_fetch_us: u64,
    pub total_fetch_ms: u64,
    #[serde(default)]
    pub average_fetch_us: u64,
    pub average_fetch_ms: u64,
    #[serde(default)]
    pub total_page_process_us: u64,
    pub total_page_process_ms: u64,
    #[serde(default)]
    pub average_page_process_us: u64,
    pub average_page_process_ms: u64,
    #[serde(default)]
    pub total_partial_audit_us: u64,
    pub total_partial_audit_ms: u64,
    #[serde(default)]
    pub average_partial_audit_us: u64,
    pub average_partial_audit_ms: u64,
    #[serde(default)]
    pub total_optional_artifact_fetch_us: u64,
    #[serde(default)]
    pub total_sitemap_seed_us: u64,
    #[serde(default)]
    pub total_snapshot_build_us: u64,
    #[serde(default)]
    pub total_snapshot_write_us: u64,
    #[serde(default)]
    pub total_queue_selection_us: u64,
    #[serde(default)]
    pub total_planner_update_us: u64,
    #[serde(default)]
    pub total_link_extraction_us: u64,
    #[serde(default)]
    pub total_progress_callback_us: u64,
    #[serde(default)]
    pub total_checkpoint_write_us: u64,
    #[serde(default)]
    pub total_progress_artifact_write_us: u64,
    #[serde(default)]
    pub total_partial_audit_build_us: u64,
    #[serde(default)]
    pub total_partial_artifact_write_us: u64,
    #[serde(default)]
    pub total_rule_evaluation_us: u64,
    #[serde(default)]
    pub total_policy_apply_us: u64,
    #[serde(default)]
    pub total_final_audit_us: u64,
    #[serde(default)]
    pub total_overhead_us: u64,
    pub slowest_paths: Vec<SlowCrawlPath>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditArtifact {
    #[serde(default = "default_audit_artifact_version")]
    pub version: u32,
    pub command: String,
    #[serde(default)]
    pub status: AuditStatus,
    #[serde(default)]
    pub generated_at: u64,
    #[serde(default)]
    pub summary: AuditSummary,
    #[serde(default)]
    pub completion_ratio: Option<String>,
    #[serde(default)]
    pub truncation_reason: Option<String>,
    #[serde(default)]
    pub crawl: Option<CrawlStats>,
    #[serde(default)]
    pub performance: Option<AuditPerformance>,
    #[serde(default)]
    pub site: Option<AuditSiteSnapshot>,
    #[serde(default)]
    pub findings: Vec<Finding>,
}

/// Identity of a finding across runs (fingerprint version 2). Message wording
/// and source position are deliberately left out: both change without the
/// underlying problem changing.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FindingFingerprint {
    pub rule_id: String,
    pub path: String,
    pub subject: Option<String>,
}

/// Fingerprint version 1, computed from the fields every stored finding has.
/// Baselines written before `subject` existed keep matching through it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LegacyFindingFingerprint {
    pub rule_id: String,
    pub path: String,
    pub line: usize,
    pub column: usize,
    pub message: String,
}

fn default_line() -> usize {
    1
}

fn default_column() -> usize {
    1
}

fn default_severity() -> String {
    "error".to_string()
}

fn default_audit_artifact_version() -> u32 {
    2
}

fn default_audit_site_snapshot_version() -> u32 {
    1
}

impl Finding {
    pub fn render(&self) -> String {
        let base = format!(
            "{}:{}:{} {} {}",
            self.path, self.line, self.column, self.rule_id, self.message
        );
        match &self.suggestion {
            Some(suggestion) => format!("{base} [{suggestion}]"),
            None => base,
        }
    }

    pub fn is_error(&self) -> bool {
        self.severity != "warning"
    }

    pub fn with_subject(mut self, subject: impl Into<String>) -> Self {
        self.subject = Some(subject.into());
        self
    }

    pub fn fingerprint(&self) -> FindingFingerprint {
        FindingFingerprint {
            rule_id: self.rule_id.clone(),
            path: self.path.clone(),
            subject: self.subject.clone(),
        }
    }

    pub fn legacy_fingerprint(&self) -> LegacyFindingFingerprint {
        LegacyFindingFingerprint {
            rule_id: self.rule_id.clone(),
            path: self.path.clone(),
            line: self.line,
            column: self.column,
            // Preserve the historic LNK002 baseline fingerprint while making
            // the displayed diagnostic explicit about observation limits.
            message: if self.rule_id == "LNK002" {
                self.message
                    .strip_prefix("no inbound links observed in the analyzed pages: ")
                    .map(|route| format!("orphan page: {route}"))
                    .unwrap_or_else(|| self.message.clone())
            } else {
                self.message.clone()
            },
        }
    }
}

impl AuditArtifact {
    pub fn is_partial(&self) -> bool {
        matches!(self.status, AuditStatus::Partial)
    }
}

impl Default for AuditArtifact {
    fn default() -> Self {
        Self {
            version: default_audit_artifact_version(),
            command: String::new(),
            status: AuditStatus::Complete,
            generated_at: 0,
            summary: AuditSummary::default(),
            completion_ratio: None,
            truncation_reason: None,
            crawl: None,
            performance: None,
            site: None,
            findings: Vec::new(),
        }
    }
}

impl RuleClass {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Hard => "hard",
            Self::Policy => "policy",
            Self::Heuristic => "heuristic",
        }
    }
}

impl ConfidenceLevel {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::High => "high",
            Self::Medium => "medium",
            Self::Low => "low",
        }
    }
}

impl RuleMetadata {
    pub fn render_tag(&self) -> String {
        format!("{}/{}", self.class.as_str(), self.confidence.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AuditArtifact, AuditPageSnapshot, AuditSiteSnapshot, AuditStatus, Finding, FindingScope,
    };

    #[test]
    fn finding_renders_with_suggestion() {
        let finding = Finding {
            rule_id: "SEO001".into(),
            message: "missing title".into(),
            path: "index.html".into(),
            line: 1,
            column: 1,
            severity: "error".into(),
            suggestion: Some("add a title".into()),
            scope: FindingScope::Page,
            subject: None,
        };
        assert_eq!(
            finding.render(),
            "index.html:1:1 SEO001 missing title [add a title]"
        );
    }

    #[test]
    fn audit_artifact_defaults_to_complete() {
        let artifact = AuditArtifact {
            command: "check".into(),
            generated_at: 1,
            ..AuditArtifact::default()
        };
        assert_eq!(artifact.status, AuditStatus::Complete);
        assert_eq!(artifact.version, 2);
    }

    #[test]
    fn orphan_observation_keeps_legacy_baseline_fingerprint() {
        let old = Finding {
            rule_id: "LNK002".into(),
            message: "orphan page: /guide".into(),
            path: "guide.html".into(),
            line: 1,
            column: 1,
            severity: "warning".into(),
            suggestion: None,
            scope: FindingScope::Page,
            subject: None,
        };
        let mut current = old.clone();
        current.message = "no inbound links observed in the analyzed pages: /guide".into();
        assert_eq!(old.legacy_fingerprint(), current.legacy_fingerprint());
        current.rule_id = "OTHER".into();
        assert_ne!(old.legacy_fingerprint(), current.legacy_fingerprint());
    }

    #[test]
    fn fingerprint_ignores_wording_and_position_but_not_subject() {
        let old = Finding {
            rule_id: "LNK001".into(),
            message: "broken internal link: /a".into(),
            path: "guide.html".into(),
            line: 3,
            column: 1,
            severity: "error".into(),
            suggestion: None,
            scope: FindingScope::Page,
            subject: None,
        }
        .with_subject("a");
        let mut reworded = old.clone();
        reworded.message = "link target does not exist: /a".into();
        reworded.line = 9;
        assert_eq!(old.fingerprint(), reworded.fingerprint());
        assert_ne!(old.fingerprint(), reworded.with_subject("b").fingerprint());
    }

    #[test]
    fn audit_artifact_can_embed_a_site_snapshot() {
        let artifact = AuditArtifact {
            command: "crawl".into(),
            site: Some(AuditSiteSnapshot {
                root: "crawl".into(),
                deployment_model: "runtime_snapshot".into(),
                pages: vec![AuditPageSnapshot {
                    relative_path: "index.html".into(),
                    route: String::new(),
                    raw_html: "<html><body><h1>Home</h1></body></html>".into(),
                    ..AuditPageSnapshot::default()
                }],
                ..AuditSiteSnapshot::default()
            }),
            ..AuditArtifact::default()
        };
        let snapshot = artifact.site.as_ref().unwrap();
        assert_eq!(snapshot.pages.len(), 1);
        assert_eq!(snapshot.pages[0].relative_path, "index.html");
    }
}

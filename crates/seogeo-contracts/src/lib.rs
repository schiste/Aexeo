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
    pub fetch_ms: u64,
    pub process_ms: u64,
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
    pub truncated: bool,
    pub elapsed_ms: u64,
    pub pages_per_minute: usize,
    pub checkpoints_written: usize,
    pub partial_artifacts_written: usize,
    pub total_fetch_ms: u64,
    pub average_fetch_ms: u64,
    pub total_page_process_ms: u64,
    pub average_page_process_ms: u64,
    pub total_partial_audit_ms: u64,
    pub average_partial_audit_ms: u64,
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
    pub findings: Vec<Finding>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FindingFingerprint {
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

    pub fn fingerprint(&self) -> FindingFingerprint {
        FindingFingerprint {
            rule_id: self.rule_id.clone(),
            path: self.path.clone(),
            line: self.line,
            column: self.column,
            message: self.message.clone(),
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
    use super::{AuditArtifact, AuditStatus, Finding, FindingScope};

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
}

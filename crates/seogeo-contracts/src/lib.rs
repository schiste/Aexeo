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
    use super::Finding;

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
        };
        assert_eq!(
            finding.render(),
            "index.html:1:1 SEO001 missing title [add a title]"
        );
    }
}

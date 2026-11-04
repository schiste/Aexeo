//! Portable, offline checks for content supplied by an adapter.
//!
//! Missing input is reported separately from a passing gate. Adapters must not
//! represent an unavailable field as an empty string.
use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

/// The smallest common input shared by content adapters. The identifier is
/// opaque: it may be a file name, a URL, or a CMS document identifier.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContentInput {
    pub id: String,
    pub text: Option<String>,
    #[serde(default)]
    pub metadata: Option<BTreeMap<String, String>>,
}

/// A portable preset. Rules are evaluated in their declared order.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContentPreset {
    pub version: u32,
    pub name: String,
    pub gates: Vec<ContentGate>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ContentGate {
    WordCount {
        id: String,
        min: Option<usize>,
        max: Option<usize>,
    },
    ForbiddenTerms {
        id: String,
        terms: Vec<String>,
    },
    RequiredMetadata {
        id: String,
        fields: Vec<String>,
    },
}

impl ContentGate {
    pub fn id(&self) -> &str {
        match self {
            Self::WordCount { id, .. }
            | Self::ForbiddenTerms { id, .. }
            | Self::RequiredMetadata { id, .. } => id,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GateStatus {
    Passed,
    Failed,
    NotChecked,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentGateResult {
    pub gate_id: String,
    pub status: GateStatus,
    pub findings: Vec<aexeo_contracts::Finding>,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentReport {
    pub preset: String,
    pub input_id: String,
    pub gates: Vec<ContentGateResult>,
}

impl ContentPreset {
    pub fn from_toml(source: &str) -> Result<Self> {
        let preset: Self = toml::from_str(source)?;
        preset.validate()?;
        Ok(preset)
    }

    pub fn validate(&self) -> Result<()> {
        if self.version != 1 {
            bail!("unsupported content preset version: {}", self.version);
        }
        if self.name.trim().is_empty() {
            bail!("preset name must not be empty");
        }
        let mut ids = BTreeSet::new();
        for gate in &self.gates {
            if gate.id().trim().is_empty() || !ids.insert(gate.id()) {
                bail!("gate IDs must be nonempty and unique: {:?}", gate.id());
            }
            match gate {
                ContentGate::WordCount { min, max, .. }
                    if min.is_none() && max.is_none()
                        || matches!((min, max), (Some(a), Some(b)) if a > b) =>
                {
                    bail!("invalid word count bounds for {}", gate.id())
                }
                ContentGate::ForbiddenTerms { terms, .. }
                    if terms.is_empty() || terms.iter().any(|s| s.trim().is_empty()) =>
                {
                    bail!("forbidden terms must not be empty for {}", gate.id())
                }
                ContentGate::RequiredMetadata { fields, .. }
                    if fields.is_empty() || fields.iter().any(|s| s.trim().is_empty()) =>
                {
                    bail!("metadata fields must not be empty for {}", gate.id())
                }
                _ => {}
            }
        }
        Ok(())
    }

    pub fn evaluate(&self, input: &ContentInput) -> Result<ContentReport> {
        self.validate()?;
        let mut results = Vec::with_capacity(self.gates.len());
        for gate in &self.gates {
            let mut findings = Vec::new();
            let mut reason = None;
            let mut emit = |message: String, line: usize, column: usize| {
                findings.push(aexeo_contracts::Finding {
                    rule_id: gate.id().to_owned(),
                    message,
                    path: input.id.clone(),
                    line,
                    column,
                    severity: "warning".into(),
                    suggestion: None,
                    scope: aexeo_contracts::FindingScope::Page,
                })
            };
            match gate {
                ContentGate::WordCount { min, max, .. } => match &input.text {
                    Some(text) => {
                        let count = text.split_whitespace().count();
                        if min.is_some_and(|n| count < n) {
                            emit(
                                format!(
                                    "Word count {count} is below the minimum of {}",
                                    min.unwrap()
                                ),
                                1,
                                1,
                            );
                        }
                        if max.is_some_and(|n| count > n) {
                            emit(
                                format!(
                                    "Word count {count} exceeds the maximum of {}",
                                    max.unwrap()
                                ),
                                1,
                                1,
                            );
                        }
                    }
                    None => reason = Some("text is unavailable from this adapter".into()),
                },
                ContentGate::ForbiddenTerms { terms, .. } => match &input.text {
                    Some(text) => {
                        for term in terms {
                            for (offset, _) in text.match_indices(term) {
                                let before = &text[..offset];
                                let line = before.bytes().filter(|b| *b == b'\n').count() + 1;
                                let column =
                                    before.rsplit('\n').next().unwrap_or("").chars().count() + 1;
                                emit(format!("Forbidden term: {term}"), line, column);
                            }
                        }
                    }
                    None => reason = Some("text is unavailable from this adapter".into()),
                },
                ContentGate::RequiredMetadata { fields, .. } => match &input.metadata {
                    Some(metadata) => {
                        for field in fields {
                            if metadata.get(field).is_none_or(|v| v.trim().is_empty()) {
                                emit(format!("Required metadata is missing: {field}"), 1, 1);
                            }
                        }
                    }
                    None => reason = Some("metadata is unavailable from this adapter".into()),
                },
            }
            let status = if reason.is_some() {
                GateStatus::NotChecked
            } else if findings.is_empty() {
                GateStatus::Passed
            } else {
                GateStatus::Failed
            };
            results.push(ContentGateResult {
                gate_id: gate.id().into(),
                status,
                findings,
                reason,
            });
        }
        Ok(ContentReport {
            preset: self.name.clone(),
            input_id: input.id.clone(),
            gates: results,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const PRESET: &str = r#"
version = 1
name = "example"
[[gates]]
id = "length"
kind = "word_count"
min = 3
[[gates]]
id = "terms"
kind = "forbidden_terms"
terms = ["magic"]
[[gates]]
id = "title"
kind = "required_metadata"
fields = ["title"]
"#;
    #[test]
    fn checks_content_and_reports_locations() {
        let p = ContentPreset::from_toml(PRESET).unwrap();
        let input = ContentInput {
            id: "article.md".into(),
            text: Some("é\nmagic words".into()),
            metadata: Some(BTreeMap::new()),
            ..Default::default()
        };
        let r = p.evaluate(&input).unwrap();
        assert_eq!(
            r.gates.iter().map(|g| g.status).collect::<Vec<_>>(),
            [GateStatus::Passed, GateStatus::Failed, GateStatus::Failed]
        );
        assert_eq!(
            (r.gates[1].findings[0].line, r.gates[1].findings[0].column),
            (2, 1)
        );
    }
    #[test]
    fn unavailable_text_is_not_a_pass() {
        let r = ContentPreset::from_toml(PRESET)
            .unwrap()
            .evaluate(&ContentInput::default())
            .unwrap();
        assert_eq!(r.gates[0].status, GateStatus::NotChecked);
        assert!(r.gates[0].reason.is_some());
        assert_eq!(r.gates[2].status, GateStatus::NotChecked);
    }
    #[test]
    fn rejects_invalid_presets() {
        assert!(ContentPreset::from_toml(&PRESET.replace("min = 3", "min = 3\nmax = 2")).is_err());
        assert!(
            ContentPreset::from_toml(&PRESET.replace("id = \"terms\"", "id = \"length\"")).is_err()
        );
        assert!(ContentPreset::from_toml(&PRESET.replace("version = 1", "version = 2")).is_err());
        assert!(ContentPreset::from_toml(&PRESET.replace("min = 3", "mn = 3")).is_err());
    }
}

//! Portable constraints for structured editorial composition.
//!
//! These constraints describe structure, not editorial merit or authorship.
use std::collections::BTreeSet;

use anyhow::{Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const MAX_FIELDS: usize = 128;
const MAX_TEXT_BYTES: usize = 1_000_000;
const MAX_ARRAY_ITEMS: usize = 4096;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompositionProfile {
    pub version: u32,
    pub name: String,
    #[serde(default = "default_separator")]
    pub separator: String,
    pub fields: Vec<CompositionField>,
}

fn default_separator() -> String {
    "\n\n".to_string()
}

/// A built-in composition profile name or a complete custom profile.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CompositionProfileRef {
    Builtin(String),
    Inline(CompositionProfile),
}

impl CompositionProfileRef {
    pub fn resolve(&self) -> Result<CompositionProfile> {
        match self {
            Self::Builtin(name) => CompositionProfile::from_toml(
                builtin_profile(name)
                    .ok_or_else(|| anyhow::anyhow!("unknown composition profile"))?,
            ),
            Self::Inline(profile) => {
                profile.validate()?;
                Ok(profile.clone())
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompositionField {
    pub name: String,
    #[serde(default)]
    pub min_characters: Option<usize>,
    #[serde(default)]
    pub max_characters: Option<usize>,
    /// Empty means the field is a string. Otherwise it is a list of strings.
    #[serde(default)]
    pub allowed_counts: Vec<CountRange>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CountRange {
    pub min: Option<usize>,
    pub max: Option<usize>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CompositionIssue {
    pub field: String,
    pub code: &'static str,
    pub actual: Option<usize>,
    pub expected: Option<usize>,
}

/// Built-in starter profiles. Bounds are starting points, not editorial standards.
pub fn builtin_profile(name: &str) -> Option<&'static str> {
    match name {
        "news" => Some(include_str!("../composition-profiles/news.toml")),
        "opinion" => Some(include_str!("../composition-profiles/opinion.toml")),
        _ => None,
    }
}

impl CompositionProfile {
    pub fn from_toml(source: &str) -> Result<Self> {
        let profile: Self = toml::from_str(source)?;
        profile.validate()?;
        Ok(profile)
    }

    pub fn validate(&self) -> Result<()> {
        ensure!(self.version == 1, "unsupported composition profile version");
        ensure!(!self.name.trim().is_empty(), "profile name is empty");
        ensure!(
            !self.fields.is_empty() && self.fields.len() <= MAX_FIELDS,
            "profile must contain 1 to {MAX_FIELDS} fields"
        );
        ensure!(self.separator.len() <= 1024, "separator is too long");
        let mut names = BTreeSet::new();
        for field in &self.fields {
            ensure!(
                !field.name.trim().is_empty() && field.name.len() <= 1024,
                "field name is empty or too long"
            );
            ensure!(names.insert(&field.name), "duplicate field name");
            ensure!(
                field.allowed_counts.len() <= 32,
                "too many allowed count ranges"
            );
            if let (Some(min), Some(max)) = (field.min_characters, field.max_characters) {
                ensure!(min <= max, "inverted character bounds");
            }
            for range in &field.allowed_counts {
                ensure!(
                    range.min.is_some() || range.max.is_some(),
                    "empty count range"
                );
                if let (Some(min), Some(max)) = (range.min, range.max) {
                    ensure!(min <= max, "inverted count bounds");
                }
            }
        }
        Ok(())
    }

    /// Export a Draft 2020-12 schema equivalent to the structural checks.
    pub fn json_schema(&self) -> Result<Value> {
        self.validate()?;
        let mut properties = serde_json::Map::new();
        for field in &self.fields {
            let mut text = json!({"type":"string"});
            if let Some(min) = field.min_characters {
                text["minLength"] = json!(min);
            }
            if let Some(max) = field.max_characters {
                text["maxLength"] = json!(max);
            }
            let schema = if field.allowed_counts.is_empty() {
                text
            } else {
                let ranges: Vec<Value> = field
                    .allowed_counts
                    .iter()
                    .map(|range| {
                        let mut schema = json!({});
                        if let Some(min) = range.min {
                            schema["minItems"] = json!(min);
                        }
                        if let Some(max) = range.max {
                            schema["maxItems"] = json!(max);
                        }
                        schema
                    })
                    .collect();
                json!({"type":"array", "items":text, "anyOf":ranges})
            };
            properties.insert(field.name.clone(), schema);
        }
        Ok(
            json!({"$schema":"https://json-schema.org/draft/2020-12/schema", "type":"object", "properties":properties, "required":self.fields.iter().map(|field| &field.name).collect::<Vec<_>>(), "additionalProperties":false}),
        )
    }

    /// Check the exact supplied structure. No content is returned in diagnostics.
    pub fn assess(&self, input: &Value) -> Result<Vec<CompositionIssue>> {
        self.validate()?;
        let object = input
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("composition must be a JSON object"))?;
        ensure!(
            object.len() <= MAX_FIELDS,
            "composition object exceeds resource limit"
        );
        let mut issues = Vec::new();
        for name in object.keys() {
            if !self.fields.iter().any(|field| &field.name == name) {
                issues.push(CompositionIssue {
                    field: name.clone(),
                    code: "unexpected_field",
                    actual: None,
                    expected: None,
                });
            }
        }
        let mut total_bytes = 0usize;
        let mut total_items = 0usize;
        for field in &self.fields {
            let Some(value) = object.get(&field.name) else {
                issues.push(CompositionIssue {
                    field: field.name.clone(),
                    code: "missing_field",
                    actual: None,
                    expected: None,
                });
                continue;
            };
            let values: Vec<&str> = if field.allowed_counts.is_empty() {
                match value.as_str() {
                    Some(text) => vec![text],
                    None => {
                        issues.push(CompositionIssue {
                            field: field.name.clone(),
                            code: "expected_string",
                            actual: None,
                            expected: None,
                        });
                        continue;
                    }
                }
            } else {
                let Some(array) = value.as_array() else {
                    issues.push(CompositionIssue {
                        field: field.name.clone(),
                        code: "expected_array",
                        actual: None,
                        expected: None,
                    });
                    continue;
                };
                ensure!(
                    array.len() <= MAX_ARRAY_ITEMS,
                    "composition array exceeds resource limit"
                );
                if !field.allowed_counts.iter().any(|range| {
                    range.min.is_none_or(|min| array.len() >= min)
                        && range.max.is_none_or(|max| array.len() <= max)
                }) {
                    issues.push(CompositionIssue {
                        field: field.name.clone(),
                        code: "disallowed_count",
                        actual: Some(array.len()),
                        expected: None,
                    });
                }
                total_items = total_items
                    .checked_add(array.len())
                    .ok_or_else(|| anyhow::anyhow!("composition exceeds resource limit"))?;
                ensure!(
                    total_items <= MAX_ARRAY_ITEMS,
                    "composition arrays exceed resource limit"
                );
                let mut texts = Vec::with_capacity(array.len());
                for item in array {
                    if let Some(text) = item.as_str() {
                        texts.push(text);
                    } else {
                        issues.push(CompositionIssue {
                            field: field.name.clone(),
                            code: "expected_string_item",
                            actual: None,
                            expected: None,
                        });
                    }
                }
                texts
            };
            for text in values {
                total_bytes = total_bytes
                    .checked_add(text.len())
                    .ok_or_else(|| anyhow::anyhow!("composition exceeds resource limit"))?;
                ensure!(
                    total_bytes <= MAX_TEXT_BYTES,
                    "composition exceeds resource limit"
                );
                let length = text.chars().count();
                if let Some(min) = field.min_characters
                    && length < min
                {
                    issues.push(CompositionIssue {
                        field: field.name.clone(),
                        code: "too_short",
                        actual: Some(length),
                        expected: Some(min),
                    });
                }
                if let Some(max) = field.max_characters
                    && length > max
                {
                    issues.push(CompositionIssue {
                        field: field.name.clone(),
                        code: "too_long",
                        actual: Some(length),
                        expected: Some(max),
                    });
                }
            }
        }
        Ok(issues)
    }

    /// Assemble only a structurally valid snapshot, without rewriting its text.
    pub fn assemble(&self, input: &Value) -> Result<String> {
        if !self.assess(input)?.is_empty() {
            bail!("composition does not satisfy the profile");
        }
        self.assemble_assessed(input)
    }

    /// Reconstruct a well-formed snapshot even when advisory bounds fail.
    pub fn assemble_snapshot(&self, input: &Value) -> Result<String> {
        let issues = self.assess(input)?;
        if issues
            .iter()
            .any(|issue| !matches!(issue.code, "too_short" | "too_long" | "disallowed_count"))
        {
            bail!("composition has invalid structure");
        }
        self.assemble_assessed(input)
    }

    // Both callers have already assessed the same immutable input.
    fn assemble_assessed(&self, input: &Value) -> Result<String> {
        let object = input
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("composition must be a JSON object"))?;
        let mut parts = Vec::new();
        for field in &self.fields {
            let value = object
                .get(&field.name)
                .ok_or_else(|| anyhow::anyhow!("composition field is missing"))?;
            if let Some(text) = value.as_str() {
                parts.push(text);
            } else if let Some(array) = value.as_array() {
                for item in array {
                    parts.push(
                        item.as_str()
                            .ok_or_else(|| anyhow::anyhow!("composition item is not a string"))?,
                    );
                }
            }
        }
        let total = parts
            .iter()
            .try_fold(0usize, |sum, part| sum.checked_add(part.len()))
            .and_then(|sum| {
                self.separator
                    .len()
                    .checked_mul(parts.len().saturating_sub(1))
                    .and_then(|separators| sum.checked_add(separators))
            })
            .ok_or_else(|| anyhow::anyhow!("assembled composition exceeds resource limit"))?;
        ensure!(
            total <= MAX_TEXT_BYTES,
            "assembled composition exceeds resource limit"
        );
        Ok(parts.join(&self.separator))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const PROFILE: &str = "version=1\nname='test'\n[[fields]]\nname='hook'\nmax_characters=3\n[[fields]]\nname='examples'\nallowed_counts=[{max=2},{min=4}]\n";

    #[test]
    fn schema_and_validation_share_bounds_and_preserve_order() {
        let profile = CompositionProfile::from_toml(PROFILE).unwrap();
        let schema = profile.json_schema().unwrap();
        assert_eq!(
            schema["properties"]["examples"]["anyOf"],
            json!([{"maxItems":2},{"minItems":4}])
        );
        assert_eq!(schema["properties"]["hook"]["maxLength"], 3);
        let valid = json!({"examples":["one","two"],"hook":"é😀!"});
        assert!(profile.assess(&valid).unwrap().is_empty());
        assert!(
            profile
                .assess(&json!({"hook":"ok", "examples":["a","b","c","d"]}))
                .unwrap()
                .is_empty()
        );
        assert_eq!(profile.assemble(&valid).unwrap(), "é😀!\n\none\n\ntwo");
        let bad = json!({"hook":"four", "examples":["a","b","c"]});
        let issues = profile.assess(&bad).unwrap();
        assert_eq!(
            issues.iter().map(|i| i.code).collect::<Vec<_>>(),
            vec!["too_long", "disallowed_count"]
        );
        assert!(profile.assemble(&bad).is_err());
        assert_eq!(
            profile.assemble_snapshot(&bad).unwrap(),
            "four\n\na\n\nb\n\nc"
        );
    }

    #[test]
    fn gate_only_reports_bound_document_findings() {
        use crate::content_gate::{ContentGate, ContentInput, ContentPreset, GateStatus, Severity};
        use std::collections::BTreeMap;
        let profile = CompositionProfile::from_toml(PROFILE).unwrap();
        let preset = ContentPreset {
            version: 1,
            name: "pacing".into(),
            gates: vec![ContentGate::EditorialPacing {
                id: "pacing".into(),
                severity: Severity::Warning,
                blocking: false,
                source: "composition".into(),
                profile: CompositionProfileRef::Inline(profile),
                projection_separator: None,
            }],
        };
        let mut input = ContentInput {
            id: "draft".into(),
            source_text: Some("four\n\na\n\nb\n\nc".into()),
            evidence_sources: Some(BTreeMap::from([(
                "composition".into(),
                r#"{"hook":"four","examples":["a","b","c"]}"#.into(),
            )])),
            ..Default::default()
        };
        let report = preset.evaluate(&input).unwrap();
        assert_eq!(report.gates[0].status, GateStatus::Failed);
        assert_eq!(report.gates[0].findings.len(), 2);
        assert!(!report.gates[0].blocking);
        input.source_text = Some("edited text".into());
        let stale = preset.evaluate(&input).unwrap();
        assert_eq!(stale.gates[0].status, GateStatus::NotChecked);
        assert!(stale.gates[0].findings.is_empty());
        input.evidence_sources = None;
        assert_eq!(
            preset.evaluate(&input).unwrap().gates[0].status,
            GateStatus::NotChecked
        );
    }

    #[test]
    fn oversized_assembled_output_is_rejected() {
        let profile = CompositionProfile::from_toml("version=1\nname='x'\nseparator='---'\n[[fields]]\nname='items'\nallowed_counts=[{min=0}]\n").unwrap();
        let input = json!({"items": vec!["a"; MAX_ARRAY_ITEMS + 1]});
        assert!(profile.assess(&input).is_err());
        let mut profile = profile;
        profile.separator = "x".repeat(1024);
        let input = json!({"items": vec![""; 1000]});
        assert!(profile.assess(&input).is_ok());
        assert!(profile.assemble(&input).is_err());
        assert!(profile.assemble_snapshot(&input).is_err());
        let boundary = json!({"items": ["a".repeat(MAX_TEXT_BYTES - 1024), ""]});
        assert_eq!(profile.assemble(&boundary).unwrap().len(), MAX_TEXT_BYTES);
        assert_eq!(
            profile.assemble_snapshot(&boundary).unwrap().len(),
            MAX_TEXT_BYTES
        );
    }

    #[test]
    fn missing_unknown_wrong_types_and_invalid_profiles_are_rejected() {
        let profile = CompositionProfile::from_toml(PROFILE).unwrap();
        let issues = profile
            .assess(&json!({"extra":1,"examples":[false]}))
            .unwrap();
        assert_eq!(
            issues.iter().map(|i| i.code).collect::<Vec<_>>(),
            vec!["unexpected_field", "missing_field", "expected_string_item"]
        );
        assert!(
            CompositionProfile::from_toml(
                "version=1\nname='x'\n[[fields]]\nname='x'\nmin_characters=4\nmax_characters=1\n"
            )
            .is_err()
        );
        assert!(
            CompositionProfile::from_toml(
                "version=1\nname='x'\n[[fields]]\nname='x'\n[[fields]]\nname='x'\n"
            )
            .is_err()
        );
    }
}

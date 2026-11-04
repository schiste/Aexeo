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
    pub source_text: Option<String>,
    #[serde(default)]
    pub headings: Option<Vec<String>>,
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
        #[serde(default)]
        severity: Severity,
        #[serde(default = "default_blocking")]
        blocking: bool,
        min: Option<usize>,
        max: Option<usize>,
    },
    ForbiddenTerms {
        id: String,
        #[serde(default)]
        severity: Severity,
        #[serde(default = "default_blocking")]
        blocking: bool,
        terms: Vec<String>,
    },
    RequiredMetadata {
        id: String,
        #[serde(default)]
        severity: Severity,
        #[serde(default = "default_blocking")]
        blocking: bool,
        fields: Vec<String>,
    },
    RequiredHeadings {
        id: String,
        #[serde(default)]
        severity: Severity,
        #[serde(default = "default_blocking")]
        blocking: bool,
        headings: Vec<String>,
    },
    FinalNewline {
        id: String,
        #[serde(default)]
        severity: Severity,
        #[serde(default = "default_blocking")]
        blocking: bool,
    },
    /// A host-provided implementation, selected by a portable identifier.
    Custom {
        id: String,
        extension: String,
        #[serde(default)]
        parameters: BTreeMap<String, String>,
        #[serde(default)]
        severity: Severity,
        #[serde(default = "default_blocking")]
        blocking: bool,
    },
    /// One existing native website rule group.
    SiteGroup {
        id: String,
        group: String,
        #[serde(default = "default_blocking")]
        blocking: bool,
    },
    /// The existing SEO/GEO site evaluator, including its configured policy.
    SiteAudit {
        id: String,
        #[serde(default = "default_blocking")]
        blocking: bool,
    },
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    #[default]
    Warning,
}

fn default_blocking() -> bool {
    true
}

impl ContentGate {
    pub fn severity(&self) -> Severity {
        match self {
            Self::WordCount { severity, .. }
            | Self::ForbiddenTerms { severity, .. }
            | Self::RequiredMetadata { severity, .. }
            | Self::RequiredHeadings { severity, .. }
            | Self::FinalNewline { severity, .. }
            | Self::Custom { severity, .. } => *severity,
            Self::SiteAudit { .. } | Self::SiteGroup { .. } => Severity::Warning,
        }
    }
    pub fn blocking(&self) -> bool {
        match self {
            Self::WordCount { blocking, .. }
            | Self::ForbiddenTerms { blocking, .. }
            | Self::RequiredMetadata { blocking, .. }
            | Self::RequiredHeadings { blocking, .. }
            | Self::FinalNewline { blocking, .. }
            | Self::Custom { blocking, .. }
            | Self::SiteAudit { blocking, .. }
            | Self::SiteGroup { blocking, .. } => *blocking,
        }
    }
    pub fn required_capability(&self) -> &'static str {
        match self {
            Self::WordCount { .. } | Self::ForbiddenTerms { .. } => "text",
            Self::RequiredMetadata { .. } => "metadata",
            Self::RequiredHeadings { .. } => "headings",
            Self::FinalNewline { .. } => "source_text",
            Self::Custom { .. } => "extension",
            Self::SiteAudit { .. } | Self::SiteGroup { .. } => "site",
        }
    }
    pub fn id(&self) -> &str {
        match self {
            Self::WordCount { id, .. }
            | Self::ForbiddenTerms { id, .. }
            | Self::RequiredMetadata { id, .. }
            | Self::RequiredHeadings { id, .. }
            | Self::FinalNewline { id, .. }
            | Self::Custom { id, .. }
            | Self::SiteAudit { id, .. }
            | Self::SiteGroup { id, .. } => id,
        }
    }
}

/// A diagnostic returned by a trusted host extension. Positions refer to the
/// input's projected text, matching the built-in content gate convention.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionDiagnostic {
    pub message: String,
    pub line: usize,
    pub column: usize,
}

/// Resolve and evaluate custom gate identifiers in the host. Return `None`
/// when no implementation is available; this becomes `not_checked`.
pub trait ContentGateExtensions {
    fn evaluate(
        &self,
        extension: &str,
        parameters: &BTreeMap<String, String>,
        input: &ContentInput,
    ) -> Result<Option<Vec<ExtensionDiagnostic>>>;
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
    pub blocking: bool,
    pub scope: String,
    pub required_capability: String,
    pub findings: Vec<aexeo_contracts::Finding>,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentReport {
    pub preset: String,
    pub input_id: String,
    pub gates: Vec<ContentGateResult>,
    #[serde(default)]
    pub applied_fixes: Vec<String>,
}

impl ContentPreset {
    /// Serialize a resolved preset as portable TOML.
    pub fn to_toml(&self) -> Result<String> {
        self.validate()?;
        Ok(toml::to_string_pretty(self)?)
    }

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
                ContentGate::RequiredHeadings { headings, .. }
                    if headings.is_empty() || headings.iter().any(|s| s.trim().is_empty()) =>
                {
                    bail!("required headings must not be empty for {}", gate.id())
                }
                ContentGate::Custom { extension, .. } if extension.trim().is_empty() => {
                    bail!("extension name must not be empty for {}", gate.id())
                }
                ContentGate::SiteGroup { group, .. }
                    if !crate::static_check::has_native_site_rule_group(group) =>
                {
                    bail!("unknown site rule group for {}: {group}", gate.id())
                }
                _ => {}
            }
        }
        Ok(())
    }

    pub fn evaluate(&self, input: &ContentInput) -> Result<ContentReport> {
        self.evaluate_with_site(input, None)
    }

    /// A site adapter can provide the existing site and its resolved config.
    /// Without this capability site gates are reported as not checked.
    pub fn evaluate_with_site(
        &self,
        input: &ContentInput,
        site: Option<(&crate::site::Site, &crate::config::Config)>,
    ) -> Result<ContentReport> {
        self.evaluate_with_extensions(input, site, None)
    }

    /// Evaluate with optional host extensions. Missing implementations are
    /// never treated as successful gates.
    pub fn evaluate_with_extensions(
        &self,
        input: &ContentInput,
        site: Option<(&crate::site::Site, &crate::config::Config)>,
        extensions: Option<&dyn ContentGateExtensions>,
    ) -> Result<ContentReport> {
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
                    severity: match gate.severity() {
                        Severity::Error => "error",
                        Severity::Warning => "warning",
                    }
                    .into(),
                    suggestion: None,
                    scope: aexeo_contracts::FindingScope::Page,
                })
            };
            match gate {
                ContentGate::WordCount { min, max, .. } => match &input.text {
                    Some(text) => {
                        let count = text.split_whitespace().count();
                        if let Some(minimum) = min.filter(|n| count < *n) {
                            emit(
                                format!("Word count {count} is below the minimum of {}", minimum),
                                1,
                                1,
                            );
                        }
                        if let Some(maximum) = max.filter(|n| count > *n) {
                            emit(
                                format!("Word count {count} exceeds the maximum of {}", maximum),
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
                            let (mut cursor, mut line, mut column) = (0, 1, 1);
                            for (offset, _) in text.match_indices(term) {
                                for ch in text[cursor..offset].chars() {
                                    if ch == '\n' {
                                        line += 1;
                                        column = 1;
                                    } else {
                                        column += 1;
                                    }
                                }
                                emit(format!("Forbidden term: {term}"), line, column);
                                cursor = offset;
                            }
                        }
                    }
                    None => reason = Some("text is unavailable from this adapter".into()),
                },
                ContentGate::RequiredHeadings { headings, .. } => match &input.headings {
                    Some(actual) => {
                        for heading in headings {
                            if !actual.iter().any(|item| item == heading) {
                                emit(format!("Required heading is missing: {heading}"), 1, 1);
                            }
                        }
                    }
                    None => reason = Some("headings are unavailable from this adapter".into()),
                },
                ContentGate::FinalNewline { .. } => match &input.source_text {
                    Some(source) if !source.is_empty() && !source.ends_with('\n') => {
                        emit(
                            "Source has no final newline".into(),
                            source.lines().count().max(1),
                            1,
                        );
                    }
                    Some(_) => {}
                    None => reason = Some("source text is unavailable from this adapter".into()),
                },
                ContentGate::Custom {
                    extension,
                    parameters,
                    ..
                } => {
                    match extensions
                        .map(|host| host.evaluate(extension, parameters, input))
                        .transpose()?
                        .flatten()
                    {
                        Some(diagnostics) => {
                            for diagnostic in diagnostics {
                                emit(diagnostic.message, diagnostic.line, diagnostic.column);
                            }
                        }
                        None => reason = Some(format!("extension is unavailable: {extension}")),
                    }
                }
                ContentGate::SiteGroup { group, .. } => match site {
                    Some((site, config)) => {
                        match crate::static_check::run_native_site_rule_group(site, config, group)?
                        {
                            Some(results) => findings.extend(results),
                            None => {
                                reason = Some(format!(
                                    "site rule group {group} is disabled by site config"
                                ))
                            }
                        }
                    }
                    None => reason = Some("site is unavailable from this adapter".into()),
                },
                ContentGate::SiteAudit { .. } => match site {
                    Some((site, config)) => {
                        findings.extend(crate::static_check::run_checks_for_site(site, config));
                    }
                    None => reason = Some("site is unavailable from this adapter".into()),
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
                blocking: gate.blocking(),
                scope: if matches!(
                    gate,
                    ContentGate::SiteAudit { .. } | ContentGate::SiteGroup { .. }
                ) {
                    "site"
                } else {
                    "document"
                }
                .into(),
                required_capability: gate.required_capability().into(),
                findings,
                reason,
            });
        }
        Ok(ContentReport {
            preset: self.name.clone(),
            input_id: input.id.clone(),
            gates: results,
            applied_fixes: Vec::new(),
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
    fn custom_extensions_are_configurable_and_missing_is_not_checked() {
        struct Example;
        impl ContentGateExtensions for Example {
            fn evaluate(
                &self,
                extension: &str,
                parameters: &BTreeMap<String, String>,
                input: &ContentInput,
            ) -> Result<Option<Vec<ExtensionDiagnostic>>> {
                if extension != "example.contains" {
                    return Ok(None);
                }
                let needle = parameters.get("term").context("missing term")?;
                let text = input.text.as_deref().context("missing text")?;
                Ok(Some(if text.contains(needle) {
                    vec![ExtensionDiagnostic {
                        message: "term found".into(),
                        line: 2,
                        column: 3,
                    }]
                } else {
                    vec![]
                }))
            }
        }
        use anyhow::Context;
        let preset = ContentPreset::from_toml(
            r#"version=1
name="custom"
[[gates]]
id="style.custom"
kind="custom"
extension="example.contains"
severity="error"
[gates.parameters]
term="draft"
"#,
        )
        .unwrap();
        let input = ContentInput {
            id: "doc".into(),
            text: Some("draft".into()),
            ..Default::default()
        };
        let absent = preset.evaluate(&input).unwrap();
        assert_eq!(absent.gates[0].status, GateStatus::NotChecked);
        assert_eq!(absent.gates[0].required_capability, "extension");
        let found = preset
            .evaluate_with_extensions(&input, None, Some(&Example))
            .unwrap();
        assert_eq!(found.gates[0].status, GateStatus::Failed);
        let finding = &found.gates[0].findings[0];
        assert_eq!(
            (
                &finding.rule_id,
                &finding.severity,
                finding.line,
                finding.column
            ),
            (&"style.custom".to_string(), &"error".to_string(), 2, 3)
        );
        let clean = preset
            .evaluate_with_extensions(
                &ContentInput {
                    text: Some("clean".into()),
                    ..input
                },
                None,
                Some(&Example),
            )
            .unwrap();
        assert_eq!(clean.gates[0].status, GateStatus::Passed);
        assert!(ContentPreset::from_toml(&preset.to_toml().unwrap()).is_ok());
    }

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
    #[test]
    fn site_groups_keep_policy_and_report_disabled_checks() {
        let preset = ContentPreset::from_toml(
            r#"
version = 1
name = "site"
[[gates]]
id = "html"
kind = "site_group"
group = "html"
"#,
        )
        .unwrap();
        let root = tempfile::tempdir().unwrap();
        std::fs::write(
            root.path().join("index.html"),
            "<html><body>Example</body></html>",
        )
        .unwrap();
        let site = crate::load_site(root.path()).unwrap();
        let mut config = crate::Config::default();
        config.ignore_rules.push("SEO002".into());
        let input = ContentInput::default();
        let report = preset
            .evaluate_with_site(&input, Some((&site, &config)))
            .unwrap();
        let expected = crate::static_check::run_native_site_rule_group(&site, &config, "html")
            .unwrap()
            .unwrap();
        assert_eq!(report.gates[0].findings, expected);
        assert!(
            !report.gates[0]
                .findings
                .iter()
                .any(|finding| finding.rule_id == "SEO002")
        );
        config.checks.insert("html".into(), false);
        let report = preset
            .evaluate_with_site(&input, Some((&site, &config)))
            .unwrap();
        assert_eq!(report.gates[0].status, GateStatus::NotChecked);
        assert!(
            ContentPreset::from_toml(
                &toml::to_string(&preset)
                    .unwrap()
                    .replace("group = \"html\"", "group = \"typo\"")
            )
            .is_err()
        );
    }
    #[test]
    fn repeated_terms_keep_unicode_line_and_column_positions() {
        let preset = ContentPreset::from_toml(
            r#"
version = 1
name = "terms"
[[gates]]
id = "terms"
kind = "forbidden_terms"
terms = ["bad"]
"#,
        )
        .unwrap();
        let input = ContentInput {
            text: Some("é bad bad\n🦀 bad".into()),
            ..Default::default()
        };
        let result = preset.evaluate(&input).unwrap();
        let positions = result.gates[0]
            .findings
            .iter()
            .map(|f| (f.line, f.column))
            .collect::<Vec<_>>();
        assert_eq!(positions, [(1, 3), (1, 7), (2, 3)]);
    }
}

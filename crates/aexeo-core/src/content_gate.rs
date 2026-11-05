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
    /// Caller-supplied source snapshots keyed by a stable local name.
    #[serde(default)]
    pub evidence_sources: Option<BTreeMap<String, String>>,
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
    /// Heuristic sentence length using configured punctuation and whitespace words.
    SentenceLength {
        id: String,
        #[serde(default)]
        severity: Severity,
        #[serde(default = "default_blocking")]
        blocking: bool,
        max_words: usize,
        #[serde(default = "default_sentence_delimiters")]
        delimiters: String,
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
    /// Checks literal anchors only; it does not assess entailment or truth.
    EvidenceAnchors {
        id: String,
        source: String,
        claim: String,
        quote: String,
        #[serde(default)]
        severity: Severity,
        #[serde(default = "default_blocking")]
        blocking: bool,
    },
    /// Compare an explicitly mapped claim with a field in a JSON snapshot.
    StructuredFact {
        id: String,
        source: String,
        claim: String,
        pointer: String,
        expected: serde_json::Value,
        #[serde(default)]
        severity: Severity,
        #[serde(default = "default_blocking")]
        blocking: bool,
    },
    /// Consume a caller-supplied review bound to the exact claim and snapshot.
    EvidenceReview {
        id: String,
        source: String,
        review: String,
        claim: String,
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

fn default_sentence_delimiters() -> String {
    ".!?".into()
}

fn default_blocking() -> bool {
    true
}

impl ContentGate {
    pub fn severity(&self) -> Severity {
        match self {
            Self::WordCount { severity, .. }
            | Self::SentenceLength { severity, .. }
            | Self::ForbiddenTerms { severity, .. }
            | Self::RequiredMetadata { severity, .. }
            | Self::RequiredHeadings { severity, .. }
            | Self::FinalNewline { severity, .. }
            | Self::Custom { severity, .. }
            | Self::EvidenceAnchors { severity, .. }
            | Self::StructuredFact { severity, .. }
            | Self::EvidenceReview { severity, .. } => *severity,
            Self::SiteAudit { .. } | Self::SiteGroup { .. } => Severity::Warning,
        }
    }
    pub fn blocking(&self) -> bool {
        match self {
            Self::WordCount { blocking, .. }
            | Self::SentenceLength { blocking, .. }
            | Self::ForbiddenTerms { blocking, .. }
            | Self::RequiredMetadata { blocking, .. }
            | Self::RequiredHeadings { blocking, .. }
            | Self::FinalNewline { blocking, .. }
            | Self::Custom { blocking, .. }
            | Self::EvidenceAnchors { blocking, .. }
            | Self::StructuredFact { blocking, .. }
            | Self::EvidenceReview { blocking, .. }
            | Self::SiteAudit { blocking, .. }
            | Self::SiteGroup { blocking, .. } => *blocking,
        }
    }
    pub fn required_capability(&self) -> &'static str {
        match self {
            Self::WordCount { .. } | Self::SentenceLength { .. } | Self::ForbiddenTerms { .. } => {
                "text"
            }
            Self::RequiredMetadata { .. } => "metadata",
            Self::RequiredHeadings { .. } => "headings",
            Self::FinalNewline { .. } => "source_text",
            Self::Custom { .. } => "extension",
            Self::EvidenceAnchors { .. }
            | Self::StructuredFact { .. }
            | Self::EvidenceReview { .. } => "text_and_evidence",
            Self::SiteAudit { .. } | Self::SiteGroup { .. } => "site",
        }
    }
    pub fn id(&self) -> &str {
        match self {
            Self::WordCount { id, .. }
            | Self::SentenceLength { id, .. }
            | Self::ForbiddenTerms { id, .. }
            | Self::RequiredMetadata { id, .. }
            | Self::RequiredHeadings { id, .. }
            | Self::FinalNewline { id, .. }
            | Self::Custom { id, .. }
            | Self::EvidenceAnchors { id, .. }
            | Self::StructuredFact { id, .. }
            | Self::EvidenceReview { id, .. }
            | Self::SiteAudit { id, .. }
            | Self::SiteGroup { id, .. } => id,
        }
    }
}

/// An assessment produced by an external reviewer; it is not verified by Aexeo.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EvidenceReviewRecord {
    claim: String,
    source_text: String,
    support: SupportVerdict,
    reliability: ReliabilityVerdict,
    reviewer: String,
    #[serde(default, rename = "provenance")]
    _provenance: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum SupportVerdict {
    Supported,
    Contradicted,
    Uncertain,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ReliabilityVerdict {
    Reliable,
    Unreliable,
    Uncertain,
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
    /// Non-sensitive measurements and configured limits for this gate.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub measurements: std::collections::BTreeMap<String, usize>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ContentSummary {
    pub passed: usize,
    pub failed: usize,
    pub not_checked: usize,
    pub blocking_failed: usize,
    pub blocking_not_checked: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentReport {
    pub preset: String,
    pub input_id: String,
    pub gates: Vec<ContentGateResult>,
    #[serde(default)]
    pub summary: ContentSummary,
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
                ContentGate::SentenceLength {
                    max_words,
                    delimiters,
                    ..
                } if *max_words == 0 || delimiters.is_empty() => {
                    bail!(
                        "sentence length needs a positive limit and delimiters for {}",
                        gate.id()
                    )
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
                ContentGate::EvidenceAnchors {
                    source,
                    claim,
                    quote,
                    ..
                } if source.trim().is_empty()
                    || claim.trim().is_empty()
                    || quote.trim().is_empty() =>
                {
                    bail!(
                        "evidence source, claim and quote must not be empty for {}",
                        gate.id()
                    )
                }
                ContentGate::StructuredFact {
                    source,
                    claim,
                    pointer,
                    ..
                } if source.trim().is_empty()
                    || claim.trim().is_empty()
                    || (!pointer.is_empty() && !pointer.starts_with('/')) =>
                {
                    bail!(
                        "structured fact needs a source, claim and valid JSON pointer for {}",
                        gate.id()
                    )
                }
                ContentGate::EvidenceReview {
                    source,
                    review,
                    claim,
                    ..
                } if source.trim().is_empty()
                    || review.trim().is_empty()
                    || claim.trim().is_empty()
                    || source == review =>
                {
                    bail!(
                        "evidence review requires distinct source and review names and a claim for {}",
                        gate.id()
                    )
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
        let mut word_count = None;
        for gate in &self.gates {
            let mut findings = Vec::new();
            let mut reason = None;
            let mut measurements = std::collections::BTreeMap::new();
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
                ContentGate::WordCount { min, max, .. } => {
                    if let Some(min) = min {
                        measurements.insert("min_words".into(), *min);
                    }
                    if let Some(max) = max {
                        measurements.insert("max_words".into(), *max);
                    }
                    match &input.text {
                        Some(text) => {
                            let count =
                                *word_count.get_or_insert_with(|| text.split_whitespace().count());
                            measurements.insert("word_count".into(), count);
                            if let Some(minimum) = min.filter(|n| count < *n) {
                                emit(
                                    format!(
                                        "Word count {count} is below the minimum of {}",
                                        minimum
                                    ),
                                    1,
                                    1,
                                );
                            }
                            if let Some(maximum) = max.filter(|n| count > *n) {
                                emit(
                                    format!(
                                        "Word count {count} exceeds the maximum of {}",
                                        maximum
                                    ),
                                    1,
                                    1,
                                );
                            }
                        }
                        None => reason = Some("text is unavailable from this adapter".into()),
                    }
                }
                ContentGate::SentenceLength {
                    max_words,
                    delimiters,
                    ..
                } => {
                    measurements.insert("max_words".into(), *max_words);
                    match &input.text {
                        Some(text) => {
                            let mut segment_start = 0;
                            let mut sentence_count = 0;
                            let mut longest = 0;
                            let mut line = 1;
                            let mut column = 1;
                            let mut first_position = None;
                            for (offset, ch) in text
                                .char_indices()
                                .map(|(offset, ch)| (offset, Some(ch)))
                                .chain(std::iter::once((text.len(), None)))
                            {
                                if ch.is_none_or(|ch| delimiters.contains(ch)) {
                                    let count =
                                        text[segment_start..offset].split_whitespace().count();
                                    if count > 0 {
                                        sentence_count += 1;
                                        longest = longest.max(count);
                                        if count > *max_words {
                                            let (start_line, start_column) =
                                                first_position.unwrap_or((line, column));
                                            emit(
                                                format!(
                                                    "Sentence has {count} words; maximum is {max_words}"
                                                ),
                                                start_line,
                                                start_column,
                                            );
                                        }
                                    }
                                    segment_start = offset + ch.map_or(0, char::len_utf8);
                                    first_position = None;
                                } else if ch.is_some_and(|ch| !ch.is_whitespace())
                                    && first_position.is_none()
                                {
                                    first_position = Some((line, column));
                                }
                                if ch == Some('\n') {
                                    line += 1;
                                    column = 1;
                                } else {
                                    column += 1;
                                }
                            }
                            measurements.insert("sentence_count".into(), sentence_count);
                            measurements.insert("longest_sentence_words".into(), longest);
                        }
                        None => reason = Some("text is unavailable from this adapter".into()),
                    }
                }
                ContentGate::ForbiddenTerms { terms, .. } => {
                    measurements.insert("configured_terms".into(), terms.len());
                    match &input.text {
                        Some(text) => {
                            let mut matches = 0;
                            for term in terms {
                                let (mut cursor, mut line, mut column) = (0, 1, 1);
                                for (offset, _) in text.match_indices(term) {
                                    matches += 1;
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
                            measurements.insert("matches".into(), matches);
                        }
                        None => reason = Some("text is unavailable from this adapter".into()),
                    }
                }
                ContentGate::RequiredHeadings { headings, .. } => {
                    measurements.insert("required_headings".into(), headings.len());
                    match &input.headings {
                        Some(actual) => {
                            measurements.insert(
                                "present_headings".into(),
                                headings
                                    .iter()
                                    .filter(|heading| actual.contains(*heading))
                                    .count(),
                            );
                            for heading in headings {
                                if !actual.iter().any(|item| item == heading) {
                                    emit(format!("Required heading is missing: {heading}"), 1, 1);
                                }
                            }
                        }
                        None => reason = Some("headings are unavailable from this adapter".into()),
                    }
                }
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
                ContentGate::EvidenceAnchors {
                    source,
                    claim,
                    quote,
                    ..
                } => match (&input.text, &input.evidence_sources) {
                    (Some(text), Some(sources)) => match sources.get(source) {
                        Some(snapshot) => {
                            if !text.contains(claim) {
                                emit("Claim anchor is absent from content".into(), 1, 1);
                            }
                            if !snapshot.contains(quote) {
                                emit(format!("Quote anchor is absent from source {source}"), 1, 1);
                            }
                        }
                        None => reason = Some(format!("evidence source is unavailable: {source}")),
                    },
                    _ => reason = Some("content text or evidence sources are unavailable".into()),
                },
                ContentGate::StructuredFact {
                    source,
                    claim,
                    pointer,
                    expected,
                    ..
                } => {
                    match (
                        &input.text,
                        input.evidence_sources.as_ref().and_then(|s| s.get(source)),
                    ) {
                        (Some(text), Some(snapshot)) => match serde_json::from_str::<
                            serde_json::Value,
                        >(snapshot)
                        {
                            Ok(value) => match value.pointer(pointer) {
                                Some(actual) => {
                                    if !text.contains(claim) {
                                        emit("Claim anchor is absent from content".into(), 1, 1);
                                    }
                                    if actual != expected {
                                        emit(
                                            format!(
                                                "Structured source value differs from expected at {pointer}"
                                            ),
                                            1,
                                            1,
                                        );
                                    }
                                }
                                None => {
                                    reason = Some(format!(
                                        "JSON pointer is unavailable in evidence source {source}: {pointer}"
                                    ))
                                }
                            },
                            Err(_) => {
                                reason =
                                    Some(format!("evidence source is not valid JSON: {source}"))
                            }
                        },
                        _ => {
                            reason = Some(format!(
                                "content text or evidence source is unavailable: {source}"
                            ))
                        }
                    }
                }
                ContentGate::EvidenceReview {
                    source,
                    review,
                    claim,
                    ..
                } => match (&input.text, &input.evidence_sources) {
                    (Some(text), Some(sources)) => match (sources.get(source), sources.get(review))
                    {
                        (Some(snapshot), Some(record)) => {
                            match serde_json::from_str::<EvidenceReviewRecord>(record) {
                                Ok(record)
                                    if record.claim == *claim
                                        && record.source_text == *snapshot
                                        && !record.reviewer.trim().is_empty() =>
                                {
                                    if !text.contains(claim) {
                                        emit("Claim anchor is absent from content".into(), 1, 1);
                                    }
                                    match (&record.support, &record.reliability) {
                                        (SupportVerdict::Contradicted, _) => emit("External review reports that the source contradicts the claim".into(), 1, 1),
                                        (_, ReliabilityVerdict::Unreliable) => emit("External review reports that the source is unreliable".into(), 1, 1),
                                        (SupportVerdict::Supported, ReliabilityVerdict::Reliable) => {},
                                        _ => reason = Some("external review is uncertain".into()),
                                    }
                                }
                                Ok(_) => {
                                    reason = Some(
                                        "external review is stale or missing a reviewer".into(),
                                    )
                                }
                                Err(_) => reason = Some("external review is invalid".into()),
                            }
                        }
                        _ => reason = Some("evidence source or review is unavailable".into()),
                    },
                    _ => reason = Some("content text or evidence sources are unavailable".into()),
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
                ContentGate::RequiredMetadata { fields, .. } => {
                    measurements.insert("required_fields".into(), fields.len());
                    match &input.metadata {
                        Some(metadata) => {
                            measurements.insert(
                                "present_fields".into(),
                                fields
                                    .iter()
                                    .filter(|field| {
                                        metadata.get(*field).is_some_and(|v| !v.trim().is_empty())
                                    })
                                    .count(),
                            );
                            for field in fields {
                                if metadata.get(field).is_none_or(|v| v.trim().is_empty()) {
                                    emit(format!("Required metadata is missing: {field}"), 1, 1);
                                }
                            }
                        }
                        None => reason = Some("metadata is unavailable from this adapter".into()),
                    }
                }
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
                measurements,
            });
        }
        let mut summary = ContentSummary::default();
        for gate in &results {
            match gate.status {
                GateStatus::Passed => summary.passed += 1,
                GateStatus::Failed => {
                    summary.failed += 1;
                    if gate.blocking {
                        summary.blocking_failed += 1;
                    }
                }
                GateStatus::NotChecked => {
                    summary.not_checked += 1;
                    if gate.blocking {
                        summary.blocking_not_checked += 1;
                    }
                }
            }
        }
        Ok(ContentReport {
            summary,
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
    fn multiple_word_count_gates_share_the_same_count_and_handle_missing_text() {
        let preset = ContentPreset::from_toml(
            r#"version = 1
name = "counts"
[[gates]]
id = "lower"
kind = "word_count"
min = 3
[[gates]]
id = "upper"
kind = "word_count"
max = 1
"#,
        )
        .unwrap();
        let input = ContentInput {
            id: "example".into(),
            text: Some("é\n deux".into()),
            ..Default::default()
        };
        let report = preset.evaluate(&input).unwrap();
        assert_eq!(report.gates[0].status, GateStatus::Failed);
        assert_eq!(report.gates[1].status, GateStatus::Failed);
        assert!(report.gates[0].findings[0].message.contains("Word count 2"));
        assert!(report.gates[1].findings[0].message.contains("Word count 2"));
        let missing = preset
            .evaluate(&ContentInput {
                text: None,
                ..input
            })
            .unwrap();
        assert!(
            missing
                .gates
                .iter()
                .all(|gate| gate.status == GateStatus::NotChecked)
        );
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
    fn external_review_is_bound_to_claim_and_source_and_preserves_uncertainty() {
        let preset = ContentPreset::from_toml("version=1\nname='review'\n[[gates]]\nid='review'\nkind='evidence_review'\nsource='study'\nreview='assessment'\nclaim='The count is 14.'\n").unwrap();
        let check = |support: &str, reliability: &str, snapshot: &str, reviewed_source: &str| {
            let review = serde_json::json!({"claim": "The count is 14.", "source_text": reviewed_source, "support": support, "reliability": reliability, "reviewer": "reviewer-1", "provenance": {"threshold":"0.9"}});
            let input = ContentInput {
                text: Some("The count is 14.".into()),
                evidence_sources: Some(BTreeMap::from([
                    ("study".into(), snapshot.into()),
                    ("assessment".into(), review.to_string()),
                ])),
                ..Default::default()
            };
            preset.evaluate(&input).unwrap().gates[0].status
        };
        assert_eq!(
            check("supported", "reliable", "source", "source"),
            GateStatus::Passed
        );
        assert_eq!(
            check("contradicted", "reliable", "source", "source"),
            GateStatus::Failed
        );
        assert_eq!(
            check("supported", "unreliable", "source", "source"),
            GateStatus::Failed
        );
        assert_eq!(
            check("uncertain", "reliable", "source", "source"),
            GateStatus::NotChecked
        );
        assert_eq!(
            check("supported", "uncertain", "source", "source"),
            GateStatus::NotChecked
        );
        assert_eq!(
            check("supported", "reliable", "updated", "source"),
            GateStatus::NotChecked
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

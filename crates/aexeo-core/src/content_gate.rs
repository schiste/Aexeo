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

/// Paragraph text and its position in the adapter's projected text.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentParagraph {
    pub text: String,
    pub line: usize,
    pub column: usize,
}

/// Optional adapter capabilities beyond the stable content input contract.
#[derive(Debug, Clone, Default)]
pub struct ContentContext {
    pub paragraphs: Option<Vec<ContentParagraph>>,
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
    /// Limits whitespace-separated words in adapter-supplied paragraphs.
    ParagraphLength {
        id: String,
        #[serde(default)]
        severity: Severity,
        #[serde(default = "default_blocking")]
        blocking: bool,
        max_words: usize,
    },
    /// Requires a minimum number of hyperlinks; does not validate destinations.
    LinkCount {
        id: String,
        #[serde(default)]
        severity: Severity,
        #[serde(default = "default_blocking")]
        blocking: bool,
        min: usize,
    },
    /// Checks destination syntax and permitted schemes without network access.
    LinkDestinations {
        id: String,
        #[serde(default)]
        severity: Severity,
        #[serde(default = "default_blocking")]
        blocking: bool,
        allowed_schemes: Vec<String>,
        #[serde(default = "default_blocking")]
        allow_relative: bool,
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
    /// Limits exact occurrences of user-configured terms.
    RepeatedTerms {
        id: String,
        #[serde(default)]
        severity: Severity,
        #[serde(default = "default_blocking")]
        blocking: bool,
        terms: Vec<String>,
        max_occurrences: usize,
        #[serde(default = "default_whole_words")]
        whole_words: bool,
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
    /// Limit heading length in Unicode scalar values after trimming whitespace.
    HeadingLength {
        id: String,
        #[serde(default)]
        severity: Severity,
        #[serde(default = "default_blocking")]
        blocking: bool,
        max_chars: usize,
    },
    /// Detect repeated non-empty headings within one content item.
    DuplicateHeadings {
        id: String,
        #[serde(default)]
        severity: Severity,
        #[serde(default = "default_blocking")]
        blocking: bool,
        #[serde(default = "default_case_sensitive")]
        case_sensitive: bool,
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
    /// Require a metadata field to be unique across a supplied collection.
    UniqueCollectionMetadata {
        id: String,
        field: String,
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

fn default_whole_words() -> bool {
    true
}

fn default_case_sensitive() -> bool {
    true
}

/// Lifecycle events for an optional, host-provided gate observer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateObservation {
    Started,
    Finished,
}

impl ContentGate {
    pub fn severity(&self) -> Severity {
        match self {
            Self::WordCount { severity, .. }
            | Self::LinkCount { severity, .. }
            | Self::LinkDestinations { severity, .. }
            | Self::SentenceLength { severity, .. }
            | Self::ParagraphLength { severity, .. }
            | Self::RepeatedTerms { severity, .. }
            | Self::ForbiddenTerms { severity, .. }
            | Self::RequiredMetadata { severity, .. }
            | Self::RequiredHeadings { severity, .. }
            | Self::DuplicateHeadings { severity, .. }
            | Self::HeadingLength { severity, .. }
            | Self::FinalNewline { severity, .. }
            | Self::UniqueCollectionMetadata { severity, .. }
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
            | Self::LinkCount { blocking, .. }
            | Self::LinkDestinations { blocking, .. }
            | Self::SentenceLength { blocking, .. }
            | Self::ParagraphLength { blocking, .. }
            | Self::RepeatedTerms { blocking, .. }
            | Self::ForbiddenTerms { blocking, .. }
            | Self::RequiredMetadata { blocking, .. }
            | Self::RequiredHeadings { blocking, .. }
            | Self::DuplicateHeadings { blocking, .. }
            | Self::HeadingLength { blocking, .. }
            | Self::FinalNewline { blocking, .. }
            | Self::UniqueCollectionMetadata { blocking, .. }
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
            Self::WordCount { .. }
            | Self::SentenceLength { .. }
            | Self::RepeatedTerms { .. }
            | Self::ForbiddenTerms { .. } => "text",
            Self::LinkCount { .. } => "links",
            Self::LinkDestinations { .. } => "link_destinations",
            Self::RequiredMetadata { .. } => "metadata",
            Self::ParagraphLength { .. } => "paragraphs",
            Self::RequiredHeadings { .. }
            | Self::DuplicateHeadings { .. }
            | Self::HeadingLength { .. } => "headings",
            Self::FinalNewline { .. } => "source_text",
            Self::UniqueCollectionMetadata { .. } => "collection_metadata",
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
            | Self::LinkCount { id, .. }
            | Self::LinkDestinations { id, .. }
            | Self::SentenceLength { id, .. }
            | Self::ParagraphLength { id, .. }
            | Self::RepeatedTerms { id, .. }
            | Self::ForbiddenTerms { id, .. }
            | Self::RequiredMetadata { id, .. }
            | Self::RequiredHeadings { id, .. }
            | Self::DuplicateHeadings { id, .. }
            | Self::HeadingLength { id, .. }
            | Self::FinalNewline { id, .. }
            | Self::UniqueCollectionMetadata { id, .. }
            | Self::Custom { id, .. }
            | Self::EvidenceAnchors { id, .. }
            | Self::StructuredFact { id, .. }
            | Self::EvidenceReview { id, .. }
            | Self::SiteAudit { id, .. }
            | Self::SiteGroup { id, .. } => id,
        }
    }
}

/// Return offending heading indices for heading gates, in finding order.
/// Other gate types return `None`.
pub fn heading_finding_indices(gate: &ContentGate, headings: &[String]) -> Option<Vec<usize>> {
    match gate {
        ContentGate::HeadingLength { max_chars, .. } => Some(
            headings
                .iter()
                .enumerate()
                .filter_map(|(index, heading)| {
                    (heading.trim().chars().count() > *max_chars).then_some(index)
                })
                .collect(),
        ),
        ContentGate::DuplicateHeadings { case_sensitive, .. } => {
            let mut seen = std::collections::HashSet::new();
            Some(
                headings
                    .iter()
                    .enumerate()
                    .filter_map(|(index, heading)| {
                        let trimmed = heading.trim();
                        if trimmed.is_empty() {
                            return None;
                        }
                        let key = if *case_sensitive {
                            trimmed.to_owned()
                        } else {
                            trimmed.to_lowercase()
                        };
                        (!seen.insert(key)).then_some(index)
                    })
                    .collect(),
            )
        }
        _ => None,
    }
}

/// Check a destination against a configured offline scheme policy.
pub fn valid_link_destination(destination: &str, schemes: &[String], allow_relative: bool) -> bool {
    if destination.is_empty()
        || destination.chars().any(char::is_whitespace)
        || destination.chars().any(char::is_control)
    {
        return false;
    }
    match url::Url::parse(destination) {
        Ok(parsed) => schemes.iter().any(|allowed| allowed == parsed.scheme()),
        Err(url::ParseError::RelativeUrlWithoutBase) if allow_relative => {
            !destination.starts_with("//")
        }
        Err(_) => false,
    }
}

// Matching is literal and case-sensitive. Word boundaries use Unicode
// alphanumeric characters and underscore; positions remain in the original text.
fn term_matches<'a>(
    text: &'a str,
    term: &'a str,
    whole_words: bool,
) -> impl Iterator<Item = (usize, &'a str)> + 'a {
    fn word(ch: char) -> bool {
        ch.is_alphanumeric() || ch == '_'
    }
    text.match_indices(term).filter(move |(offset, _)| {
        !whole_words
            || ((!term.chars().next().is_some_and(word)
                || text[..*offset]
                    .chars()
                    .next_back()
                    .is_none_or(|ch| !word(ch)))
                && (!term.chars().next_back().is_some_and(word)
                    || text[*offset + term.len()..]
                        .chars()
                        .next()
                        .is_none_or(|ch| !word(ch))))
    })
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

/// Capabilities supplied by an adapter to a content evaluation.
/// Absent capabilities produce `not_checked` for the corresponding gates.
#[derive(Default)]
pub struct ContentEvaluationContext<'a> {
    pub site: Option<(&'a crate::site::Site, &'a crate::config::Config)>,
    pub extensions: Option<&'a dyn ContentGateExtensions>,
    pub adapter: Option<&'a ContentContext>,
    pub link_count: Option<usize>,
    /// The host is responsible for supplying a complete collection.
    pub collection: Option<&'a [ContentInput]>,
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
                ContentGate::LinkDestinations {
                    allowed_schemes, ..
                } if allowed_schemes.is_empty()
                    || allowed_schemes.iter().any(|scheme| {
                        let mut bytes = scheme.bytes();
                        !bytes.next().is_some_and(|b| b.is_ascii_lowercase())
                            || !bytes.all(|b| {
                                b.is_ascii_lowercase()
                                    || b.is_ascii_digit()
                                    || matches!(b, b'+' | b'-' | b'.')
                            })
                    })
                    || allowed_schemes.iter().collect::<BTreeSet<_>>().len()
                        != allowed_schemes.len() =>
                {
                    bail!(
                        "link destination schemes must be unique lowercase URI schemes for {}",
                        gate.id()
                    )
                }
                ContentGate::HeadingLength { max_chars, .. } if *max_chars == 0 => {
                    bail!("heading length max_chars must be greater than zero");
                }
                ContentGate::ParagraphLength { max_words, .. } if *max_words == 0 => {
                    bail!("paragraph length needs a positive limit for {}", gate.id())
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
                ContentGate::RepeatedTerms { terms, .. }
                    if terms.is_empty()
                        || terms.iter().any(|s| s.trim().is_empty())
                        || terms.iter().collect::<BTreeSet<_>>().len() != terms.len() =>
                {
                    bail!(
                        "repeated terms must be nonempty and unique for {}",
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
                ContentGate::UniqueCollectionMetadata { field, .. } if field.trim().is_empty() => {
                    bail!(
                        "collection metadata field must not be empty for {}",
                        gate.id()
                    )
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
        self.evaluate_with_context(input, site, extensions, None)
    }

    /// Evaluate optional adapter capabilities while preserving the original input API.
    pub fn evaluate_with_context(
        &self,
        input: &ContentInput,
        site: Option<(&crate::site::Site, &crate::config::Config)>,
        extensions: Option<&dyn ContentGateExtensions>,
        context: Option<&ContentContext>,
    ) -> Result<ContentReport> {
        self.evaluate_with_links(input, site, extensions, context, None)
    }

    /// Evaluate with an optional adapter-supplied hyperlink count.
    pub fn evaluate_with_links(
        &self,
        input: &ContentInput,
        site: Option<(&crate::site::Site, &crate::config::Config)>,
        extensions: Option<&dyn ContentGateExtensions>,
        context: Option<&ContentContext>,
        link_count: Option<usize>,
    ) -> Result<ContentReport> {
        self.evaluate_with_capabilities(
            input,
            &ContentEvaluationContext {
                site,
                extensions,
                adapter: context,
                link_count,
                collection: None,
            },
        )
    }

    /// Evaluate using caller-supplied, optional adapter capabilities.
    pub fn evaluate_with_capabilities(
        &self,
        input: &ContentInput,
        capabilities: &ContentEvaluationContext<'_>,
    ) -> Result<ContentReport> {
        self.evaluate_with_destinations(input, capabilities, None)
    }

    /// Evaluate with an optional complete list of adapter-supplied link destinations.
    pub fn evaluate_with_destinations(
        &self,
        input: &ContentInput,
        capabilities: &ContentEvaluationContext<'_>,
        destinations: Option<&[String]>,
    ) -> Result<ContentReport> {
        self.evaluate_with_observer_inner(input, capabilities, destinations, None)
    }

    /// Evaluate while notifying the host before and after each gate, in preset order.
    /// An evaluation error can terminate before a matching `Finished` event.
    pub fn evaluate_with_gate_observer(
        &self,
        input: &ContentInput,
        capabilities: &ContentEvaluationContext<'_>,
        destinations: Option<&[String]>,
        observer: &mut dyn FnMut(GateObservation),
    ) -> Result<ContentReport> {
        self.evaluate_with_observer_inner(input, capabilities, destinations, Some(observer))
    }

    fn evaluate_with_observer_inner(
        &self,
        input: &ContentInput,
        capabilities: &ContentEvaluationContext<'_>,
        destinations: Option<&[String]>,
        mut observer: Option<&mut dyn FnMut(GateObservation)>,
    ) -> Result<ContentReport> {
        let site = capabilities.site;
        let extensions = capabilities.extensions;
        let context = capabilities.adapter;
        let link_count = capabilities.link_count;
        let collection = capabilities.collection;
        self.validate()?;
        let mut results = Vec::with_capacity(self.gates.len());
        let mut word_count = None;
        for gate in &self.gates {
            if let Some(observe) = observer.as_deref_mut() {
                observe(GateObservation::Started);
            }
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
                    subject: None,
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
                ContentGate::UniqueCollectionMetadata { field, .. } => match collection {
                    Some(items) => {
                        measurements.insert("collection_items".into(), items.len());
                        let mut first_by_value = BTreeMap::new();
                        let mut missing = 0;
                        let mut duplicates = 0;
                        for item in items {
                            let value = item
                                .metadata
                                .as_ref()
                                .and_then(|m| m.get(field))
                                .filter(|v| !v.trim().is_empty());
                            match value {
                                Some(value) => {
                                    if let Some(first_id) = first_by_value.get(value) {
                                        duplicates += 1;
                                        findings.push(aexeo_contracts::Finding {
                                                rule_id: gate.id().to_owned(),
                                                message: format!("Metadata field {field:?} duplicates the value in {first_id:?}"),
                                                path: item.id.clone(),
                                                line: 1,
                                                column: 1,
                                                severity: match gate.severity() { Severity::Error => "error", Severity::Warning => "warning" }.into(),
                                                suggestion: None,
                                                scope: aexeo_contracts::FindingScope::Page,
                                                subject: None,
                                            });
                                    } else {
                                        first_by_value.insert(value, &item.id);
                                    }
                                }
                                None => missing += 1,
                            }
                        }
                        measurements.insert("items_missing_field".into(), missing);
                        measurements.insert("duplicate_items".into(), duplicates);
                        if missing > 0 {
                            reason = Some(format!(
                                "metadata field {field:?} is unavailable for {missing} collection item(s)"
                            ));
                        }
                    }
                    None => reason = Some("collection is unavailable from this adapter".into()),
                },
                ContentGate::LinkCount { min, .. } => {
                    measurements.insert("min_links".into(), *min);
                    match link_count {
                        Some(count) => {
                            measurements.insert("link_count".into(), count);
                            if count < *min {
                                emit(
                                    format!("Link count {count} is below the minimum of {min}"),
                                    1,
                                    1,
                                );
                            }
                        }
                        None => reason = Some("links are unavailable from this adapter".into()),
                    }
                }
                ContentGate::LinkDestinations {
                    allowed_schemes,
                    allow_relative,
                    ..
                } => match destinations {
                    Some(values) => {
                        let mut invalid = 0;
                        for (index, destination) in values.iter().enumerate() {
                            if !valid_link_destination(
                                destination,
                                allowed_schemes,
                                *allow_relative,
                            ) {
                                invalid += 1;
                                emit(
                                    format!(
                                        "Link destination #{} is invalid or has a disallowed scheme",
                                        index + 1
                                    ),
                                    1,
                                    1,
                                );
                            }
                        }
                        measurements.insert("checked_destinations".into(), values.len());
                        measurements.insert("invalid_destinations".into(), invalid);
                    }
                    None => {
                        reason = Some("link destinations are unavailable from this adapter".into())
                    }
                },
                ContentGate::ParagraphLength { max_words, .. } => {
                    measurements.insert("max_words".into(), *max_words);
                    match context.and_then(|c| c.paragraphs.as_ref()) {
                        Some(paragraphs) => {
                            let mut longest = 0;
                            for paragraph in paragraphs {
                                let count = paragraph.text.split_whitespace().count();
                                longest = longest.max(count);
                                if count > *max_words {
                                    emit(
                                        format!(
                                            "Paragraph has {count} words; maximum is {max_words}"
                                        ),
                                        paragraph.line,
                                        paragraph.column,
                                    );
                                }
                            }
                            measurements.insert("paragraph_count".into(), paragraphs.len());
                            measurements.insert("longest_paragraph_words".into(), longest);
                        }
                        None => {
                            reason = Some("paragraphs are unavailable from this adapter".into())
                        }
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
                                let decimal_point = ch == Some('.')
                                    && text[..offset]
                                        .chars()
                                        .next_back()
                                        .is_some_and(|c| c.is_ascii_digit())
                                    && text[offset + 1..]
                                        .chars()
                                        .next()
                                        .is_some_and(|c| c.is_ascii_digit());
                                if !decimal_point && ch.is_none_or(|ch| delimiters.contains(ch)) {
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
                ContentGate::RepeatedTerms {
                    terms,
                    max_occurrences,
                    whole_words,
                    ..
                } => {
                    measurements.insert("configured_terms".into(), terms.len());
                    measurements.insert("max_occurrences".into(), *max_occurrences);
                    match &input.text {
                        Some(text) => {
                            let mut total = 0;
                            let mut exceeded = 0;
                            for term in terms {
                                let mut first_excess = None;
                                let mut count = 0;
                                for (offset, _) in term_matches(text, term, *whole_words) {
                                    count += 1;
                                    if count > *max_occurrences && first_excess.is_none() {
                                        first_excess = Some(offset);
                                    }
                                }
                                total += count;
                                if let Some(offset) = first_excess {
                                    exceeded += 1;
                                    let before = &text[..offset];
                                    let line = before.chars().filter(|ch| *ch == '\n').count() + 1;
                                    let column =
                                        before.rsplit('\n').next().unwrap_or("").chars().count()
                                            + 1;
                                    emit(
                                        format!(
                                            "Term {term:?} occurs {count} times; maximum is {max_occurrences}"
                                        ),
                                        line,
                                        column,
                                    );
                                }
                            }
                            measurements.insert("matching_occurrences".into(), total);
                            measurements.insert("exceeded_terms".into(), exceeded);
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
                                for (offset, _) in term_matches(text, term, false) {
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
                ContentGate::HeadingLength { max_chars, .. } => {
                    measurements.insert("max_chars".into(), *max_chars);
                    match &input.headings {
                        Some(headings) => {
                            let indices =
                                heading_finding_indices(gate, headings).unwrap_or_default();
                            measurements.insert("heading_count".into(), headings.len());
                            measurements.insert(
                                "longest_heading_chars".into(),
                                headings
                                    .iter()
                                    .map(|h| h.trim().chars().count())
                                    .max()
                                    .unwrap_or(0),
                            );
                            measurements.insert("headings_exceeding_limit".into(), indices.len());
                            for index in indices {
                                let length = headings[index].trim().chars().count();
                                emit(
                                    format!(
                                        "Heading #{} has {length} characters; maximum is {max_chars}",
                                        index + 1
                                    ),
                                    1,
                                    1,
                                );
                            }
                        }
                        None => reason = Some("headings are unavailable from this adapter".into()),
                    }
                }
                ContentGate::DuplicateHeadings { .. } => match &input.headings {
                    Some(headings) => {
                        let indices = heading_finding_indices(gate, headings).unwrap_or_default();
                        measurements.insert("heading_count".into(), headings.len());
                        measurements.insert("duplicate_headings".into(), indices.len());
                        for index in indices {
                            emit(
                                format!("Heading #{} repeats an earlier heading", index + 1),
                                1,
                                1,
                            );
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
                            Some(results) => {
                                findings.extend(results);
                                if site.crawl_meta.as_ref().is_some_and(|meta| meta.truncated) {
                                    reason = Some("site crawl is incomplete; findings cover only visited pages".into());
                                }
                            }
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
                        if site.crawl_meta.as_ref().is_some_and(|meta| meta.truncated) {
                            reason = Some(
                                "site crawl is incomplete; findings cover only visited pages"
                                    .into(),
                            );
                        }
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
                scope: match gate {
                    ContentGate::SiteAudit { .. } | ContentGate::SiteGroup { .. } => "site",
                    ContentGate::UniqueCollectionMetadata { .. } => "collection",
                    _ => "document",
                }
                .into(),
                required_capability: gate.required_capability().into(),
                findings,
                reason,
                measurements,
            });
            if let Some(observe) = observer.as_deref_mut() {
                observe(GateObservation::Finished);
            }
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
    fn repeated_terms_use_unicode_boundaries_and_report_extra_occurrence() {
        let preset = ContentPreset::from_toml(
            r#"
version = 1
name = "repetition"
[[gates]]
id = "repeat"
kind = "repeated_terms"
terms = ["café", "cat"]
max_occurrences = 1
blocking = false
"#,
        )
        .unwrap();
        let input = ContentInput {
            id: "draft".into(),
            text: Some("café cats caféine\ncat café".into()),
            ..Default::default()
        };
        let gate = &preset.evaluate(&input).unwrap().gates[0];
        assert_eq!(gate.status, GateStatus::Failed);
        assert_eq!(gate.measurements["matching_occurrences"], 3);
        assert_eq!(gate.measurements["exceeded_terms"], 1);
        assert_eq!((gate.findings[0].line, gate.findings[0].column), (2, 5));
        assert_eq!(
            preset.evaluate(&ContentInput::default()).unwrap().gates[0].status,
            GateStatus::NotChecked
        );
        let substrings = ContentPreset::from_toml(
            r#"
version = 1
name = "substrings"
[[gates]]
id = "repeat"
kind = "repeated_terms"
terms = ["cat"]
max_occurrences = 1
whole_words = false
"#,
        )
        .unwrap();
        assert_eq!(
            substrings.evaluate(&input).unwrap().gates[0].status,
            GateStatus::Failed
        );
        assert!(
            ContentPreset::from_toml(
                r#"
version = 1
name = "duplicates"
[[gates]]
id = "repeat"
kind = "repeated_terms"
terms = ["cat", "cat"]
max_occurrences = 1
"#
            )
            .is_err()
        );
    }

    #[test]
    fn collection_metadata_gate_exposes_scope_duplicates_and_missing_data() {
        let preset = ContentPreset::from_toml(
            r#"
version = 1
name = "collection"
[[gates]]
id = "unique"
kind = "unique_collection_metadata"
field = "slug"
"#,
        )
        .unwrap();
        let input = ContentInput {
            id: "collection".into(),
            ..Default::default()
        };
        let single = preset.evaluate(&input).unwrap();
        assert_eq!(single.gates[0].status, GateStatus::NotChecked);
        assert_eq!(single.gates[0].scope, "collection");
        assert_eq!(single.gates[0].required_capability, "collection_metadata");
        let item = |id: &str, value: Option<&str>| ContentInput {
            id: id.into(),
            metadata: value.map(|v| BTreeMap::from([("slug".into(), v.into())])),
            ..Default::default()
        };
        let items = vec![
            item("one", Some("same")),
            item("two", Some("same")),
            item("three", Some("other")),
        ];
        let report = preset
            .evaluate_with_capabilities(
                &input,
                &ContentEvaluationContext {
                    collection: Some(&items),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(report.gates[0].status, GateStatus::Failed);
        assert_eq!(report.gates[0].measurements["duplicate_items"], 1);
        assert_eq!(report.gates[0].findings[0].path, "two");
        let partial = vec![items[0].clone(), items[1].clone(), item("three", None)];
        let report = preset
            .evaluate_with_capabilities(
                &input,
                &ContentEvaluationContext {
                    collection: Some(&partial),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(report.gates[0].status, GateStatus::NotChecked);
        assert_eq!(report.gates[0].measurements["items_missing_field"], 1);
        assert_eq!(report.gates[0].findings.len(), 1);
        let unique = vec![items[0].clone(), items[2].clone()];
        assert_eq!(
            preset
                .evaluate_with_capabilities(
                    &input,
                    &ContentEvaluationContext {
                        collection: Some(&unique),
                        ..Default::default()
                    }
                )
                .unwrap()
                .gates[0]
                .status,
            GateStatus::Passed
        );
    }

    #[test]
    fn gate_observer_preserves_report_and_reports_each_gate_in_order() {
        let preset = ContentPreset::from_toml("version=1\nname='observer'\n[[gates]]\nid='one'\nkind='word_count'\nmin=1\n[[gates]]\nid='two'\nkind='required_headings'\nheadings=['Intro']\n").unwrap();
        let input = ContentInput {
            text: Some("content".into()),
            ..Default::default()
        };
        let capabilities = ContentEvaluationContext::default();
        let baseline = preset
            .evaluate_with_destinations(&input, &capabilities, None)
            .unwrap();
        let mut events = Vec::new();
        let measured = preset
            .evaluate_with_gate_observer(&input, &capabilities, None, &mut |event| {
                events.push(event)
            })
            .unwrap();
        assert_eq!(
            serde_json::to_value(baseline).unwrap(),
            serde_json::to_value(&measured).unwrap()
        );
        assert_eq!(
            events,
            [
                GateObservation::Started,
                GateObservation::Finished,
                GateObservation::Started,
                GateObservation::Finished
            ]
        );
        assert_eq!(
            measured
                .gates
                .iter()
                .map(|gate| gate.status)
                .collect::<Vec<_>>(),
            [GateStatus::Passed, GateStatus::NotChecked]
        );
    }

    #[test]
    fn heading_length_counts_unicode_scalars_and_requires_headings() {
        let preset = ContentPreset::from_toml("version=1\nname='headings'\n[[gates]]\nid='length'\nkind='heading_length'\nmax_chars=3\nblocking=false\n").unwrap();
        let input = ContentInput {
            headings: Some(vec![" é😀a ".into(), "abcd".into(), "".into()]),
            ..Default::default()
        };
        let result = preset.evaluate(&input).unwrap();
        let gate = &result.gates[0];
        assert_eq!(gate.status, GateStatus::Failed);
        assert!(!gate.blocking);
        assert_eq!(gate.measurements["heading_count"], 3);
        assert_eq!(gate.measurements["longest_heading_chars"], 4);
        assert_eq!(gate.measurements["headings_exceeding_limit"], 1);
        assert!(gate.findings[0].message.contains("Heading #2"));
        let absent = preset.evaluate(&ContentInput::default()).unwrap();
        assert_eq!(absent.gates[0].status, GateStatus::NotChecked);
        assert!(!absent.gates[0].measurements.contains_key("heading_count"));
        let empty = preset
            .evaluate(&ContentInput {
                headings: Some(vec![]),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(empty.gates[0].status, GateStatus::Passed);
        assert_eq!(empty.gates[0].measurements["longest_heading_chars"], 0);
        assert!(ContentPreset::from_toml("version=1\nname='headings'\n[[gates]]\nid='length'\nkind='heading_length'\nmax_chars=0\n").is_err());
    }

    #[test]
    fn duplicate_headings_respect_capability_and_case_policy() {
        let preset = ContentPreset::from_toml(
            "version=1\nname='headings'\n[[gates]]\nid='unique'\nkind='duplicate_headings'\n",
        )
        .unwrap();
        let mut input = ContentInput::default();
        assert_eq!(
            preset.evaluate(&input).unwrap().gates[0].status,
            GateStatus::NotChecked
        );
        input.headings = Some(vec![
            " Heading ".into(),
            "heading".into(),
            "Heading".into(),
            " ".into(),
        ]);
        let exact = preset.evaluate(&input).unwrap();
        assert_eq!(exact.gates[0].status, GateStatus::Failed);
        assert_eq!(exact.gates[0].measurements["heading_count"], 4);
        assert_eq!(exact.gates[0].measurements["duplicate_headings"], 1);
        assert!(exact.gates[0].findings[0].message.contains("#3"));
        let folded = ContentPreset::from_toml("version=1\nname='headings'\n[[gates]]\nid='unique'\nkind='duplicate_headings'\ncase_sensitive=false\nblocking=false\n").unwrap();
        let result = folded.evaluate(&input).unwrap();
        assert_eq!(result.gates[0].measurements["duplicate_headings"], 2);
        assert!(!result.gates[0].blocking);
        input.headings = Some(vec![]);
        assert_eq!(
            folded.evaluate(&input).unwrap().gates[0].status,
            GateStatus::Passed
        );
    }

    #[test]
    fn destination_gate_checks_schemes_and_requires_capability() {
        let preset = ContentPreset::from_toml("version=1\nname='links'\n[[gates]]\nid='links'\nkind='link_destinations'\nallowed_schemes=['https','mailto']\n").unwrap();
        let input = ContentInput::default();
        assert_eq!(
            preset.evaluate(&input).unwrap().gates[0].status,
            GateStatus::NotChecked
        );
        let valid = vec![
            "https://example.org/a".into(),
            "/local".into(),
            "#fragment".into(),
            "mailto:a@example.org".into(),
        ];
        let passed = preset
            .evaluate_with_destinations(&input, &ContentEvaluationContext::default(), Some(&valid))
            .unwrap();
        assert_eq!(passed.gates[0].status, GateStatus::Passed);
        assert_eq!(passed.gates[0].measurements["checked_destinations"], 4);
        let invalid = vec![
            "javascript:alert(1)".into(),
            "https://".into(),
            "".into(),
            "https://example.org/has space".into(),
            "//example.org/path".into(),
        ];
        let failed = preset
            .evaluate_with_destinations(
                &input,
                &ContentEvaluationContext::default(),
                Some(&invalid),
            )
            .unwrap();
        assert_eq!(failed.gates[0].status, GateStatus::Failed);
        assert_eq!(failed.gates[0].measurements["invalid_destinations"], 5);
        assert!(failed.gates[0].findings[0].message.contains("#1"));
        assert!(failed.gates[0].findings[4].message.contains("#5"));
        let absolute = ContentPreset::from_toml("version=1\nname='links'\n[[gates]]\nid='links'\nkind='link_destinations'\nallowed_schemes=['https']\nallow_relative=false\n").unwrap();
        assert_eq!(
            absolute
                .evaluate_with_destinations(
                    &input,
                    &ContentEvaluationContext::default(),
                    Some(&valid)
                )
                .unwrap()
                .gates[0]
                .status,
            GateStatus::Failed
        );
        assert!(ContentPreset::from_toml("version=1\nname='links'\n[[gates]]\nid='links'\nkind='link_destinations'\nallowed_schemes=['HTTPS']\n").is_err());
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
    fn partial_site_gates_report_incomplete_coverage_and_keep_findings() {
        let preset = ContentPreset::from_toml("version=1\nname='site'\n[[gates]]\nid='links'\nkind='site_group'\ngroup='links'\n[[gates]]\nid='audit'\nkind='site_audit'\n").unwrap();
        let root = tempfile::tempdir().unwrap();
        std::fs::write(
            root.path().join("index.html"),
            "<html><body><a href='/missing'>missing</a></body></html>",
        )
        .unwrap();
        let mut site = crate::load_site(root.path()).unwrap();
        let config = crate::Config::default();
        let input = ContentInput::default();
        let complete = preset
            .evaluate_with_site(&input, Some((&site, &config)))
            .unwrap();
        assert!(
            complete.gates[0]
                .findings
                .iter()
                .any(|f| f.rule_id == "LNK001")
        );
        site.crawl_meta = Some(crate::site::CrawlMeta {
            truncated: true,
            ..Default::default()
        });
        let report = preset
            .evaluate_with_site(&input, Some((&site, &config)))
            .unwrap();
        for gate in &report.gates {
            assert_eq!(gate.status, GateStatus::NotChecked);
            assert!(gate.reason.as_deref().unwrap().contains("incomplete"));
        }
        assert!(
            !report.gates[0]
                .findings
                .iter()
                .any(|f| f.rule_id == "LNK001")
        );
        assert!(!report.gates[1].findings.is_empty());
        assert_eq!(report.summary.blocking_not_checked, 2);
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

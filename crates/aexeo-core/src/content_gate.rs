//! Portable, offline checks for content supplied by an adapter.
//!
//! Missing input is reported separately from a passing gate. Adapters must not
//! represent an unavailable field as an empty string.
use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

/// The smallest common input shared by content adapters. The identifier is
/// opaque: it may be a file name, a URL, or a CMS document identifier.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
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
    /// Sorted byte offsets in the projected text where a block ends: a heading,
    /// paragraph, list item or table cell. A sentence ends there and a literal
    /// term does not match across it. Word counts treat it as a separator.
    /// Raw text has no known block structure.
    pub block_ends: Option<Vec<usize>>,
    /// Why the adapter withheld the input. It explains the `not_checked` status
    /// of each gate that needed that input.
    pub input_limit: Option<String>,
    /// Metadata keys whose values or absence were not observed by the adapter.
    pub unavailable_metadata_fields: Vec<String>,
}

/// Largest input the CLI adapters read. Larger input is reported as `not_checked`.
pub const MAX_CONTENT_INPUT_BYTES: usize = 16 * 1024 * 1024;

/// Findings kept for one document gate. The `findings_omitted` measurement
/// counts any others, so a truncated gate still reports its full total.
pub const MAX_FINDINGS_PER_GATE: usize = 1000;

// Keep the advisory openings gate bounded for adapters other than the CLI.
const MAX_DISTINCT_PARAGRAPH_OPENINGS: usize = 10_000;
const MAX_PARAGRAPH_OPENING_BYTES: usize = 4_096;

/// The measurement that records findings dropped by `MAX_FINDINGS_PER_GATE`.
pub const FINDINGS_OMITTED: &str = "findings_omitted";

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
    /// Bounds the number of adapter-supplied paragraphs.
    ParagraphCount {
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
    /// Flags repeated literal paragraph openings; this is an editorial prompt.
    RepeatedParagraphOpenings {
        id: String,
        #[serde(default)]
        severity: Severity,
        #[serde(default = "default_blocking")]
        blocking: bool,
        opening_words: usize,
        max_occurrences: usize,
    },
    /// Limits Unicode scalar values in adapter-supplied paragraphs.
    ParagraphCharacterLength {
        id: String,
        #[serde(default)]
        severity: Severity,
        #[serde(default = "default_blocking")]
        blocking: bool,
        max_chars: usize,
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
    /// Reports observed HTTP not-found responses from an explicitly supplied check.
    LinkReachability {
        id: String,
        #[serde(default)]
        severity: Severity,
        #[serde(default)]
        blocking: bool,
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
        /// Exact whitespace-delimited abbreviations whose final period is not a boundary.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        abbreviations: Vec<String>,
        /// Restrict sentence checks to adapter-provided paragraphs.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        paragraphs_only: bool,
    },
    /// Limits Unicode scalar values per heuristic sentence.
    SentenceCharacterLength {
        id: String,
        #[serde(default)]
        severity: Severity,
        #[serde(default = "default_blocking")]
        blocking: bool,
        max_chars: usize,
        #[serde(default = "default_sentence_delimiters")]
        delimiters: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        abbreviations: Vec<String>,
        /// Restrict sentence checks to adapter-provided paragraphs.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        paragraphs_only: bool,
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
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        case_insensitive: bool,
    },
    ForbiddenTerms {
        id: String,
        #[serde(default)]
        severity: Severity,
        #[serde(default = "default_blocking")]
        blocking: bool,
        terms: Vec<String>,
        #[serde(default = "default_whole_words")]
        whole_words: bool,
        #[serde(default)]
        case_insensitive: bool,
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
        #[serde(default, skip_serializing_if = "Option::is_none")]
        group_by: Option<String>,
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
            | Self::LinkReachability { severity, .. }
            | Self::SentenceLength { severity, .. }
            | Self::SentenceCharacterLength { severity, .. }
            | Self::ParagraphCount { severity, .. }
            | Self::ParagraphLength { severity, .. }
            | Self::RepeatedParagraphOpenings { severity, .. }
            | Self::ParagraphCharacterLength { severity, .. }
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
            | Self::LinkReachability { blocking, .. }
            | Self::SentenceLength { blocking, .. }
            | Self::SentenceCharacterLength { blocking, .. }
            | Self::ParagraphCount { blocking, .. }
            | Self::ParagraphLength { blocking, .. }
            | Self::RepeatedParagraphOpenings { blocking, .. }
            | Self::ParagraphCharacterLength { blocking, .. }
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
            Self::SentenceLength {
                paragraphs_only: true,
                ..
            }
            | Self::SentenceCharacterLength {
                paragraphs_only: true,
                ..
            } => "paragraphs",
            Self::WordCount { .. }
            | Self::SentenceLength { .. }
            | Self::SentenceCharacterLength { .. }
            | Self::RepeatedTerms { .. }
            | Self::ForbiddenTerms { .. } => "text",
            Self::LinkCount { .. } => "links",
            Self::LinkDestinations { .. } => "link_destinations",
            Self::LinkReachability { .. } => "link_reachability",
            Self::RequiredMetadata { .. } => "metadata",
            Self::ParagraphCount { .. }
            | Self::ParagraphLength { .. }
            | Self::ParagraphCharacterLength { .. }
            | Self::RepeatedParagraphOpenings { .. } => "paragraphs",
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
            | Self::LinkReachability { id, .. }
            | Self::SentenceLength { id, .. }
            | Self::SentenceCharacterLength { id, .. }
            | Self::ParagraphCount { id, .. }
            | Self::ParagraphLength { id, .. }
            | Self::RepeatedParagraphOpenings { id, .. }
            | Self::ParagraphCharacterLength { id, .. }
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

/// Explain why `valid_link_destination` rejected a destination.
fn link_destination_problem(destination: &str, schemes: &[String], allow_relative: bool) -> String {
    const MAX_PREVIEW_CHARS: usize = 200;
    let mut preview: String = destination.chars().take(MAX_PREVIEW_CHARS).collect();
    if preview.len() < destination.len() {
        preview.push('…');
    }
    if destination.is_empty()
        || destination.chars().any(char::is_whitespace)
        || destination.chars().any(char::is_control)
    {
        return format!("{preview:?} is empty or contains whitespace or control characters");
    }
    match url::Url::parse(destination) {
        Ok(parsed) => format!(
            "{preview:?} has disallowed scheme {:?}; allowed: {}",
            parsed.scheme(),
            schemes.join(", ")
        ),
        Err(url::ParseError::RelativeUrlWithoutBase) if !allow_relative => {
            format!("{preview:?} is relative, and relative links are not allowed")
        }
        Err(url::ParseError::RelativeUrlWithoutBase) => {
            format!("{preview:?} is scheme-relative; its scheme depends on the page")
        }
        Err(_) => format!("{preview:?} is malformed"),
    }
}

/// Collapse whitespace so a value compares equal however its source wrapped it.
pub fn collapse_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Scripts written without spaces between words, where the characters adjacent
/// to a term say nothing about whether it is a whole word.
fn unsegmented_script(ch: char) -> bool {
    matches!(ch,
        '\u{0E00}'..='\u{0EFF}' // Thai, Lao
        | '\u{1000}'..='\u{109F}' // Myanmar
        | '\u{1780}'..='\u{17FF}' // Khmer
        | '\u{3040}'..='\u{30FF}' // Hiragana, Katakana
        | '\u{31F0}'..='\u{31FF}' // Katakana extensions
        | '\u{3400}'..='\u{4DBF}' // CJK extension A
        | '\u{4E00}'..='\u{9FFF}' // CJK unified ideographs
        | '\u{AC00}'..='\u{D7A3}' // Hangul, whose particles attach to nouns
        | '\u{F900}'..='\u{FAFF}' // CJK compatibility ideographs
        | '\u{FF66}'..='\u{FF9D}' // Halfwidth Katakana
        | '\u{20000}'..='\u{3FFFF}' // CJK supplementary planes
    )
}

/// Collapse whitespace runs and optionally lowercase. A run containing a blank
/// line becomes a newline, so a single space in a term never spans paragraphs.
/// `segments` holds a `(folded, original)` offset pair wherever their difference
/// changes.
fn fold_literal(text: &str, case_insensitive: bool) -> (String, Vec<(usize, usize)>) {
    let mut folded = String::with_capacity(text.len());
    let mut segments = vec![(0, 0)];
    let mut chars = text.char_indices().peekable();
    while let Some((offset, ch)) = chars.next() {
        let mut end = offset + ch.len_utf8();
        if ch.is_whitespace() {
            let mut newlines = usize::from(ch == '\n');
            while let Some((next, following)) = chars.next_if(|(_, c)| c.is_whitespace()) {
                newlines += usize::from(following == '\n');
                end = next + following.len_utf8();
            }
            folded.push(if newlines > 1 { '\n' } else { ' ' });
            if end - offset != 1 {
                segments.push((folded.len(), end));
            }
        } else {
            let start = folded.len();
            if case_insensitive {
                folded.extend(ch.to_lowercase());
            } else {
                folded.push(ch);
            }
            if folded.len() - start != end - offset {
                segments.push((folded.len(), end));
            }
        }
    }
    (folded, segments)
}

/// A text prepared for literal matching. Whitespace differences never prevent a
/// match, and reported offsets remain in the original text.
struct LiteralText<'a> {
    original: &'a str,
    folded: String,
    segments: Vec<(usize, usize)>,
    block_ends: &'a [usize],
    case_insensitive: bool,
}

impl<'a> LiteralText<'a> {
    fn new(original: &'a str, block_ends: &'a [usize], case_insensitive: bool) -> Self {
        let (folded, segments) = fold_literal(original, case_insensitive);
        Self {
            original,
            folded,
            segments,
            block_ends,
            case_insensitive,
        }
    }

    fn original_offset(&self, folded: usize) -> usize {
        let index = self.segments.partition_point(|&(start, _)| start <= folded) - 1;
        let (start, original) = self.segments[index];
        let mut offset = (original + folded - start).min(self.original.len());
        // Lowercasing can lengthen a character; report its start from inside it.
        while !self.original.is_char_boundary(offset) {
            offset -= 1;
        }
        offset
    }

    fn crosses_block_end(&self, start: usize, end: usize) -> bool {
        let next = self.block_ends.partition_point(|&block| block <= start);
        self.block_ends.get(next).is_some_and(|&block| block < end)
    }

    /// Visit non-overlapping occurrences as original byte offsets, stopping when
    /// the visitor returns true. Word boundaries use Unicode alphanumeric
    /// characters and underscore, outside scripts written without spaces.
    fn visit_matches(
        &self,
        term: &str,
        whole_words: bool,
        mut visit: impl FnMut(usize) -> bool,
    ) -> bool {
        fn word(ch: char) -> bool {
            (ch.is_alphanumeric() || ch == '_') && !unsegmented_script(ch)
        }
        let (needle, _) = fold_literal(term, self.case_insensitive);
        let text = self.folded.as_str();
        if needle.is_empty() {
            return false;
        }
        let mut cursor = 0;
        while let Some(relative) = text[cursor..].find(&needle) {
            let offset = cursor + relative;
            let end = offset + needle.len();
            let original_start = self.original_offset(offset);
            let original_end = self.original_offset(end);
            let boundaries_match = !whole_words
                || ((!needle.chars().next().is_some_and(word)
                    || text[..offset]
                        .chars()
                        .next_back()
                        .is_none_or(|ch| !word(ch))
                    || self.block_ends.binary_search(&original_start).is_ok())
                    && (!needle.chars().next_back().is_some_and(word)
                        || text[end..].chars().next().is_none_or(|ch| !word(ch))
                        || self.block_ends.binary_search(&original_end).is_ok()));
            if boundaries_match && !self.crosses_block_end(original_start, original_end) {
                if visit(original_start) {
                    return true;
                }
                cursor = end;
            } else {
                // A rejected match must not hide an overlapping valid one.
                let Some(ch) = text[offset..].chars().next() else {
                    break;
                };
                cursor = offset + ch.len_utf8();
            }
        }
        false
    }
}

// Whitespace remains the word heuristic; known block ends add boundaries
// only when they split a run of non-whitespace characters.
fn count_words(text: &str, block_ends: &[usize]) -> usize {
    let mut count = text.split_whitespace().count();
    let mut previous = None;
    for &end in block_ends {
        if previous == Some(end) {
            continue;
        }
        previous = Some(end);
        if let (Some(before), Some(after)) = (text.get(..end), text.get(end..))
            && before
                .chars()
                .next_back()
                .is_some_and(|ch| !ch.is_whitespace())
            && after.chars().next().is_some_and(|ch| !ch.is_whitespace())
        {
            count += 1;
        }
    }
    count
}

/// Whether a claim anchor occurs, allowing for how the content happens to wrap.
fn claim_is_present(text: &str, claim: &str, block_ends: &[usize]) -> bool {
    (block_ends.is_empty() && text.contains(claim))
        || LiteralText::new(text, block_ends, false).visit_matches(claim, false, |_| true)
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
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

/// A host observation for one destination, in adapter occurrence order.
/// The host must bind observations to the same unchanged input and destinations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkReachabilityObservation {
    Reachable,
    NotFound,
    NotChecked,
    NotApplicable,
}

/// Capabilities supplied by an adapter to a content evaluation.
/// Absent capabilities produce `not_checked` for the corresponding gates.
#[derive(Default)]
pub struct ContentEvaluationContext<'a> {
    pub site: Option<(&'a crate::site::Site, &'a crate::config::Config)>,
    pub extensions: Option<&'a dyn ContentGateExtensions>,
    pub adapter: Option<&'a ContentContext>,
    pub link_count: Option<usize>,
    /// One observation per destination; absent or mismatched data is not checked.
    pub link_reachability: Option<&'a [LinkReachabilityObservation]>,
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

impl ContentGateResult {
    /// All findings of this gate, including any omitted from `findings`.
    pub fn total_findings(&self) -> usize {
        self.findings.len()
            + self
                .measurements
                .get(FINDINGS_OMITTED)
                .copied()
                .unwrap_or(0)
    }
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
        if self.gates.is_empty() {
            bail!(
                "preset {:?} has no gates, so it would check nothing",
                self.name
            );
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
                ContentGate::ParagraphCount { min, max, .. }
                    if min.is_none() && max.is_none()
                        || matches!((min, max), (Some(a), Some(b)) if a > b) =>
                {
                    bail!("invalid paragraph count bounds for {}", gate.id())
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
                ContentGate::RepeatedParagraphOpenings {
                    opening_words,
                    max_occurrences,
                    ..
                } if *opening_words == 0 || *opening_words > 16 || *max_occurrences == 0 => {
                    bail!(
                        "paragraph openings need 1 to 16 words and a positive occurrence limit for {}",
                        gate.id()
                    )
                }
                ContentGate::ParagraphLength { max_words, .. } if *max_words == 0 => {
                    bail!("paragraph length needs a positive limit for {}", gate.id())
                }
                ContentGate::ParagraphCharacterLength { max_chars, .. } if *max_chars == 0 => {
                    bail!(
                        "paragraph character length needs a positive limit for {}",
                        gate.id()
                    )
                }
                ContentGate::SentenceLength {
                    max_words,
                    delimiters,
                    ..
                }
                | ContentGate::SentenceCharacterLength {
                    max_chars: max_words,
                    delimiters,
                    ..
                } if *max_words == 0 || delimiters.is_empty() => {
                    bail!(
                        "sentence length needs a positive limit and delimiters for {}",
                        gate.id()
                    )
                }
                ContentGate::SentenceLength { abbreviations, .. }
                | ContentGate::SentenceCharacterLength { abbreviations, .. }
                    if abbreviations.iter().any(|abbr| {
                        abbr.strip_suffix('.').is_none_or(|word| {
                            word.is_empty()
                                || word.contains('.')
                                || word.chars().any(char::is_whitespace)
                        })
                    }) || abbreviations.iter().collect::<BTreeSet<_>>().len()
                        != abbreviations.len() =>
                {
                    bail!(
                        "sentence abbreviations must be unique single tokens ending in a period for {}",
                        gate.id()
                    )
                }
                ContentGate::RepeatedTerms {
                    terms,
                    case_insensitive,
                    ..
                } if terms.is_empty()
                    || terms.iter().any(|s| s.trim().is_empty())
                    || terms.iter().collect::<BTreeSet<_>>().len() != terms.len()
                    || (*case_insensitive
                        && terms
                            .iter()
                            .map(|term| fold_literal(term, true).0)
                            .collect::<BTreeSet<_>>()
                            .len()
                            != terms.len()) =>
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
                ContentGate::UniqueCollectionMetadata {
                    group_by: Some(group),
                    ..
                } if group.trim().is_empty() => {
                    bail!(
                        "collection metadata group field must not be empty for {}",
                        gate.id()
                    );
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
                link_reachability: None,
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
        self.evaluate_with_observer_inner(input, capabilities, destinations, true, None)
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
        self.evaluate_with_observer_inner(input, capabilities, destinations, true, Some(observer))
    }

    /// Evaluate with an explicit completeness declaration for collection gates.
    /// An incomplete collection retains observed findings but cannot pass.
    pub fn evaluate_with_collection_completeness(
        &self,
        input: &ContentInput,
        capabilities: &ContentEvaluationContext<'_>,
        destinations: Option<&[String]>,
        complete: bool,
        observer: Option<&mut dyn FnMut(GateObservation)>,
    ) -> Result<ContentReport> {
        self.evaluate_with_observer_inner(input, capabilities, destinations, complete, observer)
    }

    fn evaluate_with_observer_inner(
        &self,
        input: &ContentInput,
        capabilities: &ContentEvaluationContext<'_>,
        destinations: Option<&[String]>,
        collection_complete: bool,
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
        let block_ends = context
            .and_then(|context| context.block_ends.as_deref())
            .unwrap_or_default();
        for gate in &self.gates {
            if let Some(observe) = observer.as_deref_mut() {
                observe(GateObservation::Started);
            }
            let mut findings = Vec::new();
            let mut reason = None;
            let mut measurements = std::collections::BTreeMap::new();
            let mut omitted = 0;
            let mut emit = |message: String, line: usize, column: usize| {
                if findings.len() >= MAX_FINDINGS_PER_GATE {
                    omitted += 1;
                    return;
                }
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
                                *word_count.get_or_insert_with(|| count_words(text, block_ends));
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
                ContentGate::UniqueCollectionMetadata {
                    field, group_by, ..
                } => match collection {
                    Some(items) => {
                        measurements.insert("collection_items".into(), items.len());
                        let mut first_by_value = BTreeMap::new();
                        let mut missing = 0;
                        let mut missing_group = 0;
                        let mut duplicates = 0;
                        for item in items {
                            let value = item
                                .metadata
                                .as_ref()
                                .and_then(|m| m.get(field))
                                .filter(|v| !v.trim().is_empty());
                            let group = group_by.as_ref().and_then(|field| {
                                item.metadata
                                    .as_ref()
                                    .and_then(|metadata| metadata.get(field))
                                    .filter(|value| !value.trim().is_empty())
                            });
                            if value.is_none() {
                                missing += 1;
                            }
                            if group_by.is_some() && group.is_none() {
                                missing_group += 1;
                            }
                            if let (Some(value), true) =
                                (value, group_by.is_none() || group.is_some())
                            {
                                let key = (group, value);
                                if let Some(first_id) = first_by_value.get(&key) {
                                    duplicates += 1;
                                    if findings.len() >= MAX_FINDINGS_PER_GATE {
                                        omitted += 1;
                                        continue;
                                    }
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
                                    first_by_value.insert(key, &item.id);
                                }
                            }
                        }
                        measurements.insert("items_missing_field".into(), missing);
                        if group_by.is_some() {
                            measurements.insert("items_missing_group".into(), missing_group);
                        }
                        measurements.insert("duplicate_items".into(), duplicates);
                        if missing > 0 || missing_group > 0 || !collection_complete {
                            let mut causes = Vec::new();
                            if !collection_complete {
                                causes.push("collection is incomplete".to_owned());
                            }
                            if missing > 0 {
                                causes.push(format!("metadata field {field:?} is unavailable for {missing} collection item(s)"));
                            }
                            if let Some(group_field) =
                                group_by.as_ref().filter(|_| missing_group > 0)
                            {
                                causes.push(format!("group metadata field {group_field:?} is unavailable for {missing_group} collection item(s)"));
                            }
                            reason = Some(causes.join("; "));
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
                                        "Link destination #{} {}",
                                        index + 1,
                                        link_destination_problem(
                                            destination,
                                            allowed_schemes,
                                            *allow_relative,
                                        )
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
                ContentGate::LinkReachability { .. } => {
                    match (destinations, capabilities.link_reachability) {
                        (Some(values), Some(observations))
                            if values.len() == observations.len() =>
                        {
                            let mut unchecked = 0usize;
                            let mut not_found = 0usize;
                            let mut checked = 0usize;
                            for (index, observation) in observations.iter().enumerate() {
                                match observation {
                                    LinkReachabilityObservation::Reachable => checked += 1,
                                    LinkReachabilityObservation::NotFound => {
                                        checked += 1;
                                        not_found += 1;
                                        emit(
                                            format!(
                                                "Link destination #{} returned HTTP 404 or 410; review availability",
                                                index + 1
                                            ),
                                            1,
                                            1,
                                        );
                                    }
                                    LinkReachabilityObservation::NotChecked => unchecked += 1,
                                    LinkReachabilityObservation::NotApplicable => {}
                                }
                            }
                            measurements.insert("checked_destinations".into(), checked);
                            measurements.insert("not_found_destinations".into(), not_found);
                            measurements.insert("unchecked_destinations".into(), unchecked);
                            if unchecked > 0 {
                                reason = Some("some link destinations could not be checked; findings cover only observed responses".into());
                            }
                        }
                        (Some(_), Some(_)) => {
                            reason =
                                Some("link observations do not match the destination count".into())
                        }
                        _ => {
                            reason = Some(
                                "link reachability observations are unavailable from this adapter"
                                    .into(),
                            )
                        }
                    }
                }
                ContentGate::ParagraphCount { min, max, .. } => {
                    if let Some(minimum) = min {
                        measurements.insert("min_paragraphs".into(), *minimum);
                    }
                    if let Some(maximum) = max {
                        measurements.insert("max_paragraphs".into(), *maximum);
                    }
                    match context.and_then(|c| c.paragraphs.as_ref()) {
                        Some(paragraphs) => {
                            let count = paragraphs.len();
                            measurements.insert("paragraph_count".into(), count);
                            if let Some(minimum) = min.filter(|n| count < *n) {
                                emit(
                                    format!(
                                        "Paragraph count {count} is below the minimum of {minimum}"
                                    ),
                                    1,
                                    1,
                                );
                            }
                            if let Some(maximum) = max.filter(|n| count > *n) {
                                emit(
                                    format!(
                                        "Paragraph count {count} exceeds the maximum of {maximum}"
                                    ),
                                    1,
                                    1,
                                );
                            }
                        }
                        None => {
                            reason = Some("paragraphs are unavailable from this adapter".into())
                        }
                    }
                }
                ContentGate::ParagraphLength {
                    max_words: limit, ..
                }
                | ContentGate::ParagraphCharacterLength {
                    max_chars: limit, ..
                } => {
                    let chars = matches!(gate, ContentGate::ParagraphCharacterLength { .. });
                    let (limit_key, longest_key, unit) = if chars {
                        ("max_chars", "longest_paragraph_chars", "characters")
                    } else {
                        ("max_words", "longest_paragraph_words", "words")
                    };
                    measurements.insert(limit_key.into(), *limit);
                    match context.and_then(|c| c.paragraphs.as_ref()) {
                        Some(paragraphs) => {
                            let mut longest = 0;
                            let mut exceeding = 0;
                            for paragraph in paragraphs {
                                let count = if chars {
                                    paragraph.text.chars().count()
                                } else {
                                    paragraph.text.split_whitespace().count()
                                };
                                longest = longest.max(count);
                                if count > *limit {
                                    exceeding += 1;
                                    emit(
                                        format!("Paragraph has {count} {unit}; maximum is {limit}"),
                                        paragraph.line,
                                        paragraph.column,
                                    );
                                }
                            }
                            measurements.insert("paragraph_count".into(), paragraphs.len());
                            measurements.insert("paragraphs_exceeding_limit".into(), exceeding);
                            measurements.insert(longest_key.into(), longest);
                        }
                        None => {
                            reason = Some("paragraphs are unavailable from this adapter".into())
                        }
                    }
                }
                ContentGate::RepeatedParagraphOpenings {
                    opening_words,
                    max_occurrences,
                    ..
                } => {
                    measurements.insert("opening_words".into(), *opening_words);
                    measurements.insert("max_occurrences".into(), *max_occurrences);
                    match context.and_then(|c| c.paragraphs.as_ref()) {
                        Some(paragraphs) => {
                            let mut openings = BTreeMap::<Vec<String>, usize>::new();
                            let mut exceeding = 0;
                            let mut evaluated = 0;
                            let mut skipped = 0;
                            for paragraph in paragraphs {
                                let mut words = Vec::with_capacity(*opening_words);
                                let mut bytes = 0usize;
                                for word in paragraph.text.split_whitespace().take(*opening_words) {
                                    bytes += word.len();
                                    if bytes > MAX_PARAGRAPH_OPENING_BYTES {
                                        break;
                                    }
                                    words.push(
                                        word.trim_matches(|ch: char| !ch.is_alphanumeric())
                                            .to_lowercase(),
                                    );
                                }
                                if bytes > MAX_PARAGRAPH_OPENING_BYTES {
                                    reason = Some(
                                        "paragraph opening exceeds the supported byte limit".into(),
                                    );
                                    break;
                                }
                                if words.len() != *opening_words
                                    || words.iter().any(String::is_empty)
                                {
                                    skipped += 1;
                                    continue;
                                }
                                if openings.len() >= MAX_DISTINCT_PARAGRAPH_OPENINGS
                                    && !openings.contains_key(&words)
                                {
                                    reason = Some(
                                        "too many distinct paragraph openings to assess completely"
                                            .into(),
                                    );
                                    break;
                                }
                                evaluated += 1;
                                let count = openings.entry(words).or_default();
                                *count += 1;
                                if *count > *max_occurrences {
                                    exceeding += 1;
                                    emit(
                                        format!(
                                            "Paragraph opening occurs {count} times; maximum is {max_occurrences}"
                                        ),
                                        paragraph.line,
                                        paragraph.column,
                                    );
                                }
                            }
                            measurements.insert("paragraph_count".into(), paragraphs.len());
                            measurements.insert("openings_evaluated".into(), evaluated);
                            measurements.insert("openings_skipped".into(), skipped);
                            measurements.insert("openings_exceeding_limit".into(), exceeding);
                            if reason.is_none() && skipped > 0 {
                                reason = Some(
                                    "some paragraph openings could not be assessed using whitespace-separated words"
                                        .into(),
                                );
                            }
                        }
                        None => {
                            reason = Some("paragraphs are unavailable from this adapter".into())
                        }
                    }
                }
                ContentGate::SentenceLength {
                    max_words: limit,
                    delimiters,
                    abbreviations,
                    paragraphs_only,
                    ..
                }
                | ContentGate::SentenceCharacterLength {
                    max_chars: limit,
                    delimiters,
                    abbreviations,
                    paragraphs_only,
                    ..
                } => {
                    let chars = matches!(gate, ContentGate::SentenceCharacterLength { .. });
                    let (limit_key, longest_key, unit) = if chars {
                        ("max_chars", "longest_sentence_chars", "characters")
                    } else {
                        ("max_words", "longest_sentence_words", "words")
                    };
                    measurements.insert(limit_key.into(), *limit);
                    let paragraphs = context.and_then(|context| context.paragraphs.as_ref());
                    let available = if *paragraphs_only {
                        paragraphs.is_some()
                    } else {
                        input.text.is_some()
                    };
                    if available {
                        let mut sentence_count = 0;
                        let mut longest = 0;
                        let mut exceeding = 0;
                        let segments = paragraphs
                            .filter(|_| *paragraphs_only)
                            .into_iter()
                            .flatten()
                            .map(|paragraph| {
                                (
                                    paragraph.text.as_str(),
                                    paragraph.line,
                                    paragraph.column,
                                    &[][..],
                                )
                            })
                            .chain(
                                input
                                    .text
                                    .iter()
                                    .filter(|_| !*paragraphs_only)
                                    .map(|text| (text.as_str(), 1, 1, block_ends)),
                            );
                        for (text, mut line, mut column, segment_block_ends) in segments {
                            let mut segment_start = 0;
                            let mut first_position = None;
                            let mut next_block = 0;
                            for (offset, ch) in text
                                .char_indices()
                                .map(|(offset, ch)| (offset, Some(ch)))
                                .chain(std::iter::once((text.len(), None)))
                            {
                                while segment_block_ends
                                    .get(next_block)
                                    .is_some_and(|&end| end < offset)
                                {
                                    next_block += 1;
                                }
                                let block_end = segment_block_ends.get(next_block) == Some(&offset);
                                // A block boundary precedes the character at its byte offset.
                                // A separator is not required, and decimal/abbreviation
                                // context must not cross into the next block.
                                let decimal_point = !block_end
                                    && ch == Some('.')
                                    && text[..offset]
                                        .chars()
                                        .next_back()
                                        .is_some_and(|c| c.is_ascii_digit())
                                    && text[offset + 1..]
                                        .chars()
                                        .next()
                                        .is_some_and(|c| c.is_ascii_digit());
                                let abbreviation = !block_end
                                    && ch == Some('.')
                                    && !abbreviations.is_empty()
                                    && text[segment_start..offset]
                                        .split_whitespace()
                                        .next_back()
                                        .is_some_and(|word| {
                                            abbreviations
                                                .iter()
                                                .any(|abbr| abbr.strip_suffix('.') == Some(word))
                                        });
                                let delimiter = !decimal_point
                                    && !abbreviation
                                    && ch.is_none_or(|ch| delimiters.contains(ch));
                                if block_end || delimiter {
                                    let segment = &text[segment_start..offset];
                                    let count = if chars {
                                        segment.trim().chars().count()
                                    } else {
                                        segment.split_whitespace().count()
                                    };
                                    if count > 0 {
                                        sentence_count += 1;
                                        longest = longest.max(count);
                                        if count > *limit {
                                            exceeding += 1;
                                            let (start_line, start_column) =
                                                first_position.unwrap_or((line, column));
                                            emit(
                                                format!(
                                                    "Sentence has {count} {unit}; maximum is {limit}"
                                                ),
                                                start_line,
                                                start_column,
                                            );
                                        }
                                    }
                                    segment_start = offset
                                        + if delimiter {
                                            ch.map_or(0, char::len_utf8)
                                        } else {
                                            0
                                        };
                                    first_position =
                                        if !delimiter && ch.is_some_and(|ch| !ch.is_whitespace()) {
                                            Some((line, column))
                                        } else {
                                            None
                                        };
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
                        }
                        measurements.insert("sentence_count".into(), sentence_count);
                        measurements.insert("sentences_exceeding_limit".into(), exceeding);
                        measurements.insert(longest_key.into(), longest);
                    } else {
                        reason = Some(
                            if *paragraphs_only {
                                "paragraphs are unavailable from this adapter"
                            } else {
                                "text is unavailable from this adapter"
                            }
                            .into(),
                        );
                    }
                }
                ContentGate::RepeatedTerms {
                    terms,
                    max_occurrences,
                    whole_words,
                    case_insensitive,
                    ..
                } => {
                    measurements.insert("configured_terms".into(), terms.len());
                    measurements.insert("max_occurrences".into(), *max_occurrences);
                    match &input.text {
                        Some(text) => {
                            let mut total = 0;
                            let mut exceeded = 0;
                            let literal = LiteralText::new(text, block_ends, *case_insensitive);
                            for term in terms {
                                let mut first_excess = None;
                                let mut count = 0;
                                literal.visit_matches(term, *whole_words, |offset| {
                                    count += 1;
                                    if count > *max_occurrences && first_excess.is_none() {
                                        first_excess = Some(offset);
                                    }
                                    false
                                });
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
                ContentGate::ForbiddenTerms {
                    terms,
                    whole_words,
                    case_insensitive,
                    ..
                } => {
                    measurements.insert("configured_terms".into(), terms.len());
                    match &input.text {
                        Some(text) => {
                            let mut matches = 0;
                            let literal = LiteralText::new(text, block_ends, *case_insensitive);
                            for term in terms {
                                let (mut cursor, mut line, mut column) = (0, 1, 1);
                                literal.visit_matches(term, *whole_words, |offset| {
                                    matches += 1;
                                    if matches <= MAX_FINDINGS_PER_GATE {
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
                                    false
                                });
                            }
                            omitted += matches.saturating_sub(MAX_FINDINGS_PER_GATE);
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
                            if !claim_is_present(text, claim, block_ends) {
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
                                    if !claim_is_present(text, claim, block_ends) {
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
                                    if !claim_is_present(text, claim, block_ends) {
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
                            let unavailable = context
                                .map(|ctx| ctx.unavailable_metadata_fields.as_slice())
                                .unwrap_or_default();
                            let unknown = fields
                                .iter()
                                .filter(|field| unavailable.contains(field))
                                .count();
                            measurements.insert("unavailable_fields".into(), unknown);
                            measurements.insert(
                                "present_fields".into(),
                                fields
                                    .iter()
                                    .filter(|field| {
                                        !unavailable.contains(field)
                                            && metadata
                                                .get(*field)
                                                .is_some_and(|v| !v.trim().is_empty())
                                    })
                                    .count(),
                            );
                            for field in fields {
                                if !unavailable.contains(field)
                                    && metadata.get(field).is_none_or(|v| v.trim().is_empty())
                                {
                                    emit(format!("Required metadata is missing: {field}"), 1, 1);
                                }
                            }
                            if unknown > 0 {
                                reason = Some("some required metadata fields are unavailable from this adapter".into());
                            }
                        }
                        None => reason = Some("metadata is unavailable from this adapter".into()),
                    }
                }
            }
            if omitted > 0 {
                measurements.insert(FINDINGS_OMITTED.into(), omitted);
            }
            if reason.is_some()
                && !matches!(
                    gate,
                    ContentGate::RequiredMetadata { .. }
                        | ContentGate::UniqueCollectionMetadata { .. }
                        | ContentGate::Custom { .. }
                        | ContentGate::SiteGroup { .. }
                        | ContentGate::SiteAudit { .. }
                )
                && let Some(limit) = context.and_then(|context| context.input_limit.as_ref())
            {
                reason = Some(limit.clone());
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
    fn literal_input(text: &str) -> ContentInput {
        ContentInput {
            id: "t".into(),
            text: Some(text.into()),
            ..Default::default()
        }
    }

    fn forbidden(settings: &str, text: &str) -> ContentGateResult {
        ContentPreset::from_toml(&format!(
            "version=1\nname='t'\n[[gates]]\nid='g'\nkind='forbidden_terms'\n{settings}\n"
        ))
        .unwrap()
        .evaluate(&literal_input(text))
        .unwrap()
        .gates
        .remove(0)
    }

    #[test]
    fn forbidden_terms_default_to_whole_words_like_repeated_terms() {
        assert_eq!(
            forbidden("terms=['free']", "freedom and carefree").status,
            GateStatus::Passed
        );
        assert_eq!(
            forbidden("terms=['free']", "a free trial").status,
            GateStatus::Failed
        );
        assert_eq!(
            forbidden("terms=['free']\nwhole_words=false", "freedom").status,
            GateStatus::Failed
        );
    }

    #[test]
    fn whole_words_still_match_in_scripts_without_spaces() {
        assert_eq!(
            forbidden("terms=['価格']", "この価格は安い").status,
            GateStatus::Failed
        );
        assert_eq!(
            forbidden("terms=['free']", "無料free試用").status,
            GateStatus::Failed
        );
    }

    #[test]
    fn forbidden_terms_are_case_sensitive_unless_configured() {
        assert_eq!(
            forbidden("terms=['free']", "FREE").status,
            GateStatus::Passed
        );
        assert_eq!(
            forbidden("terms=['free']\ncase_insensitive=true", "A FREE trial").status,
            GateStatus::Failed
        );
        let result = forbidden("terms=['fin']\ncase_insensitive=true", "İİ\nİ FIN");
        assert_eq!((result.findings[0].line, result.findings[0].column), (2, 3));
    }

    #[test]
    fn literal_matches_ignore_whitespace_differences_and_keep_positions() {
        let result = forbidden(
            "terms=['best price']",
            "é  best\n  price and best\u{a0}price and best  price",
        );
        let positions: Vec<_> = result.findings.iter().map(|f| (f.line, f.column)).collect();
        assert_eq!(positions, [(1, 4), (2, 13), (2, 28)]);
        assert_eq!(
            forbidden("terms=['best price']", "best\n\nprice").status,
            GateStatus::Passed
        );
    }

    #[test]
    fn claim_anchors_respect_adapter_block_boundaries() {
        assert!(!claim_is_present("alpha beta", "alpha beta", &[5]));
        assert!(claim_is_present("alpha beta", "alpha beta", &[]));
        assert!(claim_is_present(
            "alpha beta alpha beta",
            "alpha beta",
            &[5]
        ));
        assert!(!claim_is_present("alpha  beta", "alpha beta", &[5]));
        assert!(claim_is_present("aaa", "aa", &[1]));
    }

    #[test]
    fn sentence_gates_preserve_text_at_adjacent_block_boundaries() {
        for (kind, limit, longest) in [
            ("sentence_length", "max_words=1", "longest_sentence_words"),
            (
                "sentence_character_length",
                "max_chars=3",
                "longest_sentence_chars",
            ),
        ] {
            let preset = ContentPreset::from_toml(&format!(
                "version=1\nname='t'\n[[gates]]\nid='g'\nkind='{kind}'\n{limit}\n"
            ))
            .unwrap();
            let context = ContentContext {
                block_ends: Some(vec![0, 1, 1, 10]),
                ..Default::default()
            };
            let report = preset
                .evaluate_with_context(&literal_input("Aété fin"), None, None, Some(&context))
                .unwrap();
            let gate = &report.gates[0];
            assert_eq!(gate.measurements["sentence_count"], 2, "{kind}");
            assert_eq!(
                gate.measurements[longest],
                if kind == "sentence_length" { 2 } else { 7 },
                "{kind}"
            );
            assert_eq!(gate.findings.len(), 1, "{kind}");
            assert_eq!(
                (gate.findings[0].line, gate.findings[0].column),
                (1, 2),
                "{kind}"
            );
        }
    }

    #[test]
    fn punctuation_at_adjacent_block_boundary_starts_a_new_context() {
        let preset = ContentPreset::from_toml(
            "version=1\nname='t'\n[[gates]]\nid='g'\nkind='sentence_length'\nmax_words=1\n",
        )
        .unwrap();
        for (text, boundary, position) in [("1.2 trois", 1, (1, 3)), ("Un.deux trois", 3, (1, 4))] {
            let context = ContentContext {
                block_ends: Some(vec![boundary]),
                ..Default::default()
            };
            let report = preset
                .evaluate_with_context(&literal_input(text), None, None, Some(&context))
                .unwrap();
            let gate = &report.gates[0];
            assert_eq!(gate.measurements["sentence_count"], 2, "{text}");
            assert_eq!(gate.findings.len(), 1, "{text}");
            assert_eq!(
                (gate.findings[0].line, gate.findings[0].column),
                position,
                "{text}"
            );
        }
    }

    #[test]
    fn whole_word_gates_respect_adjacent_block_boundaries() {
        let preset = ContentPreset::from_toml("version=1\nname='t'\n[[gates]]\nid='forbidden'\nkind='forbidden_terms'\nterms=['café','chat','caféchat']\ncase_insensitive=true\n[[gates]]\nid='repeat'\nkind='repeated_terms'\nterms=['chat']\nmax_occurrences=1\n").unwrap();
        let input = literal_input("CAFÉchatchat");
        let context = ContentContext {
            block_ends: Some(vec![5, 9]),
            ..Default::default()
        };
        let report = preset
            .evaluate_with_context(&input, None, None, Some(&context))
            .unwrap();
        let forbidden = &report.gates[0];
        assert_eq!(forbidden.measurements["matches"], 3);
        assert_eq!(
            forbidden
                .findings
                .iter()
                .map(|f| (f.line, f.column))
                .collect::<Vec<_>>(),
            [(1, 1), (1, 5), (1, 9)]
        );
        let repeat = &report.gates[1];
        assert_eq!(repeat.measurements["matching_occurrences"], 2);
        assert_eq!(repeat.status, GateStatus::Failed);
        assert_eq!((repeat.findings[0].line, repeat.findings[0].column), (1, 9));
        let adjacent = LiteralText::new("caféchat", &[5], false);
        assert!(!adjacent.visit_matches("caféchat", true, |_| true));
        assert!(!adjacent.visit_matches("caféchat", false, |_| true));
        assert!(LiteralText::new("caféchat", &[], false).visit_matches("caféchat", true, |_| true));
        let no_boundaries = preset.evaluate(&input).unwrap();
        assert_eq!(no_boundaries.gates[0].measurements["matches"], 0);
        assert_eq!(
            no_boundaries.gates[1].measurements["matching_occurrences"],
            0
        );
    }

    #[test]
    fn literal_match_visitor_stops_early_and_counts_all_occurrences() {
        let text = "word ".repeat(100_000);
        let literal = LiteralText::new(&text, &[], false);
        let mut visited = 0;
        assert!(literal.visit_matches("word", true, |_| {
            visited += 1;
            true
        }));
        assert_eq!(visited, 1);
        assert!(!literal.visit_matches("word", true, |_| {
            visited += 1;
            false
        }));
        assert_eq!(visited, 100_001);
    }

    #[test]
    fn evidence_gate_rejects_claim_spanning_blocks() {
        let preset = ContentPreset::from_toml("version=1\nname='t'\n[[gates]]\nid='g'\nkind='evidence_anchors'\nsource='s'\nclaim='alpha beta'\nquote='quote'\n").unwrap();
        let input = ContentInput {
            evidence_sources: Some(BTreeMap::from([("s".into(), "quote".into())])),
            ..literal_input("alpha beta")
        };
        let context = ContentContext {
            block_ends: Some(vec![5]),
            ..Default::default()
        };
        let report = preset
            .evaluate_with_context(&input, None, None, Some(&context))
            .unwrap();
        assert_eq!(report.gates[0].status, GateStatus::Failed);
        assert_eq!(
            report.gates[0].findings[0].message,
            "Claim anchor is absent from content"
        );
    }

    #[test]
    fn wrapped_claims_are_present_in_every_anchor_gate() {
        for gate in [
            "kind='evidence_anchors'\nsource='s'\nclaim='rose by ten percent'\nquote='x'",
            "kind='structured_fact'\nsource='s'\nclaim='rose by ten percent'\npointer=''\nexpected='x'",
        ] {
            let preset = ContentPreset::from_toml(&format!(
                "version=1\nname='t'\n[[gates]]\nid='g'\n{gate}\n"
            ))
            .unwrap();
            let report = preset
                .evaluate(&ContentInput {
                    evidence_sources: Some(BTreeMap::from([("s".into(), "\"x\"".into())])),
                    ..literal_input("Sales rose\nby ten\n  percent.")
                })
                .unwrap();
            assert_eq!(report.gates[0].status, GateStatus::Passed, "{gate}");
        }
    }

    #[test]
    fn length_measurements_count_all_violations_and_distinguish_unavailable() {
        for (kind, limit, key) in [
            (
                "paragraph_length",
                "max_words",
                "paragraphs_exceeding_limit",
            ),
            (
                "paragraph_character_length",
                "max_chars",
                "paragraphs_exceeding_limit",
            ),
            ("sentence_length", "max_words", "sentences_exceeding_limit"),
            (
                "sentence_character_length",
                "max_chars",
                "sentences_exceeding_limit",
            ),
        ] {
            let preset = ContentPreset::from_toml(&format!(
                "version=1\nname='t'\n[[gates]]\nid='g'\nkind='{kind}'\n{limit}=1\n"
            ))
            .unwrap();
            let count = MAX_FINDINGS_PER_GATE + 5;
            let context = ContentContext {
                paragraphs: Some(
                    (0..count)
                        .map(|_| ContentParagraph {
                            text: "two words".into(),
                            line: 1,
                            column: 1,
                        })
                        .collect(),
                ),
                ..Default::default()
            };
            let text = "two words. ".repeat(count);
            let input = literal_input(&text);
            let report = preset
                .evaluate_with_context(&input, None, None, Some(&context))
                .unwrap();
            let result = &report.gates[0];
            assert_eq!(result.measurements[key], count, "{kind}");
            assert_eq!(result.findings.len(), MAX_FINDINGS_PER_GATE, "{kind}");
            assert_eq!(result.measurements[FINDINGS_OMITTED], 5, "{kind}");

            let empty = literal_input("");
            let empty_context = ContentContext {
                paragraphs: Some(vec![]),
                ..Default::default()
            };
            let report = preset
                .evaluate_with_context(&empty, None, None, Some(&empty_context))
                .unwrap();
            assert_eq!(report.gates[0].measurements[key], 0, "{kind}");
            let missing = ContentInput {
                text: None,
                ..literal_input("")
            };
            let report = preset.evaluate(&missing).unwrap();
            assert_eq!(report.gates[0].status, GateStatus::NotChecked, "{kind}");
            assert!(!report.gates[0].measurements.contains_key(key), "{kind}");
        }
    }

    #[test]
    fn findings_are_capped_and_the_rest_counted() {
        let result = forbidden("terms=['x']", &"x ".repeat(MAX_FINDINGS_PER_GATE + 5));
        assert_eq!(result.status, GateStatus::Failed);
        assert_eq!(result.findings.len(), MAX_FINDINGS_PER_GATE);
        assert_eq!(result.measurements[FINDINGS_OMITTED], 5);
        assert_eq!(result.measurements["matches"], MAX_FINDINGS_PER_GATE + 5);
        assert_eq!(result.total_findings(), MAX_FINDINGS_PER_GATE + 5);
    }

    #[test]
    fn forbidden_cap_across_terms_preserves_positions_and_total_count() {
        let text = format!("{}\nété fin fin", "x ".repeat(MAX_FINDINGS_PER_GATE - 1));
        let result = forbidden("terms=['x','été','fin']", &text);
        assert_eq!(result.findings.len(), MAX_FINDINGS_PER_GATE);
        assert_eq!(
            result.findings.last().unwrap().message,
            "Forbidden term: été"
        );
        assert_eq!(
            (
                result.findings.last().unwrap().line,
                result.findings.last().unwrap().column
            ),
            (2, 1)
        );
        assert_eq!(result.measurements["matches"], MAX_FINDINGS_PER_GATE + 2);
        assert_eq!(result.measurements[FINDINGS_OMITTED], 2);
        assert_eq!(result.total_findings(), MAX_FINDINGS_PER_GATE + 2);
    }

    #[test]
    fn a_preset_without_gates_is_invalid() {
        assert!(ContentPreset::from_toml("version=1\nname='x'\ngates=[]\n").is_err());
    }

    #[test]
    fn link_finding_names_value_and_scheme() {
        let preset = ContentPreset::from_toml(
            "version=1\nname='t'\n[[gates]]\nid='g'\nkind='link_destinations'\nallowed_schemes=['https']\nallow_relative=false\n",
        )
        .unwrap();
        let destinations = [
            "http://a.example".to_owned(),
            "/local".to_owned(),
            "a b".to_owned(),
        ];
        let report = preset
            .evaluate_with_destinations(
                &literal_input(""),
                &ContentEvaluationContext::default(),
                Some(&destinations),
            )
            .unwrap();
        let messages: Vec<_> = report.gates[0]
            .findings
            .iter()
            .map(|f| f.message.as_str())
            .collect();
        assert_eq!(
            messages,
            [
                "Link destination #1 \"http://a.example\" has disallowed scheme \"http\"; allowed: https",
                "Link destination #2 \"/local\" is relative, and relative links are not allowed",
                "Link destination #3 \"a b\" is empty or contains whitespace or control characters",
            ]
        );
    }

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
    fn word_count_respects_adjacent_adapter_blocks() {
        let preset = ContentPreset::from_toml("version=1\nname='t'\n[[gates]]\nid='min'\nkind='word_count'\nmin=3\n[[gates]]\nid='max'\nkind='word_count'\nmax=2\n").unwrap();
        assert_eq!(count_words("é a", &[0, 1, 2, 3, 4, 20]), 2);
        let input = literal_input("étéchat chien");
        let context = ContentContext {
            block_ends: Some(vec![0, 5, 5, 10, input.text.as_ref().unwrap().len()]),
            ..Default::default()
        };
        let report = preset
            .evaluate_with_context(&input, None, None, Some(&context))
            .unwrap();
        assert_eq!(report.gates[0].measurements["word_count"], 3);
        assert_eq!(report.gates[0].status, GateStatus::Passed);
        assert_eq!(report.gates[1].measurements["word_count"], 3);
        assert_eq!(report.gates[1].status, GateStatus::Failed);
        assert_eq!(
            preset.evaluate(&input).unwrap().gates[0].measurements["word_count"],
            2
        );
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
    fn repeated_terms_can_match_case_insensitively_without_changing_defaults() {
        let source = "version=1\nname='repeat'\n[[gates]]\nid='repeat'\nkind='repeated_terms'\nterms=['café']\nmax_occurrences=1\n";
        let input = ContentInput {
            text: Some("Café puis\nCAFÉ".into()),
            ..Default::default()
        };
        let default = ContentPreset::from_toml(source).unwrap();
        assert_eq!(
            default.evaluate(&input).unwrap().gates[0].status,
            GateStatus::Passed
        );
        assert!(
            serde_json::to_string(&default)
                .unwrap()
                .find("case_insensitive")
                .is_none()
        );
        let insensitive =
            ContentPreset::from_toml(&format!("{source}case_insensitive=true\n")).unwrap();
        let result = &insensitive.evaluate(&input).unwrap().gates[0];
        assert_eq!(result.status, GateStatus::Failed);
        assert_eq!(result.measurements["matching_occurrences"], 2);
        assert_eq!((result.findings[0].line, result.findings[0].column), (2, 1));
        assert!(ContentPreset::from_toml("version=1\nname='repeat'\n[[gates]]\nid='repeat'\nkind='repeated_terms'\nterms=['Café','CAFÉ']\nmax_occurrences=1\ncase_insensitive=true\n").is_err());
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
    fn collection_uniqueness_can_be_scoped_by_metadata() {
        let preset = ContentPreset::from_toml("version=1\nname='collection'\n[[gates]]\nid='unique'\nkind='unique_collection_metadata'\nfield='slug'\ngroup_by='lang'\n").unwrap();
        let item = |id: &str, slug: Option<&str>, lang: Option<&str>| {
            let mut metadata = BTreeMap::new();
            if let Some(value) = slug {
                metadata.insert("slug".into(), value.into());
            }
            if let Some(value) = lang {
                metadata.insert("lang".into(), value.into());
            }
            ContentInput {
                id: id.into(),
                metadata: Some(metadata),
                ..Default::default()
            }
        };
        let input = ContentInput::default();
        let items = vec![
            item("en", Some("shared"), Some("en")),
            item("fr", Some("shared"), Some("fr")),
        ];
        let run = |items: &[ContentInput], complete| {
            preset
                .evaluate_with_collection_completeness(
                    &input,
                    &ContentEvaluationContext {
                        collection: Some(items),
                        ..Default::default()
                    },
                    None,
                    complete,
                    None,
                )
                .unwrap()
        };
        assert_eq!(run(&items, true).gates[0].status, GateStatus::Passed);
        let duplicate = vec![
            items[0].clone(),
            items[1].clone(),
            item("en2", Some("shared"), Some("en")),
        ];
        let result = run(&duplicate, true);
        assert_eq!(result.gates[0].status, GateStatus::Failed);
        assert_eq!(result.gates[0].measurements["duplicate_items"], 1);
        assert_eq!(result.gates[0].findings[0].path, "en2");
        let mut incomplete = duplicate.clone();
        incomplete.push(item("unknown", Some("shared"), None));
        incomplete.push(item("blank", None, Some(" ")));
        let result = run(&incomplete, true);
        assert_eq!(result.gates[0].status, GateStatus::NotChecked);
        assert_eq!(result.gates[0].measurements["items_missing_field"], 1);
        assert_eq!(result.gates[0].measurements["items_missing_group"], 2);
        assert_eq!(result.gates[0].findings.len(), 1);
        assert!(
            result.gates[0]
                .reason
                .as_deref()
                .unwrap()
                .contains("group metadata field")
        );
        assert_eq!(
            run(&duplicate, false).gates[0].status,
            GateStatus::NotChecked
        );
        assert!(ContentPreset::from_toml("version=1\nname='bad'\n[[gates]]\nid='u'\nkind='unique_collection_metadata'\nfield='slug'\ngroup_by='  '\n").is_err());
        let old = ContentPreset::from_toml("version=1\nname='old'\n[[gates]]\nid='u'\nkind='unique_collection_metadata'\nfield='slug'\n").unwrap();
        assert!(
            serde_json::to_value(&old.gates[0])
                .unwrap()
                .get("group_by")
                .is_none()
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
    fn incomplete_collection_never_certifies_uniqueness_and_preserves_findings() {
        let preset = ContentPreset::from_toml("version=1\nname='test'\n[[gates]]\nid='unique'\nkind='unique_collection_metadata'\nfield='slug'\n").unwrap();
        let root = ContentInput {
            id: "collection".into(),
            ..Default::default()
        };
        let items = [
            ContentInput {
                id: "/a".into(),
                metadata: Some(BTreeMap::from([("slug".into(), "same".into())])),
                ..Default::default()
            },
            ContentInput {
                id: "/b".into(),
                metadata: Some(BTreeMap::from([("slug".into(), "same".into())])),
                ..Default::default()
            },
            ContentInput {
                id: "/c".into(),
                ..Default::default()
            },
        ];
        for observed in [&items[..], &items[..1], &items[..0]] {
            let report = preset
                .evaluate_with_collection_completeness(
                    &root,
                    &ContentEvaluationContext {
                        collection: Some(observed),
                        ..Default::default()
                    },
                    None,
                    false,
                    None,
                )
                .unwrap();
            let gate = &report.gates[0];
            assert_eq!(gate.status, GateStatus::NotChecked);
            assert!(
                gate.reason
                    .as_ref()
                    .unwrap()
                    .contains("collection is incomplete")
            );
            if observed.len() == 3 {
                assert_eq!(gate.findings.len(), 1);
                assert_eq!(gate.findings[0].path, "/b");
                assert!(gate.reason.as_ref().unwrap().contains("unavailable for 1"));
            }
        }
    }

    #[test]
    fn collection_duplicates_respect_finding_limit_and_keep_full_counts() {
        let preset = ContentPreset::from_toml("version=1\nname='test'\n[[gates]]\nid='unique'\nkind='unique_collection_metadata'\nfield='slug'\n").unwrap();
        let input = ContentInput {
            id: "collection".into(),
            ..Default::default()
        };
        let items: Vec<_> = (0..MAX_FINDINGS_PER_GATE + 9)
            .map(|index| ContentInput {
                id: index.to_string(),
                metadata: Some(BTreeMap::from([("slug".into(), "same".into())])),
                ..Default::default()
            })
            .collect();
        let report = preset
            .evaluate_with_capabilities(
                &input,
                &ContentEvaluationContext {
                    collection: Some(&items),
                    ..Default::default()
                },
            )
            .unwrap();
        let result = &report.gates[0];
        assert_eq!(result.status, GateStatus::Failed);
        assert_eq!(result.findings.len(), MAX_FINDINGS_PER_GATE);
        assert_eq!(
            result.measurements["duplicate_items"],
            MAX_FINDINGS_PER_GATE + 8
        );
        assert_eq!(result.measurements[FINDINGS_OMITTED], 8);
        assert_eq!(result.total_findings(), MAX_FINDINGS_PER_GATE + 8);
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
    #[test]
    fn paragraph_only_sentence_checks_distinguish_unavailable_from_empty() {
        for (kind, limit) in [
            ("sentence_length", "max_words=2"),
            ("sentence_character_length", "max_chars=5"),
        ] {
            let preset = ContentPreset::from_toml(&format!("version=1\nname='t'\n[[gates]]\nid='s'\nkind='{kind}'\n{limit}\nparagraphs_only=true\n")).unwrap();
            assert_eq!(
                serde_json::to_value(&preset.gates[0]).unwrap()["paragraphs_only"],
                true
            );
            let default_preset = ContentPreset::from_toml(&format!(
                "version=1\nname='t'\n[[gates]]\nid='s'\nkind='{kind}'\n{limit}\n"
            ))
            .unwrap();
            assert!(
                serde_json::to_value(&default_preset.gates[0])
                    .unwrap()
                    .get("paragraphs_only")
                    .is_none()
            );
            let input = ContentInput {
                text: Some("Long heading here".into()),
                ..Default::default()
            };
            let missing = preset.evaluate(&input).unwrap();
            assert_eq!(missing.gates[0].status, GateStatus::NotChecked);
            assert_eq!(missing.gates[0].required_capability, "paragraphs");
            let empty = ContentContext {
                paragraphs: Some(vec![]),
                ..Default::default()
            };
            let observed = preset
                .evaluate_with_context(&input, None, None, Some(&empty))
                .unwrap();
            assert_eq!(observed.gates[0].status, GateStatus::Passed);
            assert_eq!(observed.gates[0].measurements["sentence_count"], 0);
        }
    }

    #[test]
    fn paragraph_openings_report_coverage_and_bound_resource_use() {
        let preset = ContentPreset::from_toml(
            "version=1\nname='openings'\n[[gates]]\nid='opening'\nkind='repeated_paragraph_openings'\nopening_words=2\nmax_occurrences=1\n",
        ).unwrap();
        let input = ContentInput {
            text: Some(String::new()),
            ..Default::default()
        };
        let paragraph = |text: String| ContentParagraph {
            text,
            line: 1,
            column: 1,
        };
        let run = |paragraphs: Vec<ContentParagraph>| {
            preset
                .evaluate_with_context(
                    &input,
                    None,
                    None,
                    Some(&ContentContext {
                        paragraphs: Some(paragraphs),
                        ..Default::default()
                    }),
                )
                .unwrap()
                .gates
                .remove(0)
        };
        let unsupported = run(vec![paragraph("中文文章沒有空格".into())]);
        assert_eq!(unsupported.status, GateStatus::NotChecked);
        assert_eq!(unsupported.measurements["openings_skipped"], 1);
        let empty = run(vec![]);
        assert_eq!(empty.status, GateStatus::Passed);
        let mixed = run(vec![
            paragraph("one".into()),
            paragraph("same words".into()),
            paragraph("same words again".into()),
        ]);
        assert_eq!(mixed.status, GateStatus::NotChecked);
        assert_eq!(mixed.findings.len(), 1);
        assert!(
            mixed
                .reason
                .as_deref()
                .unwrap()
                .contains("could not be assessed")
        );
        assert_eq!(mixed.measurements["openings_evaluated"], 2);
        assert_eq!(mixed.measurements["openings_skipped"], 1);
        let partial_without_findings = run(vec![
            paragraph("valid opening".into()),
            paragraph("中文文章沒有空格".into()),
        ]);
        assert_eq!(partial_without_findings.status, GateStatus::NotChecked);
        assert!(partial_without_findings.findings.is_empty());
        assert_eq!(
            partial_without_findings.measurements["openings_evaluated"],
            1
        );
        assert_eq!(partial_without_findings.measurements["openings_skipped"], 1);
        let complete = run(vec![paragraph("valid opening".into())]);
        assert_eq!(complete.status, GateStatus::Passed);
        let large = run(vec![
            paragraph("same words".into()),
            paragraph("same words again".into()),
            paragraph(format!(
                "{} tail",
                "x".repeat(MAX_PARAGRAPH_OPENING_BYTES + 1)
            )),
        ]);
        assert_eq!(large.status, GateStatus::NotChecked);
        assert_eq!(large.findings.len(), 1);
        assert_eq!(large.measurements["openings_evaluated"], 2);
        let unique = (0..=MAX_DISTINCT_PARAGRAPH_OPENINGS)
            .map(|index| paragraph(format!("prefix{index} word")))
            .collect();
        let capped = run(unique);
        assert_eq!(capped.status, GateStatus::NotChecked);
        assert_eq!(
            capped.measurements["openings_evaluated"],
            MAX_DISTINCT_PARAGRAPH_OPENINGS
        );
    }
    #[test]
    fn reachability_gate_preserves_partial_findings_without_network() {
        use LinkReachabilityObservation::{NotApplicable, NotChecked, NotFound, Reachable};
        let preset = ContentPreset::from_toml(
            "version=1\nname='links'\n[[gates]]\nid='external'\nkind='link_reachability'\n",
        )
        .unwrap();
        let input = ContentInput {
            id: "test".into(),
            ..Default::default()
        };
        let destinations = vec![
            "https://a.example".to_owned(),
            "https://b.example".to_owned(),
            "mailto:test@example.org".to_owned(),
        ];
        let evaluate = |observations: Option<&[LinkReachabilityObservation]>| {
            preset
                .evaluate_with_destinations(
                    &input,
                    &ContentEvaluationContext {
                        link_reachability: observations,
                        ..Default::default()
                    },
                    Some(&destinations),
                )
                .unwrap()
                .gates
                .remove(0)
        };
        let unavailable = evaluate(None);
        assert_eq!(unavailable.status, GateStatus::NotChecked);
        assert!(unavailable.findings.is_empty());
        let mismatched = evaluate(Some(&[Reachable]));
        assert_eq!(mismatched.status, GateStatus::NotChecked);
        let partial = evaluate(Some(&[NotFound, NotChecked, NotApplicable]));
        assert_eq!(partial.status, GateStatus::NotChecked);
        assert_eq!(partial.findings.len(), 1);
        assert_eq!(partial.measurements["unchecked_destinations"], 1);
        assert!(partial.findings[0].message.contains("#1"));
        let complete = evaluate(Some(&[NotFound, Reachable, NotApplicable]));
        assert_eq!(complete.status, GateStatus::Failed);
        assert!(!complete.blocking);
        let passed = evaluate(Some(&[Reachable, Reachable, NotApplicable]));
        assert_eq!(passed.status, GateStatus::Passed);
    }
}

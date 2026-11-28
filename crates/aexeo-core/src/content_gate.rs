//! Portable, offline checks for content supplied by an adapter.
//!
//! Missing input is reported separately from a passing gate. Adapters must not
//! represent an unavailable field as an empty string.
use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use unicode_normalization::{UnicodeNormalization, char::is_combining_mark};

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
    /// Explicit literal-Markdown link observations in the projected text, if available.
    pub literal_markdown_links: Option<Vec<(usize, usize)>>,
    /// Sorted byte offsets in the projected text where a block ends: a heading,
    /// paragraph, list item or table cell. A sentence ends there and a literal
    /// term does not match across it. Word counts treat it as a separator.
    /// Raw text has no known block structure.
    pub block_ends: Option<Vec<usize>>,
    /// Why the adapter withheld the body. This explains missing text, paragraph,
    /// heading, and source-text capabilities; independent errors keep their own reason.
    pub input_limit: Option<String>,
    /// Metadata keys whose values or absence were not observed by the adapter.
    pub unavailable_metadata_fields: Vec<String>,
    /// If supplied, only these keys have observed values or observed absence.
    pub observed_metadata_fields: Option<BTreeSet<String>>,
}

impl ContentContext {
    fn metadata_field_unavailable(&self, field: &str) -> bool {
        self.unavailable_metadata_fields
            .iter()
            .any(|item| item == field)
            || self
                .observed_metadata_fields
                .as_ref()
                .is_some_and(|observed| !observed.contains(field))
    }
}

/// Builds an unmasked literal projection only when a numeric reference occurs.
/// Supported reference labels retain their byte and character offsets in the projected text.
#[derive(Default)]
pub struct ReferenceLiteralProjection {
    original: Option<String>,
    copied: usize,
}

// Keep each masked character's UTF-8 byte width and character position.
fn reference_mask_char(ch: char) -> Option<char> {
    match ch.len_utf8() {
        1 => Some(' '),
        2 => Some('\u{00a0}'),
        3 => Some('\u{3000}'),
        _ => None,
    }
}

impl ReferenceLiteralProjection {
    pub fn push_masked(&mut self, projected: &mut String, label: &str) {
        let original = self.original.get_or_insert_with(String::new);
        original.push_str(&projected[self.copied..]);
        original.push_str(label);
        for ch in label.chars() {
            if let Some(mask) = reference_mask_char(ch) {
                projected.push(mask);
            } else {
                projected.extend(std::iter::repeat_n(' ', ch.len_utf8()));
            }
        }
        self.copied = projected.len();
    }

    pub fn finish(mut self, projected: &str) -> Option<String> {
        if let Some(original) = &mut self.original {
            original.push_str(&projected[self.copied..]);
        }
        self.original
    }
}

pub(crate) fn literal_projection_matches(projected: &str, literal: &str) -> bool {
    if projected.len() != literal.len() {
        return false;
    }
    let mut masked = projected.chars();
    let mut original = literal.chars();
    loop {
        match (masked.next(), original.next()) {
            (None, None) => return true,
            (Some(a), Some(b))
                if a == b
                    || (reference_label_character(b) && reference_mask_char(b) == Some(a)) => {}
            _ => return false,
        }
    }
}

// Accept only a complete calendar date; timestamps and implicit time zones are
// deliberately excluded so the caller chooses and binds the evaluation date.
pub(crate) fn parse_iso_calendar_date(value: &str) -> Option<time::Date> {
    let bytes = value.as_bytes();
    if bytes.len() != 10
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || !bytes
            .iter()
            .enumerate()
            .all(|(i, b)| i == 4 || i == 7 || b.is_ascii_digit())
    {
        return None;
    }
    let year = value.get(..4)?.parse().ok()?;
    let month = time::Month::try_from(value.get(5..7)?.parse::<u8>().ok()?).ok()?;
    let day = value.get(8..10)?.parse().ok()?;
    time::Date::from_calendar_date(year, month, day).ok()
}

/// Largest input the CLI adapters read. Larger input is reported as `not_checked`.
pub const MAX_CONTENT_INPUT_BYTES: usize = 16 * 1024 * 1024;

/// Findings kept for one document gate. The `findings_omitted` measurement
/// counts any others, so a truncated gate still reports its full total.
pub const MAX_FINDINGS_PER_GATE: usize = 1000;

// Bound collection work for adapters that do not impose CLI input limits.
const MAX_COLLECTION_ITEMS: usize = 100_000;
const MAX_COLLECTION_KEYS: usize = 10_000;
const MAX_COLLECTION_KEY_BYTES: usize = 4 * 1024 * 1024;
const MAX_COLLECTION_VALUE_BYTES: usize = 16 * 1024;

// Keep the advisory openings gate bounded for adapters other than the CLI.
const MAX_PARAGRAPH_OPENINGS: usize = 100_000;
const MAX_DISTINCT_PARAGRAPH_OPENINGS: usize = 10_000;
const MAX_PARAGRAPH_OPENING_KEY_BYTES: usize = 4 * 1024 * 1024;
const MAX_PARAGRAPH_OPENING_BYTES: usize = 4_096;
const MAX_DUPLICATE_PARAGRAPHS: usize = 100_000;
const MAX_DUPLICATE_PARAGRAPH_BYTES: usize = 16 * 1024;
const MAX_DUPLICATE_PARAGRAPH_KEYS: usize = 10_000;
const MAX_DUPLICATE_PARAGRAPH_KEY_BYTES: usize = 4 * 1024 * 1024;

fn is_false(value: &bool) -> bool {
    !value
}

// Preserve accents while ignoring outer punctuation. Canonically equivalent
// Unicode spellings must have the same key; ASCII avoids normalization overhead.
fn opening_word_key(word: &str) -> String {
    let word = word.trim_matches(|ch: char| {
        !ch.is_alphanumeric() && (ch.is_ascii() || !is_combining_mark(ch))
    });
    if word.is_ascii() {
        word.to_ascii_lowercase()
    } else if word.chars().all(is_combining_mark) {
        String::new()
    } else {
        word.nfc().flat_map(char::to_lowercase).nfc().collect()
    }
}

// Visit exactly the same heuristic sentence boundaries for every sentence gate.
// Positions refer to the projected adapter text, not necessarily the source file.
// Restrict short-sentence certainty to scripts with whitespace word boundaries
// that we explicitly recognize. Other scripts stay not_checked.
fn is_simple_latin_char(ch: char) -> bool {
    ch.is_ascii()
        || ch.is_whitespace()
        || matches!(ch, '«' | '»' | '“' | '”' | '‘' | '’' | '…' | '–' | '—')
        || (ch.is_alphabetic() && matches!(ch, '\u{00c0}'..='\u{024f}' | '\u{1e00}'..='\u{1eff}'))
        || matches!(ch, '\u{0300}'..='\u{036f}')
}

fn is_simple_latin_sentence(text: &str) -> bool {
    text.chars().all(is_simple_latin_char)
}

// This is an explicit whitespace-token heuristic, not Arabic morphology or
// language detection. Unsupported characters keep affected windows unchecked.
fn has_supported_word_boundaries(text: &str, allow_arabic: bool) -> bool {
    text.chars().all(|ch| {
        is_simple_latin_char(ch)
            || (allow_arabic
                && ((matches!(ch, '\u{0600}'..='\u{06ff}' | '\u{0750}'..='\u{077f}' | '\u{08a0}'..='\u{08ff}' | '\u{fb50}'..='\u{fdff}' | '\u{fe70}'..='\u{feff}')
                    && (ch.is_alphanumeric() || is_combining_mark(ch)))
                    || matches!(ch, '،' | '؛' | '؟')))
    })
}

// Sentence slices originate in `text`; use their byte offset so quote state also
// includes delimiters and closing marks skipped by sentence segmentation.
fn sentence_is_quoted(
    tracker: &mut crate::quote_tracking::QuoteTracker<'_>,
    text: &str,
    segment: &str,
) -> bool {
    let segment = segment.trim_start();
    let offset = segment.as_ptr() as usize - text.as_ptr() as usize;
    tracker.quoted_at(offset)
        || segment
            .chars()
            .any(|ch| matches!(ch, '"' | '«' | '»' | '“' | '”' | '‘'))
}

/// Whether a fragment identifies a supported reference target. This only
/// recognizes target conventions; it does not establish that the source exists.
pub fn numeric_reference_destination(href: &str) -> bool {
    ["#reference-", "#cite_note-"].iter().any(|prefix| {
        href.strip_prefix(prefix)
            .is_some_and(|suffix| !suffix.is_empty())
    })
}

fn supported_decimal_digit(ch: char) -> bool {
    ch.is_ascii_digit() || matches!(ch, '\u{0660}'..='\u{0669}' | '\u{06f0}'..='\u{06f9}')
}

fn reference_label_character(ch: char) -> bool {
    supported_decimal_digit(ch)
        || matches!(
            ch,
            '[' | ']' | ',' | '،' | '-' | '–' | ' ' | '\u{00a0}' | '\u{202f}'
        )
}

/// Validate one contiguous fragment of a reference label. `Some` indicates
/// supported characters; its value records whether the fragment contains a digit.
/// A complete label must contain a digit across its fragments.
pub fn numeric_reference_label_fragment(text: &str) -> Option<bool> {
    let mut has_digit = false;
    for ch in text.chars() {
        if !reference_label_character(ch) {
            return None;
        }
        has_digit |= supported_decimal_digit(ch);
    }
    Some(has_digit)
}

/// Whether an explicitly linked reference marker can be excluded from prose.
pub(crate) fn numeric_reference_marker(href: &str, text: &str) -> bool {
    numeric_reference_destination(href) && numeric_reference_label_fragment(text) == Some(true)
}

// Recognize explicit HTTP(S) URLs without changing their text or crossing an
// adapter block. Bounded scanning avoids repeatedly parsing unbounded tokens.
fn http_url_end(
    text: &str,
    start: usize,
    block_end: Option<usize>,
    starts_segment: bool,
    delimiters: &str,
) -> Option<usize> {
    const MAX_URL_BYTES: usize = 2048;
    let tail = text.get(start..)?;
    if !tail
        .get(..8)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("https://"))
        && !tail
            .get(..7)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("http://"))
    {
        return None;
    }
    if start > 0
        && !starts_segment
        && !text[..start].chars().next_back().is_some_and(|c| {
            c.is_whitespace()
                || matches!(
                    c,
                    '(' | '['
                        | '{'
                        | '«'
                        | '“'
                        | '\''
                        | '"'
                        | ':'
                        | ';'
                        | ','
                        | '—'
                        | '–'
                        | '،'
                        | '：'
                )
        })
    {
        return None;
    }
    let mut end = start;
    let authority_start = if tail
        .get(..8)
        .is_some_and(|s| s.eq_ignore_ascii_case("https://"))
    {
        start + 8
    } else {
        start + 7
    };
    let mut after_authority = false;
    for (relative, ch) in tail.char_indices() {
        let offset = start + relative;
        if offset >= authority_start && matches!(ch, '/' | '?' | '#') {
            after_authority = true;
        }
        // An ideographic stop can also be an IDNA domain separator. Do not
        // split the authority; only treat configured stops after it as prose.
        if block_end.is_some_and(|boundary| offset >= boundary)
            || ch.is_whitespace()
            || (after_authority
                && matches!(ch, '。' | '！' | '？' | '؟')
                && delimiters.contains(ch))
        {
            break;
        }
        if relative + ch.len_utf8() > MAX_URL_BYTES {
            return None;
        }
        end = offset + ch.len_utf8();
    }
    let candidate = text[start..end].trim_end_matches(|ch| {
        matches!(
            ch,
            '.' | ',' | ';' | ':' | '!' | '?' | ')' | ']' | '}' | '»' | '”' | '\'' | '"'
        )
    });
    let parsed = url::Url::parse(candidate).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return None;
    }
    Some(start + candidate.len())
}

// Only match a configured abbreviation at the beginning of a token. Opening
// punctuation can attach to the token without changing the abbreviation.
fn abbreviation_starts_at(text: &str, start: usize, segment_start: usize) -> bool {
    start == segment_start
        || text[..start].chars().next_back().is_some_and(|ch| {
            ch.is_whitespace() || matches!(ch, '(' | '[' | '{' | '«' | '“' | '‘' | '\'' | '"')
        })
}

fn for_each_sentence(
    text: &str,
    mut line: usize,
    mut column: usize,
    block_ends: &[usize],
    delimiters: &str,
    abbreviations: &[String],
    mut visit: impl FnMut(&str, usize, usize),
) {
    let mut segment_start = 0;
    let mut first_position = None;
    let mut next_block = 0;
    let mut after_delimiter = false;
    let mut adjacent_to_delimiter = false;
    let mut protected_url_end = 0;
    for (offset, ch) in text
        .char_indices()
        .map(|(offset, ch)| (offset, Some(ch)))
        .chain(std::iter::once((text.len(), None)))
    {
        while block_ends.get(next_block).is_some_and(|&end| end < offset) {
            next_block += 1;
        }
        let block_end = block_ends.get(next_block) == Some(&offset);
        if after_delimiter && !block_end {
            if ch.is_some_and(char::is_whitespace) {
                adjacent_to_delimiter = false;
            } else if let Some(c) = ch.filter(|c| {
                matches!(c, '»' | '”' | '’' | ')' | ']' | '}')
                    || (adjacent_to_delimiter && matches!(c, '\'' | '"'))
            }) {
                segment_start = offset + c.len_utf8();
                first_position = None;
                column += 1;
                continue;
            } else {
                after_delimiter = false;
            }
        }
        if matches!(ch, Some('h' | 'H')) && offset >= protected_url_end {
            let following_block = block_ends[next_block..]
                .iter()
                .copied()
                .find(|&end| end > offset);
            if let Some(end) = http_url_end(
                text,
                offset,
                following_block,
                block_end || offset == segment_start,
                delimiters,
            ) {
                protected_url_end = end;
            }
        }
        let in_url = offset < protected_url_end;
        let decimal_point = !block_end
            && ch == Some('.')
            && text[..offset]
                .chars()
                .next_back()
                .is_some_and(supported_decimal_digit)
            && text[offset + 1..]
                .chars()
                .next()
                .is_some_and(supported_decimal_digit);
        let abbreviation = !block_end
            && ch == Some('.')
            && !abbreviations.is_empty()
            && abbreviations.iter().any(|abbr| {
                let Some(word) = abbr.strip_suffix('.') else {
                    return false;
                };
                // Preserve the existing single-period behavior. Multi-period
                // tokens need a complete look-ahead so an internal dot cannot
                // split the abbreviation, including across adapter blocks.
                if !word.contains('.') {
                    let Some(start) = offset.checked_sub(word.len()) else {
                        return false;
                    };
                    return start >= segment_start
                        && text.get(start..offset) == Some(word)
                        && abbreviation_starts_at(text, start, segment_start);
                }
                abbr.match_indices('.').any(|(position, _)| {
                    let Some(start) = offset.checked_sub(position) else {
                        return false;
                    };
                    let Some(end) = start.checked_add(abbr.len()) else {
                        return false;
                    };
                    start >= segment_start
                        && text.get(start..end) == Some(abbr.as_str())
                        && abbreviation_starts_at(text, start, segment_start)
                        && block_ends
                            .get(next_block)
                            .is_none_or(|&boundary| boundary >= end)
                        && text
                            .get(end..)
                            .and_then(|suffix| suffix.chars().next())
                            .is_none_or(|ch| {
                                ch.is_whitespace()
                                    || matches!(
                                        ch,
                                        ',' | ';'
                                            | ':'
                                            | '!'
                                            | '?'
                                            | ')'
                                            | ']'
                                            | '}'
                                            | '»'
                                            | '”'
                                            | '’'
                                            | '\''
                                            | '"'
                                    )
                            })
                })
            });
        let delimiter = !in_url
            && !decimal_point
            && !abbreviation
            && ch.is_none_or(|ch| delimiters.contains(ch));
        if block_end || delimiter {
            after_delimiter = delimiter && ch.is_some();
            adjacent_to_delimiter = after_delimiter;
            let (start_line, start_column) = first_position.unwrap_or((line, column));
            visit(&text[segment_start..offset], start_line, start_column);
            segment_start = offset
                + if delimiter {
                    ch.map_or(0, char::len_utf8)
                } else {
                    0
                };
            first_position = if !delimiter && ch.is_some_and(|ch| !ch.is_whitespace()) {
                Some((line, column))
            } else {
                None
            };
        } else if ch.is_some_and(|ch| !ch.is_whitespace()) && first_position.is_none() {
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

fn default_min_paragraph_chars() -> usize {
    1
}

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
    /// Check a supplied structured snapshot against the exact content source.
    EditorialPacing {
        id: String,
        #[serde(default)]
        severity: Severity,
        #[serde(default)]
        blocking: bool,
        source: String,
        profile: crate::editorial_pacing::CompositionProfileRef,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        projection_separator: Option<String>,
    },
    /// Advisory review of explicit Markdown links embedded as literal rich text.
    LiteralMarkdownLinks {
        id: String,
        #[serde(default)]
        severity: Severity,
        #[serde(default)]
        blocking: bool,
    },
    /// Advisory review of structured lists with configured item counts.
    ListItemCount {
        id: String,
        #[serde(default)]
        severity: Severity,
        #[serde(default)]
        blocking: bool,
        flagged_counts: Vec<usize>,
    },
    WordCount {
        id: String,
        #[serde(default)]
        severity: Severity,
        #[serde(default = "default_blocking")]
        blocking: bool,
        min: Option<usize>,
        max: Option<usize>,
    },
    /// Bounds Unicode scalar values in the adapter's projected text.
    CharacterCount {
        id: String,
        #[serde(default)]
        severity: Severity,
        #[serde(default = "default_blocking")]
        blocking: bool,
        min: Option<usize>,
        max: Option<usize>,
        #[serde(default)]
        ignore_whitespace: bool,
    },
    /// Bounds Unicode scalar values in one observed metadata field.
    MetadataLength {
        id: String,
        field: String,
        #[serde(default)]
        severity: Severity,
        #[serde(default = "default_blocking")]
        blocking: bool,
        min: Option<usize>,
        max: Option<usize>,
    },
    /// Advisory review when a supplied date is older than a fixed reference date.
    MetadataAge {
        id: String,
        field: String,
        as_of: String,
        max_days: usize,
        #[serde(default)]
        severity: Severity,
        #[serde(default)]
        blocking: bool,
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
    /// Flags consecutive sentences with the same literal opening within a paragraph.
    RepeatedSentenceOpenings {
        id: String,
        #[serde(default)]
        severity: Severity,
        #[serde(default)]
        blocking: bool,
        opening_words: usize,
        max_consecutive: usize,
        /// Opt in to whitespace-token counting for recognized Arabic script characters.
        #[serde(default, skip_serializing_if = "is_false")]
        allow_arabic_word_boundaries: bool,
        #[serde(default = "default_sentence_delimiters")]
        delimiters: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        abbreviations: Vec<String>,
    },
    /// Reviews repeated openings in a bounded sliding sentence window.
    SentenceOpeningWindow {
        id: String,
        #[serde(default)]
        severity: Severity,
        #[serde(default)]
        blocking: bool,
        opening_words: usize,
        window_size: usize,
        max_occurrences: usize,
        #[serde(default, skip_serializing_if = "is_false")]
        window_across_paragraphs: bool,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        ignored_words: Vec<String>,
        /// Opt in to whitespace-token counting for recognized Arabic script characters.
        #[serde(default, skip_serializing_if = "is_false")]
        allow_arabic_word_boundaries: bool,
        #[serde(default = "default_sentence_delimiters")]
        delimiters: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        abbreviations: Vec<String>,
    },
    /// Reviews configured connective phrases at sentence openings.
    SentenceOpeningTerms {
        id: String,
        #[serde(default)]
        severity: Severity,
        #[serde(default)]
        blocking: bool,
        terms: Vec<String>,
        /// Match an opening only when its configured text is followed by a colon.
        #[serde(default, skip_serializing_if = "is_false")]
        require_colon: bool,
        #[serde(default)]
        max_per_paragraph: usize,
        #[serde(default = "default_sentence_delimiters")]
        delimiters: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        abbreviations: Vec<String>,
    },
    /// Reviews repeated occurrences of one punctuation character per paragraph.
    RepeatedPunctuation {
        id: String,
        #[serde(default)]
        severity: Severity,
        #[serde(default)]
        blocking: bool,
        mark: char,
        max_per_paragraph: usize,
    },
    /// Reviews English sentence tails that resemble comma + an -ing word.
    TrailingIngClause {
        id: String,
        #[serde(default)]
        severity: Severity,
        #[serde(default)]
        blocking: bool,
        #[serde(default = "default_sentence_delimiters")]
        delimiters: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        abbreviations: Vec<String>,
    },
    /// Reviews runs of sentences with similar whitespace-separated word counts.
    SimilarSentenceLengths {
        id: String,
        #[serde(default)]
        severity: Severity,
        #[serde(default)]
        blocking: bool,
        run_length: usize,
        tolerance_words: usize,
        /// Minimum adjacent length difference as a percentage of the longer sentence.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min_difference_percent: Option<usize>,
        /// Inclusive bounds for reviewing adjacent sentences within a configured range.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        range_min_words: Option<usize>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        range_max_words: Option<usize>,
        /// Opt in to whitespace-token counting for recognized Arabic script characters.
        #[serde(default, skip_serializing_if = "is_false")]
        allow_arabic_word_boundaries: bool,
        #[serde(default = "default_sentence_delimiters")]
        delimiters: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        abbreviations: Vec<String>,
    },
    /// Reviews low population standard deviation in rolling sentence windows.
    SentenceLengthVariance {
        id: String,
        #[serde(default)]
        severity: Severity,
        #[serde(default)]
        blocking: bool,
        window_size: usize,
        /// Restrict each rolling window to one observed paragraph.
        #[serde(default, skip_serializing_if = "is_false")]
        reset_at_paragraph: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min_stddev_words: Option<usize>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        random_min_stddev_words: Option<usize>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        random_max_stddev_words: Option<usize>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        random_seed: Option<u64>,
        /// Opt in to whitespace-token counting for recognized Arabic script characters.
        #[serde(default, skip_serializing_if = "is_false")]
        allow_arabic_word_boundaries: bool,
        #[serde(default = "default_sentence_delimiters")]
        delimiters: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        abbreviations: Vec<String>,
    },
    /// Reviews sentence variance in classified prose, excluding list paragraphs.
    ProseSentenceLengthVariance {
        id: String,
        #[serde(default)]
        severity: Severity,
        #[serde(default)]
        blocking: bool,
        window_size: usize,
        /// Restrict each rolling window to one observed paragraph.
        #[serde(default, skip_serializing_if = "is_false")]
        reset_at_paragraph: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min_stddev_words: Option<usize>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        random_min_stddev_words: Option<usize>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        random_max_stddev_words: Option<usize>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        random_seed: Option<u64>,
        /// Opt in to whitespace-token counting for recognized Arabic script characters.
        #[serde(default, skip_serializing_if = "is_false")]
        allow_arabic_word_boundaries: bool,
        #[serde(default = "default_sentence_delimiters")]
        delimiters: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        abbreviations: Vec<String>,
    },
    /// Flags surface-form candidates for three coordinated inline items.
    InlineTripleCandidates {
        id: String,
        #[serde(default)]
        severity: Severity,
        #[serde(default)]
        blocking: bool,
        conjunction: String,
        /// Opt in to reviewing enumerations of at least this many items.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min_items: Option<usize>,
        /// Separator used between items; defaults to a comma.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        item_separator: Option<char>,
        #[serde(default = "default_sentence_delimiters")]
        delimiters: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        abbreviations: Vec<String>,
    },
    /// Finds exact repeated paragraphs after whitespace normalization.
    DuplicateParagraphs {
        id: String,
        #[serde(default)]
        severity: Severity,
        #[serde(default)]
        blocking: bool,
        max_occurrences: usize,
        #[serde(default = "default_min_paragraph_chars")]
        min_chars: usize,
        #[serde(default)]
        case_insensitive: bool,
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
        /// Withhold word counts outside the conservative Latin character subset.
        #[serde(default, skip_serializing_if = "is_false")]
        require_latin_word_boundaries: bool,
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
    /// Require every configured literal term in the adapter-supplied text.
    RequiredTerms {
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
    ForbiddenTerms {
        id: String,
        #[serde(default)]
        severity: Severity,
        #[serde(default = "default_blocking")]
        blocking: bool,
        /// Present matches as candidates for contextual review.
        #[serde(default, skip_serializing_if = "is_false")]
        review_only: bool,
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
    /// Report skipped heading levels in a complete ordered heading sequence.
    HeadingHierarchy {
        id: String,
        #[serde(default)]
        severity: Severity,
        #[serde(default = "default_blocking")]
        blocking: bool,
        /// Optional preceding heading level supplied by the surrounding template.
        #[serde(default)]
        initial_level: Option<u8>,
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
    /// Review the age of a claim-specific source assessment against a fixed date.
    EvidenceReviewAge {
        id: String,
        source: String,
        review: String,
        claim: String,
        as_of: String,
        max_days: usize,
        #[serde(default)]
        severity: Severity,
        #[serde(default)]
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
            Self::EditorialPacing { severity, .. }
            | Self::ListItemCount { severity, .. }
            | Self::LiteralMarkdownLinks { severity, .. }
            | Self::WordCount { severity, .. }
            | Self::CharacterCount { severity, .. }
            | Self::MetadataLength { severity, .. }
            | Self::MetadataAge { severity, .. }
            | Self::LinkCount { severity, .. }
            | Self::LinkDestinations { severity, .. }
            | Self::LinkReachability { severity, .. }
            | Self::SentenceLength { severity, .. }
            | Self::SentenceCharacterLength { severity, .. }
            | Self::ParagraphCount { severity, .. }
            | Self::ParagraphLength { severity, .. }
            | Self::RepeatedParagraphOpenings { severity, .. }
            | Self::RepeatedSentenceOpenings { severity, .. }
            | Self::SentenceOpeningWindow { severity, .. }
            | Self::SentenceOpeningTerms { severity, .. }
            | Self::RepeatedPunctuation { severity, .. }
            | Self::TrailingIngClause { severity, .. }
            | Self::SimilarSentenceLengths { severity, .. }
            | Self::SentenceLengthVariance { severity, .. }
            | Self::ProseSentenceLengthVariance { severity, .. }
            | Self::InlineTripleCandidates { severity, .. }
            | Self::DuplicateParagraphs { severity, .. }
            | Self::ParagraphCharacterLength { severity, .. }
            | Self::RepeatedTerms { severity, .. }
            | Self::RequiredTerms { severity, .. }
            | Self::ForbiddenTerms { severity, .. }
            | Self::RequiredMetadata { severity, .. }
            | Self::RequiredHeadings { severity, .. }
            | Self::HeadingHierarchy { severity, .. }
            | Self::DuplicateHeadings { severity, .. }
            | Self::HeadingLength { severity, .. }
            | Self::FinalNewline { severity, .. }
            | Self::UniqueCollectionMetadata { severity, .. }
            | Self::Custom { severity, .. }
            | Self::EvidenceAnchors { severity, .. }
            | Self::StructuredFact { severity, .. }
            | Self::EvidenceReview { severity, .. }
            | Self::EvidenceReviewAge { severity, .. } => *severity,
            Self::SiteAudit { .. } | Self::SiteGroup { .. } => Severity::Warning,
        }
    }
    pub fn blocking(&self) -> bool {
        match self {
            Self::EditorialPacing { blocking, .. }
            | Self::ListItemCount { blocking, .. }
            | Self::LiteralMarkdownLinks { blocking, .. }
            | Self::WordCount { blocking, .. }
            | Self::CharacterCount { blocking, .. }
            | Self::MetadataLength { blocking, .. }
            | Self::MetadataAge { blocking, .. }
            | Self::LinkCount { blocking, .. }
            | Self::LinkDestinations { blocking, .. }
            | Self::LinkReachability { blocking, .. }
            | Self::SentenceLength { blocking, .. }
            | Self::SentenceCharacterLength { blocking, .. }
            | Self::ParagraphCount { blocking, .. }
            | Self::ParagraphLength { blocking, .. }
            | Self::RepeatedParagraphOpenings { blocking, .. }
            | Self::RepeatedSentenceOpenings { blocking, .. }
            | Self::SentenceOpeningWindow { blocking, .. }
            | Self::SentenceOpeningTerms { blocking, .. }
            | Self::RepeatedPunctuation { blocking, .. }
            | Self::TrailingIngClause { blocking, .. }
            | Self::SimilarSentenceLengths { blocking, .. }
            | Self::SentenceLengthVariance { blocking, .. }
            | Self::ProseSentenceLengthVariance { blocking, .. }
            | Self::InlineTripleCandidates { blocking, .. }
            | Self::DuplicateParagraphs { blocking, .. }
            | Self::ParagraphCharacterLength { blocking, .. }
            | Self::RepeatedTerms { blocking, .. }
            | Self::RequiredTerms { blocking, .. }
            | Self::ForbiddenTerms { blocking, .. }
            | Self::RequiredMetadata { blocking, .. }
            | Self::RequiredHeadings { blocking, .. }
            | Self::HeadingHierarchy { blocking, .. }
            | Self::DuplicateHeadings { blocking, .. }
            | Self::HeadingLength { blocking, .. }
            | Self::FinalNewline { blocking, .. }
            | Self::UniqueCollectionMetadata { blocking, .. }
            | Self::Custom { blocking, .. }
            | Self::EvidenceAnchors { blocking, .. }
            | Self::StructuredFact { blocking, .. }
            | Self::EvidenceReview { blocking, .. }
            | Self::EvidenceReviewAge { blocking, .. }
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
            | Self::CharacterCount { .. }
            | Self::SentenceLength { .. }
            | Self::SentenceCharacterLength { .. }
            | Self::RepeatedTerms { .. }
            | Self::RequiredTerms { .. }
            | Self::ForbiddenTerms { .. } => "text",
            Self::ListItemCount { .. } => "structured_lists",
            Self::LiteralMarkdownLinks { .. } => "literal_markdown_links",
            Self::LinkCount { .. } => "links",
            Self::LinkDestinations { .. } => "link_destinations",
            Self::LinkReachability { .. } => "link_reachability",
            Self::RequiredMetadata { .. }
            | Self::MetadataLength { .. }
            | Self::MetadataAge { .. } => "metadata",
            Self::ParagraphCount { .. }
            | Self::ParagraphLength { .. }
            | Self::ParagraphCharacterLength { .. }
            | Self::RepeatedParagraphOpenings { .. }
            | Self::RepeatedSentenceOpenings { .. }
            | Self::SentenceOpeningWindow { .. }
            | Self::SentenceOpeningTerms { .. }
            | Self::RepeatedPunctuation { .. }
            | Self::TrailingIngClause { .. }
            | Self::SimilarSentenceLengths { .. }
            | Self::SentenceLengthVariance { .. }
            | Self::InlineTripleCandidates { .. }
            | Self::DuplicateParagraphs { .. } => "paragraphs",
            Self::ProseSentenceLengthVariance { .. } => "classified_paragraphs",
            Self::HeadingHierarchy { .. } => "heading_levels",
            Self::RequiredHeadings { .. }
            | Self::DuplicateHeadings { .. }
            | Self::HeadingLength { .. } => "headings",
            Self::FinalNewline { .. } => "source_text",
            Self::UniqueCollectionMetadata { .. } => "collection_metadata",
            Self::Custom { .. } => "extension",
            Self::EditorialPacing { .. }
            | Self::EvidenceAnchors { .. }
            | Self::StructuredFact { .. }
            | Self::EvidenceReview { .. }
            | Self::EvidenceReviewAge { .. } => "text_and_evidence",
            Self::SiteAudit { .. } | Self::SiteGroup { .. } => "site",
        }
    }
    pub fn id(&self) -> &str {
        match self {
            Self::EditorialPacing { id, .. }
            | Self::ListItemCount { id, .. }
            | Self::LiteralMarkdownLinks { id, .. }
            | Self::WordCount { id, .. }
            | Self::CharacterCount { id, .. }
            | Self::MetadataLength { id, .. }
            | Self::MetadataAge { id, .. }
            | Self::LinkCount { id, .. }
            | Self::LinkDestinations { id, .. }
            | Self::LinkReachability { id, .. }
            | Self::SentenceLength { id, .. }
            | Self::SentenceCharacterLength { id, .. }
            | Self::ParagraphCount { id, .. }
            | Self::ParagraphLength { id, .. }
            | Self::RepeatedParagraphOpenings { id, .. }
            | Self::RepeatedSentenceOpenings { id, .. }
            | Self::SentenceOpeningWindow { id, .. }
            | Self::SentenceOpeningTerms { id, .. }
            | Self::RepeatedPunctuation { id, .. }
            | Self::TrailingIngClause { id, .. }
            | Self::SimilarSentenceLengths { id, .. }
            | Self::SentenceLengthVariance { id, .. }
            | Self::ProseSentenceLengthVariance { id, .. }
            | Self::InlineTripleCandidates { id, .. }
            | Self::DuplicateParagraphs { id, .. }
            | Self::ParagraphCharacterLength { id, .. }
            | Self::RepeatedTerms { id, .. }
            | Self::RequiredTerms { id, .. }
            | Self::ForbiddenTerms { id, .. }
            | Self::RequiredMetadata { id, .. }
            | Self::RequiredHeadings { id, .. }
            | Self::HeadingHierarchy { id, .. }
            | Self::DuplicateHeadings { id, .. }
            | Self::HeadingLength { id, .. }
            | Self::FinalNewline { id, .. }
            | Self::UniqueCollectionMetadata { id, .. }
            | Self::Custom { id, .. }
            | Self::EvidenceAnchors { id, .. }
            | Self::StructuredFact { id, .. }
            | Self::EvidenceReview { id, .. }
            | Self::EvidenceReviewAge { id, .. }
            | Self::SiteAudit { id, .. }
            | Self::SiteGroup { id, .. } => id,
        }
    }
}

// Guidance is advisory: thresholds and literal matches do not establish editorial quality.
fn editorial_suggestion(gate: &ContentGate) -> Option<&'static str> {
    match gate {
        ContentGate::LiteralMarkdownLinks { .. } => Some(
            "Check the host rendering; if the markup displays literally, use a native rich-text link without changing its destination.",
        ),
        ContentGate::ListItemCount { .. } => Some(
            "Review the list in context; preserve every necessary item and change the structure only when it improves the content.",
        ),
        ContentGate::WordCount { .. }
        | ContentGate::CharacterCount { .. }
        | ContentGate::ParagraphCount { .. } => Some(
            "Review the content length against the configured limits; adjust the content or the policy where appropriate.",
        ),
        ContentGate::ParagraphLength { .. } | ContentGate::ParagraphCharacterLength { .. } => Some(
            "Review the paragraph for opportunities to split or shorten it without losing context.",
        ),
        ContentGate::SentenceLength { .. } | ContentGate::SentenceCharacterLength { .. } => Some(
            "Review the sentence boundary and consider shortening the sentence while preserving its meaning.",
        ),
        ContentGate::TrailingIngClause { .. } => {
            Some("Review the clause in context; preserve its facts and any causal relationship.")
        }
        ContentGate::RepeatedPunctuation { .. } => Some(
            "Review punctuation in context; retain it when it serves the author's meaning or voice.",
        ),
        ContentGate::SentenceOpeningTerms { .. } => Some(
            "Review the connective in context; retain it when it conveys a meaningful relationship.",
        ),
        ContentGate::RepeatedParagraphOpenings { .. }
        | ContentGate::RepeatedSentenceOpenings { .. }
        | ContentGate::SentenceOpeningWindow { .. } => {
            Some("Review the repeated openings and vary them where repetition is unintended.")
        }
        ContentGate::DuplicateParagraphs { .. } => Some(
            "Review the repeated paragraphs and remove or revise duplication where it is unintended.",
        ),
        ContentGate::RepeatedTerms { .. } => Some(
            "Review the repeated term in context; keep necessary technical or factual repetition.",
        ),
        ContentGate::RequiredTerms { .. } => Some(
            "Check whether the required term belongs in this content or the configured policy needs adjustment.",
        ),
        ContentGate::ForbiddenTerms {
            review_only: true, ..
        } => Some(
            "Review this phrase in context; keep it when it fits the meaning and intended voice.",
        ),
        ContentGate::ForbiddenTerms { .. } => Some(
            "Review the matched term in context and replace it if the configured policy applies.",
        ),
        ContentGate::RequiredHeadings { .. } => Some(
            "Check whether the missing heading belongs in this content or the configured policy needs adjustment.",
        ),
        ContentGate::HeadingHierarchy { .. } => {
            Some("Review the heading levels and nesting in the context of the surrounding page.")
        }
        ContentGate::DuplicateHeadings { .. } => Some(
            "Review the repeated heading and distinguish it where the repetition is unintended.",
        ),
        ContentGate::HeadingLength { .. } => {
            Some("Review the heading and consider shortening it without losing meaning.")
        }
        ContentGate::LinkCount { .. } => Some(
            "Review whether relevant links should be added or the configured minimum adjusted.",
        ),
        ContentGate::LinkDestinations { .. } => {
            Some("Review the destination against the configured URL and scheme rules.")
        }
        ContentGate::LinkReachability { .. } => Some(
            "Recheck the destination and update or remove the link if it is no longer appropriate.",
        ),
        ContentGate::RequiredMetadata { .. } => {
            Some("Provide the required metadata if applicable, or update the configured policy.")
        }
        ContentGate::MetadataLength { .. } => {
            Some("Review the metadata length against the configured limits without losing meaning.")
        }
        ContentGate::MetadataAge { .. } | ContentGate::EvidenceReviewAge { .. } => Some(
            "Review the content and its sources; the supplied date alone does not establish accuracy or currency.",
        ),
        ContentGate::FinalNewline { .. } => Some("Add a final newline to the source text."),
        _ => None,
    }
}

/// Return offending heading indices for heading gates, in finding order.
/// Other gate types return `None`.
/// Indices whose level skips at least one step from the preceding heading.
pub fn heading_hierarchy_indices(
    levels: &[u8],
    initial_level: Option<u8>,
) -> impl Iterator<Item = usize> + '_ {
    levels
        .iter()
        .copied()
        .enumerate()
        .scan(initial_level, |previous, (index, level)| {
            let skipped = previous.is_some_and(|prior| level > prior.saturating_add(1));
            *previous = Some(level);
            Some(skipped.then_some(index))
        })
        .flatten()
}

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

// URL parsers may repair backslashes or preserve malformed percent escapes.
// Flag these explicitly rather than silently accepting a different destination.
fn link_encoding_problem(destination: &str) -> Option<&'static str> {
    let bytes = destination.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => return Some("contains a backslash"),
            b'%' => {
                if index + 2 >= bytes.len()
                    || !bytes[index + 1].is_ascii_hexdigit()
                    || !bytes[index + 2].is_ascii_hexdigit()
                {
                    return Some("contains an invalid percent escape");
                }
                index += 2;
            }
            _ => {}
        }
        index += 1;
    }
    None
}

/// Check a destination against a configured offline scheme policy.
pub fn valid_link_destination(destination: &str, schemes: &[String], allow_relative: bool) -> bool {
    if destination.is_empty()
        || destination.chars().any(char::is_whitespace)
        || destination.chars().any(char::is_control)
        || link_encoding_problem(destination).is_some()
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
    if let Some(problem) = link_encoding_problem(destination) {
        return format!("{preview:?} {problem}");
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
    /// the visitor returns true. Word boundaries use Unicode alphanumeric and identifier-continuation
    /// characters, outside scripts written without spaces.
    fn visit_matches(
        &self,
        term: &str,
        whole_words: bool,
        mut visit: impl FnMut(usize) -> bool,
    ) -> bool {
        fn word(ch: char) -> bool {
            (ch.is_alphanumeric() || unicode_ident::is_xid_continue(ch)) && !unsegmented_script(ch)
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

// Cache only the most recent matching mode for this evaluation. This bounds the
// retained allocation to one projection and never shares content across inputs.
fn cached_literal<'a, 'cache>(
    cache: &'cache mut Option<LiteralText<'a>>,
    text: &'a str,
    block_ends: &'a [usize],
    case_insensitive: bool,
) -> &'cache LiteralText<'a> {
    if cache
        .as_ref()
        .is_some_and(|literal| literal.case_insensitive != case_insensitive)
    {
        *cache = None;
    }
    cache.get_or_insert_with(|| LiteralText::new(text, block_ends, case_insensitive))
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
    provenance: BTreeMap<String, String>,
}

impl EvidenceReviewRecord {
    fn parse(value: &str) -> Option<Self> {
        aexeo_contracts::content_gate_change_validation::parse_unique_json_value(value.as_bytes())
            .ok()
            .and_then(|value| serde_json::from_value(value).ok())
    }

    fn matches(&self, claim: &str, source: &str) -> bool {
        self.claim == claim && self.source_text == source && !self.reviewer.trim().is_empty()
    }
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
    /// Saturates for malformed or externally supplied counts that exceed `usize`.
    pub fn total_findings(&self) -> usize {
        self.findings.len().saturating_add(
            self.measurements
                .get(FINDINGS_OMITTED)
                .copied()
                .unwrap_or(0),
        )
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
                ContentGate::EditorialPacing {
                    source,
                    profile,
                    projection_separator,
                    ..
                } => {
                    if projection_separator
                        .as_ref()
                        .is_some_and(|separator| separator.len() > 1024)
                    {
                        bail!("projection separator is too long for {}", gate.id());
                    }
                    if source.trim().is_empty() {
                        bail!("composition source name is empty for {}", gate.id());
                    }
                    profile.resolve()?;
                }
                ContentGate::ListItemCount { flagged_counts, .. }
                    if flagged_counts.is_empty()
                        || flagged_counts.len() > 256
                        || flagged_counts.contains(&0)
                        || flagged_counts.iter().collect::<BTreeSet<_>>().len()
                            != flagged_counts.len() =>
                {
                    bail!(
                        "list item counts must contain 1 to 256 positive, unique values for {}",
                        gate.id()
                    )
                }
                ContentGate::WordCount { min, max, .. }
                    if min.is_none() && max.is_none()
                        || matches!((min, max), (Some(a), Some(b)) if a > b) =>
                {
                    bail!("invalid word count bounds for {}", gate.id())
                }
                ContentGate::CharacterCount { min, max, .. }
                    if min.is_none() && max.is_none()
                        || matches!((min, max), (Some(a), Some(b)) if a > b) =>
                {
                    bail!("invalid character count bounds for {}", gate.id())
                }
                ContentGate::MetadataAge { field, as_of, .. }
                    if field.trim().is_empty() || parse_iso_calendar_date(as_of).is_none() =>
                {
                    bail!(
                        "invalid metadata age field or reference date for {}",
                        gate.id()
                    )
                }
                ContentGate::MetadataLength {
                    field, min, max, ..
                } if field.trim().is_empty()
                    || min.is_none() && max.is_none()
                    || matches!((min, max), (Some(a), Some(b)) if a > b) =>
                {
                    bail!("invalid metadata length field or bounds for {}", gate.id())
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
                ContentGate::HeadingHierarchy {
                    initial_level: Some(level),
                    ..
                } if !(1..=6).contains(level) => {
                    bail!(
                        "initial heading level must be between 1 and 6 for {}",
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
                ContentGate::RepeatedSentenceOpenings {
                    opening_words,
                    max_consecutive,
                    delimiters,
                    ..
                } if *opening_words == 0
                    || *opening_words > 16
                    || *max_consecutive == 0
                    || delimiters.is_empty() =>
                {
                    bail!(
                        "sentence openings need 1 to 16 words, a positive consecutive limit and delimiters for {}",
                        gate.id()
                    )
                }
                ContentGate::SentenceOpeningWindow {
                    opening_words,
                    window_size,
                    max_occurrences,
                    ignored_words,
                    delimiters,
                    ..
                } if *opening_words == 0
                    || *opening_words > 16
                    || !(2..=64).contains(window_size)
                    || *max_occurrences == 0
                    || max_occurrences >= window_size
                    || delimiters.is_empty()
                    || ignored_words.len() > 64
                    || ignored_words.iter().any(|word| {
                        word.is_empty()
                            || word.len() > MAX_PARAGRAPH_OPENING_BYTES
                            || word.split_whitespace().count() != 1
                            || opening_word_key(word).is_empty()
                    })
                    || ignored_words
                        .iter()
                        .map(|word| opening_word_key(word))
                        .collect::<BTreeSet<_>>()
                        .len()
                        != ignored_words.len() =>
                {
                    bail!(
                        "sentence opening window needs 1 to 16 opening words, a window of 2 to 64, a positive occurrence limit smaller than the window, delimiters and unique valid ignored words for {}",
                        gate.id()
                    )
                }
                ContentGate::InlineTripleCandidates {
                    conjunction,
                    min_items,
                    item_separator,
                    delimiters,
                    ..
                } if min_items.is_some_and(|n| !(3..=8).contains(&n))
                    || item_separator.is_some_and(|c| {
                        c.is_alphanumeric() || c.is_whitespace() || delimiters.contains(c)
                    })
                    || conjunction.is_empty()
                    || conjunction.len() > 32
                    || !conjunction.chars().all(char::is_alphabetic)
                    || delimiters.is_empty() =>
                {
                    bail!(
                        "inline triple candidates need a bounded alphabetic conjunction and sentence delimiters for {}",
                        gate.id()
                    );
                }
                ContentGate::SentenceLengthVariance {
                    window_size,
                    min_stddev_words,
                    random_min_stddev_words,
                    random_max_stddev_words,
                    random_seed,
                    delimiters,
                    ..
                }
                | ContentGate::ProseSentenceLengthVariance {
                    window_size,
                    min_stddev_words,
                    random_min_stddev_words,
                    random_max_stddev_words,
                    random_seed,
                    delimiters,
                    ..
                } if !(2..=64).contains(window_size)
                    || delimiters.is_empty()
                    || !matches!(
                        (
                            min_stddev_words,
                            random_min_stddev_words,
                            random_max_stddev_words,
                            random_seed
                        ),
                        (Some(1..=1000), None, None, None)
                            | (None, Some(1..=1000), Some(1..=1000), Some(_))
                    )
                    || matches!((random_min_stddev_words, random_max_stddev_words), (Some(lo), Some(hi)) if lo > hi) =>
                {
                    bail!(
                        "sentence variance needs a window of 2 to 64, delimiters, and either a fixed threshold or a seeded range (1 to 1000) for {}",
                        gate.id()
                    );
                }
                ContentGate::SimilarSentenceLengths {
                    run_length,
                    tolerance_words,
                    min_difference_percent,
                    range_min_words,
                    range_max_words,
                    delimiters,
                    ..
                } if *run_length < 2
                    || *run_length > 64
                    || delimiters.is_empty()
                    || min_difference_percent.is_some_and(|percent| {
                        !(1..=100).contains(&percent) || *run_length != 2 || *tolerance_words != 0
                    })
                    || match (range_min_words, range_max_words) {
                        (None, None) => false,
                        (Some(lo), Some(hi)) => {
                            *lo == 0
                                || *hi > 1000
                                || lo > hi
                                || *run_length != 2
                                || *tolerance_words != 0
                                || min_difference_percent.is_some()
                        }
                        _ => true,
                    } =>
                {
                    bail!(
                        "sentence rhythm needs a run of 2 to 64 and delimiters; percentage and range modes are exclusive and require a run of 2 and tolerance 0 with valid bounds for {}",
                        gate.id()
                    );
                }
                ContentGate::TrailingIngClause { delimiters, .. } if delimiters.is_empty() => {
                    bail!("sentence delimiters must not be empty for {}", gate.id());
                }
                ContentGate::RepeatedPunctuation { mark, .. }
                    if mark.is_alphanumeric() || mark.is_whitespace() || mark.is_control() =>
                {
                    bail!(
                        "punctuation mark must be a non-whitespace symbol for {}",
                        gate.id()
                    );
                }
                ContentGate::SentenceOpeningTerms {
                    terms, delimiters, ..
                } if terms.is_empty()
                    || terms.len() > 128
                    || delimiters.is_empty()
                    || terms.iter().any(|term| {
                        term.trim() != term
                            || term.is_empty()
                            || term.len() > 256
                            || term.chars().any(char::is_control)
                    }) =>
                {
                    bail!(
                        "sentence opening terms need 1 to 128 nonempty bounded terms and delimiters for {}",
                        gate.id()
                    );
                }
                ContentGate::DuplicateParagraphs {
                    max_occurrences,
                    min_chars,
                    ..
                } if *max_occurrences == 0 || *min_chars == 0 => {
                    bail!(
                        "duplicate paragraphs need positive occurrence and character limits for {}",
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
                | ContentGate::RepeatedSentenceOpenings { abbreviations, .. }
                | ContentGate::SentenceOpeningWindow { abbreviations, .. }
                | ContentGate::SentenceOpeningTerms { abbreviations, .. }
                | ContentGate::TrailingIngClause { abbreviations, .. }
                | ContentGate::SimilarSentenceLengths { abbreviations, .. }
                | ContentGate::SentenceLengthVariance { abbreviations, .. }
                | ContentGate::ProseSentenceLengthVariance { abbreviations, .. }
                | ContentGate::InlineTripleCandidates { abbreviations, .. }
                    if abbreviations.iter().any(|abbr| {
                        abbr.strip_suffix('.').is_none_or(|word| {
                            word.is_empty()
                                || word.split('.').any(str::is_empty)
                                || word.chars().any(char::is_whitespace)
                                || (word.contains('.')
                                    && (abbr.len() > 128
                                        || word.bytes().filter(|&byte| byte == b'.').count() >= 16))
                        })
                    }) || abbreviations.iter().collect::<BTreeSet<_>>().len()
                        != abbreviations.len() =>
                {
                    bail!(
                        "sentence abbreviations must be unique single tokens ending in a period (multi-period: at most 128 bytes and 16 periods) for {}",
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
                ContentGate::RequiredTerms {
                    terms,
                    case_insensitive,
                    ..
                } if terms.is_empty()
                    || terms.len() > 256
                    || terms.iter().any(|term| term.trim().is_empty())
                    || terms
                        .iter()
                        .map(|term| fold_literal(term, *case_insensitive).0)
                        .collect::<BTreeSet<_>>()
                        .len()
                        != terms.len() =>
                {
                    bail!(
                        "required terms must contain 1 to 256 nonempty, distinct literals for {}",
                        gate.id()
                    )
                }
                ContentGate::ForbiddenTerms {
                    review_only: true,
                    blocking: true,
                    ..
                } => {
                    bail!("review-only terms must be non-blocking for {}", gate.id())
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
                ContentGate::EvidenceReviewAge {
                    source,
                    review,
                    claim,
                    as_of,
                    ..
                } if source.trim().is_empty()
                    || review.trim().is_empty()
                    || claim.trim().is_empty()
                    || source == review
                    || parse_iso_calendar_date(as_of).is_none() =>
                {
                    bail!(
                        "evidence review age needs distinct sources, a claim and valid reference date for {}",
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
        self.evaluate_with_observer_inner(
            input,
            capabilities,
            destinations,
            true,
            None,
            None,
            None,
            None,
            None,
        )
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
        self.evaluate_with_observer_inner(
            input,
            capabilities,
            destinations,
            true,
            Some(observer),
            None,
            None,
            None,
            None,
        )
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
        self.evaluate_with_observer_inner(
            input,
            capabilities,
            destinations,
            complete,
            observer,
            None,
            None,
            None,
            None,
        )
    }

    /// Evaluate a complete ordered sequence of heading levels supplied by the host.
    /// Levels must correspond one-to-one with `input.headings`. Omit them when
    /// any heading is unobserved; malformed or mismatched data is not checked.
    pub fn evaluate_with_heading_levels(
        &self,
        input: &ContentInput,
        capabilities: &ContentEvaluationContext<'_>,
        destinations: Option<&[String]>,
        heading_levels: Option<&[u8]>,
        collection_complete: bool,
        observer: Option<&mut dyn FnMut(GateObservation)>,
    ) -> Result<ContentReport> {
        self.evaluate_with_observer_inner(
            input,
            capabilities,
            destinations,
            collection_complete,
            observer,
            heading_levels,
            None,
            None,
            None,
        )
    }

    /// Evaluate with a complete list observation, or `None` when unavailable.
    #[allow(clippy::too_many_arguments)] // Preserve the existing evaluation capabilities in an additive API.
    pub fn evaluate_with_structured_lists(
        &self,
        input: &ContentInput,
        capabilities: &ContentEvaluationContext<'_>,
        destinations: Option<&[String]>,
        heading_levels: Option<&[u8]>,
        collection_complete: bool,
        observer: Option<&mut dyn FnMut(GateObservation)>,
        lists: Option<&[crate::markdown_adapter::StructuredList]>,
    ) -> Result<ContentReport> {
        self.evaluate_with_observer_inner(
            input,
            capabilities,
            destinations,
            collection_complete,
            observer,
            heading_levels,
            lists,
            None,
            None,
        )
    }

    /// Evaluate with complete, ordered paragraph list membership, or `None` when
    /// the adapter cannot establish that the body and its classifications are complete.
    #[allow(clippy::too_many_arguments)] // Additive API retains all existing observations.
    pub fn evaluate_with_paragraph_list_items(
        &self,
        input: &ContentInput,
        capabilities: &ContentEvaluationContext<'_>,
        destinations: Option<&[String]>,
        heading_levels: Option<&[u8]>,
        collection_complete: bool,
        observer: Option<&mut dyn FnMut(GateObservation)>,
        lists: Option<&[crate::markdown_adapter::StructuredList]>,
        paragraph_list_items: Option<&[bool]>,
    ) -> Result<ContentReport> {
        self.evaluate_with_observer_inner(
            input,
            capabilities,
            destinations,
            collection_complete,
            observer,
            heading_levels,
            lists,
            paragraph_list_items,
            None,
        )
    }

    /// Evaluate with an optional literal projection retaining numeric reference labels.
    /// The projection is checked against the text observed by the adapter.
    #[allow(clippy::too_many_arguments)]
    pub fn evaluate_with_literal_projection(
        &self,
        input: &ContentInput,
        capabilities: &ContentEvaluationContext<'_>,
        destinations: Option<&[String]>,
        heading_levels: Option<&[u8]>,
        collection_complete: bool,
        observer: Option<&mut dyn FnMut(GateObservation)>,
        lists: Option<&[crate::markdown_adapter::StructuredList]>,
        paragraph_list_items: Option<&[bool]>,
        literal_projection: Option<&str>,
    ) -> Result<ContentReport> {
        self.evaluate_with_observer_inner(
            input,
            capabilities,
            destinations,
            collection_complete,
            observer,
            heading_levels,
            lists,
            paragraph_list_items,
            literal_projection,
        )
    }

    #[allow(clippy::too_many_arguments)] // Internal continuation of the public evaluation APIs.
    fn evaluate_with_observer_inner(
        &self,
        input: &ContentInput,
        capabilities: &ContentEvaluationContext<'_>,
        destinations: Option<&[String]>,
        collection_complete: bool,
        mut observer: Option<&mut dyn FnMut(GateObservation)>,
        heading_levels: Option<&[u8]>,
        lists: Option<&[crate::markdown_adapter::StructuredList]>,
        paragraph_list_items: Option<&[bool]>,
        literal_projection: Option<&str>,
    ) -> Result<ContentReport> {
        let site = capabilities.site;
        let extensions = capabilities.extensions;
        let context = capabilities.adapter;
        let link_count = capabilities.link_count;
        let collection = capabilities.collection;
        self.validate()?;
        let mut results = Vec::with_capacity(self.gates.len());
        let mut word_count = None;
        let mut character_counts = [None, None];
        let mut literal_cache = None;
        // Validate the alternate projection only when literal or external gates can use it.
        let literal_input = match (input.text.as_deref(), literal_projection) {
            (Some(projected), Some(literal))
                if self.gates.iter().any(|gate| {
                    matches!(
                        gate,
                        ContentGate::RepeatedTerms { .. }
                            | ContentGate::RequiredTerms { .. }
                            | ContentGate::ForbiddenTerms { .. }
                            | ContentGate::Custom { .. }
                    )
                }) =>
            {
                if literal_projection_matches(projected, literal) {
                    Ok(Some(literal))
                } else {
                    Err(())
                }
            }
            (text, _) => Ok(text),
        };
        // External results bind to the exact text they evaluated. A masked
        // citation must not make two different revisions appear identical.
        let extension_input = if extensions.is_some()
            && literal_projection.is_some()
            && self
                .gates
                .iter()
                .any(|gate| matches!(gate, ContentGate::Custom { .. }))
        {
            match literal_input {
                Ok(Some(text)) => {
                    let mut exact = input.clone();
                    exact.text = Some(text.to_owned());
                    Some(exact)
                }
                _ => None,
            }
        } else {
            None
        };
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
                    suggestion: editorial_suggestion(gate).map(str::to_owned),
                    scope: aexeo_contracts::FindingScope::Page,
                    subject: None,
                })
            };
            match gate {
                ContentGate::LiteralMarkdownLinks { .. } => {
                    match context.and_then(|context| context.literal_markdown_links.as_ref()) {
                        Some(positions)
                            if positions.len() <= MAX_COLLECTION_ITEMS
                                && positions
                                    .iter()
                                    .all(|(line, column)| *line > 0 && *column > 0) =>
                        {
                            measurements.insert("candidates".into(), positions.len());
                            for &(line, column) in positions {
                                emit("Possible literal Markdown link in rich text; check how the host renders it".into(), line, column);
                            }
                        }
                        Some(_) => {
                            reason = Some(
                                "literal Markdown observations are invalid or exceed the limit"
                                    .into(),
                            )
                        }
                        None => {
                            reason = Some(
                                "literal rich-text structure is unavailable from this adapter"
                                    .into(),
                            )
                        }
                    }
                }
                ContentGate::ListItemCount { flagged_counts, .. } => match lists {
                    Some(lists)
                        if lists.len() <= MAX_COLLECTION_ITEMS
                            && lists.iter().all(|list| {
                                list.item_count > 0
                                    && (1..=100).contains(&list.depth)
                                    && list.line > 0
                                    && list.column > 0
                            }) =>
                    {
                        measurements.insert("lists_observed".into(), lists.len());
                        let mut matching = 0;
                        let mut largest = 0;
                        for (index, list) in lists.iter().enumerate() {
                            largest = largest.max(list.item_count);
                            if flagged_counts.contains(&list.item_count) {
                                matching += 1;
                                emit(
                                    format!(
                                        "List {} has {} direct items; review whether this structure suits the content",
                                        index + 1,
                                        list.item_count
                                    ),
                                    1,
                                    1,
                                );
                            }
                        }
                        measurements.insert("lists_matching_counts".into(), matching);
                        measurements.insert("largest_list_items".into(), largest);
                    }
                    Some(_) => {
                        reason = Some("list observations are invalid or exceed the limit".into())
                    }
                    None => {
                        reason = Some(
                            "complete structured lists are unavailable from this adapter".into(),
                        )
                    }
                },
                ContentGate::EditorialPacing {
                    source,
                    profile,
                    projection_separator,
                    ..
                } => {
                    let document = input.source_text.as_ref().or(input.text.as_ref());
                    let snapshot = input
                        .evidence_sources
                        .as_ref()
                        .and_then(|sources| sources.get(source));
                    match (document, snapshot) {
                        (Some(document), Some(snapshot))
                            if snapshot.len() <= MAX_CONTENT_INPUT_BYTES =>
                        {
                            let parsed = aexeo_contracts::content_gate_change_validation::parse_unique_json_value(snapshot.as_bytes());
                            let mut profile = profile.resolve()?;
                            if input.source_text.is_none()
                                && let Some(separator) = projection_separator
                            {
                                profile.separator.clone_from(separator);
                            }
                            match parsed
                                .ok()
                                .and_then(|value| profile.assess_and_assemble_snapshot(&value).ok())
                            {
                                Some((issues, text)) if text == *document => {
                                    measurements.insert("issue_count".into(), issues.len());
                                    for issue in issues {
                                        emit(
                                            format!(
                                                "Field {:?}: {} (actual {:?}, expected {:?})",
                                                issue.field,
                                                issue.code,
                                                issue.actual,
                                                issue.expected
                                            ),
                                            1,
                                            1,
                                        );
                                    }
                                }
                                Some(_) => reason = Some(
                                    "structured composition does not match this document snapshot"
                                        .into(),
                                ),
                                None => {
                                    reason = Some(
                                        "structured composition is invalid or exceeds limits"
                                            .into(),
                                    )
                                }
                            }
                        }
                        _ => {
                            reason =
                                Some("document or structured composition is unavailable".into())
                        }
                    }
                }
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
                ContentGate::CharacterCount {
                    min,
                    max,
                    ignore_whitespace,
                    ..
                } => {
                    if let Some(min) = min {
                        measurements.insert("min_characters".into(), *min);
                    }
                    if let Some(max) = max {
                        measurements.insert("max_characters".into(), *max);
                    }
                    match &input.text {
                        Some(text) => {
                            let count = *character_counts[usize::from(*ignore_whitespace)]
                                .get_or_insert_with(|| {
                                    text.chars()
                                        .filter(|c| !ignore_whitespace || !c.is_whitespace())
                                        .count()
                                });
                            measurements.insert("character_count".into(), count);
                            if let Some(minimum) = min.filter(|n| count < *n) {
                                emit(
                                    format!(
                                        "Character count {count} is below the minimum of {minimum}"
                                    ),
                                    1,
                                    1,
                                );
                            }
                            if let Some(maximum) = max.filter(|n| count > *n) {
                                emit(
                                    format!(
                                        "Character count {count} exceeds the maximum of {maximum}"
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
                        let mut item_ids = BTreeSet::new();
                        let mut missing = 0;
                        let mut missing_group = 0;
                        let mut duplicates = 0;
                        let mut evaluated = 0;
                        let mut key_bytes = 0;
                        let mut limit_reason = None;
                        for item in items.iter().take(MAX_COLLECTION_ITEMS) {
                            if item.id.trim().is_empty() || !item_ids.insert(item.id.as_str()) {
                                bail!("collection item IDs must be nonempty and unique");
                            }
                            let value = item.metadata.as_ref().and_then(|m| m.get(field));
                            let group = group_by.as_ref().and_then(|field| {
                                item.metadata
                                    .as_ref()
                                    .and_then(|metadata| metadata.get(field))
                            });
                            if value.is_some_and(|value| value.len() > MAX_COLLECTION_VALUE_BYTES)
                                || group
                                    .is_some_and(|group| group.len() > MAX_COLLECTION_VALUE_BYTES)
                            {
                                limit_reason =
                                    Some("collection metadata exceeds the byte limit".to_owned());
                                break;
                            }
                            let value = value.filter(|v| !v.trim().is_empty());
                            let group = group.filter(|v| !v.trim().is_empty());
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
                                        evaluated += 1;
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
                                    let added_bytes =
                                        value.len() + group.map_or(0, |value| value.len());
                                    if first_by_value.len() >= MAX_COLLECTION_KEYS
                                        || key_bytes + added_bytes > MAX_COLLECTION_KEY_BYTES
                                    {
                                        limit_reason = Some("too many distinct collection metadata values to assess completely".to_owned());
                                        break;
                                    }
                                    key_bytes += added_bytes;
                                    first_by_value.insert(key, &item.id);
                                }
                            }
                            evaluated += 1;
                        }
                        if limit_reason.is_none() && items.len() > MAX_COLLECTION_ITEMS {
                            limit_reason =
                                Some("too many collection items to assess completely".to_owned());
                        }
                        measurements.insert("items_evaluated".into(), evaluated);
                        measurements.insert("items_missing_field".into(), missing);
                        if group_by.is_some() {
                            measurements.insert("items_missing_group".into(), missing_group);
                        }
                        measurements.insert("duplicate_items".into(), duplicates);
                        if missing > 0
                            || missing_group > 0
                            || !collection_complete
                            || limit_reason.is_some()
                        {
                            let mut causes = Vec::new();
                            if let Some(cause) = limit_reason {
                                causes.push(cause);
                            }
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
                    if let Some(values) = destinations {
                        measurements.insert("destination_count".into(), values.len());
                    }
                    if let Some(observations) = capabilities.link_reachability {
                        measurements.insert("observation_count".into(), observations.len());
                    }
                    match (destinations, capabilities.link_reachability) {
                        (Some(values), Some(observations))
                            if values.len() == observations.len() =>
                        {
                            let mut unchecked = 0usize;
                            let mut not_found = 0usize;
                            let mut not_applicable = 0usize;
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
                                    LinkReachabilityObservation::NotApplicable => {
                                        not_applicable += 1
                                    }
                                }
                            }
                            measurements.insert("checked_destinations".into(), checked);
                            measurements.insert("not_found_destinations".into(), not_found);
                            measurements.insert("unchecked_destinations".into(), unchecked);
                            measurements
                                .insert("not_applicable_destinations".into(), not_applicable);
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
                ContentGate::DuplicateParagraphs {
                    max_occurrences,
                    min_chars,
                    case_insensitive,
                    ..
                } => {
                    measurements.insert("max_occurrences".into(), *max_occurrences);
                    measurements.insert("min_chars".into(), *min_chars);
                    match context.and_then(|c| c.paragraphs.as_ref()) {
                        Some(paragraphs) => {
                            let mut counts = BTreeMap::<String, usize>::new();
                            let (mut key_bytes, mut evaluated, mut skipped, mut exceeding) =
                                (0usize, 0usize, 0usize, 0usize);
                            for paragraph in paragraphs.iter().take(MAX_DUPLICATE_PARAGRAPHS) {
                                if paragraph.text.len() > MAX_DUPLICATE_PARAGRAPH_BYTES {
                                    reason = Some(
                                        "paragraph exceeds the duplicate-check byte limit".into(),
                                    );
                                    break;
                                }
                                let text = collapse_whitespace(&paragraph.text);
                                if text.chars().count() < *min_chars {
                                    skipped += 1;
                                    continue;
                                }
                                let key = if *case_insensitive {
                                    text.to_lowercase()
                                } else {
                                    text
                                };
                                if key.len() > MAX_DUPLICATE_PARAGRAPH_BYTES {
                                    reason = Some("normalized paragraph exceeds the duplicate-check byte limit".into());
                                    break;
                                }
                                if !counts.contains_key(&key) {
                                    if counts.len() >= MAX_DUPLICATE_PARAGRAPH_KEYS
                                        || key_bytes + key.len() > MAX_DUPLICATE_PARAGRAPH_KEY_BYTES
                                    {
                                        reason = Some(
                                            "too many distinct paragraphs to assess completely"
                                                .into(),
                                        );
                                        break;
                                    }
                                    key_bytes += key.len();
                                }
                                evaluated += 1;
                                let count = counts.entry(key).or_default();
                                *count += 1;
                                if *count > *max_occurrences {
                                    exceeding += 1;
                                    emit(
                                        format!(
                                            "Paragraph occurs {count} times; maximum is {max_occurrences}"
                                        ),
                                        paragraph.line,
                                        paragraph.column,
                                    );
                                }
                            }
                            if reason.is_none() && paragraphs.len() > MAX_DUPLICATE_PARAGRAPHS {
                                reason = Some("too many paragraphs to assess completely".into());
                            }
                            measurements.insert("paragraph_count".into(), paragraphs.len());
                            measurements.insert("paragraphs_evaluated".into(), evaluated);
                            measurements.insert("paragraphs_skipped".into(), skipped);
                            measurements.insert("paragraphs_exceeding_limit".into(), exceeding);
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
                            let mut key_bytes = 0usize;
                            let mut exceeding = 0;
                            let mut evaluated = 0;
                            let mut skipped = 0;
                            for paragraph in paragraphs.iter().take(MAX_PARAGRAPH_OPENINGS) {
                                let mut words = Vec::with_capacity(*opening_words);
                                let mut bytes = 0usize;
                                for word in paragraph.text.split_whitespace().take(*opening_words) {
                                    bytes += word.len();
                                    if bytes > MAX_PARAGRAPH_OPENING_BYTES {
                                        break;
                                    }
                                    words.push(opening_word_key(word));
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
                                if !openings.contains_key(&words) {
                                    if openings.len() >= MAX_DISTINCT_PARAGRAPH_OPENINGS {
                                        reason = Some(
                                            "too many distinct paragraph openings to assess completely"
                                                .into(),
                                        );
                                        break;
                                    }
                                    let new_key_bytes: usize = words.iter().map(String::len).sum();
                                    if key_bytes + new_key_bytes > MAX_PARAGRAPH_OPENING_KEY_BYTES {
                                        reason = Some(
                                            "paragraph opening keys exceed the supported byte limit".into(),
                                        );
                                        break;
                                    }
                                    key_bytes += new_key_bytes;
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
                            if reason.is_none() && paragraphs.len() > MAX_PARAGRAPH_OPENINGS {
                                reason = Some("too many paragraphs to assess completely".into());
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
                ContentGate::InlineTripleCandidates {
                    conjunction,
                    min_items,
                    item_separator,
                    delimiters,
                    abbreviations,
                    ..
                } => {
                    if let Some(paragraphs) =
                        context.and_then(|context| context.paragraphs.as_ref())
                    {
                        let marker = format!(" {} ", conjunction.to_lowercase());
                        let mut bytes_seen = 0usize;
                        let mut candidates = 0usize;
                        for paragraph in paragraphs.iter().take(MAX_PARAGRAPH_OPENINGS) {
                            bytes_seen = bytes_seen.saturating_add(paragraph.text.len());
                            if bytes_seen > MAX_CONTENT_INPUT_BYTES {
                                reason =
                                    Some("paragraph text exceeds the supported byte limit".into());
                                break;
                            }
                            let mut quotes =
                                crate::quote_tracking::QuoteTracker::new(&paragraph.text);
                            for_each_sentence(
                                &paragraph.text,
                                paragraph.line,
                                paragraph.column,
                                &[],
                                delimiters,
                                abbreviations,
                                |segment, line, column| {
                                    if sentence_is_quoted(&mut quotes, &paragraph.text, segment) {
                                        return;
                                    }
                                    let segment = segment.trim();
                                    if segment.starts_with(['"', '\'', '«', '“', '‘']) {
                                        return;
                                    }
                                    let folded = segment.to_lowercase();
                                    let Some((left, right)) = folded.rsplit_once(&marker) else {
                                        return;
                                    };
                                    let separator = item_separator.unwrap_or(',');
                                    let left = left.trim_end_matches(separator).trim_end();
                                    let valid_item = |part: &str, max_words: usize| {
                                        let part = part.trim();
                                        !part.is_empty()
                                            && part.split_whitespace().count() <= max_words
                                            && !part.contains(separator)
                                            && part.chars().any(char::is_alphabetic)
                                    };
                                    let minimum = min_items.unwrap_or(3);
                                    if minimum == 3 {
                                        // Keep legacy triple matching and diagnostics unchanged.
                                        let Some((first, second)) = left.rsplit_once(separator)
                                        else {
                                            return;
                                        };
                                        if valid_item(first, 6)
                                            && valid_item(second, 6)
                                            && valid_item(right, 6)
                                        {
                                            candidates += 1;
                                            emit("Possible three-item inline enumeration; review whether the repetition is intentional".into(), line, column);
                                        }
                                    } else {
                                        // The first item may include the preceding clause. This is
                                        // a bounded surface heuristic, not grammatical parsing.
                                        let parts: Vec<_> =
                                            left.rsplit(separator).take(minimum - 1).collect();
                                        let last = right.split(separator).next().unwrap_or("");
                                        if parts.len() == minimum - 1
                                            && parts
                                                .iter()
                                                .take(parts.len() - 1)
                                                .all(|part| valid_item(part, 6))
                                            && valid_item(parts[parts.len() - 1], 40)
                                            && valid_item(last, 6)
                                        {
                                            candidates += 1;
                                            emit(
                                                format!(
                                                    "Possible inline enumeration of at least {minimum} items; review whether the repetition is intentional"
                                                ),
                                                line,
                                                column,
                                            );
                                        }
                                    }
                                },
                            );
                            if !quotes.finish() {
                                reason = Some(
                                    "a quotation continues beyond the observed paragraph".into(),
                                );
                                break;
                            }
                        }
                        measurements.insert("paragraph_count".into(), paragraphs.len());
                        measurements.insert("candidate_enumerations".into(), candidates);
                        if paragraphs.len() > MAX_PARAGRAPH_OPENINGS {
                            reason = Some("too many paragraphs to assess completely".into());
                        }
                    } else {
                        reason = Some("paragraphs are unavailable from this adapter".into());
                    }
                }
                ContentGate::SentenceLengthVariance {
                    window_size,
                    reset_at_paragraph,
                    min_stddev_words,
                    random_min_stddev_words,
                    random_max_stddev_words,
                    random_seed,
                    delimiters,
                    abbreviations,
                    allow_arabic_word_boundaries,
                    ..
                }
                | ContentGate::ProseSentenceLengthVariance {
                    window_size,
                    reset_at_paragraph,
                    min_stddev_words,
                    random_min_stddev_words,
                    random_max_stddev_words,
                    random_seed,
                    delimiters,
                    abbreviations,
                    allow_arabic_word_boundaries,
                    ..
                } => {
                    let prose_only =
                        matches!(gate, ContentGate::ProseSentenceLengthVariance { .. });
                    let threshold = if let Some(value) = min_stddev_words {
                        *value
                    } else {
                        use sha2::{Digest, Sha256};
                        let (Some(lo), Some(hi), Some(seed)) = (
                            *random_min_stddev_words,
                            *random_max_stddev_words,
                            *random_seed,
                        ) else {
                            bail!("variance range and seed are required for {}", gate.id());
                        };
                        let mut hash = Sha256::new();
                        hash.update(b"aexeo-sentence-variance-v1\0");
                        hash.update(seed.to_le_bytes());
                        hash.update(gate.id().as_bytes());
                        let mut bytes = [0u8; 8];
                        bytes.copy_from_slice(&hash.finalize()[..8]);
                        lo + (u64::from_le_bytes(bytes) % ((hi - lo + 1) as u64)) as usize
                    };
                    measurements.insert("window_size".into(), *window_size);
                    measurements.insert("effective_min_stddev_words".into(), threshold);
                    let paragraphs = context.and_then(|context| context.paragraphs.as_ref());
                    let classification = paragraph_list_items
                        .filter(|items| paragraphs.is_some_and(|p| items.len() == p.len()));
                    if prose_only && classification.is_none() {
                        reason = Some("complete paragraph list classification is unavailable from this adapter".into());
                    } else if let Some(paragraphs) = paragraphs {
                        let mut bytes_seen = 0usize;
                        let mut flagged = 0usize;
                        let mut windows = 0usize;
                        let mut uncertain = 0usize;
                        let mut paragraphs_without_windows = 0usize;
                        let mut lengths = std::collections::VecDeque::new();
                        let (mut sum, mut sum_squares) = (0u128, 0u128);
                        let mut excluded_list_paragraphs = 0usize;
                        for (index, paragraph) in
                            paragraphs.iter().take(MAX_PARAGRAPH_OPENINGS).enumerate()
                        {
                            if prose_only && classification.is_some_and(|items| items[index]) {
                                excluded_list_paragraphs += 1;
                                lengths.clear();
                                sum = 0;
                                sum_squares = 0;
                                continue;
                            }
                            if *reset_at_paragraph {
                                lengths.clear();
                                sum = 0;
                                sum_squares = 0;
                            }
                            let windows_before = windows;
                            bytes_seen = bytes_seen.saturating_add(paragraph.text.len());
                            if bytes_seen > MAX_CONTENT_INPUT_BYTES {
                                reason =
                                    Some("paragraph text exceeds the supported byte limit".into());
                                break;
                            }
                            let mut quotes =
                                crate::quote_tracking::QuoteTracker::new(&paragraph.text);
                            for_each_sentence(
                                &paragraph.text,
                                paragraph.line,
                                paragraph.column,
                                &[],
                                delimiters,
                                abbreviations,
                                |segment, line, column| {
                                    if sentence_is_quoted(&mut quotes, &paragraph.text, segment) {
                                        lengths.clear();
                                        sum = 0;
                                        sum_squares = 0;
                                        return;
                                    }
                                    if segment.trim().is_empty() {
                                        return;
                                    }
                                    if !has_supported_word_boundaries(
                                        segment,
                                        *allow_arabic_word_boundaries,
                                    ) {
                                        lengths.clear();
                                        sum = 0;
                                        sum_squares = 0;
                                        uncertain += 1;
                                        return;
                                    }
                                    let count = segment.split_whitespace().count();
                                    let count_wide = count as u128;
                                    sum += count_wide;
                                    sum_squares += count_wide * count_wide;
                                    lengths.push_back((count, line, column));
                                    if lengths.len() > *window_size
                                        && let Some((old, _, _)) = lengths.pop_front()
                                    {
                                        let old = old as u128;
                                        sum -= old;
                                        sum_squares -= old * old;
                                    }
                                    if lengths.len() == *window_size {
                                        windows += 1;
                                        let n = *window_size as u128;
                                        let numerator = n * sum_squares - sum * sum;
                                        if numerator < (threshold as u128).pow(2) * n * n {
                                            flagged += 1;
                                            let deviation = (numerator as f64).sqrt() / n as f64;
                                            let (minimum, maximum) = lengths.iter().fold(
                                                (usize::MAX, 0),
                                                |(lo, hi), (value, _, _)| {
                                                    (lo.min(*value), hi.max(*value))
                                                },
                                            );
                                            let (_, start_line, start_column) = lengths[0];
                                            emit(
                                                format!(
                                                    "In {window_size} sentences, lengths range from {minimum} to {maximum} words (standard deviation {deviation:.2}, threshold {threshold}); review the rhythm in context"
                                                ),
                                                start_line,
                                                start_column,
                                            );
                                        }
                                    }
                                },
                            );
                            if *reset_at_paragraph && windows == windows_before {
                                paragraphs_without_windows += 1;
                            }
                            if !quotes.finish() {
                                reason = Some(
                                    "a quotation continues beyond the observed paragraph".into(),
                                );
                                break;
                            }
                        }
                        measurements.insert("paragraph_count".into(), paragraphs.len());
                        if prose_only {
                            measurements.insert(
                                "list_paragraphs_excluded".into(),
                                excluded_list_paragraphs,
                            );
                        }
                        measurements.insert("windows_checked".into(), windows);
                        measurements.insert("low_variance_windows".into(), flagged);
                        if *reset_at_paragraph {
                            measurements.insert(
                                "paragraphs_without_windows".into(),
                                paragraphs_without_windows,
                            );
                        }
                        if paragraphs.len() > MAX_PARAGRAPH_OPENINGS {
                            reason = Some("too many paragraphs to assess completely".into());
                        }
                        if reason.is_none() && uncertain > 0 {
                            reason = Some("some sentences use unsupported word boundaries".into());
                        }
                        if reason.is_none() && windows == 0 {
                            reason =
                                Some("not enough eligible sentences to assess variance".into());
                        } else if reason.is_none()
                            && *reset_at_paragraph
                            && paragraphs_without_windows > 0
                        {
                            reason = Some(
                                "some paragraphs lack enough eligible sentences to assess variance"
                                    .into(),
                            );
                        }
                    } else {
                        reason = Some("paragraphs are unavailable from this adapter".into());
                    }
                }
                ContentGate::SimilarSentenceLengths {
                    run_length,
                    tolerance_words,
                    min_difference_percent,
                    range_min_words,
                    range_max_words,
                    delimiters,
                    abbreviations,
                    allow_arabic_word_boundaries,
                    ..
                } => {
                    measurements.insert("run_length".into(), *run_length);
                    if let Some(percent) = min_difference_percent {
                        measurements.insert("min_difference_percent".into(), *percent);
                    }
                    measurements.insert("tolerance_words".into(), *tolerance_words);
                    if let (Some(lo), Some(hi)) = (range_min_words, range_max_words) {
                        measurements.insert("range_min_words".into(), *lo);
                        measurements.insert("range_max_words".into(), *hi);
                    }
                    if let Some(paragraphs) =
                        context.and_then(|context| context.paragraphs.as_ref())
                    {
                        let mut bytes_seen = 0usize;
                        let mut flagged = 0usize;
                        let mut uncertain = 0usize;
                        let mut pairs_assessed = 0usize;
                        for paragraph in paragraphs.iter().take(MAX_PARAGRAPH_OPENINGS) {
                            bytes_seen = bytes_seen.saturating_add(paragraph.text.len());
                            if bytes_seen > MAX_CONTENT_INPUT_BYTES {
                                reason =
                                    Some("paragraph text exceeds the supported byte limit".into());
                                break;
                            }
                            let mut lengths = std::collections::VecDeque::new();
                            let mut quotes =
                                crate::quote_tracking::QuoteTracker::new(&paragraph.text);
                            for_each_sentence(
                                &paragraph.text,
                                paragraph.line,
                                paragraph.column,
                                &[],
                                delimiters,
                                abbreviations,
                                |segment, line, column| {
                                    if sentence_is_quoted(&mut quotes, &paragraph.text, segment) {
                                        lengths.clear();
                                        return;
                                    }
                                    if segment.trim().is_empty() {
                                        return;
                                    }
                                    if !has_supported_word_boundaries(
                                        segment,
                                        *allow_arabic_word_boundaries,
                                    ) {
                                        lengths.clear();
                                        uncertain += 1;
                                        return;
                                    }
                                    lengths.push_back(segment.split_whitespace().count());
                                    if lengths.len() > *run_length {
                                        lengths.pop_front();
                                    }
                                    if lengths.len() == *run_length {
                                        let min = lengths.iter().copied().min().unwrap_or(0);
                                        let max = lengths.iter().copied().max().unwrap_or(0);
                                        pairs_assessed += 1;
                                        let similar = match (
                                            range_min_words,
                                            range_max_words,
                                            min_difference_percent,
                                        ) {
                                            (Some(lo), Some(hi), _) => min >= *lo && max <= *hi,
                                            (_, _, Some(percent)) => {
                                                max > 0
                                                    && ((max - min) as u128) * 100
                                                        < (*percent as u128) * (max as u128)
                                            }
                                            _ => max - min <= *tolerance_words,
                                        };
                                        if similar {
                                            flagged += 1;
                                            let message = if range_min_words.is_some() {
                                                format!(
                                                    "Adjacent sentences contain {min} and {max} words within the configured range; review their rhythm in context"
                                                )
                                            } else {
                                                format!(
                                                    "A run of {run_length} sentences has similar word counts; review its rhythm in context"
                                                )
                                            };
                                            emit(message, line, column);
                                        }
                                    }
                                },
                            );
                            if !quotes.finish() {
                                reason = Some(
                                    "a quotation continues beyond the observed paragraph".into(),
                                );
                                break;
                            }
                        }
                        measurements.insert("paragraph_count".into(), paragraphs.len());
                        measurements.insert("similar_runs".into(), flagged);
                        if range_min_words.is_some() {
                            measurements.insert("range_pairs".into(), flagged);
                        }
                        if min_difference_percent.is_some() || range_min_words.is_some() {
                            measurements.insert("pairs_assessed".into(), pairs_assessed);
                            if pairs_assessed == 0 && reason.is_none() {
                                reason = Some(
                                    "not enough eligible adjacent sentences to compare".into(),
                                );
                            }
                        }
                        if paragraphs.len() > MAX_PARAGRAPH_OPENINGS {
                            reason = Some("too many paragraphs to assess completely".into());
                        }
                        if reason.is_none() && uncertain > 0 {
                            reason = Some("some sentences use unsupported word boundaries".into());
                        }
                    } else {
                        reason = Some("paragraphs are unavailable from this adapter".into());
                    }
                }
                ContentGate::TrailingIngClause {
                    delimiters,
                    abbreviations,
                    ..
                } => {
                    if let Some(paragraphs) =
                        context.and_then(|context| context.paragraphs.as_ref())
                    {
                        let mut bytes_seen = 0usize;
                        let mut candidates = 0usize;
                        for paragraph in paragraphs.iter().take(MAX_PARAGRAPH_OPENINGS) {
                            bytes_seen = bytes_seen.saturating_add(paragraph.text.len());
                            if bytes_seen > MAX_CONTENT_INPUT_BYTES {
                                reason =
                                    Some("paragraph text exceeds the supported byte limit".into());
                                break;
                            }
                            let mut quotes =
                                crate::quote_tracking::QuoteTracker::new(&paragraph.text);
                            for_each_sentence(
                                &paragraph.text,
                                paragraph.line,
                                paragraph.column,
                                &[],
                                delimiters,
                                abbreviations,
                                |segment, line, column| {
                                    if sentence_is_quoted(&mut quotes, &paragraph.text, segment) {
                                        return;
                                    }
                                    if segment.trim_start().starts_with(['"', '\'', '«', '“', '‘'])
                                    {
                                        return;
                                    }
                                    let Some((prefix, tail)) = segment.rsplit_once(',') else {
                                        return;
                                    };
                                    if prefix.trim().is_empty()
                                        || !tail.starts_with(char::is_whitespace)
                                    {
                                        return;
                                    }
                                    let first = tail.split_whitespace().next().unwrap_or_default();
                                    if first.len() >= 5
                                        && first.chars().all(|ch| ch.is_ascii_alphabetic())
                                        && first.to_ascii_lowercase().ends_with("ing")
                                    {
                                        candidates += 1;
                                        emit("Sentence ends with a possible English -ing clause; review it without removing information".into(), line, column);
                                    }
                                },
                            );
                            if !quotes.finish() {
                                reason = Some(
                                    "a quotation continues beyond the observed paragraph".into(),
                                );
                                break;
                            }
                        }
                        measurements.insert("paragraph_count".into(), paragraphs.len());
                        measurements.insert("candidate_clauses".into(), candidates);
                        if paragraphs.len() > MAX_PARAGRAPH_OPENINGS {
                            reason = Some("too many paragraphs to assess completely".into());
                        }
                    } else {
                        reason = Some("paragraphs are unavailable from this adapter".into());
                    }
                }
                ContentGate::RepeatedPunctuation {
                    mark,
                    max_per_paragraph,
                    ..
                } => {
                    measurements.insert("max_per_paragraph".into(), *max_per_paragraph);
                    if let Some(paragraphs) =
                        context.and_then(|context| context.paragraphs.as_ref())
                    {
                        let mut bytes_seen = 0usize;
                        let mut exceeding = 0usize;
                        for paragraph in paragraphs.iter().take(MAX_PARAGRAPH_OPENINGS) {
                            bytes_seen = bytes_seen.saturating_add(paragraph.text.len());
                            if bytes_seen > MAX_CONTENT_INPUT_BYTES {
                                reason =
                                    Some("paragraph text exceeds the supported byte limit".into());
                                break;
                            }
                            let count = paragraph.text.chars().filter(|ch| ch == mark).count();
                            if count > *max_per_paragraph {
                                exceeding += 1;
                                emit(
                                    format!(
                                        "Punctuation mark occurs {count} times in this paragraph; configured maximum is {max_per_paragraph}"
                                    ),
                                    paragraph.line,
                                    paragraph.column,
                                );
                            }
                        }
                        measurements.insert("paragraph_count".into(), paragraphs.len());
                        measurements.insert("paragraphs_exceeding_limit".into(), exceeding);
                        if paragraphs.len() > MAX_PARAGRAPH_OPENINGS {
                            reason = Some("too many paragraphs to assess completely".into());
                        }
                    } else {
                        reason = Some("paragraphs are unavailable from this adapter".into());
                    }
                }
                ContentGate::SentenceOpeningTerms {
                    terms,
                    require_colon,
                    max_per_paragraph,
                    delimiters,
                    abbreviations,
                    ..
                } => {
                    measurements.insert("max_per_paragraph".into(), *max_per_paragraph);
                    if let Some(paragraphs) =
                        context.and_then(|context| context.paragraphs.as_ref())
                    {
                        let normalized: Vec<String> = terms
                            .iter()
                            .map(|term| term.nfc().flat_map(char::to_lowercase).nfc().collect())
                            .collect();
                        let mut bytes_seen = 0usize;
                        let mut matches = 0usize;
                        for paragraph in paragraphs.iter().take(MAX_PARAGRAPH_OPENINGS) {
                            bytes_seen = bytes_seen.saturating_add(paragraph.text.len());
                            if bytes_seen > MAX_CONTENT_INPUT_BYTES {
                                reason =
                                    Some("paragraph text exceeds the supported byte limit".into());
                                break;
                            }
                            let mut in_paragraph = 0usize;
                            let mut quotes =
                                crate::quote_tracking::QuoteTracker::new(&paragraph.text);
                            for_each_sentence(
                                &paragraph.text,
                                paragraph.line,
                                paragraph.column,
                                &[],
                                delimiters,
                                abbreviations,
                                |segment, line, column| {
                                    if sentence_is_quoted(&mut quotes, &paragraph.text, segment) {
                                        return;
                                    }
                                    let segment = segment.trim_start();
                                    // Do not propose changes inside explicitly quoted openings.
                                    if segment.starts_with(['"', '\'', '«', '“', '‘']) {
                                        return;
                                    }
                                    let folded: String =
                                        segment.nfc().flat_map(char::to_lowercase).nfc().collect();
                                    if normalized.iter().any(|term| {
                                        folded.strip_prefix(term).is_some_and(|tail| {
                                            if *require_colon {
                                                tail.trim_start().starts_with(':')
                                            } else {
                                                tail.chars().next().is_none_or(|ch| {
                                                    !ch.is_alphanumeric() && ch != '_'
                                                })
                                            }
                                        })
                                    }) {
                                        matches += 1;
                                        in_paragraph += 1;
                                        if in_paragraph > *max_per_paragraph {
                                            emit(
                                                format!(
                                                    "Configured sentence-opening expression occurs {in_paragraph} times in this paragraph; review the wording in context"
                                                ),
                                                line,
                                                column,
                                            );
                                        }
                                    }
                                },
                            );
                            if !quotes.finish() {
                                reason = Some(
                                    "a quotation continues beyond the observed paragraph".into(),
                                );
                                break;
                            }
                        }
                        measurements.insert("paragraph_count".into(), paragraphs.len());
                        measurements.insert("matching_openings".into(), matches);
                        if paragraphs.len() > MAX_PARAGRAPH_OPENINGS {
                            reason = Some("too many paragraphs to assess completely".into());
                        }
                    } else {
                        reason = Some("paragraphs are unavailable from this adapter".into());
                    }
                }
                ContentGate::RepeatedSentenceOpenings {
                    opening_words,
                    max_consecutive,
                    allow_arabic_word_boundaries,
                    delimiters,
                    abbreviations,
                    ..
                } => {
                    measurements.insert("opening_words".into(), *opening_words);
                    measurements.insert("max_consecutive".into(), *max_consecutive);
                    if let Some(paragraphs) =
                        context.and_then(|context| context.paragraphs.as_ref())
                    {
                        let mut evaluated = 0usize;
                        let mut skipped = 0usize;
                        let mut too_short = 0usize;
                        let mut exceeding = 0usize;
                        let mut bytes_seen = 0usize;
                        for paragraph in paragraphs.iter().take(MAX_PARAGRAPH_OPENINGS) {
                            bytes_seen = bytes_seen.saturating_add(paragraph.text.len());
                            if bytes_seen > MAX_CONTENT_INPUT_BYTES {
                                reason =
                                    Some("paragraph text exceeds the supported byte limit".into());
                                break;
                            }
                            let mut previous: Option<Vec<String>> = None;
                            let mut consecutive = 0usize;
                            for_each_sentence(
                                &paragraph.text,
                                paragraph.line,
                                paragraph.column,
                                &[],
                                delimiters,
                                abbreviations,
                                |segment, line, column| {
                                    if segment.trim().is_empty() {
                                        return;
                                    }
                                    if !has_supported_word_boundaries(
                                        segment,
                                        *allow_arabic_word_boundaries,
                                    ) {
                                        skipped += 1;
                                        previous = None;
                                        consecutive = 0;
                                        return;
                                    }
                                    let mut bytes = 0usize;
                                    let mut words = Vec::with_capacity(*opening_words);
                                    for word in segment.split_whitespace().take(*opening_words) {
                                        bytes = bytes.saturating_add(word.len());
                                        if bytes > MAX_PARAGRAPH_OPENING_BYTES {
                                            break;
                                        }
                                        words.push(opening_word_key(word));
                                    }
                                    // A short sentence with supported boundaries cannot contain
                                    // a complete opening. Other scripts remain uncertain.
                                    if bytes <= MAX_PARAGRAPH_OPENING_BYTES
                                        && words.len() < *opening_words
                                        && !words.is_empty()
                                        && words.iter().all(|word| !word.is_empty())
                                    {
                                        too_short += 1;
                                        previous = None;
                                        consecutive = 0;
                                        return;
                                    }
                                    if bytes > MAX_PARAGRAPH_OPENING_BYTES
                                        || words.len() != *opening_words
                                        || words.iter().any(String::is_empty)
                                    {
                                        skipped += 1;
                                        previous = None;
                                        consecutive = 0;
                                        return;
                                    }
                                    evaluated += 1;
                                    if previous.as_ref() == Some(&words) {
                                        consecutive += 1;
                                    } else {
                                        consecutive = 1;
                                    }
                                    previous = Some(words);
                                    if consecutive > *max_consecutive {
                                        exceeding += 1;
                                        emit(
                                            format!(
                                                "Sentence opening repeats {consecutive} times consecutively; maximum is {max_consecutive}"
                                            ),
                                            line,
                                            column,
                                        );
                                    }
                                },
                            );
                        }
                        if reason.is_none() && paragraphs.len() > MAX_PARAGRAPH_OPENINGS {
                            reason = Some("too many paragraphs to assess completely".into());
                        }
                        measurements.insert("paragraph_count".into(), paragraphs.len());
                        measurements.insert("sentences_evaluated".into(), evaluated);
                        measurements.insert("sentences_skipped".into(), skipped);
                        measurements.insert("sentences_too_short".into(), too_short);
                        measurements.insert("sentences_exceeding_limit".into(), exceeding);
                        if reason.is_none() && skipped > 0 {
                            reason = Some("some sentence openings could not be assessed using whitespace-separated words".into());
                        }
                    } else {
                        reason = Some("paragraphs are unavailable from this adapter".into());
                    }
                }
                ContentGate::SentenceOpeningWindow {
                    opening_words,
                    window_size,
                    window_across_paragraphs,
                    max_occurrences,
                    ignored_words,
                    delimiters,
                    abbreviations,
                    allow_arabic_word_boundaries,
                    ..
                } => {
                    measurements.insert("opening_words".into(), *opening_words);
                    measurements.insert("window_size".into(), *window_size);
                    measurements.insert("max_occurrences".into(), *max_occurrences);
                    let ignored: BTreeSet<String> = ignored_words
                        .iter()
                        .map(|word| opening_word_key(word))
                        .collect();
                    if let Some(paragraphs) =
                        context.and_then(|context| context.paragraphs.as_ref())
                    {
                        let mut evaluated = 0usize;
                        let mut assessable_windows = 0usize;
                        let mut skipped = 0usize;
                        let mut quoted = 0usize;
                        let mut too_short = 0usize;
                        let mut exceeding = 0usize;
                        let mut bytes_seen = 0usize;
                        let mut window = std::collections::VecDeque::<Option<Vec<String>>>::new();
                        for paragraph in paragraphs.iter().take(MAX_PARAGRAPH_OPENINGS) {
                            if !window_across_paragraphs {
                                window.clear();
                            }
                            bytes_seen = bytes_seen.saturating_add(paragraph.text.len());
                            if bytes_seen > MAX_CONTENT_INPUT_BYTES {
                                reason =
                                    Some("paragraph text exceeds the supported byte limit".into());
                                break;
                            }
                            let mut quotes =
                                crate::quote_tracking::QuoteTracker::new(&paragraph.text);
                            for_each_sentence(
                                &paragraph.text,
                                paragraph.line,
                                paragraph.column,
                                &[],
                                delimiters,
                                abbreviations,
                                |segment, line, column| {
                                    if segment.trim().is_empty() {
                                        return;
                                    }
                                    if sentence_is_quoted(&mut quotes, &paragraph.text, segment) {
                                        quoted += 1;
                                        window.push_back(None);
                                        if window.len() > *window_size {
                                            window.pop_front();
                                        }
                                        return;
                                    }
                                    if !has_supported_word_boundaries(
                                        segment,
                                        *allow_arabic_word_boundaries,
                                    ) {
                                        skipped += 1;
                                        window.push_back(None);
                                        if window.len() > *window_size {
                                            window.pop_front();
                                        }
                                        return;
                                    }
                                    let mut bytes = 0usize;
                                    let mut words = Vec::with_capacity(*opening_words);
                                    let mut ignored_count = 0usize;
                                    for word in segment.split_whitespace() {
                                        bytes = bytes.saturating_add(word.len());
                                        if bytes > MAX_PARAGRAPH_OPENING_BYTES {
                                            break;
                                        }
                                        let key = opening_word_key(word);
                                        if words.is_empty() && ignored.contains(&key) {
                                            ignored_count += 1;
                                            continue;
                                        }
                                        words.push(key);
                                        if words.len() == *opening_words {
                                            break;
                                        }
                                    }
                                    // A short sentence with supported boundaries cannot contain
                                    // a complete opening. Other scripts remain uncertain.
                                    if bytes <= MAX_PARAGRAPH_OPENING_BYTES
                                        && words.len() < *opening_words
                                        && (!words.is_empty() || ignored_count > 0)
                                        && has_supported_word_boundaries(
                                            segment,
                                            *allow_arabic_word_boundaries,
                                        )
                                        && words.iter().all(|word| !word.is_empty())
                                    {
                                        too_short += 1;
                                        window.push_back(None);
                                        if window.len() > *window_size {
                                            window.pop_front();
                                        }
                                        return;
                                    }
                                    if bytes > MAX_PARAGRAPH_OPENING_BYTES
                                        || words.len() != *opening_words
                                        || words.iter().any(String::is_empty)
                                    {
                                        skipped += 1;
                                        window.push_back(None);
                                        if window.len() > *window_size {
                                            window.pop_front();
                                        }
                                        return;
                                    }
                                    evaluated += 1;
                                    window.push_back(Some(words.clone()));
                                    if window.len() > *window_size {
                                        window.pop_front();
                                    }
                                    if window.iter().filter(|entry| entry.is_some()).count()
                                        > *max_occurrences
                                    {
                                        assessable_windows += 1;
                                    }
                                    let count = window
                                        .iter()
                                        .filter(|entry| entry.as_ref() == Some(&words))
                                        .count();
                                    if count > *max_occurrences {
                                        exceeding += 1;
                                        emit(
                                            format!(
                                                "Sentence opening occurs {count} times in a window of up to {window_size} sentences; maximum is {max_occurrences}; review repetition in context"
                                            ),
                                            line,
                                            column,
                                        );
                                    }
                                },
                            );
                            if !quotes.finish() {
                                reason = Some(
                                    "a quotation continues beyond the observed paragraph".into(),
                                );
                                break;
                            }
                        }
                        if reason.is_none() && paragraphs.len() > MAX_PARAGRAPH_OPENINGS {
                            reason = Some("too many paragraphs to assess completely".into());
                        }
                        measurements.insert("paragraph_count".into(), paragraphs.len());
                        measurements.insert("sentences_evaluated".into(), evaluated);
                        measurements.insert("assessable_windows".into(), assessable_windows);
                        measurements.insert("sentences_skipped".into(), skipped);
                        measurements.insert("sentences_quoted".into(), quoted);
                        measurements.insert("sentences_too_short".into(), too_short);
                        measurements.insert("sentences_exceeding_limit".into(), exceeding);
                        if reason.is_none() && skipped > 0 {
                            reason = Some("some sentence openings could not be assessed using whitespace-separated words".into());
                        } else if reason.is_none() && assessable_windows == 0 {
                            reason = Some("no sentence window contains enough evaluable openings to exceed the configured occurrence limit".into());
                        }
                    } else {
                        reason = Some("paragraphs are unavailable from this adapter".into());
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
                        let mut unsupported_sentences = 0;
                        let require_latin = matches!(
                            gate,
                            ContentGate::SentenceLength {
                                require_latin_word_boundaries: true,
                                ..
                            }
                        );
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
                        for (text, line, column, segment_block_ends) in segments {
                            for_each_sentence(
                                text,
                                line,
                                column,
                                segment_block_ends,
                                delimiters,
                                abbreviations,
                                |segment, start_line, start_column| {
                                    if require_latin && !is_simple_latin_sentence(segment) {
                                        unsupported_sentences += 1;
                                        return;
                                    }
                                    let count = if chars {
                                        segment.trim().chars().count()
                                    } else {
                                        segment
                                            .split_whitespace()
                                            .filter(|word| {
                                                !word.chars().all(|ch| {
                                                    matches!(
                                                        ch,
                                                        '«' | '»'
                                                            | '“'
                                                            | '”'
                                                            | '‘'
                                                            | '’'
                                                            | '('
                                                            | ')'
                                                            | '['
                                                            | ']'
                                                            | '{'
                                                            | '}'
                                                            | '\''
                                                            | '"'
                                                    )
                                                })
                                            })
                                            .count()
                                    };
                                    if count > 0 {
                                        sentence_count += 1;
                                        longest = longest.max(count);
                                        if count > *limit {
                                            exceeding += 1;
                                            emit(
                                                format!(
                                                    "Sentence has {count} {unit}; maximum is {limit}"
                                                ),
                                                start_line,
                                                start_column,
                                            );
                                        }
                                    }
                                },
                            );
                        }
                        measurements.insert("sentence_count".into(), sentence_count);
                        measurements.insert("sentences_exceeding_limit".into(), exceeding);
                        measurements.insert(longest_key.into(), longest);
                        if require_latin {
                            measurements.insert(
                                "sentences_with_unverified_word_boundaries".into(),
                                unsupported_sentences,
                            );
                            if unsupported_sentences > 0 {
                                reason = Some("some sentences are outside the conservative Latin character subset; whitespace word boundaries are unverified".into());
                            }
                        }
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
                    match literal_input {
                        Ok(Some(text)) => {
                            let mut total = 0;
                            let mut exceeded = 0;
                            let literal = cached_literal(
                                &mut literal_cache,
                                text,
                                block_ends,
                                *case_insensitive,
                            );
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
                        Ok(None) => reason = Some("text is unavailable from this adapter".into()),
                        Err(()) => {
                            reason = Some(
                                "literal text projection does not match the observed text".into(),
                            )
                        }
                    }
                }
                ContentGate::RequiredTerms {
                    terms,
                    whole_words,
                    case_insensitive,
                    ..
                } => {
                    measurements.insert("configured_terms".into(), terms.len());
                    match literal_input {
                        Ok(Some(text)) => {
                            let literal = cached_literal(
                                &mut literal_cache,
                                text,
                                block_ends,
                                *case_insensitive,
                            );
                            let mut missing = 0;
                            for term in terms {
                                if !literal.visit_matches(term, *whole_words, |_| true) {
                                    missing += 1;
                                    emit(format!("Required term is missing: {term}"), 1, 1);
                                }
                            }
                            measurements.insert("present_terms".into(), terms.len() - missing);
                            measurements.insert("missing_terms".into(), missing);
                        }
                        Ok(None) => reason = Some("text is unavailable from this adapter".into()),
                        Err(()) => {
                            reason = Some(
                                "literal text projection does not match the observed text".into(),
                            )
                        }
                    }
                }
                ContentGate::ForbiddenTerms {
                    terms,
                    whole_words,
                    case_insensitive,
                    review_only,
                    ..
                } => {
                    measurements.insert("configured_terms".into(), terms.len());
                    match literal_input {
                        Ok(Some(text)) => {
                            let mut matches = 0;
                            let literal = cached_literal(
                                &mut literal_cache,
                                text,
                                block_ends,
                                *case_insensitive,
                            );
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
                                        let message = if *review_only {
                                            format!("Phrase to review in context: {term}")
                                        } else {
                                            format!("Forbidden term: {term}")
                                        };
                                        emit(message, line, column);
                                        cursor = offset;
                                    }
                                    false
                                });
                            }
                            omitted += matches.saturating_sub(MAX_FINDINGS_PER_GATE);
                            measurements.insert("matches".into(), matches);
                        }
                        Ok(None) => reason = Some("text is unavailable from this adapter".into()),
                        Err(()) => {
                            reason = Some(
                                "literal text projection does not match the observed text".into(),
                            )
                        }
                    }
                }
                ContentGate::RequiredHeadings { headings, .. } => {
                    measurements.insert("required_headings".into(), headings.len());
                    match &input.headings {
                        Some(actual) => {
                            let mut missing = 0;
                            for heading in headings {
                                if !actual.contains(heading) {
                                    missing += 1;
                                    emit(format!("Required heading is missing: {heading}"), 1, 1);
                                }
                            }
                            measurements
                                .insert("present_headings".into(), headings.len() - missing);
                            measurements.insert("missing_headings".into(), missing);
                        }
                        None => reason = Some("headings are unavailable from this adapter".into()),
                    }
                }
                ContentGate::HeadingHierarchy { initial_level, .. } => {
                    if let Some(level) = initial_level {
                        measurements.insert("initial_level".into(), usize::from(*level));
                    }
                    match (input.headings.as_ref(), heading_levels) {
                        (Some(headings), Some(levels))
                            if headings.len() == levels.len()
                                && levels.iter().all(|level| (1..=6).contains(level)) =>
                        {
                            measurements.insert("heading_count".into(), levels.len());
                            if let Some(level) = levels.first() {
                                measurements
                                    .insert("first_heading_level".into(), usize::from(*level));
                            }
                            if let Some(level) = levels.iter().max() {
                                measurements
                                    .insert("deepest_heading_level".into(), usize::from(*level));
                            }
                            let mut skipped = 0;
                            for index in heading_hierarchy_indices(levels, *initial_level) {
                                skipped += 1;
                                let prior = if index == 0 {
                                    initial_level.unwrap_or(levels[index])
                                } else {
                                    levels[index - 1]
                                };
                                let level = levels[index];
                                emit(
                                    format!(
                                        "Heading #{} skips from H{prior} to H{level}",
                                        index + 1
                                    ),
                                    1,
                                    1,
                                );
                            }
                            measurements.insert("skipped_levels".into(), skipped);
                        }
                        _ => {
                            reason = Some(
                                "complete heading levels are unavailable from this adapter".into(),
                            )
                        }
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
                        (Some(text), Some(snapshot)) => match aexeo_contracts::content_gate_change_validation::parse_unique_json_value(
                            snapshot.as_bytes(),
                        )
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
                                    Some(format!("evidence source has invalid or ambiguous JSON: {source}"))
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
                            match EvidenceReviewRecord::parse(record) {
                                Some(record) if record.matches(claim, snapshot) => {
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
                                Some(_) => {
                                    reason = Some(
                                        "external review is stale or missing a reviewer".into(),
                                    )
                                }
                                None => {
                                    reason =
                                        Some("external review has invalid or ambiguous JSON".into())
                                }
                            }
                        }
                        _ => reason = Some("evidence source or review is unavailable".into()),
                    },
                    _ => reason = Some("content text or evidence sources are unavailable".into()),
                },
                ContentGate::EvidenceReviewAge {
                    source,
                    review,
                    claim,
                    as_of,
                    max_days,
                    ..
                } => {
                    measurements.insert("max_days".into(), *max_days);
                    if let Ok(compact) = as_of.replace('-', "").parse::<usize>() {
                        measurements.insert("as_of_yyyymmdd".into(), compact);
                    }
                    let sources = input.evidence_sources.as_ref();
                    match (
                        &input.text,
                        sources.and_then(|s| s.get(source)),
                        sources.and_then(|s| s.get(review)),
                    ) {
                        (Some(text), Some(snapshot), Some(record)) => {
                            let parsed = EvidenceReviewRecord::parse(record);
                            match parsed {
                                Some(record)
                                    if record.matches(claim, snapshot)
                                        && claim_is_present(text, claim, block_ends) =>
                                {
                                    match (record.provenance.get("reviewed_at").and_then(|date| parse_iso_calendar_date(date)), parse_iso_calendar_date(as_of)) {
                                        (Some(date), Some(reference)) if date <= reference => {
                                            let age = (reference - date).whole_days() as usize;
                                            measurements.insert("age_days".into(), age);
                                            if age > *max_days {
                                                emit(format!("Source review is {age} days old as of {as_of}; reassess the claim and source"), 1, 1);
                                            }
                                        }
                                        (Some(_), Some(_)) => reason = Some("source review date is after the reference date".into()),
                                        _ => reason = Some("source review date is missing or invalid; expected YYYY-MM-DD".into()),
                                    }
                                }
                                Some(_) => reason = Some(
                                    "source review is not bound to the current claim and source"
                                        .into(),
                                ),
                                None => {
                                    reason =
                                        Some("external review has invalid or ambiguous JSON".into())
                                }
                            }
                        }
                        _ => {
                            reason =
                                Some("content, evidence source or review is unavailable".into())
                        }
                    }
                }
                ContentGate::Custom {
                    extension,
                    parameters,
                    ..
                } => {
                    if literal_input.is_err() {
                        reason =
                            Some("literal text projection does not match the observed text".into());
                    } else if context
                        .is_some_and(|context| context.observed_metadata_fields.is_some())
                    {
                        reason = Some(
                            "partial metadata coverage cannot be passed to the extension".into(),
                        );
                    } else {
                        match extensions
                            .map(|host| {
                                host.evaluate(
                                    extension,
                                    parameters,
                                    extension_input.as_ref().unwrap_or(input),
                                )
                            })
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
                ContentGate::MetadataAge {
                    field,
                    as_of,
                    max_days,
                    ..
                } => {
                    measurements.insert("max_days".into(), *max_days);
                    if let Ok(compact) = as_of.replace('-', "").parse::<usize>() {
                        measurements.insert("as_of_yyyymmdd".into(), compact);
                    }
                    let unavailable =
                        context.is_some_and(|ctx| ctx.metadata_field_unavailable(field));
                    if unavailable || input.metadata.is_none() {
                        reason = Some("metadata field is unavailable from this adapter".into());
                    } else if let Some(value) = input
                        .metadata
                        .as_ref()
                        .and_then(|metadata| metadata.get(field))
                    {
                        match (
                            parse_iso_calendar_date(value),
                            parse_iso_calendar_date(as_of),
                        ) {
                            (Some(date), Some(reference)) if date <= reference => {
                                let age = (reference - date).whole_days() as usize;
                                measurements.insert("age_days".into(), age);
                                if age > *max_days {
                                    emit(
                                        format!(
                                            "Metadata date {field:?} is {age} days before {as_of}; review the content and sources"
                                        ),
                                        1,
                                        1,
                                    );
                                }
                            }
                            (Some(_), Some(_)) => {
                                reason = Some("metadata date is after the reference date".into())
                            }
                            _ => {
                                reason = Some(
                                    "metadata date is not a valid YYYY-MM-DD calendar date".into(),
                                )
                            }
                        }
                    } else {
                        reason = Some("metadata date is absent; its age cannot be assessed".into());
                    }
                }
                ContentGate::MetadataLength {
                    field, min, max, ..
                } => {
                    if let Some(min) = min {
                        measurements.insert("min_characters".into(), *min);
                    }
                    if let Some(max) = max {
                        measurements.insert("max_characters".into(), *max);
                    }
                    let unavailable =
                        context.is_some_and(|ctx| ctx.metadata_field_unavailable(field));
                    if unavailable || input.metadata.is_none() {
                        reason = Some("metadata field is unavailable from this adapter".into());
                    } else if let Some(value) = input
                        .metadata
                        .as_ref()
                        .and_then(|metadata| metadata.get(field))
                    {
                        let count = value.chars().count();
                        measurements.insert("character_count".into(), count);
                        if let Some(minimum) = min.filter(|n| count < *n) {
                            emit(
                                format!(
                                    "Metadata {field} length {count} is below the minimum of {minimum}"
                                ),
                                1,
                                1,
                            );
                        }
                        if let Some(maximum) = max.filter(|n| count > *n) {
                            emit(
                                format!(
                                    "Metadata {field} length {count} exceeds the maximum of {maximum}"
                                ),
                                1,
                                1,
                            );
                        }
                    } else {
                        reason =
                            Some("metadata field is absent; its length cannot be assessed".into());
                    }
                }
                ContentGate::RequiredMetadata { fields, .. } => {
                    measurements.insert("required_fields".into(), fields.len());
                    match &input.metadata {
                        Some(metadata) => {
                            let (mut unknown, mut present, mut missing) = (0, 0, 0);
                            for field in fields {
                                if context.is_some_and(|ctx| ctx.metadata_field_unavailable(field))
                                {
                                    unknown += 1;
                                } else if metadata.get(field).is_some_and(|v| !v.trim().is_empty())
                                {
                                    present += 1;
                                } else {
                                    missing += 1;
                                    emit(format!("Required metadata is missing: {field}"), 1, 1);
                                }
                            }
                            measurements.insert("unavailable_fields".into(), unknown);
                            measurements.insert("present_fields".into(), present);
                            measurements.insert("missing_fields".into(), missing);
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
            // An adapter's body limitation only explains missing body capabilities.
            // It must not overwrite independent evidence, structural, or resource errors.
            if matches!(
                reason.as_deref(),
                Some("text is unavailable from this adapter")
                    | Some("paragraphs are unavailable from this adapter")
                    | Some("headings are unavailable from this adapter")
                    | Some("source text is unavailable from this adapter")
            ) && let Some(limit) = context.and_then(|context| context.input_limit.as_ref())
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
    #[test]
    fn list_measurements_cover_all_observations_without_fabricating_unavailable_counts() {
        use crate::markdown_adapter::StructuredList;
        let preset = ContentPreset::from_toml("version=1\nname='lists'\n[[gates]]\nid='list'\nkind='list_item_count'\nflagged_counts=[3,5]\n").unwrap();
        let input = ContentInput {
            id: "article".into(),
            ..Default::default()
        };
        let capabilities = ContentEvaluationContext::default();
        let evaluate = |lists: Option<&[StructuredList]>| {
            preset
                .evaluate_with_structured_lists(
                    &input,
                    &capabilities,
                    None,
                    None,
                    true,
                    None,
                    lists,
                )
                .unwrap()
        };
        let unavailable = evaluate(None);
        assert_eq!(unavailable.gates[0].status, GateStatus::NotChecked);
        assert!(
            !unavailable.gates[0]
                .measurements
                .contains_key("lists_matching_counts")
        );
        assert!(
            !unavailable.gates[0]
                .measurements
                .contains_key("largest_list_items")
        );

        let invalid_lists = [StructuredList {
            item_count: 0,
            depth: 1,
            line: 1,
            column: 1,
        }];
        let invalid = evaluate(Some(&invalid_lists));
        assert_eq!(invalid.gates[0].status, GateStatus::NotChecked);
        assert!(!invalid.gates[0].measurements.contains_key("lists_observed"));
        assert!(
            !invalid.gates[0]
                .measurements
                .contains_key("lists_matching_counts")
        );
        assert!(
            !invalid.gates[0]
                .measurements
                .contains_key("largest_list_items")
        );

        let empty = evaluate(Some(&[]));
        assert_eq!(empty.gates[0].status, GateStatus::Passed);
        assert_eq!(empty.gates[0].measurements["lists_observed"], 0);
        assert_eq!(empty.gates[0].measurements["lists_matching_counts"], 0);
        assert_eq!(empty.gates[0].measurements["largest_list_items"], 0);

        let mut lists = vec![
            StructuredList {
                item_count: 3,
                depth: 1,
                line: 1,
                column: 1
            };
            MAX_FINDINGS_PER_GATE + 3
        ];
        lists.push(StructuredList {
            item_count: 8,
            depth: 1,
            line: 1,
            column: 1,
        });
        let report = evaluate(Some(&lists));
        assert_eq!(report.gates[0].status, GateStatus::Failed);
        assert_eq!(report.gates[0].measurements["lists_observed"], lists.len());
        assert_eq!(
            report.gates[0].measurements["lists_matching_counts"],
            MAX_FINDINGS_PER_GATE + 3
        );
        assert_eq!(report.gates[0].measurements["largest_list_items"], 8);
        assert_eq!(report.gates[0].findings.len(), MAX_FINDINGS_PER_GATE);
        assert_eq!(report.gates[0].total_findings(), MAX_FINDINGS_PER_GATE + 3);
    }

    #[test]
    fn structured_list_gate_requires_complete_observations() {
        let preset = ContentPreset::from_toml("version=1\nname='lists'\n[[gates]]\nid='list'\nkind='list_item_count'\nflagged_counts=[3]\n").unwrap();
        let input = ContentInput {
            id: "article".into(),
            ..Default::default()
        };
        let capabilities = ContentEvaluationContext::default();
        let unavailable = preset
            .evaluate_with_capabilities(&input, &capabilities)
            .unwrap();
        assert_eq!(unavailable.gates[0].status, GateStatus::NotChecked);
        let lists = [crate::markdown_adapter::StructuredList {
            item_count: 3,
            depth: 1,
            line: 1,
            column: 1,
        }];
        let flagged = preset
            .evaluate_with_structured_lists(
                &input,
                &capabilities,
                None,
                None,
                true,
                None,
                Some(&lists),
            )
            .unwrap();
        assert_eq!(flagged.gates[0].status, GateStatus::Failed);
        assert!(!flagged.gates[0].blocking);
        let empty = preset
            .evaluate_with_structured_lists(
                &input,
                &capabilities,
                None,
                None,
                true,
                None,
                Some(&[]),
            )
            .unwrap();
        assert_eq!(empty.gates[0].status, GateStatus::Passed);
        let invalid = [crate::markdown_adapter::StructuredList {
            item_count: 0,
            depth: 1,
            line: 1,
            column: 1,
        }];
        let report = preset
            .evaluate_with_structured_lists(
                &input,
                &capabilities,
                None,
                None,
                true,
                None,
                Some(&invalid),
            )
            .unwrap();
        assert_eq!(report.gates[0].status, GateStatus::NotChecked);
    }

    #[test]
    fn metadata_age_is_advisory_reproducible_and_handles_unknown_dates() {
        let config = "version=1\nname='review'\n[[gates]]\nid='age'\nkind='metadata_age'\nfield='last_reviewed'\nas_of='2024-03-01'\nmax_days=1\n";
        let preset = ContentPreset::from_toml(config).unwrap();
        for invalid in [
            config.replace("2024-03-01", "2023-02-29"),
            config.replace("field='last_reviewed'", "field=' '"),
        ] {
            assert!(ContentPreset::from_toml(&invalid).is_err());
        }
        let mut input = ContentInput {
            id: "article".into(),
            metadata: Some(BTreeMap::new()),
            ..Default::default()
        };
        assert_eq!(
            preset.evaluate(&input).unwrap().gates[0].status,
            GateStatus::NotChecked
        );
        let metadata = input.metadata.as_mut().unwrap();
        metadata.insert("last_reviewed".into(), "2024-02-29".into());
        let pass = preset.evaluate(&input).unwrap();
        assert_eq!(pass.gates[0].status, GateStatus::Passed);
        assert!(!pass.gates[0].blocking);
        assert_eq!(pass.gates[0].measurements["age_days"], 1);
        assert_eq!(pass.gates[0].measurements["as_of_yyyymmdd"], 20240301);
        input
            .metadata
            .as_mut()
            .unwrap()
            .insert("last_reviewed".into(), "2024-02-28".into());
        let failed = preset.evaluate(&input).unwrap();
        assert_eq!(failed.gates[0].status, GateStatus::Failed);
        assert_eq!(failed.gates[0].measurements["age_days"], 2);
        assert!(
            failed.gates[0].findings[0]
                .message
                .contains("review the content and sources")
        );
        let adapter = ContentContext {
            unavailable_metadata_fields: vec!["last_reviewed".into()],
            ..Default::default()
        };
        assert_eq!(
            preset
                .evaluate_with_capabilities(
                    &input,
                    &ContentEvaluationContext {
                        adapter: Some(&adapter),
                        ..Default::default()
                    }
                )
                .unwrap()
                .gates[0]
                .status,
            GateStatus::NotChecked
        );
        for value in [
            "2024-03-02",
            "2023-02-29",
            "2024-02-29T00:00:00Z",
            "",
            "not a date",
        ] {
            input
                .metadata
                .as_mut()
                .unwrap()
                .insert("last_reviewed".into(), value.into());
            let result = preset.evaluate(&input).unwrap();
            assert_eq!(result.gates[0].status, GateStatus::NotChecked, "{value}");
            assert!(result.gates[0].findings.is_empty());
        }
        input.metadata = None;
        assert_eq!(
            preset.evaluate(&input).unwrap().gates[0].status,
            GateStatus::NotChecked
        );
    }

    #[test]
    fn metadata_length_validates_bounds_and_distinguishes_observation_states() {
        let config = "version=1\nname='test'\n[[gates]]\nid='length'\nkind='metadata_length'\nfield='title'\nmin=2\nmax=3\n";
        let preset = ContentPreset::from_toml(config).unwrap();
        for invalid in [
            config.replace("field='title'", "field=' '"),
            config.replace("min=2\nmax=3", ""),
            config.replace("max=3", "max=1"),
        ] {
            assert!(ContentPreset::from_toml(&invalid).is_err());
        }
        let mut input = ContentInput {
            id: "draft".into(),
            metadata: Some(BTreeMap::new()),
            ..Default::default()
        };
        let absent = preset.evaluate(&input).unwrap();
        assert_eq!(absent.gates[0].status, GateStatus::NotChecked);
        assert!(
            absent.gates[0]
                .reason
                .as_deref()
                .unwrap()
                .contains("absent")
        );
        assert!(!absent.gates[0].measurements.contains_key("character_count"));
        input
            .metadata
            .as_mut()
            .unwrap()
            .insert("title".into(), String::new());
        let empty = preset.evaluate(&input).unwrap();
        assert_eq!(empty.gates[0].status, GateStatus::Failed);
        assert_eq!(empty.gates[0].measurements["character_count"], 0);
        input
            .metadata
            .as_mut()
            .unwrap()
            .insert("title".into(), "é🙂".into());
        let passed = preset.evaluate(&input).unwrap();
        assert_eq!(passed.gates[0].status, GateStatus::Passed);
        assert_eq!(passed.gates[0].measurements["character_count"], 2);
        input
            .metadata
            .as_mut()
            .unwrap()
            .insert("title".into(), "abcd".into());
        let failed = preset.evaluate(&input).unwrap();
        assert_eq!(failed.gates[0].status, GateStatus::Failed);
        assert_eq!(failed.gates[0].required_capability, "metadata");
        let adapter = ContentContext {
            unavailable_metadata_fields: vec!["title".into()],
            input_limit: Some("body unavailable".into()),
            ..Default::default()
        };
        let unavailable = preset
            .evaluate_with_capabilities(
                &input,
                &ContentEvaluationContext {
                    adapter: Some(&adapter),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(unavailable.gates[0].status, GateStatus::NotChecked);
        assert!(
            unavailable.gates[0]
                .reason
                .as_deref()
                .unwrap()
                .contains("unavailable")
        );
        assert!(unavailable.gates[0].findings.is_empty());
        input.metadata = None;
        assert_eq!(
            preset.evaluate(&input).unwrap().gates[0].status,
            GateStatus::NotChecked
        );
    }

    #[test]
    fn required_content_measurements_keep_missing_and_unavailable_distinct() {
        let preset = ContentPreset::from_toml("version=1\nname='counts'\n[[gates]]\nid='metadata'\nkind='required_metadata'\nfields=['title','description','author']\n[[gates]]\nid='headings'\nkind='required_headings'\nheadings=['Intro','Summary']\n").unwrap();
        let input = ContentInput {
            id: "example".into(),
            metadata: Some(BTreeMap::from([
                ("title".into(), "Title".into()),
                ("description".into(), " ".into()),
            ])),
            headings: Some(vec!["Intro".into()]),
            ..Default::default()
        };
        let adapter = ContentContext {
            unavailable_metadata_fields: vec!["author".into()],
            ..Default::default()
        };
        let context = ContentEvaluationContext {
            adapter: Some(&adapter),
            ..Default::default()
        };
        let report = preset.evaluate_with_capabilities(&input, &context).unwrap();
        let metadata = &report.gates[0];
        assert_eq!(metadata.status, GateStatus::NotChecked);
        assert_eq!(metadata.measurements["present_fields"], 1);
        assert_eq!(metadata.measurements["missing_fields"], 1);
        assert_eq!(metadata.measurements["unavailable_fields"], 1);
        assert_eq!(metadata.findings.len(), 1);
        assert!(metadata.findings[0].message.contains("description"));
        let headings = &report.gates[1];
        assert_eq!(headings.status, GateStatus::Failed);
        assert_eq!(headings.measurements["present_headings"], 1);
        assert_eq!(headings.measurements["missing_headings"], 1);
        assert_eq!(headings.findings.len(), 1);

        let unavailable = preset
            .evaluate(&ContentInput {
                id: "none".into(),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(unavailable.gates[0].status, GateStatus::NotChecked);
        assert!(
            !unavailable.gates[0]
                .measurements
                .contains_key("missing_fields")
        );
        assert_eq!(unavailable.gates[1].status, GateStatus::NotChecked);
        assert!(
            !unavailable.gates[1]
                .measurements
                .contains_key("missing_headings")
        );
    }

    #[test]
    fn editorial_findings_carry_advisory_suggestions_without_changing_coverage() {
        let preset = ContentPreset::from_toml(
            "version=1\nname='advice'\n[[gates]]\nid='length'\nkind='word_count'\nmin=10\n[[gates]]\nid='terms'\nkind='forbidden_terms'\nterms=['bad']\n[[gates]]\nid='heading'\nkind='required_headings'\nheadings=['Intro']\n",
        ).unwrap();
        let report = preset.evaluate(&literal_input("A bad sentence.")).unwrap();
        assert_eq!(report.gates[0].status, GateStatus::Failed);
        assert_eq!(report.gates[1].status, GateStatus::Failed);
        assert_eq!(report.gates[2].status, GateStatus::NotChecked);
        assert!(report.gates[2].findings.is_empty());
        assert!(
            report.gates[0].findings[0]
                .suggestion
                .as_deref()
                .unwrap()
                .contains("configured limits")
        );
        assert!(
            report.gates[1].findings[0]
                .suggestion
                .as_deref()
                .unwrap()
                .contains("in context")
        );
        let value = serde_json::to_value(&report).unwrap();
        assert!(
            value["gates"][1]["findings"][0]["suggestion"]
                .as_str()
                .unwrap()
                .contains("configured policy")
        );
    }

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
    fn guarded_sentence_word_counts_report_unsupported_scripts_without_losing_findings() {
        let preset = |guard: bool| {
            ContentPreset::from_toml(&format!(
            "version=1\nname='t'\n[[gates]]\nid='g'\nkind='sentence_length'\nmax_words=2\nrequire_latin_word_boundaries={guard}\n"
        )).unwrap()
        };
        let result = preset(true)
            .evaluate(&literal_input("One long sentence. 这是中文。"))
            .unwrap();
        let gate = &result.gates[0];
        assert_eq!(gate.status, GateStatus::NotChecked);
        assert_eq!(gate.findings.len(), 1);
        assert_eq!(gate.measurements["sentence_count"], 1);
        assert_eq!(
            gate.measurements["sentences_with_unverified_word_boundaries"],
            1
        );
        assert!(gate.reason.as_deref().unwrap().contains("unverified"));

        let all_latin = preset(true)
            .evaluate(&literal_input("Une phrase française."))
            .unwrap();
        assert_eq!(all_latin.gates[0].status, GateStatus::Failed);
        assert_eq!(
            all_latin.gates[0].measurements["sentences_with_unverified_word_boundaries"],
            0
        );

        let legacy = preset(false)
            .evaluate(&literal_input("这是中文。"))
            .unwrap();
        assert_eq!(legacy.gates[0].status, GateStatus::Passed);
        assert!(
            !legacy.gates[0]
                .measurements
                .contains_key("sentences_with_unverified_word_boundaries")
        );

        let missing = preset(true)
            .evaluate(&ContentInput {
                id: "missing".into(),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(missing.gates[0].status, GateStatus::NotChecked);
    }

    #[test]
    fn readability_starter_exposes_unsupported_word_boundaries() {
        let preset = ContentPreset::from_toml(include_str!("../presets/readability.toml")).unwrap();
        let report = preset.evaluate(&literal_input("هذه جملة عربية.")).unwrap();
        let gate = &report.gates[0];
        assert_eq!(gate.status, GateStatus::NotChecked);
        assert_eq!(
            gate.measurements["sentences_with_unverified_word_boundaries"],
            1
        );
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
    fn sentence_measurements_keep_http_urls_inside_their_sentence() {
        let preset = ContentPreset::from_toml(
            "version=1\nname='t'\n[[gates]]\nid='g'\nkind='sentence_length'\nmax_words=20\n",
        )
        .unwrap();
        for text in [
            "See https://example.org/a.b?x=1&y=2. Next sentence.",
            "Read (https://www.example.org/v1.2). Next sentence!",
            "Voir https://exemple.fr/école. Suite.",
            "Read HTTP://example.org/a.b?x=1. Next sentence.",
            "See HtTpS://example.org/a.b?x=1. Next sentence.",
        ] {
            let report = preset.evaluate(&literal_input(text)).unwrap();
            assert_eq!(report.gates[0].measurements["sentence_count"], 2, "{text}");
        }
    }

    #[test]
    fn sentence_urls_after_punctuation_and_at_new_sentence_boundaries() {
        let preset = ContentPreset::from_toml(
            "version=1\nname='t'\n[[gates]]\nid='g'\nkind='sentence_length'\nmax_words=20\n",
        )
        .unwrap();
        for text in [
            "See:https://example.org/v1.2?x=1. Next sentence.",
            "See—https://example.org/v1.2?x=1. Next sentence.",
            "رابط،https://example.org/v1.2?x=1. Next sentence.",
            "First.https://example.org/v1.2?x=1. Next sentence.",
        ] {
            let expected = if text.starts_with("First.") { 3 } else { 2 };
            let report = preset.evaluate(&literal_input(text)).unwrap();
            assert_eq!(
                report.gates[0].measurements["sentence_count"], expected,
                "{text}"
            );
        }
        assert_eq!(
            http_url_end("wordhttps://example.org", 4, None, false, ".!?"),
            None
        );
    }

    #[test]
    fn sentence_url_respects_configured_non_ascii_terminators() {
        for (terminator, following) in [
            ('。', "下一句"),
            ('！', "下一句"),
            ('？', "下一句"),
            ('؟', "التالي"),
        ] {
            let text = format!("Read https://example.org/v1.2{terminator}{following}.");
            let mut sentences = Vec::new();
            for_each_sentence(
                &text,
                1,
                1,
                &[],
                ".!?。！？؟",
                &[],
                |segment, line, column| {
                    if !segment.trim().is_empty() {
                        sentences.push((segment.trim().to_owned(), line, column));
                    }
                },
            );
            assert_eq!(sentences.len(), 2, "{text}");
            assert_eq!(sentences[0].0, "Read https://example.org/v1.2", "{text}");
            assert_eq!(sentences[1].0, following, "{text}");
            assert_eq!((sentences[1].1, sentences[1].2), (1, 31), "{text}");
            let preset = ContentPreset::from_toml(
                "version=1\nname='t'\n[[gates]]\nid='g'\nkind='sentence_character_length'\nmax_chars=1\ndelimiters='.!?。！？؟'\n",
            ).unwrap();
            let report = preset.evaluate(&literal_input(&text)).unwrap();
            assert_eq!(report.gates[0].measurements["sentence_count"], 2, "{text}");
            assert_eq!(
                (
                    report.gates[0].findings[1].line,
                    report.gates[0].findings[1].column
                ),
                (1, 31),
                "{text}"
            );
        }
    }

    #[test]
    fn sentence_url_keeps_unconfigured_and_encoded_terminators() {
        let literal = "https://example.org/資料。suffix";
        assert_eq!(
            http_url_end(literal, 0, None, false, ".!?"),
            Some(literal.len())
        );
        let international = "https://例子。测试/路径";
        assert_eq!(
            http_url_end(international, 0, None, false, ".!?。"),
            Some(international.len())
        );
        let encoded = "https://example.org/%E3%80%82suffix";
        assert_eq!(
            http_url_end(encoded, 0, None, false, ".!?。"),
            Some(encoded.len())
        );
    }

    #[test]
    fn sentence_segmentation_preserves_supported_decimal_digits_and_positions() {
        for number in ["3.14", "٣.١٤", "۳.۱۴", "3.١۴"] {
            let text = format!("Value {number} units. Next sentence.");
            let mut sentences = Vec::new();
            for_each_sentence(&text, 1, 1, &[], ".!?", &[], |segment, line, column| {
                if !segment.trim().is_empty() {
                    sentences.push((segment.trim().to_owned(), line, column));
                }
            });
            assert_eq!(
                sentences,
                vec![
                    (format!("Value {number} units"), 1, 1),
                    ("Next sentence".into(), 1, 19)
                ],
                "{number}"
            );
            let preset = ContentPreset::from_toml(
                "version=1\nname='t'\n[[gates]]\nid='g'\nkind='sentence_character_length'\nmax_chars=5\n",
            ).unwrap();
            let report = preset.evaluate(&literal_input(&text)).unwrap();
            assert_eq!(
                report.gates[0].measurements["sentence_count"], 2,
                "{number}"
            );
            assert_eq!(report.gates[0].findings.len(), 2, "{number}");
            assert_eq!(
                (
                    report.gates[0].findings[1].line,
                    report.gates[0].findings[1].column
                ),
                (1, 19)
            );
        }
    }

    #[test]
    fn http_url_segmentation_is_bounded_and_preserves_sentence_positions() {
        let text = "See https://example.org/v1.2?key=value. Next sentence.";
        let mut observed = Vec::new();
        for_each_sentence(text, 1, 1, &[], ".!?", &[], |sentence, line, column| {
            if !sentence.trim().is_empty() {
                observed.push((sentence.trim().to_string(), line, column));
            }
        });
        assert_eq!(
            observed,
            vec![
                ("See https://example.org/v1.2?key=value".into(), 1, 1),
                ("Next sentence".into(), 1, 41),
            ]
        );
        let url = "https://example.org/page";
        assert_eq!(http_url_end(url, 0, None, false, ".!?"), Some(url.len()));
        let mixed = "HtTpS://example.org/page";
        assert_eq!(
            http_url_end(mixed, 0, None, false, ".!?"),
            Some(mixed.len())
        );
        assert_eq!(http_url_end("HTTPS://", 0, None, false, ".!?"), None);
        assert_eq!(
            http_url_end("wordHTTP://example.org", 4, None, false, ".!?"),
            None
        );
        assert_eq!(http_url_end(url, 0, Some(15), false, ".!?"), Some(15));
        assert_eq!(http_url_end("https://", 0, None, false, ".!?"), None);
        assert_eq!(
            http_url_end("wordhttps://example.org", 4, None, false, ".!?"),
            None
        );
        assert_eq!(
            http_url_end(
                &format!("https://example.org/{}", "x".repeat(2048)),
                0,
                None,
                false,
                ".!?"
            ),
            None
        );
        let joined = "Firsthttps://example.org/a.b?x=1. Next.";
        let mut count = 0;
        for_each_sentence(joined, 1, 1, &[5, 5], ".!?", &[], |sentence, _, _| {
            count += usize::from(!sentence.trim().is_empty());
        });
        assert_eq!(count, 3);
    }

    #[test]
    fn configured_abbreviations_after_opening_punctuation_stay_in_sentence() {
        let abbreviations = vec!["Dr.".to_owned(), "e.g.".to_owned()];
        for text in [
            "She said: (Dr. Martin spoke). Next.",
            "She said: «Dr. Martin spoke». Next.",
            "She said: “Dr. Martin spoke”. Next.",
            "She said: \"Dr. Martin spoke\". Next.",
            "Examples (e.g. this one) help. Next.",
            "Examples «e.g. this one» help. Next.",
        ] {
            let mut sentences = Vec::new();
            for_each_sentence(text, 1, 1, &[], ".!?", &abbreviations, |s, line, column| {
                if !s.trim().is_empty() {
                    sentences.push((s.trim().to_owned(), line, column));
                }
            });
            assert_eq!(sentences.len(), 2, "{text}: {sentences:?}");
            assert_eq!(sentences[1].0, "Next", "{text}");
            assert_eq!(
                sentences[1].2,
                text.chars().position(|c| c == 'N').unwrap() + 1,
                "{text}"
            );
        }
        let preset = ContentPreset::from_toml(
            "version=1\nname='t'\n[[gates]]\nid='g'\nkind='sentence_length'\nmax_words=2\nabbreviations=['Dr.']\n",
        ).unwrap();
        let report = preset
            .evaluate(&literal_input("She said: (Dr. Martin spoke). Next."))
            .unwrap();
        assert_eq!(report.gates[0].measurements["sentence_count"], 2);
        assert_eq!(
            (
                report.gates[0].findings[0].line,
                report.gates[0].findings[0].column
            ),
            (1, 1)
        );
        let mut count = 0;
        for_each_sentence(
            "NotDr. Next.",
            1,
            1,
            &[],
            ".!?",
            &abbreviations,
            |s, _, _| {
                count += usize::from(!s.trim().is_empty());
            },
        );
        assert_eq!(
            count, 2,
            "an abbreviation suffix inside a word is not a token"
        );
    }

    #[test]
    fn configured_multi_period_abbreviation_before_closing_quote_stays_in_sentence() {
        let abbreviations = vec!["e.g.".to_owned()];
        for text in [
            "Use \"e.g.\", then explain. Next.",
            "Use 'e.g.', then explain. Next.",
            "Use ‘e.g.’, then explain. Next.",
            "Use “e.g.”, then explain. Next.",
        ] {
            let mut sentences = Vec::new();
            for_each_sentence(text, 1, 1, &[], ".!?", &abbreviations, |segment, _, _| {
                if !segment.trim().is_empty() {
                    sentences.push(segment.trim().to_owned());
                }
            });
            assert_eq!(sentences.len(), 2, "{text}: {sentences:?}");
            assert_eq!(sentences[1], "Next", "{text}");
        }
    }

    #[test]
    fn configured_multi_period_abbreviations_preserve_sentence_boundaries() {
        for kind in ["sentence_length", "sentence_character_length"] {
            let limit = if kind == "sentence_length" {
                "max_words=2"
            } else {
                "max_chars=10"
            };
            let preset = ContentPreset::from_toml(&format!(
                "version=1\nname='t'\n[[gates]]\nid='g'\nkind='{kind}'\n{limit}\nabbreviations=['e.g.', 'U.S.', 'é.g.']\n"
            )).unwrap();
            for text in [
                "First e.g. example. Next.",
                "First e.g., example. Next.",
                "First U.S. example. Next.",
                "First é.g. example. Next.",
            ] {
                let report = preset.evaluate(&literal_input(text)).unwrap();
                assert_eq!(
                    report.gates[0].measurements["sentence_count"], 2,
                    "{kind}: {text}"
                );
                assert_eq!(
                    (
                        report.gates[0].findings[0].line,
                        report.gates[0].findings[0].column
                    ),
                    (1, 1)
                );
            }
            let unicode = preset.evaluate(&literal_input("é. Next.")).unwrap();
            assert_eq!(unicode.gates[0].measurements["sentence_count"], 2);
            let partial = preset
                .evaluate(&literal_input("First e.g.fake. Next."))
                .unwrap();
            assert_eq!(partial.gates[0].measurements["sentence_count"], 4);
            let context = ContentContext {
                block_ends: Some(vec![8]),
                ..Default::default()
            };
            let split = preset
                .evaluate_with_context(
                    &literal_input("First e.g. example. Next."),
                    None,
                    None,
                    Some(&context),
                )
                .unwrap();
            assert!(
                split.gates[0].measurements["sentence_count"] > 2,
                "a token spanning adapter blocks must not suppress boundaries"
            );
        }
        let too_many_periods = "e.".repeat(17);
        let too_many_bytes = format!("{}.x.", "a".repeat(126));
        for invalid in [
            "e..g.",
            ".eg.",
            "e.g",
            "e. g.",
            &too_many_periods,
            &too_many_bytes,
        ] {
            let source = format!(
                "version=1\nname='t'\n[[gates]]\nid='g'\nkind='sentence_length'\nmax_words=2\nabbreviations=['{invalid}']\n"
            );
            assert!(ContentPreset::from_toml(&source).is_err(), "{invalid}");
        }
    }

    #[test]
    fn sentence_gates_do_not_treat_closing_quotes_as_sentences() {
        for (kind, limit) in [
            ("sentence_length", "max_words=1"),
            ("sentence_character_length", "max_chars=4"),
        ] {
            let preset = ContentPreset::from_toml(&format!(
                "version=1\nname='t'\n[[gates]]\nid='g'\nkind='{kind}'\n{limit}\n"
            ))
            .unwrap();
            for (text, expected) in [
                ("« Oui ! »", 1),
                ("“Yes!”", 1),
                ("(Yes.)", 1),
                ("Yes. \"Long words\"", 2),
                ("« Oui ! » Long words.", 2),
            ] {
                let report = preset.evaluate(&literal_input(text)).unwrap();
                assert_eq!(
                    report.gates[0].measurements["sentence_count"], expected,
                    "{kind}: {text}"
                );
            }
            if kind == "sentence_length" {
                let short = preset.evaluate(&literal_input("« Oui ! »")).unwrap();
                assert_eq!(short.gates[0].measurements["longest_sentence_words"], 1);
                assert!(short.gates[0].findings.is_empty());
            }
            let report = preset
                .evaluate(&literal_input("« Oui ! » Long words."))
                .unwrap();
            let finding = report.gates[0].findings.last().unwrap();
            assert_eq!((finding.line, finding.column), (1, 11), "{kind}");
        }
    }

    #[test]
    fn paragraph_only_sentence_closers_preserve_original_coordinates() {
        let preset = ContentPreset::from_toml("version=1\nname='t'\n[[gates]]\nid='g'\nkind='sentence_length'\nmax_words=1\nparagraphs_only=true\n").unwrap();
        let paragraph = "« Oui ! » Long words.";
        let context = ContentContext {
            paragraphs: Some(vec![ContentParagraph {
                text: paragraph.into(),
                line: 7,
                column: 4,
            }]),
            ..Default::default()
        };
        let report = preset
            .evaluate_with_context(&literal_input(paragraph), None, None, Some(&context))
            .unwrap();
        assert_eq!(report.gates[0].measurements["sentence_count"], 2);
        assert_eq!(report.gates[0].findings.len(), 1);
        assert_eq!(
            (
                report.gates[0].findings[0].line,
                report.gates[0].findings[0].column
            ),
            (7, 14)
        );
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
    fn whole_word_gates_do_not_split_combining_marks() {
        let preset = ContentPreset::from_toml("version=1\nname='t'\n[[gates]]\nid='forbidden'\nkind='forbidden_terms'\nterms=['cafe']\n[[gates]]\nid='required'\nkind='required_terms'\nterms=['cafe']\n[[gates]]\nid='repeated'\nkind='repeated_terms'\nterms=['cafe']\nmax_occurrences=1\n").unwrap();
        let report = preset
            .evaluate(&literal_input("cafe\u{301} cafe\u{301}"))
            .unwrap();
        assert_eq!(report.gates[0].status, GateStatus::Passed);
        assert_eq!(report.gates[1].status, GateStatus::Failed);
        assert_eq!(report.gates[2].status, GateStatus::Passed);
        let literal = LiteralText::new("cafe\u{301} cafe", &[], false);
        let mut positions = Vec::new();
        literal.visit_matches("cafe", true, |offset| {
            positions.push(offset);
            false
        });
        assert_eq!(positions, ["cafe\u{301} ".len()]);
        assert!(literal.visit_matches("cafe\u{301}", true, |_| true));
        assert!(literal.visit_matches("cafe", false, |_| true));
        assert!(!LiteralText::new("İ", &[], true).visit_matches("i", true, |_| true));
    }

    #[test]
    fn composed_literal_gates_match_individual_results_with_mixed_modes() {
        let preset = ContentPreset::from_toml("version=1\nname='test'\n[[gates]]\nid='first'\nkind='forbidden_terms'\nterms=['cafe']\ncase_insensitive=true\n[[gates]]\nid='second'\nkind='required_terms'\nterms=['cafe']\ncase_insensitive=true\n[[gates]]\nid='third'\nkind='repeated_terms'\nterms=['Cafe']\nmax_occurrences=1\n[[gates]]\nid='fourth'\nkind='required_terms'\nterms=['cafe']\ncase_insensitive=true\n").unwrap();
        let input = literal_input("cafe\u{301} CAFE Cafe Cafe");
        let context = ContentContext {
            block_ends: Some(vec!["cafe\u{301} CAFE ".len()]),
            ..Default::default()
        };
        let all = preset
            .evaluate_with_context(&input, None, None, Some(&context))
            .unwrap();
        for (index, gate) in preset.gates.iter().enumerate() {
            let individual = ContentPreset {
                version: preset.version,
                name: preset.name.clone(),
                gates: vec![gate.clone()],
            };
            let result = individual
                .evaluate_with_context(&input, None, None, Some(&context))
                .unwrap();
            assert_eq!(
                serde_json::to_value(&all.gates[index]).unwrap(),
                serde_json::to_value(&result.gates[0]).unwrap()
            );
        }
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
    fn externally_supplied_finding_counts_do_not_overflow() {
        let mut result = forbidden("terms=['x']", "x");
        result
            .measurements
            .insert(FINDINGS_OMITTED.into(), usize::MAX);
        assert_eq!(result.total_findings(), usize::MAX);
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
    fn character_counts_are_configurable_unicode_scalars_and_distinguish_missing() {
        let preset = ContentPreset::from_toml(
            "version=1\nname='chars'\n[[gates]]\nid='all'\nkind='character_count'\nmin=5\nmax=5\n[[gates]]\nid='compact'\nkind='character_count'\nmin=4\nmax=4\nignore_whitespace=true\n"
        ).unwrap();
        let input = literal_input("你好 é\u{301}");
        let report = preset.evaluate(&input).unwrap();
        assert_eq!(report.gates[0].status, GateStatus::Passed);
        assert_eq!(report.gates[0].measurements["character_count"], 5);
        assert_eq!(report.gates[1].status, GateStatus::Passed);
        assert_eq!(report.gates[1].measurements["character_count"], 4);
        let long = preset.evaluate(&literal_input("你好 é\u{301}!")).unwrap();
        assert_eq!(long.gates[0].status, GateStatus::Failed);
        assert!(long.gates[0].findings[0].message.contains("exceeds"));
        assert_eq!(long.gates[1].status, GateStatus::Failed);
        let empty = preset.evaluate(&literal_input("")).unwrap();
        assert_eq!(empty.gates[0].status, GateStatus::Failed);
        assert_eq!(empty.gates[0].measurements["character_count"], 0);
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
        assert!(
            missing
                .gates
                .iter()
                .all(|gate| !gate.measurements.contains_key("character_count"))
        );
        for bounds in ["", "min=5\nmax=4\n"] {
            assert!(
                ContentPreset::from_toml(&format!(
                    "version=1\nname='x'\n[[gates]]\nid='x'\nkind='character_count'\n{bounds}"
                ))
                .is_err()
            );
        }
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
    fn partial_metadata_never_reaches_external_host() {
        struct NoCall;
        impl ContentGateExtensions for NoCall {
            fn evaluate(
                &self,
                _: &str,
                _: &BTreeMap<String, String>,
                _: &ContentInput,
            ) -> Result<Option<Vec<ExtensionDiagnostic>>> {
                panic!("partial metadata reached extension host")
            }
        }
        let preset = ContentPreset::from_toml(
            "version=1\nname='t'\n[[gates]]\nid='external'\nkind='custom'\nextension='test'\n",
        )
        .unwrap();
        let context = ContentContext {
            observed_metadata_fields: Some(BTreeSet::new()),
            ..Default::default()
        };
        let report = preset
            .evaluate_with_context(&literal_input("abc"), None, Some(&NoCall), Some(&context))
            .unwrap();
        assert_eq!(report.gates[0].status, GateStatus::NotChecked);
    }

    #[test]
    fn invalid_literal_projection_never_reaches_external_host() {
        struct NoCall;
        impl ContentGateExtensions for NoCall {
            fn evaluate(
                &self,
                _: &str,
                _: &BTreeMap<String, String>,
                _: &ContentInput,
            ) -> Result<Option<Vec<ExtensionDiagnostic>>> {
                panic!("inconsistent projection reached extension host")
            }
        }
        let preset = ContentPreset::from_toml(
            "version=1\nname='t'\n[[gates]]\nid='external'\nkind='custom'\nextension='test'\n",
        )
        .unwrap();
        let capabilities = ContentEvaluationContext {
            extensions: Some(&NoCall),
            ..Default::default()
        };
        let report = preset
            .evaluate_with_literal_projection(
                &literal_input("abc"),
                &capabilities,
                None,
                None,
                true,
                None,
                None,
                None,
                Some("xyz"),
            )
            .unwrap();
        assert_eq!(report.gates[0].status, GateStatus::NotChecked);
    }

    #[test]
    fn typographic_reference_masks_preserve_offsets_and_literal_validation() {
        let label = "[1–3,\u{202f}5]";
        let mut projected = String::from("Before.");
        let mut view = ReferenceLiteralProjection::default();
        view.push_masked(&mut projected, label);
        projected.push_str(" Next.");
        let literal = view.finish(&projected).unwrap();
        assert_eq!(projected.len(), literal.len());
        assert_eq!(projected.chars().count(), literal.chars().count());
        assert!(literal_projection_matches(&projected, &literal));
        assert!(!literal_projection_matches(
            &projected,
            &literal.replace("Next", "Nest")
        ));
        assert!(!numeric_reference_marker(
            "#reference-proof",
            "[source 1–3]"
        ));
        assert!(!numeric_reference_marker("/ordinary", label));
    }

    #[test]
    fn invalid_literal_projection_is_not_checked() {
        let preset = ContentPreset::from_toml("version=1\nname='t'\n[[gates]]\nid='required'\nkind='required_terms'\nterms=['xyz']\n[[gates]]\nid='forbidden'\nkind='forbidden_terms'\nterms=['xyz']\n").unwrap();
        let report = preset
            .evaluate_with_literal_projection(
                &literal_input("abc"),
                &ContentEvaluationContext::default(),
                None,
                None,
                true,
                None,
                None,
                None,
                Some("xyz"),
            )
            .unwrap();
        assert!(
            report
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
    fn collection_gate_rejects_ambiguous_item_identities_for_direct_hosts() {
        let preset = ContentPreset::from_toml("version=1\nname='collection'\n[[gates]]\nid='unique'\nkind='unique_collection_metadata'\nfield='slug'\n").unwrap();
        let item = |id: &str| ContentInput {
            id: id.into(),
            metadata: Some(BTreeMap::from([("slug".into(), "same".into())])),
            ..Default::default()
        };
        let input = ContentInput::default();
        for items in [vec![item("/a"), item("/a")], vec![item("  ")]] {
            let result = preset.evaluate_with_capabilities(
                &input,
                &ContentEvaluationContext {
                    collection: Some(&items),
                    ..Default::default()
                },
            );
            assert!(
                result.is_err(),
                "ambiguous collection identities must not yield findings"
            );
        }
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
    fn heading_hierarchy_requires_complete_levels_and_respects_fragment_context() {
        let preset = ContentPreset::from_toml("version=1\nname='headings'\n[[gates]]\nid='hierarchy'\nkind='heading_hierarchy'\nblocking=false\n").unwrap();
        let input = ContentInput {
            headings: Some(vec!["One".into(), "Two".into(), "Three".into()]),
            ..Default::default()
        };
        let context = ContentEvaluationContext::default();
        assert_eq!(
            preset.evaluate(&input).unwrap().gates[0].status,
            GateStatus::NotChecked
        );
        let evaluate = |levels: &[u8]| {
            preset
                .evaluate_with_heading_levels(&input, &context, None, Some(levels), true, None)
                .unwrap()
        };
        assert_eq!(evaluate(&[2, 3, 2]).gates[0].status, GateStatus::Passed);
        let report = evaluate(&[2, 4, 2]);
        assert_eq!(report.gates[0].status, GateStatus::Failed);
        assert_eq!(report.gates[0].measurements["skipped_levels"], 1);
        assert_eq!(report.gates[0].measurements["first_heading_level"], 2);
        assert_eq!(report.gates[0].measurements["deepest_heading_level"], 4);
        assert!(!report.gates[0].measurements.contains_key("initial_level"));
        assert!(report.gates[0].findings[0].message.contains("#2"));
        assert_eq!(evaluate(&[2, 4]).gates[0].status, GateStatus::NotChecked);
        assert_eq!(evaluate(&[2, 7, 2]).gates[0].status, GateStatus::NotChecked);
        let fragment = ContentPreset::from_toml("version=1\nname='fragment'\n[[gates]]\nid='hierarchy'\nkind='heading_hierarchy'\ninitial_level=1\n").unwrap();
        assert_eq!(
            fragment
                .evaluate_with_heading_levels(&input, &context, None, Some(&[3, 3, 2]), true, None)
                .unwrap()
                .gates[0]
                .status,
            GateStatus::Failed
        );
        let unknown = fragment.evaluate(&input).unwrap();
        assert_eq!(unknown.gates[0].measurements["initial_level"], 1);
        assert!(
            !unknown.gates[0]
                .measurements
                .contains_key("first_heading_level")
        );
        assert!(
            !unknown.gates[0]
                .measurements
                .contains_key("deepest_heading_level")
        );
        let empty = ContentInput {
            headings: Some(vec![]),
            ..Default::default()
        };
        let empty_report = fragment
            .evaluate_with_heading_levels(&empty, &context, None, Some(&[]), true, None)
            .unwrap();
        assert_eq!(empty_report.gates[0].measurements["heading_count"], 0);
        assert!(
            !empty_report.gates[0]
                .measurements
                .contains_key("first_heading_level")
        );
        assert!(
            !empty_report.gates[0]
                .measurements
                .contains_key("deepest_heading_level")
        );
        assert!(ContentPreset::from_toml("version=1\nname='bad'\n[[gates]]\nid='h'\nkind='heading_hierarchy'\ninitial_level=7\n").is_err());
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
    fn destination_gate_rejects_backslashes_and_invalid_percent_escapes() {
        let preset = ContentPreset::from_toml("version=1\nname='links'\n[[gates]]\nid='links'\nkind='link_destinations'\nallowed_schemes=['https']\n").unwrap();
        let input = ContentInput::default();
        let destinations: Vec<String> = [
            r"/\evil.example/path",
            r"https://example.org\path",
            "/partial%2",
            "/bad%zz",
            "/?discount=100%",
            "/good%20path",
            "https://example.org/%C3%A9",
            "/été",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        let result = preset
            .evaluate_with_destinations(
                &input,
                &ContentEvaluationContext::default(),
                Some(&destinations),
            )
            .unwrap();
        assert_eq!(result.gates[0].status, GateStatus::Failed);
        assert_eq!(result.gates[0].measurements["checked_destinations"], 8);
        assert_eq!(result.gates[0].measurements["invalid_destinations"], 5);
        assert!(result.gates[0].findings[0].message.contains("backslash"));
        assert!(result.gates[0].findings[2].message.contains("percent"));
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
    fn collection_uniqueness_limits_preserve_findings_and_report_incomplete() {
        let preset = ContentPreset::from_toml("version=1\nname='test'\n[[gates]]\nid='unique'\nkind='unique_collection_metadata'\nfield='slug'\ngroup_by='lang'\n").unwrap();
        let root = ContentInput {
            id: "collection".into(),
            ..Default::default()
        };
        let item = |id: String, slug: String, lang: String| ContentInput {
            id,
            metadata: Some(BTreeMap::from([
                ("slug".into(), slug),
                ("lang".into(), lang),
            ])),
            ..Default::default()
        };
        let evaluate = |items: &[ContentInput]| {
            preset
                .evaluate_with_capabilities(
                    &root,
                    &ContentEvaluationContext {
                        collection: Some(items),
                        ..Default::default()
                    },
                )
                .unwrap()
                .gates
                .remove(0)
        };
        let mut items = vec![
            item("first".into(), "same".into(), "en".into()),
            item("duplicate".into(), "same".into(), "en".into()),
        ];
        items.push(item(
            "oversized".into(),
            "other".into(),
            "x".repeat(MAX_COLLECTION_VALUE_BYTES + 1),
        ));
        let result = evaluate(&items);
        assert_eq!(result.status, GateStatus::NotChecked);
        assert_eq!(result.findings[0].path, "duplicate");
        assert_eq!(result.measurements["items_evaluated"], 2);
        assert!(result.reason.unwrap().contains("byte limit"));

        items.pop();
        items.extend(
            (1..=MAX_COLLECTION_KEYS).map(|i| item(i.to_string(), i.to_string(), "en".into())),
        );
        let at_limit = evaluate(&items[..MAX_COLLECTION_KEYS + 1]);
        assert_eq!(at_limit.status, GateStatus::Failed);
        assert_eq!(
            at_limit.measurements["items_evaluated"],
            MAX_COLLECTION_KEYS + 1
        );
        let result = evaluate(&items);
        assert_eq!(result.status, GateStatus::NotChecked);
        assert_eq!(result.findings[0].path, "duplicate");
        assert_eq!(
            result.measurements["items_evaluated"],
            MAX_COLLECTION_KEYS + 1
        );
        assert!(result.reason.unwrap().contains("distinct"));

        let long_values: Vec<_> = (0..260)
            .map(|i| {
                item(
                    i.to_string(),
                    format!("{i:04}{}", "x".repeat(MAX_COLLECTION_VALUE_BYTES - 4)),
                    "en".into(),
                )
            })
            .collect();
        let result = evaluate(&long_values);
        assert_eq!(result.status, GateStatus::NotChecked);
        assert!(result.measurements["items_evaluated"] < long_values.len());
        assert!(result.reason.unwrap().contains("distinct"));

        let mut repeated: Vec<_> = (0..=MAX_COLLECTION_ITEMS)
            .map(|id| item(id.to_string(), "same".into(), "en".into()))
            .collect();
        // The unexamined suffix is not validated; the result must remain unchecked.
        repeated[MAX_COLLECTION_ITEMS].id = "0".into();
        let result = evaluate(&repeated);
        assert_eq!(result.status, GateStatus::NotChecked);
        assert_eq!(result.measurements["items_evaluated"], MAX_COLLECTION_ITEMS);
        assert_eq!(
            result.measurements["duplicate_items"],
            MAX_COLLECTION_ITEMS - 1
        );
        assert!(result.reason.unwrap().contains("too many collection items"));
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
    fn evidence_review_age_is_reproducible_bound_and_advisory() {
        let config = "version=1\nname='review-age'\n[[gates]]\nid='age'\nkind='evidence_review_age'\nsource='s'\nreview='r'\nclaim='Count is 14.'\nas_of='2024-03-01'\nmax_days=1\n";
        let preset = ContentPreset::from_toml(config).unwrap();
        for invalid in [
            config.replace("2024-03-01", "2023-02-29"),
            config.replace("review='r'", "review='s'"),
            config.replace("claim='Count is 14.'", "claim=' '"),
        ] {
            assert!(ContentPreset::from_toml(&invalid).is_err());
        }
        let evaluate = |date: Option<&str>, source: &str, text: Option<&str>| {
            let mut record = serde_json::json!({"claim":"Count is 14.", "source_text":"source", "support":"supported", "reliability":"reliable", "reviewer":"editor", "provenance":{}});
            if let Some(date) = date {
                record["provenance"]["reviewed_at"] = date.into();
            }
            let input = ContentInput {
                text: text.map(str::to_owned),
                evidence_sources: Some(BTreeMap::from([
                    ("s".into(), source.into()),
                    ("r".into(), record.to_string()),
                ])),
                ..Default::default()
            };
            preset.evaluate(&input).unwrap().gates.remove(0)
        };
        let fresh = evaluate(Some("2024-02-29"), "source", Some("Count is 14."));
        assert_eq!(fresh.status, GateStatus::Passed);
        assert!(!fresh.blocking);
        assert_eq!(fresh.measurements["age_days"], 1);
        assert_eq!(fresh.measurements["as_of_yyyymmdd"], 20240301);
        let stale = evaluate(Some("2024-02-28"), "source", Some("Count is 14."));
        assert_eq!(stale.status, GateStatus::Failed);
        assert_eq!(stale.measurements["age_days"], 2);
        assert!(stale.findings[0].message.contains("reassess"));
        for (date, source, text) in [
            (None, "source", Some("Count is 14.")),
            (Some("2024-02-30"), "source", Some("Count is 14.")),
            (Some("2024-03-02"), "source", Some("Count is 14.")),
            (Some("2024-02-29"), "changed", Some("Count is 14.")),
            (Some("2024-02-29"), "source", Some("different")),
            (Some("2024-02-29"), "source", None),
        ] {
            let result = evaluate(date, source, text);
            assert_eq!(result.status, GateStatus::NotChecked, "{date:?}");
            assert!(result.findings.is_empty());
        }
        let ambiguous = ContentInput {
            text: Some("Count is 14.".into()),
            evidence_sources: Some(BTreeMap::from([
                ("s".into(), "source".into()),
                ("r".into(), r#"{"claim":"Count is 14.","source_text":"source","support":"supported","reliability":"reliable","reviewer":"editor","provenance":{"reviewed_at":"2024-02-28","reviewed_at":"2024-02-29"}}"#.into()),
            ])),
            ..Default::default()
        };
        let result = preset.evaluate(&ambiguous).unwrap().gates.remove(0);
        assert_eq!(result.status, GateStatus::NotChecked);
        assert!(result.findings.is_empty());
        let blocking = ContentPreset::from_toml(&format!("{config}blocking=true\n")).unwrap();
        let missing = ContentInput {
            text: Some("Count is 14.".into()),
            ..Default::default()
        };
        let result = blocking.evaluate(&missing).unwrap().gates.remove(0);
        assert!(result.blocking);
        assert_eq!(result.status, GateStatus::NotChecked);
    }

    #[test]
    fn structured_evidence_and_reviews_reject_duplicate_json_keys() {
        let structured = ContentPreset::from_toml("version=1\nname='fact'\n[[gates]]\nid='fact'\nkind='structured_fact'\nsource='s'\nclaim='The count is 14.'\npointer='/data/count'\nexpected=14\n").unwrap();
        let check = |preset: &ContentPreset, source: &str, review: Option<&str>| {
            let mut evidence = BTreeMap::from([("s".into(), source.into())]);
            if let Some(record) = review {
                evidence.insert("r".into(), record.into());
            }
            let input = ContentInput {
                text: Some("The count is 14.".into()),
                evidence_sources: Some(evidence),
                ..Default::default()
            };
            preset.evaluate(&input).unwrap().gates.remove(0)
        };
        assert_eq!(
            check(&structured, r#"{"data":{"count":14}}"#, None).status,
            GateStatus::Passed
        );
        for json in [
            r#"{"data":{"count":0,"count":14}}"#,
            r#"{"data":{"count":14},"data":{"count":14}}"#,
        ] {
            let result = check(&structured, json, None);
            assert_eq!(result.status, GateStatus::NotChecked);
            assert!(result.blocking);
            assert!(
                result
                    .reason
                    .as_deref()
                    .unwrap()
                    .contains("invalid or ambiguous JSON")
            );
            assert!(result.findings.is_empty());
        }
        let review = ContentPreset::from_toml("version=1\nname='review'\n[[gates]]\nid='review'\nkind='evidence_review'\nsource='s'\nreview='r'\nclaim='The count is 14.'\n").unwrap();
        let valid = r#"{"claim":"The count is 14.","source_text":"source","support":"supported","reliability":"reliable","reviewer":"editor","provenance":{"version":"v1"}}"#;
        assert_eq!(
            check(&review, "source", Some(valid)).status,
            GateStatus::Passed
        );
        let ambiguous = valid.replace(r#""version":"v1""#, r#""version":"old","version":"v1""#);
        let result = check(&review, "source", Some(&ambiguous));
        assert_eq!(result.status, GateStatus::NotChecked);
        assert!(result.blocking);
        assert!(
            result
                .reason
                .as_deref()
                .unwrap()
                .contains("invalid or ambiguous JSON")
        );
        assert!(result.findings.is_empty());
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
    fn duplicate_paragraphs_are_configurable_and_preserve_partial_results() {
        let toml = "version=1\nname='duplicates'\n[[gates]]\nid='dupe'\nkind='duplicate_paragraphs'\nmax_occurrences=1\nmin_chars=4\ncase_insensitive=true\n";
        let preset = ContentPreset::from_toml(toml).unwrap();
        assert!(
            ContentPreset::from_toml(&toml.replace("max_occurrences=1", "max_occurrences=0"))
                .is_err()
        );
        assert!(ContentPreset::from_toml(&toml.replace("min_chars=4", "min_chars=0")).is_err());
        let input = ContentInput::default();
        let run = |paragraphs: Option<Vec<ContentParagraph>>| {
            preset
                .evaluate_with_context(
                    &input,
                    None,
                    None,
                    Some(&ContentContext {
                        paragraphs,
                        ..Default::default()
                    }),
                )
                .unwrap()
                .gates
                .remove(0)
        };
        let paragraph = |text: String, line| ContentParagraph {
            text,
            line,
            column: 1,
        };
        let report = run(Some(vec![
            paragraph("Café  ici".into(), 1),
            paragraph("CAFÉ\nici".into(), 4),
            paragraph("x".into(), 7),
        ]));
        assert_eq!(report.status, GateStatus::Failed);
        assert!(!report.blocking);
        assert_eq!(report.findings.len(), 1);
        assert_eq!(report.findings[0].line, 4);
        assert_eq!(report.measurements["paragraphs_evaluated"], 2);
        assert_eq!(report.measurements["paragraphs_skipped"], 1);
        assert_eq!(report.measurements["paragraphs_exceeding_limit"], 1);
        assert_eq!(run(None).status, GateStatus::NotChecked);
        assert_eq!(run(Some(vec![])).status, GateStatus::Passed);
        let partial = run(Some(vec![
            paragraph("same".into(), 1),
            paragraph("same".into(), 2),
            paragraph("x".repeat(MAX_DUPLICATE_PARAGRAPH_BYTES + 1), 3),
        ]));
        assert_eq!(partial.status, GateStatus::NotChecked);
        assert_eq!(partial.findings.len(), 1);
    }

    #[test]
    fn repeated_openings_preserve_accents_and_match_canonical_unicode() {
        let preset = ContentPreset::from_toml("version=1\nname='t'\n[[gates]]\nid='paragraph'\nkind='repeated_paragraph_openings'\nopening_words=1\nmax_occurrences=1\n[[gates]]\nid='sentence'\nkind='repeated_sentence_openings'\nopening_words=1\nmax_consecutive=1\n").unwrap();
        let input = ContentInput {
            text: Some(String::new()),
            ..Default::default()
        };
        let check = |paragraphs: &[&str]| {
            let context = ContentContext {
                paragraphs: Some(
                    paragraphs
                        .iter()
                        .map(|text| ContentParagraph {
                            text: (*text).into(),
                            line: 1,
                            column: 1,
                        })
                        .collect(),
                ),
                ..Default::default()
            };
            preset
                .evaluate_with_context(&input, None, None, Some(&context))
                .unwrap()
        };
        let same = check(&["Café arrive. CAFE\u{301} repart.", "cafe\u{301} revient."]);
        assert_eq!(same.gates[0].status, GateStatus::Failed);
        assert_eq!(same.gates[1].status, GateStatus::Failed);
        let different = check(&[
            "cafe\u{301} arrive. cafe\u{300} repart.",
            "cafe\u{300} revient.",
        ]);
        assert_eq!(different.gates[0].status, GateStatus::Passed);
        assert_eq!(different.gates[1].status, GateStatus::Passed);
    }

    #[test]
    fn consecutive_openings_require_supported_word_boundaries() {
        let source = "version=1\nname='t'\n[[gates]]\nid='opening'\nkind='repeated_sentence_openings'\nopening_words=2\nmax_consecutive=1\ndelimiters='.!?؟'\n";
        let input = ContentInput {
            text: Some(String::new()),
            ..Default::default()
        };
        let evaluate = |source: &str, text: &str| {
            let context = ContentContext {
                paragraphs: Some(vec![ContentParagraph {
                    text: text.into(),
                    line: 1,
                    column: 1,
                }]),
                ..Default::default()
            };
            ContentPreset::from_toml(source)
                .unwrap()
                .evaluate_with_context(&input, None, None, Some(&context))
                .unwrap()
                .gates
                .remove(0)
        };
        let arabic = "هذا مثال أول؟ هذا مثال آخر؟ نعم؟";
        let unsupported = evaluate(source, arabic);
        assert_eq!(unsupported.status, GateStatus::NotChecked);
        assert_eq!(unsupported.measurements["sentences_skipped"], 3);
        let opted_in = format!("{source}allow_arabic_word_boundaries=true\n");
        let supported = evaluate(&opted_in, arabic);
        assert_eq!(supported.status, GateStatus::Failed);
        assert_eq!(supported.measurements["sentences_too_short"], 1);
        assert_eq!(supported.measurements["sentences_skipped"], 0);
        let mixed = evaluate(
            &opted_in,
            "هذا مثال أول؟ هذا مثال آخر؟ これは 文です。 التالي مثال؟",
        );
        assert_eq!(mixed.status, GateStatus::NotChecked);
        assert_eq!(mixed.findings.len(), 1);
        assert_eq!(mixed.measurements["sentences_skipped"], 1);
        let built_in = crate::content_presets::resolve_content_preset(
            "arabic-rhythm-review",
            &crate::content_presets::builtin_content_presets(),
        )
        .unwrap();
        let context = ContentContext {
            paragraphs: Some(vec![ContentParagraph {
                text: "هذا مثال أول؟ هذا مثال آخر؟ هذا مثال ثالث؟ نعم؟".into(),
                line: 1,
                column: 1,
            }]),
            ..Default::default()
        };
        let report = built_in
            .evaluate_with_context(&input, None, None, Some(&context))
            .unwrap();
        let opening = report
            .gates
            .iter()
            .find(|gate| gate.gate_id == "editorial.sentence_openings")
            .unwrap();
        assert_eq!(opening.status, GateStatus::Failed);
        assert_eq!(opening.measurements["sentences_too_short"], 1);
        assert_eq!(opening.measurements["sentences_skipped"], 0);
        let unsupported_spaced = evaluate(source, "Это пример. Это пример.");
        assert_eq!(unsupported_spaced.status, GateStatus::NotChecked);
        assert!(unsupported_spaced.findings.is_empty());
    }

    #[test]
    fn repeated_sentence_openings_handles_uncertainty_and_configuration() {
        let source = "version=1\nname='t'\n[[gates]]\nid='opening'\nkind='repeated_sentence_openings'\nopening_words=2\nmax_consecutive=2\nabbreviations=['Dr.']\n";
        let preset = ContentPreset::from_toml(source).unwrap();
        let input = ContentInput {
            text: Some(String::new()),
            ..Default::default()
        };
        let context = |text: &str| ContentContext {
            paragraphs: Some(vec![ContentParagraph {
                text: text.into(),
                line: 1,
                column: 1,
            }]),
            ..Default::default()
        };
        let report = preset
            .evaluate_with_context(
                &input,
                None,
                None,
                Some(&context(
                    "Dr. Smith acts. Dr. Smith plans. Dr. Smith checks. Hi.",
                )),
            )
            .unwrap();
        assert_eq!(report.gates[0].status, GateStatus::Failed);
        assert_eq!(report.gates[0].findings.len(), 1);
        assert_eq!(report.gates[0].measurements["sentences_too_short"], 1);
        assert_eq!(report.gates[0].measurements["sentences_skipped"], 0);
        let uncertain = preset
            .evaluate_with_context(&input, None, None, Some(&context("これは文です。")))
            .unwrap();
        assert_eq!(uncertain.gates[0].status, GateStatus::NotChecked);
        for short in ["Été.", "e\u{301}te\u{301}.", "«Oui»."] {
            let result = preset
                .evaluate_with_context(&input, None, None, Some(&context(short)))
                .unwrap();
            assert_eq!(result.gates[0].status, GateStatus::Passed, "{short}");
            assert_eq!(result.gates[0].measurements["sentences_too_short"], 1);
        }
        let unicode_fold = preset
            .evaluate_with_context(&input, None, None, Some(&context("K.")))
            .unwrap();
        assert_eq!(unicode_fold.gates[0].status, GateStatus::NotChecked);
        let oversized = format!(
            "Dr. Smith acts. Dr. Smith plans. Dr. Smith checks. {} word.",
            "x".repeat(MAX_PARAGRAPH_OPENING_BYTES + 1)
        );
        let partial = preset
            .evaluate_with_context(&input, None, None, Some(&context(&oversized)))
            .unwrap();
        assert_eq!(partial.gates[0].status, GateStatus::NotChecked);
        assert_eq!(partial.gates[0].findings.len(), 1);
        let short_reset = preset
            .evaluate_with_context(
                &input,
                None,
                None,
                Some(&context(
                    "The team acts. Yes. The team plans. The team checks.",
                )),
            )
            .unwrap();
        assert_eq!(short_reset.gates[0].status, GateStatus::Passed);
        let unavailable = preset
            .evaluate_with_context(&input, None, None, None)
            .unwrap();
        assert_eq!(unavailable.gates[0].status, GateStatus::NotChecked);
        let reset = preset
            .evaluate_with_context(
                &input,
                None,
                None,
                Some(&context(
                    "The team acts. Other people help. The team plans. The team checks.",
                )),
            )
            .unwrap();
        assert_eq!(reset.gates[0].status, GateStatus::Passed);
        for (field, value) in [
            ("opening_words", "0"),
            ("opening_words", "17"),
            ("max_consecutive", "0"),
        ] {
            let invalid = format!(
                "version=1\nname='t'\n[[gates]]\nid='opening'\nkind='repeated_sentence_openings'\n{field}={value}\n"
            );
            assert!(ContentPreset::from_toml(&invalid).is_err());
        }
    }

    #[test]
    fn sentence_opening_window_and_ignored_words_are_configurable() {
        let config = "version=1\nname='window'\n[[gates]]\nid='opening'\nkind='sentence_opening_window'\nopening_words=1\nwindow_size=5\nmax_occurrences=2\nignored_words=['The','le']\n";
        let preset = ContentPreset::from_toml(config).unwrap();
        let input = ContentInput::default();
        let run = |texts: &[&str]| {
            let context = ContentContext {
                paragraphs: Some(
                    texts
                        .iter()
                        .enumerate()
                        .map(|(i, text)| ContentParagraph {
                            text: (*text).into(),
                            line: i + 1,
                            column: 1,
                        })
                        .collect(),
                ),
                ..Default::default()
            };
            preset
                .evaluate_with_context(&input, None, None, Some(&context))
                .unwrap()
                .gates
                .remove(0)
        };
        let repeated = run(&["The team acts. Other people help. team plans. Le team checks."]);
        assert_eq!(repeated.status, GateStatus::Failed);
        assert_eq!(repeated.findings.len(), 1);
        assert!(repeated.findings[0].message.contains("3 times"));
        assert_eq!(repeated.measurements["window_size"], 5);
        assert_eq!(repeated.measurements["max_occurrences"], 2);
        let reset = run(&[
            "Team acts. Team plans. Another helps.",
            "Team checks. Other works.",
        ]);
        assert_eq!(reset.status, GateStatus::Passed);
        let across =
            ContentPreset::from_toml(&format!("{config}window_across_paragraphs=true\n")).unwrap();
        let context = ContentContext {
            paragraphs: Some(vec![
                ContentParagraph {
                    text: "Team acts. Team plans.".into(),
                    line: 1,
                    column: 1,
                },
                ContentParagraph {
                    text: "Team checks. Other works.".into(),
                    line: 2,
                    column: 1,
                },
            ]),
            ..Default::default()
        };
        let result = across
            .evaluate_with_context(&input, None, None, Some(&context))
            .unwrap();
        assert_eq!(result.gates[0].status, GateStatus::Failed);
        assert_eq!(result.gates[0].findings[0].line, 2);
        let falling_out = run(&[
            "Team acts. Other works. Another helps. Someone waits. Different works. Team checks.",
        ]);
        assert_eq!(falling_out.status, GateStatus::Passed);
        let accent = run(&["Équipe agit. Autre aide. E\u{301}QUIPE planifie. équipe vérifie."]);
        assert_eq!(accent.status, GateStatus::Failed);
        assert_eq!(accent.findings.len(), 1);
        let french = run(&["L’équipe agit. Autre chose. l’équipe planifie. L’ÉQUIPE vérifie."]);
        assert_eq!(french.status, GateStatus::Failed);
        let short_preset = ContentPreset::from_toml("version=1\nname='short'\n[[gates]]\nid='opening'\nkind='sentence_opening_window'\nopening_words=2\nwindow_size=4\nmax_occurrences=2\n").unwrap();
        let short_context = ContentContext {
            paragraphs: Some(vec![ContentParagraph {
                text: "Oui.".into(),
                line: 1,
                column: 1,
            }]),
            ..Default::default()
        };
        let short = short_preset
            .evaluate_with_context(&input, None, None, Some(&short_context))
            .unwrap();
        assert_eq!(short.gates[0].status, GateStatus::NotChecked);
        let all_ignored = run(&["The. Team acts. Team plans. Other helps."]);
        assert_eq!(all_ignored.status, GateStatus::Passed);
        assert_eq!(all_ignored.measurements["sentences_too_short"], 1);
        let long = format!(
            "Team acts. Team plans. Team checks. {}.",
            "x".repeat(MAX_PARAGRAPH_OPENING_BYTES + 1)
        );
        let ellipsis = run(&["Une phrase…"]);
        assert_eq!(ellipsis.status, GateStatus::NotChecked);
        assert_eq!(ellipsis.measurements["sentences_evaluated"], 1);
        let chinese = run(&["这是一个句子。"]);
        assert_eq!(chinese.status, GateStatus::NotChecked);
        assert_eq!(chinese.measurements["sentences_skipped"], 1);
        let quoted = run(&["Team acts. « Team plans. Team checks. » Other works. More arrives."]);
        assert_eq!(quoted.status, GateStatus::Passed);
        assert_eq!(quoted.measurements["sentences_quoted"], 2);
        for text in ["", "The.", "« Team plans. Team checks. »"] {
            let unmeasurable = run(&[text]);
            assert_eq!(unmeasurable.status, GateStatus::NotChecked, "{text}");
            assert_eq!(unmeasurable.measurements["sentences_evaluated"], 0);
        }
        let too_few = run(&["Team acts. Team plans."]);
        assert_eq!(too_few.status, GateStatus::NotChecked);
        assert_eq!(too_few.measurements["sentences_evaluated"], 2);
        assert_eq!(too_few.measurements["assessable_windows"], 0);
        let split_too_few = run(&["One. Two.", "Three. Four."]);
        assert_eq!(split_too_few.status, GateStatus::NotChecked);
        let unclosed = run(&["Team acts. « Team plans. Team checks."]);
        assert_eq!(unclosed.status, GateStatus::NotChecked);
        let unknown = run(&[&long]);
        assert_eq!(unknown.status, GateStatus::NotChecked);
        assert_eq!(unknown.findings.len(), 1);
        for extra in [
            "window_size=1\nmax_occurrences=1",
            "window_size=65\nmax_occurrences=2",
            "window_size=3",
            "window_across_paragraphs=true",
            "max_occurrences=2",
            "window_size=3\nmax_occurrences=3",
            "ignored_words=['THE','the']",
            "ignored_words=['two words']",
        ] {
            let source = format!(
                "version=1\nname='bad'\n[[gates]]\nid='opening'\nkind='sentence_opening_window'\nopening_words=1\n{extra}\n"
            );
            assert!(ContentPreset::from_toml(&source).is_err(), "{extra}");
        }
    }

    #[test]
    fn style_gates_skip_multisentence_quotes_and_report_unclosed_quotes() {
        let preset = ContentPreset::from_toml("version=1\nname='quotes'\n[[gates]]\nid='opening'\nkind='sentence_opening_terms'\nterms=['Moreover']\nmax_per_paragraph=0\n[[gates]]\nid='tail'\nkind='trailing_ing_clause'\n[[gates]]\nid='list'\nkind='inline_triple_candidates'\nconjunction='and'\n").unwrap();
        let input = ContentInput::default();
        let context = |text: &str| ContentContext {
            paragraphs: Some(vec![ContentParagraph {
                text: text.into(),
                line: 1,
                column: 1,
            }]),
            ..Default::default()
        };
        for text in [
            "Before. “First sentence. Moreover, apples, pears, and plums. It grew, increasing sales.” After.",
            "Before. « First sentence. Moreover, apples, pears, and plums. It grew, increasing sales. » After.",
            "Before. \"First sentence. Moreover, apples, pears, and plums. It grew, increasing sales.\" After.",
        ] {
            let report = preset
                .evaluate_with_context(&input, None, None, Some(&context(text)))
                .unwrap();
            assert!(
                report
                    .gates
                    .iter()
                    .all(|gate| gate.status == GateStatus::Passed),
                "{text}: {:?}",
                report.gates
            );
        }
        let report = preset
            .evaluate_with_context(
                &input,
                None,
                None,
                Some(&context("Before. “First. Moreover, unclosed.")),
            )
            .unwrap();
        assert!(
            report
                .gates
                .iter()
                .all(|gate| gate.status == GateStatus::NotChecked)
        );
        let report = preset
            .evaluate_with_context(
                &input,
                None,
                None,
                Some(&context("“Quoted. Moreover, ignored.” Moreover, reviewed.")),
            )
            .unwrap();
        assert_eq!(report.gates[0].status, GateStatus::Failed);
        assert_eq!(report.gates[0].findings.len(), 1);
    }

    #[test]
    fn inline_triple_candidates_preserve_all_items_and_respect_configuration() {
        let preset = ContentPreset::from_toml("version=1\nname='t'\n[[gates]]\nid='triples'\nkind='inline_triple_candidates'\nconjunction='and'\n").unwrap();
        let text = "We have apples, pears, and plums. We have red and blue.";
        let input = ContentInput {
            text: Some(text.into()),
            ..Default::default()
        };
        let context = ContentContext {
            paragraphs: Some(vec![ContentParagraph {
                text: text.into(),
                line: 1,
                column: 1,
            }]),
            ..Default::default()
        };
        let report = preset
            .evaluate_with_context(&input, None, None, Some(&context))
            .unwrap();
        assert_eq!(report.gates[0].status, GateStatus::Failed);
        assert!(!report.gates[0].blocking);
        assert_eq!(report.gates[0].measurements["candidate_enumerations"], 1);
        assert_eq!(input.text.as_deref(), Some(text));
        assert_eq!(
            preset.evaluate(&input).unwrap().gates[0].status,
            GateStatus::NotChecked
        );
        assert!(ContentPreset::from_toml("version=1\nname='t'\n[[gates]]\nid='x'\nkind='inline_triple_candidates'\nconjunction='and or'\n").is_err());
    }

    #[test]
    fn longer_enumerations_are_configurable_and_advisory() {
        let preset = ContentPreset::from_toml("version=1\nname='t'\n[[gates]]\nid='list'\nkind='inline_triple_candidates'\nconjunction='and'\nmin_items=4\n").unwrap();
        let evaluate = |text: &str| {
            let input = ContentInput {
                text: Some(text.into()),
                ..Default::default()
            };
            let context = ContentContext {
                paragraphs: Some(vec![ContentParagraph {
                    text: text.into(),
                    line: 1,
                    column: 1,
                }]),
                ..Default::default()
            };
            preset
                .evaluate_with_context(&input, None, None, Some(&context))
                .unwrap()
                .gates
                .remove(0)
        };
        let candidate = evaluate(
            "Our team works on technology, data, safety and compliance, as well as training.",
        );
        assert_eq!(candidate.status, GateStatus::Failed);
        assert!(!candidate.blocking);
        assert_eq!(candidate.measurements["candidate_enumerations"], 1);
        assert_eq!(
            evaluate("We review apples, pears, peaches, cherries and plums.").status,
            GateStatus::Failed
        );
        assert_eq!(
            evaluate("Our team uses apples, pears and plums.").status,
            GateStatus::Passed
        );
        assert_eq!(
            evaluate("“Our team uses apples, pears, peaches and plums.”").status,
            GateStatus::Passed
        );
        assert_eq!(
            preset.evaluate(&ContentInput::default()).unwrap().gates[0].status,
            GateStatus::NotChecked
        );
        for invalid in ["min_items=2", "min_items=9", "item_separator='.'"] {
            assert!(ContentPreset::from_toml(&format!("version=1\nname='t'\n[[gates]]\nid='x'\nkind='inline_triple_candidates'\nconjunction='and'\n{invalid}\n")).is_err());
        }
        let custom = ContentPreset::from_toml("version=1\nname='t'\n[[gates]]\nid='x'\nkind='inline_triple_candidates'\nconjunction='and'\nmin_items=4\nitem_separator=';'\n").unwrap();
        let text = "Apples; pears; peaches and plums.";
        let input = ContentInput {
            text: Some(text.into()),
            ..Default::default()
        };
        let context = ContentContext {
            paragraphs: Some(vec![ContentParagraph {
                text: text.into(),
                line: 1,
                column: 1,
            }]),
            ..Default::default()
        };
        assert_eq!(
            custom
                .evaluate_with_context(&input, None, None, Some(&context))
                .unwrap()
                .gates[0]
                .status,
            GateStatus::Failed
        );
    }

    #[test]
    fn enumeration_separator_accepts_unicode_and_legacy_serialization_is_stable() {
        let legacy = ContentPreset::from_toml("version=1\nname='t'\n[[gates]]\nid='x'\nkind='inline_triple_candidates'\nconjunction='and'\n").unwrap();
        let serialized = serde_json::to_value(&legacy).unwrap();
        assert!(serialized["gates"][0].get("min_items").is_none());
        assert!(serialized["gates"][0].get("item_separator").is_none());
        let preset = ContentPreset::from_toml("version=1\nname='t'\n[[gates]]\nid='x'\nkind='inline_triple_candidates'\nconjunction='و'\nmin_items=4\nitem_separator='،'\n").unwrap();
        let text = "تفاح، كمثرى، خوخ و برقوق.";
        let input = ContentInput {
            text: Some(text.into()),
            ..Default::default()
        };
        let context = ContentContext {
            paragraphs: Some(vec![ContentParagraph {
                text: text.into(),
                line: 1,
                column: 1,
            }]),
            ..Default::default()
        };
        let report = preset
            .evaluate_with_context(&input, None, None, Some(&context))
            .unwrap();
        assert_eq!(report.gates[0].status, GateStatus::Failed);
        assert_eq!(report.gates[0].measurements["candidate_enumerations"], 1);
    }

    #[test]
    fn sentence_variance_windows_are_reproducible_and_partial_inputs_are_unchecked() {
        let fixed = ContentPreset::from_toml("version=1\nname='t'\n[[gates]]\nid='v'\nkind='sentence_length_variance'\nwindow_size=4\nmin_stddev_words=2\n").unwrap();
        assert!(
            serde_json::to_value(&fixed).unwrap()["gates"][0]
                .get("reset_at_paragraph")
                .is_none()
        );
        let input = ContentInput {
            text: Some(String::new()),
            ..Default::default()
        };
        let mut context = ContentContext {
            paragraphs: Some(vec![
                ContentParagraph {
                    text: "One two three. Four five six.".into(),
                    line: 1,
                    column: 1,
                },
                ContentParagraph {
                    text: "Seven eight nine. Ten eleven twelve.".into(),
                    line: 3,
                    column: 1,
                },
            ]),
            ..Default::default()
        };
        let report = fixed
            .evaluate_with_context(&input, None, None, Some(&context))
            .unwrap();
        assert_eq!(report.gates[0].status, GateStatus::Failed);
        assert!(!report.gates[0].blocking);
        assert_eq!(report.gates[0].measurements["windows_checked"], 1);
        assert_eq!(
            report.gates[0].measurements["effective_min_stddev_words"],
            2
        );
        assert_eq!(report.gates[0].findings[0].line, 1);
        context.paragraphs.as_mut().unwrap().pop();
        assert_eq!(
            fixed
                .evaluate_with_context(&input, None, None, Some(&context))
                .unwrap()
                .gates[0]
                .status,
            GateStatus::NotChecked
        );
        context.paragraphs.as_mut().unwrap()[0].text = "你好世界。".into();
        assert_eq!(
            fixed
                .evaluate_with_context(&input, None, None, Some(&context))
                .unwrap()
                .gates[0]
                .status,
            GateStatus::NotChecked
        );
        assert_eq!(
            fixed.evaluate(&input).unwrap().gates[0].status,
            GateStatus::NotChecked
        );

        let random = ContentPreset::from_toml("version=1\nname='t'\n[[gates]]\nid='v'\nkind='sentence_length_variance'\nwindow_size=4\nrandom_min_stddev_words=6\nrandom_max_stddev_words=10\nrandom_seed=42\n").unwrap();
        let a = random.evaluate(&input).unwrap();
        let b = random
            .evaluate(&ContentInput {
                id: "another-adapter-id".into(),
                text: Some("rewritten".into()),
                ..Default::default()
            })
            .unwrap();
        let threshold = a.gates[0].measurements["effective_min_stddev_words"];
        assert!((6..=10).contains(&threshold));
        assert_eq!(
            threshold,
            b.gates[0].measurements["effective_min_stddev_words"]
        );
        let roundtrip: ContentPreset = toml::from_str(&toml::to_string(&random).unwrap()).unwrap();
        assert_eq!(
            serde_json::to_value(random).unwrap(),
            serde_json::to_value(roundtrip).unwrap()
        );
    }

    #[test]
    fn sentence_variance_can_keep_windows_within_paragraphs() {
        let preset = ContentPreset::from_toml("version=1\nname='t'\n[[gates]]\nid='v'\nkind='sentence_length_variance'\nwindow_size=4\nmin_stddev_words=2\nreset_at_paragraph=true\n").unwrap();
        let input = ContentInput {
            text: Some(String::new()),
            ..Default::default()
        };
        let mut context = ContentContext {
            paragraphs: Some(vec![
                ContentParagraph {
                    text: "One two three. Four five six.".into(),
                    line: 1,
                    column: 1,
                },
                ContentParagraph {
                    text: "Seven eight nine. Ten eleven twelve.".into(),
                    line: 3,
                    column: 1,
                },
            ]),
            ..Default::default()
        };
        let report = preset
            .evaluate_with_context(&input, None, None, Some(&context))
            .unwrap();
        let gate = &report.gates[0];
        assert_eq!(gate.status, GateStatus::NotChecked);
        assert_eq!(gate.measurements["windows_checked"], 0);
        assert_eq!(gate.measurements["paragraphs_without_windows"], 2);
        assert!(gate.findings.is_empty());

        context.paragraphs.as_mut().unwrap()[0].text =
            "One two three. Four five six. Seven eight nine. Ten eleven twelve.".into();
        let report = preset
            .evaluate_with_context(&input, None, None, Some(&context))
            .unwrap();
        let gate = &report.gates[0];
        assert_eq!(gate.status, GateStatus::NotChecked);
        assert_eq!(gate.measurements["windows_checked"], 1);
        assert_eq!(gate.measurements["paragraphs_without_windows"], 1);
        assert_eq!(gate.findings.len(), 1);
        assert_eq!(gate.findings[0].line, 1);

        context.paragraphs.as_mut().unwrap().pop();
        let report = preset
            .evaluate_with_context(&input, None, None, Some(&context))
            .unwrap();
        assert_eq!(report.gates[0].status, GateStatus::Failed);
        assert_eq!(
            report.gates[0].measurements["paragraphs_without_windows"],
            0
        );
        let roundtrip: ContentPreset = toml::from_str(&toml::to_string(&preset).unwrap()).unwrap();
        assert_eq!(
            serde_json::to_value(preset).unwrap(),
            serde_json::to_value(roundtrip).unwrap()
        );
    }

    #[test]
    fn sentence_variance_validates_thresholds_and_detects_diverse_lengths() {
        let prefix = "version=1\nname='t'\n[[gates]]\nid='v'\nkind='sentence_length_variance'\nwindow_size=4\n";
        for invalid in [
            "",
            "min_stddev_words=0",
            "random_min_stddev_words=6\nrandom_max_stddev_words=10",
            "random_min_stddev_words=10\nrandom_max_stddev_words=6\nrandom_seed=1",
            "min_stddev_words=6\nrandom_min_stddev_words=6\nrandom_max_stddev_words=10\nrandom_seed=1",
        ] {
            assert!(
                ContentPreset::from_toml(&format!("{prefix}{invalid}\n")).is_err(),
                "accepted {invalid}"
            );
        }
        let preset = ContentPreset::from_toml(&format!("{prefix}min_stddev_words=2\n")).unwrap();
        let context = ContentContext { paragraphs: Some(vec![ContentParagraph { text: "One. Two three four five six seven. Eight. Nine ten eleven twelve thirteen fourteen.".into(), line: 1, column: 1 }]), ..Default::default() };
        let report = preset
            .evaluate_with_context(&ContentInput::default(), None, None, Some(&context))
            .unwrap();
        assert_eq!(report.gates[0].status, GateStatus::Passed);
        assert_eq!(report.gates[0].measurements["windows_checked"], 1);
    }

    #[test]
    fn sentence_variance_resets_rolling_sums_at_uncertain_and_quoted_sentences() {
        let preset = ContentPreset::from_toml("version=1\nname='t'\n[[gates]]\nid='v'\nkind='sentence_length_variance'\nwindow_size=2\nmin_stddev_words=2\n").unwrap();
        for text in [
            "One two three four five. 中文文字. One. Two.",
            "One two three four five. « Quoted words. » One. Two.",
        ] {
            let context = ContentContext {
                paragraphs: Some(vec![ContentParagraph {
                    text: text.into(),
                    line: 1,
                    column: 1,
                }]),
                ..Default::default()
            };
            let report = preset
                .evaluate_with_context(&ContentInput::default(), None, None, Some(&context))
                .unwrap();
            assert_eq!(report.gates[0].measurements["windows_checked"], 1, "{text}");
            assert_eq!(
                report.gates[0].measurements["low_variance_windows"], 1,
                "{text}"
            );
            assert_eq!(report.gates[0].findings.len(), 1, "{text}");
        }
    }

    #[test]
    fn prose_variance_excludes_list_paragraphs_and_requires_complete_classification() {
        let preset = ContentPreset::from_toml("version=1\nname='t'\n[[gates]]\nid='v'\nkind='prose_sentence_length_variance'\nwindow_size=3\nmin_stddev_words=2\n").unwrap();
        let context = ContentContext {
            paragraphs: Some(vec![
                ContentParagraph {
                    text: "One two. Three four. Five six.".into(),
                    line: 1,
                    column: 1,
                },
                ContentParagraph {
                    text: "List one. List two. List three.".into(),
                    line: 2,
                    column: 1,
                },
                ContentParagraph {
                    text: "Seven eight. Nine ten. Eleven twelve.".into(),
                    line: 3,
                    column: 1,
                },
            ]),
            ..Default::default()
        };
        let capabilities = ContentEvaluationContext {
            adapter: Some(&context),
            ..Default::default()
        };
        let input = ContentInput::default();
        let run = |classification: Option<&[bool]>| {
            preset
                .evaluate_with_paragraph_list_items(
                    &input,
                    &capabilities,
                    None,
                    None,
                    true,
                    None,
                    None,
                    classification,
                )
                .unwrap()
        };
        for missing in [None, Some(&[false][..])] {
            let report = run(missing);
            assert_eq!(report.gates[0].status, GateStatus::NotChecked);
            assert!(report.gates[0].findings.is_empty());
        }
        let report = run(Some(&[false, true, false]));
        let gate = &report.gates[0];
        assert_eq!(gate.status, GateStatus::Failed);
        assert_eq!(gate.measurements["list_paragraphs_excluded"], 1);
        assert_eq!(gate.measurements["windows_checked"], 2);
        assert_eq!(
            gate.findings.iter().map(|f| f.line).collect::<Vec<_>>(),
            vec![1, 3]
        );
        let report = run(Some(&[true, true, true]));
        assert_eq!(report.gates[0].status, GateStatus::NotChecked);
        assert_eq!(report.gates[0].measurements["list_paragraphs_excluded"], 3);
        assert!(report.gates[0].findings.is_empty());
    }

    #[test]
    fn arabic_prose_variance_respects_list_classification() {
        let preset = ContentPreset::from_toml("version=1\nname='t'\n[[gates]]\nid='v'\nkind='prose_sentence_length_variance'\nwindow_size=3\nmin_stddev_words=2\nallow_arabic_word_boundaries=true\ndelimiters='.؟'\n").unwrap();
        let context = ContentContext {
            paragraphs: Some(vec![ContentParagraph {
                text: "هذا نص واضح. هذا نص مفيد؟ هذا نص قصير.".into(),
                line: 1,
                column: 1,
            }]),
            ..Default::default()
        };
        let capabilities = ContentEvaluationContext {
            adapter: Some(&context),
            ..Default::default()
        };
        let input = ContentInput::default();
        let run = |classification: Option<&[bool]>| {
            preset
                .evaluate_with_paragraph_list_items(
                    &input,
                    &capabilities,
                    None,
                    None,
                    true,
                    None,
                    None,
                    classification,
                )
                .unwrap()
        };
        assert_eq!(run(None).gates[0].status, GateStatus::NotChecked);
        assert_eq!(run(Some(&[true])).gates[0].status, GateStatus::NotChecked);
        let report = run(Some(&[false]));
        assert_eq!(report.gates[0].status, GateStatus::Failed);
        assert_eq!(report.gates[0].measurements["windows_checked"], 1);
    }

    #[test]
    fn sentence_variance_rolling_windows_match_reference_counts() {
        let counts: Vec<usize> = (0..120).map(|i| 1 + (i * 17 % 13)).collect();
        let text = counts
            .iter()
            .map(|n| format!("{}.", vec!["word"; *n].join(" ")))
            .collect::<Vec<_>>()
            .join(" ");
        let context = ContentContext {
            paragraphs: Some(vec![ContentParagraph {
                text,
                line: 1,
                column: 1,
            }]),
            ..Default::default()
        };
        for size in [2usize, 7, 64] {
            let preset = ContentPreset::from_toml(&format!("version=1\nname='t'\n[[gates]]\nid='v'\nkind='sentence_length_variance'\nwindow_size={size}\nmin_stddev_words=3\n")).unwrap();
            let report = preset
                .evaluate_with_context(&ContentInput::default(), None, None, Some(&context))
                .unwrap();
            let expected = counts
                .windows(size)
                .filter(|window| {
                    let n = size as u128;
                    let sum: u128 = window.iter().map(|v| *v as u128).sum();
                    let squares: u128 = window.iter().map(|v| (*v as u128).pow(2)).sum();
                    n * squares - sum * sum < 9 * n * n
                })
                .count();
            assert_eq!(
                report.gates[0].measurements["windows_checked"],
                counts.len() - size + 1
            );
            assert_eq!(
                report.gates[0].measurements["low_variance_windows"],
                expected
            );
            assert_eq!(report.gates[0].findings.len(), expected);
        }
    }

    #[test]
    fn explicit_arabic_whitespace_policy_enables_rhythm_but_withholds_other_scripts() {
        let source = "version=1\nname='arabic'\n[[gates]]\nid='variance'\nkind='sentence_length_variance'\nwindow_size=3\nmin_stddev_words=1\ndelimiters='.!?؟'\nallow_arabic_word_boundaries=true\n[[gates]]\nid='rhythm'\nkind='similar_sentence_lengths'\nrun_length=2\ntolerance_words=0\ndelimiters='.!?؟'\nallow_arabic_word_boundaries=true\n[[gates]]\nid='window'\nkind='sentence_opening_window'\nopening_words=1\nwindow_size=3\nmax_occurrences=1\ndelimiters='.!?؟'\nallow_arabic_word_boundaries=true\n";
        let preset = ContentPreset::from_toml(source).unwrap();
        let input = ContentInput {
            text: Some(String::new()),
            ..Default::default()
        };
        let mut context = ContentContext {
            paragraphs: Some(vec![ContentParagraph {
                text: "هذا مثال واحد؟ هذا مثال ثان؟ هذا مثال ثالث.".into(),
                line: 1,
                column: 1,
            }]),
            ..Default::default()
        };
        let report = preset
            .evaluate_with_context(&input, None, None, Some(&context))
            .unwrap();
        assert!(
            report
                .gates
                .iter()
                .all(|gate| gate.status == GateStatus::Failed && !gate.blocking)
        );
        context.paragraphs.as_mut().unwrap()[0].text = "هذا مثال واحد؟ 你好世界。".into();
        let report = preset
            .evaluate_with_context(&input, None, None, Some(&context))
            .unwrap();
        assert!(
            report
                .gates
                .iter()
                .all(|gate| gate.status == GateStatus::NotChecked)
        );
        let default =
            ContentPreset::from_toml(&source.replace("allow_arabic_word_boundaries=true\n", ""))
                .unwrap();
        context.paragraphs.as_mut().unwrap()[0].text =
            "هذا مثال واحد؟ هذا مثال ثان؟ هذا مثال ثالث.".into();
        assert!(
            default
                .evaluate_with_context(&input, None, None, Some(&context))
                .unwrap()
                .gates
                .iter()
                .all(|gate| gate.status == GateStatus::NotChecked)
        );
    }

    #[test]
    fn similar_sentence_lengths_reports_runs_and_withholds_uncertain_scripts() {
        let preset = ContentPreset::from_toml("version=1\nname='t'\n[[gates]]\nid='rhythm'\nkind='similar_sentence_lengths'\nrun_length=3\ntolerance_words=1\n").unwrap();
        let input = ContentInput {
            text: Some(String::new()),
            ..Default::default()
        };
        let mut context = ContentContext {
            paragraphs: Some(vec![ContentParagraph {
                text: "One two three. Four five six. Seven eight nine.".into(),
                line: 1,
                column: 1,
            }]),
            ..Default::default()
        };
        let report = preset
            .evaluate_with_context(&input, None, None, Some(&context))
            .unwrap();
        assert_eq!(report.gates[0].status, GateStatus::Failed);
        assert!(!report.gates[0].blocking);
        assert_eq!(report.gates[0].measurements["similar_runs"], 1);
        context.paragraphs.as_mut().unwrap()[0].text = "你好世界。".into();
        assert_eq!(
            preset
                .evaluate_with_context(&input, None, None, Some(&context))
                .unwrap()
                .gates[0]
                .status,
            GateStatus::NotChecked
        );
        assert_eq!(
            preset.evaluate(&input).unwrap().gates[0].status,
            GateStatus::NotChecked
        );
    }

    #[test]
    fn configured_colon_openings_require_the_colon_and_skip_quotations() {
        let preset = ContentPreset::from_toml("version=1\nname='t'\n[[gates]]\nid='colon'\nkind='sentence_opening_terms'\nterms=['leur ambition est simple']\nrequire_colon=true\n").unwrap();
        let input = ContentInput {
            text: Some(String::new()),
            ..Default::default()
        };
        let mut context = ContentContext {
            paragraphs: Some(vec![ContentParagraph {
                text: "Leur ambition est simple : protéger les équipes.".into(),
                line: 1,
                column: 1,
            }]),
            ..Default::default()
        };
        let report = preset
            .evaluate_with_context(&input, None, None, Some(&context))
            .unwrap();
        assert_eq!(report.gates[0].status, GateStatus::Failed);
        context.paragraphs.as_mut().unwrap()[0].text =
            "Leur ambition est simplement de protéger les équipes.".into();
        assert_eq!(
            preset
                .evaluate_with_context(&input, None, None, Some(&context))
                .unwrap()
                .gates[0]
                .status,
            GateStatus::Passed
        );
        context.paragraphs.as_mut().unwrap()[0].text = "Leur ambition est simple et utile.".into();
        assert_eq!(
            preset
                .evaluate_with_context(&input, None, None, Some(&context))
                .unwrap()
                .gates[0]
                .status,
            GateStatus::Passed
        );
        context.paragraphs.as_mut().unwrap()[0].text =
            "« Leur ambition est simple : protéger les équipes. »".into();
        assert_eq!(
            preset
                .evaluate_with_context(&input, None, None, Some(&context))
                .unwrap()
                .gates[0]
                .status,
            GateStatus::Passed
        );
    }

    #[test]
    fn adjacent_sentence_length_percent_is_exact_and_requires_observations() {
        let config = "version=1\nname='t'\n[[gates]]\nid='rhythm'\nkind='similar_sentence_lengths'\nrun_length=2\ntolerance_words=0\nmin_difference_percent=30\n";
        let preset = ContentPreset::from_toml(config).unwrap();
        let input = ContentInput {
            text: Some(String::new()),
            ..Default::default()
        };
        let mut context = ContentContext {
            paragraphs: Some(vec![ContentParagraph {
                text: "One two three four five six seven. One two three four five six seven eight nine ten.".into(),
                line: 1, column: 1,
            }]), ..Default::default()
        };
        let report = preset
            .evaluate_with_context(&input, None, None, Some(&context))
            .unwrap();
        assert_eq!(report.gates[0].status, GateStatus::Passed);
        assert_eq!(report.gates[0].measurements["pairs_assessed"], 1);
        context.paragraphs.as_mut().unwrap()[0].text = "One two three four five six seven eight. One two three four five six seven eight nine ten.".into();
        let report = preset
            .evaluate_with_context(&input, None, None, Some(&context))
            .unwrap();
        assert_eq!(report.gates[0].status, GateStatus::Failed);
        assert!(!report.gates[0].blocking);
        context.paragraphs.as_mut().unwrap()[0].text = "Only one sentence.".into();
        assert_eq!(
            preset
                .evaluate_with_context(&input, None, None, Some(&context))
                .unwrap()
                .gates[0]
                .status,
            GateStatus::NotChecked
        );
        context.paragraphs.as_mut().unwrap()[0].text = "你好世界。".into();
        assert_eq!(
            preset
                .evaluate_with_context(&input, None, None, Some(&context))
                .unwrap()
                .gates[0]
                .status,
            GateStatus::NotChecked
        );
        for invalid in [
            config.replace("percent=30", "percent=0"),
            config.replace("percent=30", "percent=101"),
            config.replace("run_length=2", "run_length=3"),
            config.replace("tolerance_words=0", "tolerance_words=1"),
        ] {
            assert!(ContentPreset::from_toml(&invalid).is_err());
        }
    }

    #[test]
    fn adjacent_sentence_range_is_bounded_inclusive_and_requires_comparable_pairs() {
        let config = "version=1\nname='t'\n[[gates]]\nid='rhythm'\nkind='similar_sentence_lengths'\nrun_length=2\ntolerance_words=0\nrange_min_words=3\nrange_max_words=4\n";
        let preset = ContentPreset::from_toml(config).unwrap();
        let input = ContentInput {
            text: Some(String::new()),
            ..Default::default()
        };
        let mut context = ContentContext {
            paragraphs: Some(vec![ContentParagraph {
                text: "One two three. One two three four.".into(),
                line: 1,
                column: 1,
            }]),
            ..Default::default()
        };
        let report = preset
            .evaluate_with_context(&input, None, None, Some(&context))
            .unwrap();
        assert_eq!(report.gates[0].status, GateStatus::Failed);
        assert_eq!(report.gates[0].measurements["pairs_assessed"], 1);
        assert_eq!(report.gates[0].measurements["range_pairs"], 1);
        assert_eq!(report.gates[0].measurements["range_min_words"], 3);
        assert_eq!(report.gates[0].measurements["range_max_words"], 4);
        assert!(!report.gates[0].blocking);
        context.paragraphs.as_mut().unwrap()[0].text = "One two. One two three four.".into();
        assert_eq!(
            preset
                .evaluate_with_context(&input, None, None, Some(&context))
                .unwrap()
                .gates[0]
                .status,
            GateStatus::Passed
        );
        context.paragraphs.as_mut().unwrap()[0].text = "One sentence.".into();
        assert_eq!(
            preset
                .evaluate_with_context(&input, None, None, Some(&context))
                .unwrap()
                .gates[0]
                .status,
            GateStatus::NotChecked
        );
        context.paragraphs = Some(vec![
            ContentParagraph {
                text: "One two three.".into(),
                line: 1,
                column: 1,
            },
            ContentParagraph {
                text: "One two three.".into(),
                line: 2,
                column: 1,
            },
        ]);
        assert_eq!(
            preset
                .evaluate_with_context(&input, None, None, Some(&context))
                .unwrap()
                .gates[0]
                .status,
            GateStatus::NotChecked
        );
        context.paragraphs = None;
        assert_eq!(
            preset
                .evaluate_with_context(&input, None, None, Some(&context))
                .unwrap()
                .gates[0]
                .status,
            GateStatus::NotChecked
        );
        for invalid in [
            config.replace("range_min_words=3", "range_min_words=0"),
            config.replace("range_max_words=4", "range_max_words=1001"),
            config.replace("range_max_words=4", "range_max_words=2"),
            config.replace("range_max_words=4\n", ""),
            config.replace("run_length=2", "run_length=3"),
            config.replace("tolerance_words=0", "tolerance_words=1"),
            format!("{config}min_difference_percent=30\n"),
        ] {
            assert!(ContentPreset::from_toml(&invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn trailing_ing_review_is_english_heuristic_and_preserves_prose() {
        let preset = ContentPreset::from_toml(
            "version=1\nname='t'\n[[gates]]\nid='tail'\nkind='trailing_ing_clause'\n",
        )
        .unwrap();
        let text = "Sales rose, increasing profits. One,ring. Plain words.";
        let input = ContentInput {
            text: Some(text.into()),
            ..Default::default()
        };
        let context = ContentContext {
            paragraphs: Some(vec![ContentParagraph {
                text: text.into(),
                line: 1,
                column: 1,
            }]),
            ..Default::default()
        };
        let report = preset
            .evaluate_with_context(&input, None, None, Some(&context))
            .unwrap();
        assert_eq!(report.gates[0].status, GateStatus::Failed);
        assert!(!report.gates[0].blocking);
        assert_eq!(report.gates[0].measurements["candidate_clauses"], 1);
        assert_eq!(input.text.as_deref(), Some(text));
        assert_eq!(
            preset.evaluate(&input).unwrap().gates[0].status,
            GateStatus::NotChecked
        );
    }

    #[test]
    fn punctuation_review_reports_repetition_without_rewriting() {
        let preset = ContentPreset::from_toml("version=1\nname='t'\n[[gates]]\nid='dash'\nkind='repeated_punctuation'\nmark='—'\nmax_per_paragraph=1\n").unwrap();
        let original = "One — then two — and three.";
        let input = ContentInput {
            text: Some(original.into()),
            ..Default::default()
        };
        let context = ContentContext {
            paragraphs: Some(vec![ContentParagraph {
                text: original.into(),
                line: 2,
                column: 1,
            }]),
            ..Default::default()
        };
        let report = preset
            .evaluate_with_context(&input, None, None, Some(&context))
            .unwrap();
        assert_eq!(report.gates[0].status, GateStatus::Failed);
        assert!(!report.gates[0].blocking);
        assert_eq!(
            report.gates[0].measurements["paragraphs_exceeding_limit"],
            1
        );
        assert_eq!(input.text.as_deref(), Some(original));
        assert_eq!(
            preset.evaluate(&input).unwrap().gates[0].status,
            GateStatus::NotChecked
        );
        assert!(ContentPreset::from_toml("version=1\nname='t'\n[[gates]]\nid='x'\nkind='repeated_punctuation'\nmark='a'\nmax_per_paragraph=1\n").is_err());
    }

    #[test]
    fn sentence_opening_terms_are_advisory_paragraph_scoped_and_require_observation() {
        let preset = ContentPreset::from_toml("version=1\nname='t'\n[[gates]]\nid='transitions'\nkind='sentence_opening_terms'\nterms=['Moreover','En outre']\nmax_per_paragraph=1\n").unwrap();
        let input = ContentInput {
            text: Some(String::new()),
            ..Default::default()
        };
        let context = ContentContext {
            paragraphs: Some(vec![
                ContentParagraph {
                    text: "Moreover, one. En outre, deux. Moreoverish is different.".into(),
                    line: 3,
                    column: 1,
                },
                ContentParagraph {
                    text: "Moreover, another paragraph.".into(),
                    line: 4,
                    column: 1,
                },
            ]),
            ..Default::default()
        };
        let report = preset
            .evaluate_with_context(&input, None, None, Some(&context))
            .unwrap();
        assert_eq!(report.gates[0].status, GateStatus::Failed);
        assert!(!report.gates[0].blocking);
        assert_eq!(report.gates[0].findings.len(), 1);
        assert_eq!(report.gates[0].measurements["matching_openings"], 3);
        assert_eq!(report.gates[0].findings[0].line, 3);
        assert_eq!(
            preset.evaluate(&input).unwrap().gates[0].status,
            GateStatus::NotChecked
        );
        assert!(
            ContentPreset::from_toml(
                "version=1\nname='t'\n[[gates]]\nid='x'\nkind='sentence_opening_terms'\nterms=[]\n"
            )
            .is_err()
        );
    }

    #[test]
    fn repeated_sentence_openings_are_consecutive_scoped_and_located() {
        let preset = ContentPreset::from_toml("version=1\nname='t'\n[[gates]]\nid='opening'\nkind='repeated_sentence_openings'\nopening_words=2\nmax_consecutive=2\nblocking=false\n").unwrap();
        let input = ContentInput {
            text: Some(String::new()),
            ..Default::default()
        };
        let context = ContentContext {
            paragraphs: Some(vec![
                ContentParagraph {
                    text: "The team plans. The team acts. THE TEAM checks.".into(),
                    line: 3,
                    column: 1,
                },
                ContentParagraph {
                    text: "The team improves.".into(),
                    line: 5,
                    column: 1,
                },
            ]),
            ..Default::default()
        };
        let report = preset
            .evaluate_with_context(&input, None, None, Some(&context))
            .unwrap();
        let gate = &report.gates[0];
        assert_eq!(gate.status, GateStatus::Failed);
        assert!(!gate.blocking);
        assert_eq!(gate.findings.len(), 1);
        assert_eq!((gate.findings[0].line, gate.findings[0].column), (3, 32));
        assert_eq!(gate.measurements["sentences_evaluated"], 4);
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
    fn paragraph_openings_bound_repeated_work_and_total_key_bytes() {
        let preset = ContentPreset::from_toml(
            "version=1\nname='openings'\n[[gates]]\nid='opening'\nkind='repeated_paragraph_openings'\nopening_words=2\nmax_occurrences=1\n",
        ).unwrap();
        let run = |paragraphs: Vec<ContentParagraph>| {
            preset
                .evaluate_with_context(
                    &ContentInput::default(),
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
        let paragraph = |text: String| ContentParagraph {
            text,
            line: 1,
            column: 1,
        };
        let repeated = run(vec![paragraph("same opening".into()); 100_001]);
        assert_eq!(repeated.status, GateStatus::NotChecked);
        assert_eq!(repeated.measurements["openings_evaluated"], 100_000);
        assert!(!repeated.findings.is_empty());
        assert!(
            repeated
                .reason
                .as_deref()
                .unwrap()
                .contains("too many paragraphs")
        );

        let unique = (0..1_100)
            .map(|i| paragraph(format!("prefix{i} {}", "x".repeat(4_000))))
            .collect();
        let bounded = run(unique);
        assert_eq!(bounded.status, GateStatus::NotChecked);
        assert!(bounded.measurements["openings_evaluated"] < 1_100);
        assert!(bounded.reason.as_deref().unwrap().contains("opening keys"));
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
        assert_eq!(unavailable.measurements["destination_count"], 3);
        assert!(!unavailable.measurements.contains_key("observation_count"));
        let mismatched = evaluate(Some(&[Reachable]));
        assert_eq!(mismatched.status, GateStatus::NotChecked);
        assert_eq!(mismatched.measurements["destination_count"], 3);
        assert_eq!(mismatched.measurements["observation_count"], 1);
        assert!(
            !mismatched
                .measurements
                .contains_key("not_applicable_destinations")
        );
        let partial = evaluate(Some(&[NotFound, NotChecked, NotApplicable]));
        assert_eq!(partial.status, GateStatus::NotChecked);
        assert_eq!(partial.findings.len(), 1);
        assert_eq!(partial.measurements["checked_destinations"], 1);
        assert_eq!(partial.measurements["not_found_destinations"], 1);
        assert_eq!(partial.measurements["unchecked_destinations"], 1);
        assert_eq!(partial.measurements["destination_count"], 3);
        assert_eq!(partial.measurements["observation_count"], 3);
        assert_eq!(partial.measurements["not_applicable_destinations"], 1);
        assert!(partial.findings[0].message.contains("#1"));
        let complete = evaluate(Some(&[NotFound, Reachable, NotApplicable]));
        assert_eq!(complete.status, GateStatus::Failed);
        assert!(!complete.blocking);
        let passed = evaluate(Some(&[Reachable, Reachable, NotApplicable]));
        assert_eq!(passed.status, GateStatus::Passed);
    }
    #[test]
    fn required_terms_report_presence_and_unavailable_text() {
        let config = "version=1\nname='test'\n[[gates]]\nid='terms'\nkind='required_terms'\nterms=['café', 'best price']\ncase_insensitive=true\nblocking=false\n";
        let preset = ContentPreset::from_toml(config).unwrap();
        let input = ContentInput {
            text: Some("CAFÉ: best\nprice".into()),
            ..Default::default()
        };
        let report = preset.evaluate(&input).unwrap();
        assert_eq!(report.gates[0].status, GateStatus::Passed);
        assert_eq!(report.gates[0].measurements["present_terms"], 2);
        let missing = preset
            .evaluate(&ContentInput {
                text: Some("cafétéria".into()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(missing.gates[0].status, GateStatus::Failed);
        assert!(!missing.gates[0].blocking);
        assert_eq!(missing.gates[0].measurements["missing_terms"], 2);
        assert_eq!(missing.gates[0].findings.len(), 2);
        assert_eq!(
            preset.evaluate(&ContentInput::default()).unwrap().gates[0].status,
            GateStatus::NotChecked
        );
        assert_eq!(
            preset
                .evaluate(&ContentInput {
                    text: Some(String::new()),
                    ..Default::default()
                })
                .unwrap()
                .gates[0]
                .status,
            GateStatus::Failed
        );
        let distinct_blocks = ContentContext {
            block_ends: Some(vec![4]),
            ..Default::default()
        };
        let split = ContentInput {
            text: Some("best price café".into()),
            ..Default::default()
        };
        assert_eq!(
            preset
                .evaluate_with_context(&split, None, None, Some(&distinct_blocks))
                .unwrap()
                .gates[0]
                .status,
            GateStatus::Failed
        );
        assert!(
            ContentPreset::from_toml(&config.replace("['café', 'best price']", "['café', 'CAFÉ']"))
                .is_err()
        );
        assert!(ContentPreset::from_toml(&config.replace("['café', 'best price']", "[]")).is_err());
        let many = (0..257)
            .map(|i| format!("'term{i}'"))
            .collect::<Vec<_>>()
            .join(",");
        assert!(ContentPreset::from_toml(&config.replace("'café', 'best price'", &many)).is_err());
    }

    #[test]
    fn body_limit_does_not_hide_missing_composition_reason() {
        let preset = ContentPreset::from_toml("version=1\nname='coverage'\n[[gates]]\nid='words'\nkind='word_count'\nmin=1\n[[gates]]\nid='pacing'\nkind='editorial_pacing'\nsource='composition'\nprofile={version=1,name='short',fields=[{name='hook',max_characters=3}]}\n").unwrap();
        let input = ContentInput {
            id: "example".into(),
            source_text: Some("<div>Text</div>".into()),
            ..Default::default()
        };
        let context = ContentContext {
            input_limit: Some("body contains unobserved HTML".into()),
            ..Default::default()
        };
        let report = preset
            .evaluate_with_context(&input, None, None, Some(&context))
            .unwrap();
        assert_eq!(report.gates[0].status, GateStatus::NotChecked);
        assert_eq!(
            report.gates[0].reason.as_deref(),
            Some("body contains unobserved HTML")
        );
        assert_eq!(report.gates[1].status, GateStatus::NotChecked);
        assert_eq!(
            report.gates[1].reason.as_deref(),
            Some("document or structured composition is unavailable")
        );
    }
}

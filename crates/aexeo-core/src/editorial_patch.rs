//! Explicit, source-bound editorial edits. This module does not judge meaning.
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const MAX_PATCH_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_EDITS: usize = 1000;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EditorialEdit {
    /// Half-open UTF-8 byte offsets, relative to the complete original source.
    pub start: usize,
    pub end: usize,
    pub expected: String,
    pub replacement: String,
    pub reason: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EditorialPatch {
    pub version: u8,
    pub source_sha256: String,
    pub edits: Vec<EditorialEdit>,
}

/// Bind explicitly authored edits to the exact source. Does not create edits.
pub fn propose(source: &str, edits: Vec<EditorialEdit>) -> Result<EditorialPatch> {
    let patch = EditorialPatch {
        version: 1,
        source_sha256: digest(source),
        edits,
    };
    apply(source, &patch)?;
    Ok(patch)
}

/// Verify and apply a proposal in memory. This makes no semantic safety claim.
pub fn apply(source: &str, patch: &EditorialPatch) -> Result<String> {
    ensure!(source.len() <= MAX_PATCH_BYTES, "source exceeds size limit");
    ensure!(patch.version == 1, "unsupported patch version");
    ensure!(
        patch.source_sha256 == digest(source),
        "source does not match patch snapshot"
    );
    ensure!(
        !patch.edits.is_empty() && patch.edits.len() <= MAX_EDITS,
        "invalid edit count"
    );
    let mut previous_end = 0;
    let mut result = String::with_capacity(source.len());
    for (index, edit) in patch.edits.iter().enumerate() {
        ensure!(
            edit.start < edit.end,
            "edit {index} must replace a nonempty range"
        );
        ensure!(
            edit.start >= previous_end,
            "edit {index} overlaps or is out of order"
        );
        ensure!(!edit.reason.trim().is_empty(), "edit {index} has no reason");
        let unchanged = source
            .get(previous_end..edit.start)
            .context("invalid UTF-8 edit boundary")?;
        let original = source
            .get(edit.start..edit.end)
            .context("invalid UTF-8 edit boundary")?;
        ensure!(
            original == edit.expected,
            "edit {index} does not match expected source"
        );
        ensure!(
            edit.replacement.len() <= MAX_PATCH_BYTES,
            "replacement exceeds size limit"
        );
        ensure!(
            result
                .len()
                .saturating_add(unchanged.len())
                .saturating_add(edit.replacement.len())
                <= MAX_PATCH_BYTES,
            "result exceeds size limit"
        );
        result.push_str(unchanged);
        result.push_str(&edit.replacement);
        previous_end = edit.end;
    }
    let tail = source
        .get(previous_end..)
        .context("invalid UTF-8 edit boundary")?;
    if result.len().saturating_add(tail.len()) > MAX_PATCH_BYTES {
        bail!("result exceeds size limit");
    }
    result.push_str(tail);
    Ok(result)
}

fn digest(source: &str) -> String {
    format!("{:x}", Sha256::digest(source.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn edit(start: usize, end: usize, expected: &str, replacement: &str) -> EditorialEdit {
        EditorialEdit {
            start,
            end,
            expected: expected.into(),
            replacement: replacement.into(),
            reason: "editorial review".into(),
        }
    }
    #[test]
    fn preserves_untouched_unicode_and_applies_in_order() {
        let source = "Été: one, two. Fin.";
        let patch = propose(
            source,
            vec![edit(7, 10, "one", "un"), edit(12, 15, "two", "deux")],
        )
        .unwrap();
        assert_eq!(apply(source, &patch).unwrap(), "Été: un, deux. Fin.");
    }
    #[test]
    fn refuses_stale_incorrect_overlapping_and_ambiguous_ranges() {
        let source = "abcde";
        let patch = propose(source, vec![edit(1, 2, "b", "x")]).unwrap();
        assert!(apply("abcdf", &patch).is_err());
        assert!(propose(source, vec![edit(1, 2, "c", "x")]).is_err());
        assert!(propose(source, vec![edit(1, 4, "bcd", "x"), edit(3, 5, "de", "y")]).is_err());
        assert!(propose(source, vec![edit(1, 1, "", "x")]).is_err());
        assert!(propose("é", vec![edit(1, 2, "", "x")]).is_err());
    }
    #[test]
    fn rejects_unknown_fields_and_missing_reason() {
        assert!(
            serde_json::from_str::<EditorialPatch>(
                r#"{"version":1,"source_sha256":"x","edits":[],"unknown":true}"#
            )
            .is_err()
        );
        assert!(
            propose(
                "a",
                vec![EditorialEdit {
                    reason: " ".into(),
                    ..edit(0, 1, "a", "b")
                }]
            )
            .is_err()
        );
    }
}

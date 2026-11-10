//! Bounded, digest-checked access to proposed content-gate states.
//!
//! A host must provide confined file access. Lexical path validation alone
//! does not prevent symlink traversal or races. URI retrieval is not performed.

use aexeo_contracts::{
    content_gate_change_validation::validate_change_structure, content_gate_protocol::Change,
};
use sha2::{Digest, Sha256};
use std::{
    fmt,
    io::{self, Read},
    path::Path,
};

/// The host must open only files within `root`, including in the presence of
/// symlinks and concurrent filesystem changes. Errors must not expose content.
pub trait ConfinedContentFiles {
    type Reader: Read;
    fn open(&mut self, root: &Path, relative: &Path) -> io::Result<Self::Reader>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceError {
    InvalidChange,
    Unavailable,
    TooLarge,
    DigestMismatch,
    InvalidUtf8,
}

impl fmt::Display for SourceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidChange => "invalid change structure",
            Self::Unavailable => "content source unavailable",
            Self::TooLarge => "content exceeds the configured size limit",
            Self::DigestMismatch => "content digest mismatch",
            Self::InvalidUtf8 => "content is not UTF-8",
        })
    }
}
impl std::error::Error for SourceError {}

/// An evaluated entry's proposed text; a deletion has no proposed text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedEntry {
    pub entity_id: String,
    pub text: Option<String>,
}

/// Resolve every `after` state, verify its exact bytes, and enforce per-entry
/// and aggregate limits. `before` states are not read by this function.
/// Failures return no partial result. URI sources are explicitly unavailable.
pub fn resolve_proposed_content<P: ConfinedContentFiles>(
    change: &Change,
    provider: &mut P,
    max_entry_bytes: usize,
    max_total_bytes: usize,
) -> Result<Vec<ResolvedEntry>, SourceError> {
    validate_change_structure(change).map_err(|_| SourceError::InvalidChange)?;
    let mut remaining = max_total_bytes;
    let mut resolved = Vec::with_capacity(change.entries.len());
    for entry in &change.entries {
        let text = if let Some(state) = &entry.after {
            let limit = max_entry_bytes.min(remaining);
            let bytes = if let Some(inline) = &state.inline {
                if inline.len() > limit {
                    return Err(SourceError::TooLarge);
                }
                inline.as_bytes().to_vec()
            } else if let Some(relative) = &state.file {
                let root = change
                    .content_root
                    .as_deref()
                    .ok_or(SourceError::InvalidChange)?;
                let reader = provider
                    .open(Path::new(root), Path::new(relative))
                    .map_err(|_| SourceError::Unavailable)?;
                let mut bytes = Vec::new();
                reader
                    .take((limit as u64).saturating_add(1))
                    .read_to_end(&mut bytes)
                    .map_err(|_| SourceError::Unavailable)?;
                if bytes.len() > limit {
                    return Err(SourceError::TooLarge);
                }
                bytes
            } else {
                return Err(SourceError::Unavailable);
            };
            remaining -= bytes.len();
            let actual = format!("{:x}", Sha256::digest(&bytes));
            if actual != state.sha256 {
                return Err(SourceError::DigestMismatch);
            }
            Some(String::from_utf8(bytes).map_err(|_| SourceError::InvalidUtf8)?)
        } else {
            None
        };
        resolved.push(ResolvedEntry {
            entity_id: entry.entity_id.clone(),
            text,
        });
    }
    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    struct Fixture {
        bytes: Vec<u8>,
        opens: usize,
    }
    impl ConfinedContentFiles for Fixture {
        type Reader = Cursor<Vec<u8>>;
        fn open(&mut self, root: &Path, relative: &Path) -> io::Result<Self::Reader> {
            assert!(root.is_absolute());
            assert_eq!(relative, Path::new("posts/a.md"));
            self.opens += 1;
            Ok(Cursor::new(self.bytes.clone()))
        }
    }

    fn change() -> Change {
        serde_json::from_value(serde_json::json!({
            "schema": "content-gate.change/v1", "change_id": "x", "created_at": "2026-01-01T00:00:00Z",
            "origin": {"kind": "manual"}, "base": {"kind": "opaque", "revision": "1"},
            "content_root": std::env::temp_dir().to_string_lossy(), "entries": [
                {"entity_id": "inline", "operation": "create", "media_type": "text/plain",
                 "after": {"inline": "abc", "sha256": "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"}},
                {"entity_id": "file", "operation": "update", "media_type": "text/markdown",
                 "after": {"file": "posts/a.md", "sha256": "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"}},
                {"entity_id": "deleted", "operation": "delete", "media_type": "text/plain"}
            ]
        })).unwrap()
    }

    #[test]
    fn resolves_inline_file_and_deletion_in_order() {
        let mut provider = Fixture {
            bytes: b"abc".to_vec(),
            opens: 0,
        };
        let result = resolve_proposed_content(&change(), &mut provider, 3, 6).unwrap();
        assert_eq!(
            result
                .iter()
                .map(|entry| entry.text.as_deref())
                .collect::<Vec<_>>(),
            vec![Some("abc"), Some("abc"), None]
        );
        assert_eq!(provider.opens, 1);
    }

    #[test]
    fn rejects_digest_mismatch_and_enforces_both_limits() {
        let mut provider = Fixture {
            bytes: b"abc".to_vec(),
            opens: 0,
        };
        assert_eq!(
            resolve_proposed_content(&change(), &mut provider, 2, 6),
            Err(SourceError::TooLarge)
        );
        assert_eq!(
            resolve_proposed_content(&change(), &mut provider, 3, 5),
            Err(SourceError::TooLarge)
        );
        provider.bytes = b"bad".to_vec();
        assert_eq!(
            resolve_proposed_content(&change(), &mut provider, 3, 6),
            Err(SourceError::DigestMismatch)
        );
    }

    #[test]
    fn rejects_invalid_utf8_and_does_not_fetch_uris() {
        let mut provider = Fixture {
            bytes: vec![255],
            opens: 0,
        };
        let mut input = change();
        input.entries[1].after.as_mut().unwrap().sha256 = format!("{:x}", Sha256::digest([255]));
        assert_eq!(
            resolve_proposed_content(&input, &mut provider, 3, 6),
            Err(SourceError::InvalidUtf8)
        );
        let state = input.entries[1].after.as_mut().unwrap();
        state.file = None;
        state.uri = Some("https://example.org/document".into());
        provider.opens = 0;
        assert_eq!(
            resolve_proposed_content(&input, &mut provider, 3, 6),
            Err(SourceError::Unavailable)
        );
        assert_eq!(provider.opens, 0);
    }

    #[test]
    fn refuses_invalid_structure_before_calling_provider() {
        let mut provider = Fixture {
            bytes: vec![],
            opens: 0,
        };
        let mut input = change();
        input.entries[1].after.as_mut().unwrap().file = Some("../secret".into());
        assert_eq!(
            resolve_proposed_content(&input, &mut provider, 3, 6),
            Err(SourceError::InvalidChange)
        );
        assert_eq!(provider.opens, 0);
    }
}

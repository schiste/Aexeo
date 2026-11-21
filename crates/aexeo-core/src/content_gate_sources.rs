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

/// Linux directory-capability provider backed by the atomic `openat2` lookup.
/// The caller chooses the trusted root, and change documents cannot replace it.
/// The kernel must support `openat2`; unsupported kernels fail closed.
#[cfg(target_os = "linux")]
pub struct LinuxConfinedContentFiles {
    root_path: std::path::PathBuf,
    root: std::fs::File,
}

#[cfg(target_os = "linux")]
impl LinuxConfinedContentFiles {
    pub fn new(root: &Path) -> io::Result<Self> {
        if !root.is_absolute() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "content root must be absolute",
            ));
        }
        let directory = std::fs::File::open(root)?;
        if !directory.metadata()?.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "content root must be a directory",
            ));
        }
        Ok(Self {
            root_path: root.to_path_buf(),
            root: directory,
        })
    }
}

#[cfg(target_os = "linux")]
impl ConfinedContentFiles for LinuxConfinedContentFiles {
    type Reader = std::fs::File;

    fn open(&mut self, root: &Path, relative: &Path) -> io::Result<Self::Reader> {
        use rustix::fs::{Mode, OFlags, ResolveFlags, openat2};
        use std::path::Component;

        if root != self.root_path
            || relative.as_os_str().is_empty()
            || !relative
                .components()
                .all(|part| matches!(part, Component::Normal(_)))
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "unapproved content root or path",
            ));
        }
        let file = std::fs::File::from(openat2(
            &self.root,
            relative,
            OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NONBLOCK,
            Mode::empty(),
            ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
        )?);
        if !file.metadata()?.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "content is not a regular file",
            ));
        }
        Ok(file)
    }
}

/// macOS directory-capability provider using a single confined kernel lookup.
/// Unsupported kernels fail closed rather than silently ignoring the flags.
#[cfg(target_os = "macos")]
pub struct MacosConfinedContentFiles {
    root_path: std::path::PathBuf,
    root: std::fs::File,
}

#[cfg(target_os = "macos")]
impl MacosConfinedContentFiles {
    pub fn new(root: &Path) -> io::Result<Self> {
        use rustix::fs::{Mode, OFlags, openat};
        if !root.is_absolute() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "content root must be absolute",
            ));
        }
        let directory = std::fs::File::open(root)?;
        if !directory.metadata()?.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "content root must be a directory",
            ));
        }
        // Darwin's ENOTCAPABLE is 107. Do not accept a generic error: an older
        // kernel may ignore O_RESOLVE_BENEATH or reject it for another reason.
        let flags = OFlags::RDONLY | OFlags::CLOEXEC | OFlags::from_bits_retain(0x1000);
        match openat(&directory, "..", flags, Mode::empty()) {
            Err(error) if error.raw_os_error() == 107 => {}
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "kernel does not enforce O_RESOLVE_BENEATH",
                ));
            }
        }
        // O_NOFOLLOW_ANY was introduced before O_RESOLVE_BENEATH. BENEATH is
        // the containment boundary even if a platform ignores NOFOLLOW_ANY.
        Ok(Self {
            root_path: root.to_path_buf(),
            root: directory,
        })
    }
}

#[cfg(target_os = "macos")]
impl ConfinedContentFiles for MacosConfinedContentFiles {
    type Reader = std::fs::File;
    fn open(&mut self, root: &Path, relative: &Path) -> io::Result<Self::Reader> {
        use rustix::fs::{Mode, OFlags, openat};
        use std::path::Component;
        if root != self.root_path
            || relative.as_os_str().is_empty()
            || !relative
                .components()
                .all(|part| matches!(part, Component::Normal(_)))
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "unapproved content root or path",
            ));
        }
        // O_RESOLVE_BENEATH is currently absent from rustix's Darwin bindings.
        // Its ABI value is defined by Apple's sys/fcntl.h.
        let flags = OFlags::RDONLY
            | OFlags::CLOEXEC
            | OFlags::NONBLOCK
            | OFlags::NOFOLLOW_ANY
            | OFlags::from_bits_retain(0x1000);
        let file = std::fs::File::from(openat(&self.root, relative, flags, Mode::empty())?);
        if !file.metadata()?.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "content is not a regular file",
            ));
        }
        Ok(file)
    }
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

/// Which declared state the host must acquire independently.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentPhase {
    Before,
    After,
}

/// The host supplies independently acquired bytes for each declared state.
/// It must bind its response to the requested entity, phase and trusted snapshot;
/// the verifier cannot authenticate the provider or make its reads atomic.
pub trait IndependentContent {
    type Reader: Read;
    fn open(&mut self, entity_id: &str, phase: ContentPhase) -> io::Result<Self::Reader>;
}

/// Check all declared before/after bytes against independent host streams.
/// Inline content must also match the independently obtained bytes exactly.
/// Missing states are not invented, URI sources are not fetched, and snapshot
/// authenticity, freshness, completeness and publication atomicity remain the host's responsibility.
pub fn verify_declared_content<P: IndependentContent>(
    change: &Change,
    provider: &mut P,
    max_state_bytes: usize,
    max_total_bytes: usize,
) -> Result<(), SourceError> {
    validate_change_structure(change).map_err(|_| SourceError::InvalidChange)?;
    let mut remaining = max_total_bytes;
    let mut buffer = [0u8; 8192];
    for entry in &change.entries {
        for (phase, state) in [
            (ContentPhase::Before, &entry.before),
            (ContentPhase::After, &entry.after),
        ] {
            let Some(state) = state else { continue };
            let limit = max_state_bytes.min(remaining);
            let mut reader = provider
                .open(&entry.entity_id, phase)
                .map_err(|_| SourceError::Unavailable)?;
            let mut hasher = Sha256::new();
            let mut length = 0usize;
            loop {
                let count = reader
                    .read(&mut buffer)
                    .map_err(|_| SourceError::Unavailable)?;
                if count == 0 {
                    break;
                }
                let end = length.checked_add(count).ok_or(SourceError::TooLarge)?;
                if end > limit {
                    return Err(SourceError::TooLarge);
                }
                if let Some(inline) = &state.inline
                    && inline.as_bytes().get(length..end) != Some(&buffer[..count])
                {
                    return Err(SourceError::DigestMismatch);
                }
                hasher.update(&buffer[..count]);
                length = end;
            }
            if state
                .inline
                .as_ref()
                .is_some_and(|inline| inline.len() != length)
                || format!("{:x}", hasher.finalize()) != state.sha256
            {
                return Err(SourceError::DigestMismatch);
            }
            remaining -= length;
        }
    }
    Ok(())
}

/// Check declared file-backed states against the corresponding candidate bytes.
///
/// The host selects a confined provider and must establish snapshot provenance,
/// completeness, freshness and publication atomicity. URI sources are unavailable.
/// This helper does not authorize publication or evaluate the content.
pub fn verify_candidate_file_states<P: ConfinedContentFiles>(
    change: &Change,
    base: &crate::content_candidate::CandidateSnapshot,
    after: &[crate::content_candidate::CandidateItem],
    provider: &mut P,
    max_state_bytes: usize,
    max_total_bytes: usize,
) -> Result<(), SourceError> {
    use aexeo_contracts::content_gate_protocol::Operation;
    use std::collections::BTreeMap;

    crate::content_candidate::overlay_candidate(base, change, after)
        .map_err(|_| SourceError::InvalidChange)?;
    let previous: BTreeMap<&str, _> = base
        .items
        .iter()
        .map(|item| (item.entity_id.as_str(), item))
        .collect();
    let projected: BTreeMap<&str, _> = after
        .iter()
        .map(|item| (item.entity_id.as_str(), item))
        .collect();
    let mut remaining = max_total_bytes;
    let mut buffer = [0u8; 8192];
    for entry in &change.entries {
        let old_id = if matches!(entry.operation, Operation::Rename) {
            entry
                .previous_entity_id
                .as_deref()
                .ok_or(SourceError::InvalidChange)?
        } else {
            &entry.entity_id
        };
        for (state, item) in [
            (entry.before.as_ref(), previous.get(old_id)),
            (
                entry.after.as_ref(),
                projected.get(entry.entity_id.as_str()),
            ),
        ] {
            let Some(state) = state else { continue };
            let expected = item.ok_or(SourceError::InvalidChange)?.source.as_bytes();
            if expected.len() > max_state_bytes || expected.len() > remaining {
                return Err(SourceError::TooLarge);
            }
            remaining -= expected.len();
            if state.inline.is_some() {
                continue;
            }
            let relative = state.file.as_deref().ok_or(SourceError::Unavailable)?;
            let root = change
                .content_root
                .as_deref()
                .ok_or(SourceError::InvalidChange)?;
            let mut reader = provider
                .open(Path::new(root), Path::new(relative))
                .map_err(|_| SourceError::Unavailable)?;
            let mut offset = 0usize;
            loop {
                let count = reader
                    .read(&mut buffer)
                    .map_err(|_| SourceError::Unavailable)?;
                if count == 0 {
                    break;
                }
                let end = offset.checked_add(count).ok_or(SourceError::TooLarge)?;
                if expected.get(offset..end) != Some(&buffer[..count]) {
                    return Err(SourceError::DigestMismatch);
                }
                offset = end;
            }
            if offset != expected.len() {
                return Err(SourceError::DigestMismatch);
            }
        }
    }
    Ok(())
}

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
    fn candidate_file_states_match_both_snapshots_with_confined_bounded_reads() {
        use crate::content_candidate::{CandidateItem, CandidateSnapshot};
        let mut input = change();
        input.entries.retain(|entry| entry.entity_id == "file");
        input.entries[0].before = input.entries[0].after.clone();
        let item = CandidateItem {
            entity_id: "file".into(),
            public_path: "/file".into(),
            media_type: "text/markdown".into(),
            revision: None,
            sha256: input.entries[0].after.as_ref().unwrap().sha256.clone(),
            source: "abc".into(),
            links: None,
        };
        let base = CandidateSnapshot {
            scope: "published".into(),
            site_id: "site".into(),
            site_base_url: None,
            base: input.base.clone(),
            complete: true,
            items: vec![item.clone()],
        };
        let after = vec![item];
        let mut files = Fixture {
            bytes: b"abc".to_vec(),
            opens: 0,
        };
        assert_eq!(
            verify_candidate_file_states(&input, &base, &after, &mut files, 3, 6),
            Ok(())
        );
        assert_eq!(files.opens, 2);
        for bytes in [b"bad".as_slice(), b"ab".as_slice(), b"abcd".as_slice()] {
            files.bytes = bytes.to_vec();
            assert_eq!(
                verify_candidate_file_states(&input, &base, &after, &mut files, 8, 16),
                Err(SourceError::DigestMismatch)
            );
        }
        files.bytes = b"abc".to_vec();
        assert_eq!(
            verify_candidate_file_states(&input, &base, &after, &mut files, 2, 6),
            Err(SourceError::TooLarge)
        );
        assert_eq!(
            verify_candidate_file_states(&input, &base, &after, &mut files, 3, 5),
            Err(SourceError::TooLarge)
        );
        let mut uri = input.clone();
        let state = uri.entries[0].before.as_mut().unwrap();
        state.file = None;
        state.uri = Some("https://example.invalid/old".into());
        assert_eq!(
            verify_candidate_file_states(&uri, &base, &after, &mut files, 3, 6),
            Err(SourceError::Unavailable)
        );
        let mut altered = base.clone();
        altered.items[0].source = "other".into();
        assert_eq!(
            verify_candidate_file_states(&input, &altered, &after, &mut files, 8, 16),
            Err(SourceError::InvalidChange)
        );
    }

    #[cfg(target_os = "linux")]
    type PlatformProvider = LinuxConfinedContentFiles;
    #[cfg(target_os = "macos")]
    type PlatformProvider = MacosConfinedContentFiles;

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn platform_provider_refuses_escape_symlinks_and_non_files() {
        use std::{fs, os::unix::fs::symlink};
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("posts")).unwrap();
        fs::write(root.path().join("posts/article.md"), "inside").unwrap();
        fs::write(outside.path().join("secret"), "outside").unwrap();
        symlink(outside.path(), root.path().join("linked-dir")).unwrap();
        symlink(
            outside.path().join("secret"),
            root.path().join("linked-file"),
        )
        .unwrap();
        let mut provider = PlatformProvider::new(root.path()).unwrap();
        let mut text = String::new();
        provider
            .open(root.path(), Path::new("posts/article.md"))
            .unwrap()
            .read_to_string(&mut text)
            .unwrap();
        assert_eq!(text, "inside");
        for path in [
            "../secret",
            "linked-dir/secret",
            "linked-file",
            "posts",
            "/etc/passwd",
        ] {
            assert!(
                provider.open(root.path(), Path::new(path)).is_err(),
                "{path}"
            );
        }
        assert!(provider.open(outside.path(), Path::new("secret")).is_err());
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn platform_provider_pins_root_when_path_is_replaced() {
        use std::fs;
        let parent = tempfile::tempdir().unwrap();
        let root = parent.path().join("root");
        fs::create_dir(&root).unwrap();
        fs::write(root.join("article"), "original").unwrap();
        let mut provider = PlatformProvider::new(&root).unwrap();
        fs::rename(&root, parent.path().join("old")).unwrap();
        fs::create_dir(&root).unwrap();
        fs::write(root.join("article"), "replacement").unwrap();
        let mut text = String::new();
        provider
            .open(&root, Path::new("article"))
            .unwrap()
            .read_to_string(&mut text)
            .unwrap();
        assert_eq!(text, "original");
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn platform_provider_confines_concurrent_directory_replacement() {
        use std::{fs, os::unix::fs::symlink};
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("pivot")).unwrap();
        fs::write(root.path().join("pivot/article"), "inside").unwrap();
        fs::write(outside.path().join("article"), "outside").unwrap();
        let mut provider = PlatformProvider::new(root.path()).unwrap();
        std::thread::scope(|scope| {
            scope.spawn(|| {
                for _ in 0..1000 {
                    fs::rename(root.path().join("pivot"), root.path().join("held")).unwrap();
                    symlink(outside.path(), root.path().join("pivot")).unwrap();
                    fs::remove_file(root.path().join("pivot")).unwrap();
                    fs::rename(root.path().join("held"), root.path().join("pivot")).unwrap();
                }
            });
            for _ in 0..4000 {
                if let Ok(mut file) = provider.open(root.path(), Path::new("pivot/article")) {
                    let mut contents = String::new();
                    file.read_to_string(&mut contents).unwrap();
                    assert_eq!(contents, "inside");
                }
            }
        });
    }

    #[test]
    fn checks_independent_declared_states_and_rejects_missing_wrong_or_oversized_content() {
        struct IndependentFixture {
            items: Vec<(&'static str, ContentPhase, Vec<u8>)>,
        }
        impl IndependentContent for IndependentFixture {
            type Reader = Cursor<Vec<u8>>;
            fn open(&mut self, entity: &str, phase: ContentPhase) -> io::Result<Self::Reader> {
                self.items
                    .iter()
                    .find(|(id, side, _)| *id == entity && *side == phase)
                    .map(|(_, _, bytes)| Cursor::new(bytes.clone()))
                    .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "unavailable"))
            }
        }
        let mut candidate = change();
        candidate.entries[0].operation = aexeo_contracts::content_gate_protocol::Operation::Update;
        candidate.entries[0].before = candidate.entries[0].after.clone();
        let mut fixture = IndependentFixture {
            items: vec![
                ("inline", ContentPhase::Before, b"abc".to_vec()),
                ("inline", ContentPhase::After, b"abc".to_vec()),
                ("file", ContentPhase::After, b"abc".to_vec()),
            ],
        };
        assert!(verify_declared_content(&candidate, &mut fixture, 3, 9).is_ok());
        assert_eq!(
            verify_declared_content(&candidate, &mut fixture, 2, 9),
            Err(SourceError::TooLarge)
        );
        assert_eq!(
            verify_declared_content(&candidate, &mut fixture, 3, 8),
            Err(SourceError::TooLarge)
        );
        fixture.items[0].2 = b"abd".to_vec();
        assert_eq!(
            verify_declared_content(&candidate, &mut fixture, 3, 9),
            Err(SourceError::DigestMismatch)
        );
        fixture.items[0].2 = b"abc".to_vec();
        fixture.items.pop();
        assert_eq!(
            verify_declared_content(&candidate, &mut fixture, 3, 9),
            Err(SourceError::Unavailable)
        );
        fixture
            .items
            .push(("file", ContentPhase::After, b"abd".to_vec()));
        assert_eq!(
            verify_declared_content(&candidate, &mut fixture, 3, 9),
            Err(SourceError::DigestMismatch)
        );
        fixture.items[2].2 = b"abc".to_vec();
        let file_state = candidate.entries[1].after.as_mut().unwrap();
        file_state.file = None;
        file_state.uri = Some("https://example.invalid/content".into());
        assert!(verify_declared_content(&candidate, &mut fixture, 3, 9).is_ok());
        candidate.entries[0].after.as_mut().unwrap().inline = Some("different".into());
        assert_eq!(
            verify_declared_content(&candidate, &mut fixture, 20, 30),
            Err(SourceError::DigestMismatch)
        );
        let long = "a".repeat(8193);
        candidate.entries.truncate(1);
        candidate.entries[0].before = None;
        let state = candidate.entries[0].after.as_mut().unwrap();
        state.inline = Some(long.clone());
        state.sha256 = format!("{:x}", Sha256::digest(long.as_bytes()));
        fixture.items = vec![("inline", ContentPhase::After, long.into_bytes())];
        assert!(verify_declared_content(&candidate, &mut fixture, 8193, 8193).is_ok());
        fixture.items[0].2[8192] = b'b';
        assert_eq!(
            verify_declared_content(&candidate, &mut fixture, 8193, 8193),
            Err(SourceError::DigestMismatch)
        );
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

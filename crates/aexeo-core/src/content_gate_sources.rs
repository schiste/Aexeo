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

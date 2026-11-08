//! Test-only temporary directories. This is not the crates.io `tempfile` crate.
//!
//! The name is deliberate: call sites read `tempfile::tempdir()` as they would
//! with the real crate, which offers the same `tempdir()` and `TempDir::path()`.
//!
//! It stays because the tests use nothing else from that API. The real crate
//! would add `fastrand`, `getrandom` and `rustix` to `Cargo.lock`, all subject
//! to the `multiple-versions = "deny"` check in `deny.toml`, to replace about 50
//! lines. It is a dev-dependency of `aexeo-core` and `aexeo-cli`, unpublished,
//! and must not become a runtime dependency: directories get a predictable name
//! and default permissions.

#![forbid(unsafe_code)]

use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug)]
pub struct TempDir {
    path: PathBuf,
}

impl TempDir {
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

pub fn tempdir() -> io::Result<TempDir> {
    let base = env::temp_dir();
    for _ in 0..64 {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let pid = std::process::id();
        let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
        let candidate = base.join(format!("aexeo-temp-{pid}-{now}-{counter}"));
        match fs::create_dir(&candidate) {
            Ok(()) => return Ok(TempDir { path: candidate }),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "failed to allocate unique temporary directory",
    ))
}

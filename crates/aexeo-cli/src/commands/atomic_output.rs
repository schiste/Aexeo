//! Streaming writes through a sibling file so failed output preserves the old file.
use std::collections::hash_map::RandomState;
use std::fs::{self, File, OpenOptions};
use std::hash::{BuildHasher, Hasher};
use std::io;
use std::path::Path;

pub(super) enum OutputMode {
    /// The verdict protocol writes private files.
    Private,
    /// Preserve an existing destination's mode, or use the normal creation mode.
    Preserve,
}

pub(super) fn write_atomic<E>(
    destination: &Path,
    mode: OutputMode,
    write: impl FnOnce(&mut File) -> Result<(), E>,
) -> Result<(), E>
where
    E: From<io::Error>,
{
    let permissions = match mode {
        OutputMode::Private => None,
        OutputMode::Preserve => match fs::symlink_metadata(destination) {
            Ok(metadata) => {
                if !metadata.file_type().is_file() {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "output must be a regular file",
                    )
                    .into());
                }
                #[cfg(unix)]
                {
                    use std::os::unix::fs::MetadataExt;
                    if metadata.nlink() > 1 {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidInput,
                            "output must not have multiple hard links",
                        )
                        .into());
                    }
                }
                Some(metadata.permissions())
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        },
    };
    let parent = destination
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    if matches!(mode, OutputMode::Private) || permissions.is_some() {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    for _ in 0..32 {
        let token = RandomState::new().build_hasher().finish();
        let path = parent.join(format!(".aexeo-output-{token:016x}.tmp"));
        let mut file = match options.open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        };
        let result = write(&mut file).and_then(|()| {
            if let Some(permissions) = permissions {
                file.set_permissions(permissions).map_err(E::from)?;
            }
            Ok(())
        });
        drop(file);
        let result = result.and_then(|()| fs::rename(&path, destination).map_err(E::from));
        if result.is_err() {
            let _ = fs::remove_file(&path);
        }
        return result;
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not allocate output temporary file",
    )
    .into())
}

#[cfg(test)]
mod tests {
    use super::{OutputMode, write_atomic};
    use std::{
        fs,
        io::{self, Write},
    };

    #[test]
    fn failed_stream_preserves_previous_output_and_removes_temporary_file() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("report.json");
        fs::write(&target, b"previous").unwrap();
        let result: io::Result<()> = write_atomic(&target, OutputMode::Preserve, |file| {
            file.write_all(b"partial")?;
            Err(io::Error::other("injected write failure"))
        });
        assert!(result.is_err());
        assert_eq!(fs::read(&target).unwrap(), b"previous");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn successful_stream_replaces_previous_output() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("report.json");
        fs::write(&target, b"previous").unwrap();
        write_atomic(&target, OutputMode::Preserve, |file| {
            file.write_all(b"next")
        })
        .unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"next");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn refuses_symlinks_and_hard_links_without_changing_targets() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target");
        let alias = dir.path().join("alias");
        fs::write(&target, b"original").unwrap();
        symlink(&target, &alias).unwrap();
        let result: io::Result<()> =
            write_atomic(&alias, OutputMode::Preserve, |file| file.write_all(b"next"));
        assert!(result.is_err());
        assert_eq!(fs::read(&target).unwrap(), b"original");
        fs::remove_file(&alias).unwrap();
        fs::hard_link(&target, &alias).unwrap();
        let result: io::Result<()> =
            write_atomic(&alias, OutputMode::Preserve, |file| file.write_all(b"next"));
        assert!(result.is_err());
        assert_eq!(fs::read(&target).unwrap(), b"original");
    }

    #[cfg(unix)]
    #[test]
    fn preserves_existing_permissions_and_private_output() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("report.json");
        fs::write(&target, b"old").unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o640)).unwrap();
        write_atomic(&target, OutputMode::Preserve, |file| {
            file.write_all(b"next")
        })
        .unwrap();
        assert_eq!(
            fs::metadata(&target).unwrap().permissions().mode() & 0o777,
            0o640
        );
        let private = dir.path().join("verdict.json");
        write_atomic(&private, OutputMode::Private, |file| {
            file.write_all(b"next")
        })
        .unwrap();
        assert_eq!(
            fs::metadata(&private).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}

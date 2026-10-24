use crate::{CoreError, Result};
use std::{
    fs::{File, OpenOptions},
    io::Write,
    os::{fd::AsRawFd, unix::fs::OpenOptionsExt},
    path::{Component, Path},
};

/// Its lifetime spans the runtime. Never unlink a flock file: a new inode allows two owners.
pub struct LibraryLease {
    _file: File,
}
impl LibraryLease {
    pub fn acquire(directory: &Path) -> Result<Self> {
        std::fs::create_dir_all(directory).map_err(io_error)?;
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(directory.join("engine.lock"))
            .map_err(io_error)?;
        // SAFETY: this is a live owned descriptor; flock does not retain pointers.
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::WouldBlock {
                return Err(CoreError::Busy);
            }
            return Err(io_error(error));
        }
        Ok(Self { _file: file })
    }
}

pub fn io_error(error: std::io::Error) -> CoreError {
    CoreError::Storage(error.to_string())
}

pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let temp = path.with_extension("pending");
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&temp)
        .map_err(io_error)?;
    file.write_all(bytes).map_err(io_error)?;
    file.sync_all().map_err(io_error)?;
    std::fs::rename(&temp, path).map_err(io_error)?;
    File::open(
        path.parent()
            .ok_or_else(|| CoreError::Storage("Missing state directory".into()))?,
    )
    .map_err(io_error)?
    .sync_all()
    .map_err(io_error)
}

/// Validate every existing path component before the engine gets a write-capable path.
pub fn validate_destination(root: &Path, relative_paths: &[String]) -> Result<()> {
    if !root.is_absolute() || !root.is_dir() {
        return Err(CoreError::InvalidInput(
            "Choose an existing download folder.".into(),
        ));
    }
    let canonical = root.canonicalize().map_err(io_error)?;
    for name in relative_paths {
        let path = Path::new(name);
        if path.as_os_str().is_empty()
            || path
                .components()
                .any(|p| !matches!(p, Component::Normal(_)))
        {
            return Err(CoreError::InvalidInput(
                "The torrent contains an unsafe file path.".into(),
            ));
        }
        let mut current = canonical.clone();
        for component in path.components() {
            current.push(component);
            match std::fs::symlink_metadata(&current) {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    return Err(CoreError::InvalidInput(
                        "A torrent file points through a symbolic link. Choose another folder."
                            .into(),
                    ));
                }
                Ok(_) => (),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
                Err(e) => return Err(io_error(e)),
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lease_is_exclusive_and_released_without_unlinking() {
        let dir = tempfile::tempdir().unwrap();
        let owner = LibraryLease::acquire(dir.path()).unwrap();
        assert!(matches!(
            LibraryLease::acquire(dir.path()),
            Err(CoreError::Busy)
        ));
        drop(owner);
        assert!(LibraryLease::acquire(dir.path()).is_ok());
    }
    #[test]
    fn paths_cannot_escape_through_parent_or_symlink() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), dir.path().join("escape")).unwrap();
        assert!(validate_destination(dir.path(), &["../escape".into()]).is_err());
        assert!(validate_destination(dir.path(), &["escape/file".into()]).is_err());
        assert!(validate_destination(dir.path(), &["safe/file".into()]).is_ok());
    }
}

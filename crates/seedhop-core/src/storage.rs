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

// Descriptor-relative traversal keeps a concurrent symlink replacement from
// redirecting file-management writes or cleanup outside the chosen folders.
fn relative_parent(
    root: &Path,
    name: &str,
    create: bool,
) -> std::io::Result<(File, std::ffi::CString)> {
    use std::os::{fd::FromRawFd, unix::ffi::OsStrExt};
    let mut directory = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
        .open(root)?;
    let parts: Vec<_> = Path::new(name).components().collect();
    if parts.is_empty() {
        return Err(std::io::Error::other("Empty file name"));
    }
    for (index, part) in parts.iter().enumerate() {
        let Component::Normal(part) = part else {
            return Err(std::io::Error::other("Unsafe file name"));
        };
        let part = std::ffi::CString::new(part.as_bytes())?;
        if index + 1 == parts.len() {
            return Ok((directory, part));
        }
        if create {
            // SAFETY: valid owned parent descriptor and terminated component.
            let result = unsafe { libc::mkdirat(directory.as_raw_fd(), part.as_ptr(), 0o755) };
            if result != 0
                && std::io::Error::last_os_error().kind() != std::io::ErrorKind::AlreadyExists
            {
                return Err(std::io::Error::last_os_error());
            }
            directory.sync_all()?;
        }
        let fd = unsafe {
            libc::openat(
                directory.as_raw_fd(),
                part.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        directory = unsafe { File::from_raw_fd(fd) };
    }
    unreachable!()
}

pub(crate) fn copy_exclusive(
    source_root: &Path,
    source: &str,
    target_root: &Path,
    target: &str,
) -> Result<bool> {
    use std::os::fd::FromRawFd;
    let result = (|| -> std::io::Result<bool> {
        let (parent, source) = match relative_parent(source_root, source, false) {
            Ok(value) => value,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(e) => return Err(e),
        };
        let fd = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                source.as_ptr(),
                libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::NotFound {
                return Ok(false);
            }
            return Err(error);
        }
        let mut input = unsafe { File::from_raw_fd(fd) };
        if !input.metadata()?.is_file() {
            return Err(std::io::Error::other("Source is not a regular file"));
        }
        input.sync_all()?;
        let (parent, target) = relative_parent(target_root, target, true)?;
        let fd = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                target.as_ptr(),
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                0o644,
            )
        };
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        let mut output = unsafe { File::from_raw_fd(fd) };
        let result = std::io::copy(&mut input, &mut output)
            .and_then(|_| output.sync_all())
            .and_then(|_| parent.sync_all());
        if let Err(error) = result {
            unsafe {
                libc::unlinkat(parent.as_raw_fd(), target.as_ptr(), 0);
            }
            return Err(error);
        }
        Ok(true)
    })();
    result.map_err(io_error)
}

pub(crate) fn remove_relative(root: &Path, name: &str) -> Result<()> {
    let (parent, name) = relative_parent(root, name, false).map_err(io_error)?;
    let result = unsafe { libc::unlinkat(parent.as_raw_fd(), name.as_ptr(), 0) };
    if result != 0 {
        let error = std::io::Error::last_os_error();
        if error.kind() != std::io::ErrorKind::NotFound {
            return Err(io_error(error));
        }
    }
    parent.sync_all().map_err(io_error)
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

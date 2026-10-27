use crate::storage_receipt::{CopyEvent, FileStamp};
use crate::{CoreError, Result};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
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
pub(crate) fn relative_parent(
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
    progress: &mut impl FnMut(u64) -> Result<()>,
    event: &mut impl FnMut(CopyEvent) -> Result<()>,
) -> Result<bool> {
    use std::os::fd::FromRawFd;
    progress(0)?;
    (|| -> Result<bool> {
        let (parent, source) = match relative_parent(source_root, source, false) {
            Ok(value) => value,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(e) => return Err(io_error(e)),
        };
        let fd = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                source.as_ptr(),
                libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::NotFound {
                return Ok(false);
            }
            return Err(io_error(error));
        }
        let mut input = unsafe { File::from_raw_fd(fd) };
        if !input.metadata().map_err(io_error)?.is_file() {
            return Err(CoreError::Storage("Source is not a regular file".into()));
        }
        input.sync_all().map_err(io_error)?;
        let (parent, target) = relative_parent(target_root, target, true).map_err(io_error)?;
        let fd = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                target.as_ptr(),
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                0o644,
            )
        };
        if fd < 0 {
            return Err(io_error(std::io::Error::last_os_error()));
        }
        let mut output = unsafe { File::from_raw_fd(fd) };
        // Record ownership durably before writing the first byte. The journal
        // owns rollback, including a file interrupted during this callback.
        event(CopyEvent::Created(FileStamp::read(&output)?))?;
        let result = copy_chunks(&mut input, &mut output, progress).and_then(|digest| {
            output.sync_all().map_err(io_error)?;
            parent.sync_all().map_err(io_error)?;
            Ok(digest)
        })?;
        event(CopyEvent::Completed {
            source: FileStamp::read(&input)?,
            digest: result,
        })?;
        Ok(true)
    })()
}

fn copy_chunks(
    input: &mut impl Read,
    output: &mut impl Write,
    progress: &mut impl FnMut(u64) -> Result<()>,
) -> Result<[u8; 32]> {
    use sha2::{Digest, Sha256};
    let mut hash = Sha256::new();
    let mut buffer = vec![0; 1024 * 1024];
    loop {
        progress(0)?;
        let read = match input.read(&mut buffer) {
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            result => result.map_err(io_error)?,
        };
        if read == 0 {
            return Ok(hash.finalize().into());
        }
        output.write_all(&buffer[..read]).map_err(io_error)?;
        hash.update(&buffer[..read]);
        progress(read as u64)?;
    }
}

/// Copies use logical lengths, not allocated blocks. Reserve the expected final
/// length of present partial files too, since a running source may still grow.
pub(crate) fn preflight_copy(
    source_root: &Path,
    files: &[(&str, u64)],
    target_root: &Path,
) -> Result<u64> {
    let (required, logical) = copy_space_required(source_root, files)?;
    require_copy_space(required, available_space(target_root)?)?;
    Ok(logical)
}

fn copy_space_required(source_root: &Path, files: &[(&str, u64)]) -> Result<(u64, u64)> {
    use std::os::fd::FromRawFd;
    let mut required = 0u64;
    let mut logical = 0u64;
    for (name, expected) in files {
        let (parent, name) = match relative_parent(source_root, name, false) {
            Ok(value) => value,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(io_error(error)),
        };
        // SAFETY: owned parent descriptor and a terminated, validated component.
        let fd = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::NotFound {
                continue;
            }
            return Err(io_error(error));
        }
        // SAFETY: openat returned a new descriptor owned by this File.
        let input = unsafe { File::from_raw_fd(fd) };
        let metadata = input.metadata().map_err(io_error)?;
        if !metadata.is_file() {
            return Err(CoreError::Storage("Source is not a regular file".into()));
        }
        required = required
            .checked_add(metadata.len().max(*expected))
            .ok_or_else(|| CoreError::Storage("The files are too large to copy safely.".into()))?;
        // Logical lengths cannot overflow after the larger reservation passed.
        logical += metadata.len();
    }
    Ok((required, logical))
}

fn available_space(root: &Path) -> Result<u64> {
    let directory = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
        .open(root)
        .map_err(io_error)?;
    let mut stats = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    // SAFETY: a valid descriptor and writable storage for one statvfs record.
    if unsafe { libc::fstatvfs(directory.as_raw_fd(), stats.as_mut_ptr()) } != 0 {
        return Err(io_error(std::io::Error::last_os_error()));
    }
    // SAFETY: successful fstatvfs initialized the record.
    let stats = unsafe { stats.assume_init() };
    let block_size = if stats.f_frsize == 0 {
        stats.f_bsize
    } else {
        stats.f_frsize
    };
    let available = u128::from(stats.f_bavail) * u128::from(block_size);
    Ok(available.min(u128::from(u64::MAX)) as u64)
}

fn require_copy_space(required: u64, available: u64) -> Result<()> {
    if required > available {
        return Err(CoreError::Storage(format!(
            "Not enough space in the destination folder. Copying needs {required} bytes; \
             {available} bytes are available. Free up space or choose another folder. \
             Your original files have not been moved."
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cancelling_a_copy_stops_at_a_chunk_and_removes_only_its_target() {
        let source = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        let content = vec![9; 3 * 1024 * 1024 + 17];
        std::fs::write(source.path().join("original"), &content).unwrap();
        std::fs::write(target.path().join("unrelated"), b"keep").unwrap();
        let mut copied = 0;
        let mut identity = None;
        let result = copy_exclusive(
            source.path(),
            "original",
            target.path(),
            "partial",
            &mut |bytes| {
                copied += bytes;
                if copied > 0 {
                    Err(CoreError::Cancelled)
                } else {
                    Ok(())
                }
            },
            &mut |event| {
                if let CopyEvent::Created(stamp) = event {
                    identity = Some(stamp.identity);
                }
                Ok(())
            },
        );
        assert!(matches!(result, Err(CoreError::Cancelled)));
        assert_eq!(copied, 1024 * 1024);
        let original = File::open(source.path().join("original")).unwrap();
        crate::storage_receipt::remove_checked(
            target.path(),
            "partial",
            "quarantine",
            &identity.unwrap(),
            |file| crate::storage_receipt::is_prefix(&original, file),
        )
        .unwrap();
        assert!(!target.path().join("partial").exists());
        assert_eq!(
            std::fs::read(source.path().join("original")).unwrap(),
            content
        );
        assert_eq!(
            std::fs::read(target.path().join("unrelated")).unwrap(),
            b"keep"
        );
        assert!(
            copy_exclusive(
                source.path(),
                "original",
                target.path(),
                "unrelated",
                &mut |_| Ok(()),
                &mut |_| Ok(())
            )
            .is_err()
        );
        assert_eq!(
            std::fs::read(target.path().join("unrelated")).unwrap(),
            b"keep"
        );
        let mut completed = 0;
        copy_exclusive(
            source.path(),
            "original",
            target.path(),
            "complete",
            &mut |bytes| {
                completed += bytes;
                Ok(())
            },
            &mut |_| Ok(()),
        )
        .unwrap();
        assert_eq!(completed, content.len() as u64);
        assert_eq!(
            std::fs::read(target.path().join("complete")).unwrap(),
            content
        );
    }

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

    #[test]
    fn copy_capacity_counts_logical_partial_and_unselected_files() {
        let source = tempfile::tempdir().unwrap();
        std::fs::write(source.path().join("partial"), [1; 10]).unwrap();
        std::fs::write(source.path().join("unselected"), [2; 20]).unwrap();
        let sparse = File::create(source.path().join("sparse")).unwrap();
        sparse.set_len(1024 * 1024).unwrap();
        let (required, logical) = copy_space_required(
            source.path(),
            &[
                ("partial", 100),
                ("unselected", 5),
                ("sparse", 1024 * 1024),
                ("missing", 999),
                ("missing-parent/file", 999),
            ],
        )
        .unwrap();
        assert_eq!(required, 100 + 20 + 1024 * 1024);
        assert_eq!(logical, 10 + 20 + 1024 * 1024);
        assert!(require_copy_space(required, required).is_ok());
        let error = require_copy_space(required, required - 1)
            .unwrap_err()
            .to_string();
        assert!(error.contains(&format!("needs {required} bytes")));
        assert!(error.contains(&format!("{} bytes are available", required - 1)));
        assert!(error.contains("choose another folder"));
        assert!(require_copy_space(0, 0).is_ok());
    }

    #[test]
    fn copy_preflight_rejects_unsafe_sources_and_overflow_without_targets() {
        let source = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        std::fs::write(source.path().join("file"), [1]).unwrap();
        std::os::unix::fs::symlink("file", source.path().join("link")).unwrap();
        std::fs::create_dir(source.path().join("directory")).unwrap();
        for files in [
            vec![("../escape", 1)],
            vec![("link", 1)],
            vec![("directory", 1)],
            vec![("file", u64::MAX), ("file", 1)],
        ] {
            assert!(preflight_copy(source.path(), &files, target.path()).is_err());
        }
        let error = preflight_copy(source.path(), &[("file", u64::MAX)], target.path())
            .unwrap_err()
            .to_string();
        assert!(error.contains("Not enough space"));
        assert_eq!(std::fs::read_dir(target.path()).unwrap().count(), 0);
        assert_eq!(std::fs::read(source.path().join("file")).unwrap(), [1]);
        assert!(preflight_copy(source.path(), &[("file", 1)], target.path()).is_ok());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn insufficient_space_preserves_layout_intent_and_restart() {
        use crate::{Engine, EngineConfig, adapter};
        let state = tempfile::tempdir().unwrap();
        let source = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        let config = EngineConfig {
            state_directory: state.path().to_string_lossy().into(),
            download_directory: source.path().to_string_lossy().into(),
            allow_network_discovery: false,
            listen_port: 0,
        };
        let content = vec![7; 4096];
        let path = source.path().join("original");
        std::fs::write(&path, &content).unwrap();
        let engine = Engine::open(config.clone()).await.unwrap();
        let id = engine
            .add(
                adapter::create_fixture("original", &content),
                config.download_directory.clone(),
                vec![0],
                true,
                true,
            )
            .await
            .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            while engine.snapshot().unwrap().torrents[0].state != "paused" {
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        engine
            .set_file_priorities(id.clone(), vec![2])
            .await
            .unwrap();
        let before = engine.snapshot().unwrap().torrents.remove(0);
        // Extend a disposable source sparsely, without filling the disk. Copies
        // use logical length, so preflight must reject both commands before I/O.
        let file = OpenOptions::new().write(true).open(&path).unwrap();
        let length = available_space(target.path())
            .unwrap()
            .checked_add(1 << 40)
            .unwrap();
        file.set_len(length).unwrap();
        let moved = engine
            .relocate(id.clone(), target.path().to_string_lossy().into())
            .await;
        let renamed = engine.rename_file(id.clone(), 0, "renamed".into()).await;
        // Restore before asserting so a failing assertion cannot leave a huge
        // logical file for a shutdown/recheck to inspect.
        file.set_len(content.len() as u64).unwrap();
        for result in [moved, renamed] {
            assert!(
                matches!(result, Err(CoreError::Storage(ref error)) if error.contains("Not enough space"))
            );
        }
        assert_eq!(engine.snapshot().unwrap().torrents[0], before);
        assert_eq!(std::fs::read_dir(target.path()).unwrap().count(), 0);
        assert!(!source.path().join("renamed").exists());
        assert_eq!(std::fs::read(&path).unwrap(), content);
        engine.shutdown().await.unwrap();
        drop(engine);
        let restored = Engine::open(config).await.unwrap();
        let torrent = restored.snapshot().unwrap().torrents.remove(0);
        assert_eq!(torrent.id, before.id);
        assert_eq!(torrent.destination, before.destination);
        assert!(torrent.paused_intent);
        assert_eq!(torrent.files[0].selected, before.files[0].selected);
        assert_eq!(torrent.files[0].priority, 2);
        assert_eq!(torrent.files[0].name, "original");
        assert_eq!(torrent.queue_position, before.queue_position);
        restored.shutdown().await.unwrap();
    }
}

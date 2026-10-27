use crate::{CoreError, Result, storage};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{File, OpenOptions},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::fs::{FileExt, MetadataExt, OpenOptionsExt},
    },
    path::Path,
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct FileIdentity {
    device: u64,
    inode: u64,
    created: Option<(u64, u32)>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct FileStamp {
    pub identity: FileIdentity,
    pub length: u64,
    modified: (i64, i64),
}

impl FileStamp {
    pub fn read(file: &File) -> Result<Self> {
        let metadata = file.metadata().map_err(storage::io_error)?;
        Ok(Self {
            identity: FileIdentity {
                device: metadata.dev(),
                inode: metadata.ino(),
                created: metadata
                    .created()
                    .ok()
                    .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|time| (time.as_secs(), time.subsec_nanos())),
            },
            length: metadata.len(),
            modified: (metadata.mtime(), metadata.mtime_nsec()),
        })
    }
}

pub(crate) enum CopyEvent {
    Created(FileStamp),
    Completed { source: FileStamp, digest: [u8; 32] },
}

pub(crate) fn directory_identity(path: &Path) -> Result<FileIdentity> {
    let directory = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
        .open(path)
        .map_err(storage::io_error)?;
    Ok(FileStamp::read(&directory)?.identity)
}

pub(crate) fn open_relative(root: &Path, name: &str) -> Result<Option<File>> {
    let (parent, name) = match storage::relative_parent(root, name, false) {
        Ok(value) => value,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(storage::io_error(error)),
    };
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
            return Ok(None);
        }
        return Err(storage::io_error(error));
    }
    let file = unsafe { File::from_raw_fd(fd) };
    if !file.metadata().map_err(storage::io_error)?.is_file() {
        return Err(CoreError::Storage(
            "A recovery path is not a regular file. Its contents have been preserved.".into(),
        ));
    }
    Ok(Some(file))
}

pub(crate) fn fingerprint(file: &File) -> Result<[u8; 32]> {
    let before = FileStamp::read(file)?;
    let mut digest = Sha256::new();
    let mut buffer = vec![0; 1024 * 1024];
    let mut offset = 0;
    loop {
        let count = file
            .read_at(&mut buffer, offset)
            .map_err(storage::io_error)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
        offset += count as u64;
    }
    if FileStamp::read(file)? != before {
        return Err(CoreError::Storage(
            "A file changed while recovery was checking it. All copies have been preserved.".into(),
        ));
    }
    Ok(digest.finalize().into())
}

pub(crate) fn is_prefix(source: &File, target: &File) -> Result<bool> {
    let original = FileStamp::read(source)?;
    let copy = FileStamp::read(target)?;
    if copy.length > original.length {
        return Ok(false);
    }
    let mut a = vec![0; 1024 * 1024];
    let mut b = vec![0; 1024 * 1024];
    let mut offset = 0;
    while offset < copy.length {
        let count = (copy.length - offset).min(a.len() as u64) as usize;
        source
            .read_exact_at(&mut a[..count], offset)
            .map_err(storage::io_error)?;
        target
            .read_exact_at(&mut b[..count], offset)
            .map_err(storage::io_error)?;
        if a[..count] != b[..count] {
            return Ok(false);
        }
        offset += count as u64;
    }
    Ok(FileStamp::read(source)? == original && FileStamp::read(target)? == copy)
}

fn rename_exclusive(parent: &File, source: &std::ffi::CStr, target: &std::ffi::CStr) -> Result<()> {
    #[cfg(target_os = "macos")]
    let result = unsafe {
        libc::renameatx_np(
            parent.as_raw_fd(),
            source.as_ptr(),
            parent.as_raw_fd(),
            target.as_ptr(),
            libc::RENAME_EXCL,
        )
    };
    #[cfg(target_os = "linux")]
    let result = unsafe {
        libc::renameat2(
            parent.as_raw_fd(),
            source.as_ptr(),
            parent.as_raw_fd(),
            target.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    let result = -1;
    if result != 0 {
        return Err(storage::io_error(std::io::Error::last_os_error()));
    }
    parent.sync_all().map_err(storage::io_error)
}

/// Quarantine names are written in the journal before use, so a crash during
/// cleanup is retryable. Check ownership and bytes after the rename as well:
/// a pathname replaced between inspection and rename must never be deleted.
pub(crate) fn remove_checked(
    root: &Path,
    name: &str,
    quarantine: &str,
    identity: &FileIdentity,
    check: impl Fn(&File) -> Result<bool>,
) -> Result<()> {
    let quarantined = open_relative(root, quarantine)?;
    let file = if let Some(file) = quarantined {
        file
    } else {
        let Some(file) = open_relative(root, name)? else {
            return Ok(());
        };
        if &FileStamp::read(&file)?.identity != identity {
            return Err(CoreError::Storage(format!(
                "A different file now uses {}. It has been preserved.",
                root.join(name).display()
            )));
        }
        let (parent, source) =
            storage::relative_parent(root, name, false).map_err(storage::io_error)?;
        let (_, target) =
            storage::relative_parent(root, quarantine, false).map_err(storage::io_error)?;
        rename_exclusive(&parent, &source, &target)?;
        crate::layout_journal::checkpoint("quarantine");
        open_relative(root, quarantine)?.ok_or_else(|| {
            CoreError::Storage(
                "A recovery file disappeared. Inspect both folders before retrying.".into(),
            )
        })?
    };
    let valid = &FileStamp::read(&file)?.identity == identity && check(&file)?;
    let (parent, temporary) =
        storage::relative_parent(root, quarantine, false).map_err(storage::io_error)?;
    if !valid {
        let (_, original) =
            storage::relative_parent(root, name, false).map_err(storage::io_error)?;
        let _ = rename_exclusive(&parent, &temporary, &original);
        return Err(CoreError::Storage(format!(
            "A file changed since the move. It has been preserved at {} or {}. Inspect the folders before retrying.",
            root.join(name).display(),
            root.join(quarantine).display()
        )));
    }
    // Reopen the quarantined name to reject replacement during verification.
    let current = open_relative(root, quarantine)?
        .ok_or_else(|| CoreError::Storage("A recovery file disappeared.".into()))?;
    if FileStamp::read(&current)? != FileStamp::read(&file)? {
        return Err(CoreError::Storage(
            "A recovery file changed. It has been preserved.".into(),
        ));
    }
    let result = unsafe { libc::unlinkat(parent.as_raw_fd(), temporary.as_ptr(), 0) };
    if result != 0 {
        return Err(storage::io_error(std::io::Error::last_os_error()));
    }
    parent.sync_all().map_err(storage::io_error)?;
    if open_relative(root, name)?.is_some() {
        return Err(CoreError::Storage(format!(
            "Another file now uses {}. It has been preserved.",
            root.join(name).display()
        )));
    }
    Ok(())
}

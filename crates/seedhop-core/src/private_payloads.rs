//! Private payload paths are fresh capabilities, never library state or adopted files.
use crate::{CoreError, Result, storage};
use librqbit::storage::{
    BoxStorageFactory, StorageFactoryExt, filesystem::PinnedFilesystemStorageFactory,
};
use std::{
    collections::HashMap,
    ffi::CString,
    fs::{File, OpenOptions},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::fs::{MetadataExt, OpenOptionsExt},
    },
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

#[derive(Clone)]
struct Directory {
    path: PathBuf,
    file: Arc<File>,
    private: bool,
}

impl Directory {
    fn check(&self) -> Result<()> {
        let named = std::fs::symlink_metadata(&self.path).map_err(storage::io_error)?;
        let owned = self.file.metadata().map_err(storage::io_error)?;
        if !named.is_dir()
            || named.file_type().is_symlink()
            || named.dev() != owned.dev()
            || named.ino() != owned.ino()
            || (self.private
                && (owned.uid() != unsafe { libc::geteuid() } || owned.mode() & 0o077 != 0))
        {
            return Err(CoreError::Storage("The private payload directory was moved or replaced. End this session before using another folder.".into()));
        }
        Ok(())
    }

    fn child(&self, prefix: &str) -> Result<Self> {
        self.check()?;
        let name = format!("{prefix}-{}", uuid::Uuid::new_v4());
        let component = CString::new(name.as_bytes()).expect("UUID directory name contains no NUL");
        // SAFETY: a live directory descriptor and one generated, terminated name.
        let result = unsafe { libc::mkdirat(self.file.as_raw_fd(), component.as_ptr(), 0o700) };
        if result != 0 {
            return Err(storage::io_error(std::io::Error::last_os_error()));
        }
        // SAFETY: open only the new directory; never follow a replacement symlink.
        let fd = unsafe {
            libc::openat(
                self.file.as_raw_fd(),
                component.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err(storage::io_error(std::io::Error::last_os_error()));
        }
        // SAFETY: openat returned a new owned descriptor.
        let file = Arc::new(unsafe { File::from_raw_fd(fd) });
        let metadata = file.metadata().map_err(storage::io_error)?;
        if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o077 != 0 {
            return Err(CoreError::Storage(
                "The private payload directory must belong to this user and be owner-only.".into(),
            ));
        }
        librqbit::storage::filesystem::mark_download(&file)
            .map_err(crate::adapter::engine_error)?;
        self.file.sync_all().map_err(storage::io_error)?;
        let child = Self {
            path: self.path.join(name),
            file,
            private: true,
        };
        self.check()?;
        child.check()?;
        Ok(child)
    }
}

#[derive(Default)]
struct PayloadDirectories {
    session: Option<Directory>,
    transfers: HashMap<String, Directory>,
}

pub(crate) struct PrivatePayloads {
    parent: Directory,
    directories: Mutex<PayloadDirectories>,
}

impl PrivatePayloads {
    /// Pin the caller-approved parent without creating any payload directories.
    pub(crate) fn new(parent: &Path) -> Result<Self> {
        storage::validate_destination(parent, &[])?;
        let path = parent.canonicalize().map_err(storage::io_error)?;
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&path)
            .map_err(storage::io_error)?;
        let parent = Directory {
            path,
            file: Arc::new(file),
            private: false,
        };
        parent.check()?;
        Ok(Self {
            parent,
            directories: Mutex::new(PayloadDirectories::default()),
        })
    }

    pub(crate) fn prepare(
        &self,
        destination: &Path,
        id: &str,
    ) -> Result<(PathBuf, BoxStorageFactory)> {
        self.parent.check()?;
        if !destination.is_absolute()
            || destination.canonicalize().map_err(storage::io_error)? != self.parent.path
        {
            return Err(CoreError::InvalidInput(
                "Private transfers must use this session's chosen parent folder.".into(),
            ));
        }
        let mut directories = self.directories.lock().unwrap();
        if directories.session.is_none() {
            directories.session = Some(self.parent.child("SeedHop")?);
        }
        let session = directories.session.as_ref().unwrap();
        session.check()?;
        let transfer = session.child("Transfer")?;
        let factory = PinnedFilesystemStorageFactory::new(
            transfer.path.clone(),
            transfer.file.try_clone().map_err(storage::io_error)?,
        )
        .map_err(crate::adapter::engine_error)?
        .boxed();
        let path = transfer.path.clone();
        directories.transfers.insert(id.to_owned(), transfer);
        Ok((path, factory))
    }

    pub(crate) fn validate(&self, id: &str, path: &Path) -> Result<()> {
        self.parent.check()?;
        let directories = self.directories.lock().unwrap();
        let transfer = directories.transfers.get(id).ok_or_else(|| {
            CoreError::Storage("This private transfer has no owned payload directory.".into())
        })?;
        if transfer.path != path {
            return Err(CoreError::Storage(
                "The private transfer destination changed.".into(),
            ));
        }
        directories
            .session
            .as_ref()
            .ok_or_else(|| {
                CoreError::Storage("The private session destination is unavailable.".into())
            })?
            .check()?;
        transfer.check()
    }

    /// Release authority only; never delete complete, partial or failed payloads.
    pub(crate) fn forget(&self, id: &str) {
        self.directories.lock().unwrap().transfers.remove(id);
    }
}

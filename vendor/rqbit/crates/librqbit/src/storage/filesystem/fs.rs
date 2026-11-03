use std::{
    fs::OpenOptions,
    io::IoSlice,
    path::{Path, PathBuf},
};

use anyhow::Context;
use tracing::warn;

use crate::{
    storage::{StorageFactoryExt, filesystem::opened_file::OurFileExt},
    torrent_state::{ManagedTorrentShared, TorrentMetadata},
};

use crate::storage::{StorageFactory, TorrentStorage};

use super::opened_file::OpenedFile;

#[derive(Default, Clone, Copy)]
pub struct FilesystemStorageFactory {}

impl StorageFactory for FilesystemStorageFactory {
    type Storage = FilesystemStorage;

    fn create(
        &self,
        shared: &ManagedTorrentShared,
        _metadata: &TorrentMetadata,
    ) -> anyhow::Result<FilesystemStorage> {
        Ok(FilesystemStorage {
            output_folder: shared.options.output_folder.read().clone(),
            opened_files: Default::default(),
            targets: Default::default(),
        })
    }

    fn clone_box(&self) -> crate::storage::BoxStorageFactory {
        self.boxed()
    }
}

pub struct FilesystemStorage {
    pub(crate) output_folder: PathBuf,
    pub(crate) opened_files: Vec<OpenedFile>,
    targets: Vec<PathBuf>,
}

impl FilesystemStorage {
    #[allow(dead_code)]
    pub(crate) fn take_fs(&self) -> anyhow::Result<Self> {
        Ok(Self {
            opened_files: self
                .opened_files
                .iter()
                .map(|f| f.take_clone())
                .collect::<anyhow::Result<Vec<_>>>()?,
            output_folder: self.output_folder.clone(),
            targets: self.targets.clone(),
        })
    }
}

impl TorrentStorage for FilesystemStorage {
    fn prepare_selection(&self, indices: &std::collections::HashSet<usize>) -> anyhow::Result<()> {
        for &index in indices {
            let target = self.targets.get(index).context("no such file")?;
            self.opened_files[index].materialize(self.output_folder.join(target), |source| {
                let source = source.strip_prefix(&self.output_folder)?;
                #[cfg(unix)]
                secure_promote(&self.output_folder, source, target)?;
                #[cfg(not(unix))]
                {
                    let target = self.output_folder.join(target);
                    std::fs::create_dir_all(target.parent().context("no parent")?)?;
                    std::fs::hard_link(self.output_folder.join(source), target)?;
                    std::fs::remove_file(self.output_folder.join(source))?;
                }
                Ok(())
            })?;
        }
        Ok(())
    }

    fn pread_exact(&self, file_id: usize, offset: u64, buf: &mut [u8]) -> anyhow::Result<()> {
        self.opened_files
            .get(file_id)
            .context("no such file")?
            .lock_read()?
            .pread_exact(offset, buf)
    }

    fn pwrite_all(&self, file_id: usize, offset: u64, buf: &[u8]) -> anyhow::Result<()> {
        let of = self.opened_files.get(file_id).context("no such file")?;
        #[cfg(windows)]
        return of.try_mark_sparse()?.pwrite_all(offset, buf);
        #[cfg(not(windows))]
        return of.lock_read()?.pwrite_all(offset, buf);
    }

    fn pwrite_all_vectored(
        &self,
        file_id: usize,
        offset: u64,
        bufs: [IoSlice<'_>; 2],
    ) -> anyhow::Result<usize> {
        let of = self.opened_files.get(file_id).context("no such file")?;
        #[cfg(windows)]
        return of.try_mark_sparse()?.pwrite_all_vectored(offset, bufs);
        #[cfg(not(windows))]
        return of.lock_read()?.pwrite_all_vectored(offset, bufs);
    }

    fn remove_file(&self, file_id: usize, _filename: &Path) -> anyhow::Result<()> {
        let path = self
            .opened_files
            .get(file_id)
            .context("no such file")?
            .path();
        if path.as_os_str().is_empty() {
            return Ok(());
        }
        let filename = path.strip_prefix(&self.output_folder)?;
        #[cfg(unix)]
        return secure_remove(&self.output_folder, filename);
        #[cfg(not(unix))]
        Ok(std::fs::remove_file(self.output_folder.join(filename))?)
    }

    fn ensure_file_length(&self, file_id: usize, len: u64) -> anyhow::Result<()> {
        let f = &self.opened_files.get(file_id).context("no such file")?;
        #[cfg(windows)]
        f.try_mark_sparse()?;
        Ok(f.lock_read()?.set_len(len)?)
    }

    fn take(&self) -> anyhow::Result<Box<dyn TorrentStorage>> {
        Ok(Box::new(Self {
            opened_files: self
                .opened_files
                .iter()
                .map(|f| f.take_clone())
                .collect::<anyhow::Result<Vec<_>>>()?,
            output_folder: self.output_folder.clone(),
            targets: self.targets.clone(),
        }))
    }

    fn remove_directory_if_empty(&self, path: &Path) -> anyhow::Result<()> {
        let path = self.output_folder.join(path);
        if !path.is_dir() {
            anyhow::bail!("cannot remove dir: {path:?} is not a directory")
        }
        if std::fs::read_dir(&path)?.count() == 0 {
            std::fs::remove_dir(&path).with_context(|| format!("error removing {path:?}"))
        } else {
            warn!("did not remove {path:?} as it was not empty");
            Ok(())
        }
    }

    fn init(
        &mut self,
        shared: &ManagedTorrentShared,
        metadata: &TorrentMetadata,
    ) -> anyhow::Result<()> {
        let mut files = Vec::<OpenedFile>::new();
        let names = shared.options.file_names.read();
        let selected = shared.options.selected_files.read();
        let mut targets = Vec::new();
        for (index, file_details) in metadata.file_infos.iter().enumerate() {
            let mut full_path = self.output_folder.clone();
            let target = names.get(&index).unwrap_or(&file_details.relative_filename);
            targets.push(target.clone());
            let wanted = selected.as_ref().is_none_or(|files| files.contains(&index));
            // Adjacent v1 files can share a piece. Keep boundary bytes in private
            // storage, without creating or opening an unchecked payload filename.
            let part = PathBuf::from(format!(
                ".seedhop-parts/{}/{index}",
                shared.info_hash.as_string()
            ));
            let relative_path = if wanted { target } else { &part };
            full_path.push(relative_path);

            if file_details.attrs.padding {
                files.push(OpenedFile::new_dummy());
                continue;
            };
            #[cfg(unix)]
            let f = secure_open(
                &self.output_folder,
                relative_path,
                !wanted
                    || shared
                        .options
                        .allow_overwrite
                        .load(std::sync::atomic::Ordering::Relaxed),
            )?;
            #[cfg(not(unix))]
            std::fs::create_dir_all(full_path.parent().context("bug: no parent")?)?;
            #[cfg(not(unix))]
            let f = if !wanted
                || shared
                    .options
                    .allow_overwrite
                    .load(std::sync::atomic::Ordering::Relaxed)
            {
                OpenOptions::new()
                    .create(true)
                    .truncate(false)
                    .read(true)
                    .write(true)
                    .open(&full_path)
                    .with_context(|| format!("error opening {full_path:?} in read/write mode"))?
            } else {
                // create_new does not seem to work with read(true), so calling this twice.
                OpenOptions::new()
                    .create_new(true)
                    .write(true)
                    .open(&full_path)
                    .with_context(|| {
                        format!(
                            "error creating a new file (because allow_overwrite = false) {:?}",
                            full_path
                        )
                    })?;
                OpenOptions::new().read(true).write(true).open(&full_path)?
            };
            files.push(OpenedFile::new(full_path.clone(), f));
        }

        self.opened_files = files;
        self.targets = targets;
        Ok(())
    }
}

/// SeedHop: traverse using directory descriptors, so swapping a symlink between
/// preflight and open cannot redirect writes outside the selected directory.
#[cfg(unix)]
fn secure_open(root: &Path, relative: &Path, reuse: bool) -> anyhow::Result<std::fs::File> {
    use nix::libc;
    use std::os::fd::{AsRawFd, FromRawFd};
    let (directory, name) = secure_parent(root, relative)?;
    let flags = libc::O_NOFOLLOW
        | libc::O_NONBLOCK
        | libc::O_CLOEXEC
        | libc::O_RDWR
        | libc::O_CREAT
        | if reuse { 0 } else { libc::O_EXCL };
    // SAFETY: the parent descriptor is live and the leaf is a terminated name.
    let fd = unsafe { libc::openat(directory.as_raw_fd(), name.as_ptr(), flags, 0o600) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error())
            .context("refused unsafe or unavailable torrent path");
    }
    let file = unsafe { std::fs::File::from_raw_fd(fd) };
    use std::os::unix::fs::MetadataExt;
    let metadata = file.metadata()?;
    anyhow::ensure!(
        metadata.is_file() && metadata.nlink() == 1,
        "refused non-regular or multiply linked torrent file"
    );
    mark_download(&file)?;
    Ok(file)
}

/// Set quarantine before payload writes, without resolving another pathname.
/// Existing quarantine metadata belongs to the OS and is preserved verbatim.
pub fn mark_download(file: &std::fs::File) -> anyhow::Result<()> {
    #[cfg(target_os = "macos")]
    {
        use std::{
            os::fd::AsRawFd,
            time::{SystemTime, UNIX_EPOCH},
        };
        let name = c"com.apple.quarantine";
        let size = unsafe {
            nix::libc::fgetxattr(
                file.as_raw_fd(),
                name.as_ptr(),
                std::ptr::null_mut(),
                0,
                0,
                0,
            )
        };
        if size > 0 {
            return Ok(());
        }
        if size < 0 && std::io::Error::last_os_error().raw_os_error() != Some(nix::libc::ENOATTR) {
            return Err(std::io::Error::last_os_error())
                .context("could not read download quarantine");
        }
        let timestamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
        let value = format!("0081;{timestamp:x};SeedHop;");
        // SAFETY: descriptor is owned; name and value remain live for the call.
        let result = unsafe {
            nix::libc::fsetxattr(
                file.as_raw_fd(),
                name.as_ptr(),
                value.as_ptr().cast(),
                value.len(),
                0,
                0,
            )
        };
        if result != 0 {
            return Err(std::io::Error::last_os_error())
                .context("destination cannot preserve download quarantine");
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = file;
    Ok(())
}

pub fn copy_download_quarantine(
    source: &std::fs::File,
    target: &std::fs::File,
) -> anyhow::Result<()> {
    mark_download(target)?;
    #[cfg(target_os = "macos")]
    {
        use std::os::fd::AsRawFd;
        let mut value = vec![0u8; 8192];
        let length = unsafe {
            nix::libc::fgetxattr(
                source.as_raw_fd(),
                c"com.apple.quarantine".as_ptr(),
                value.as_mut_ptr().cast(),
                value.len(),
                0,
                0,
            )
        };
        if length < 0 {
            if std::io::Error::last_os_error().raw_os_error() == Some(nix::libc::ENOATTR) {
                return Ok(());
            }
            return Err(std::io::Error::last_os_error())
                .context("could not preserve download quarantine");
        }
        if length > 0 {
            let result = unsafe {
                nix::libc::fsetxattr(
                    target.as_raw_fd(),
                    c"com.apple.quarantine".as_ptr(),
                    value.as_ptr().cast(),
                    length as usize,
                    0,
                    0,
                )
            };
            if result != 0 {
                return Err(std::io::Error::last_os_error())
                    .context("could not copy download quarantine");
            }
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = source;
    Ok(())
}

#[cfg(unix)]
fn secure_parent(
    root: &Path,
    relative: &Path,
) -> anyhow::Result<(std::fs::File, std::ffi::CString)> {
    use nix::libc;
    use std::{
        ffi::CString,
        os::{
            fd::{AsRawFd, FromRawFd},
            unix::{ffi::OsStrExt, fs::OpenOptionsExt},
        },
        path::Component,
    };
    let mut directory = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
        .open(root)?;
    let components: Vec<_> = relative.components().collect();
    anyhow::ensure!(!components.is_empty(), "empty torrent filename");
    for (index, component) in components.iter().enumerate() {
        let Component::Normal(name) = component else {
            anyhow::bail!("unsafe torrent path")
        };
        let name = CString::new(name.as_bytes())?;
        if index + 1 == components.len() {
            return Ok((directory, name));
        }
        // SAFETY: names are terminated C strings; the parent descriptor is live.
        let created = unsafe { libc::mkdirat(directory.as_raw_fd(), name.as_ptr(), 0o700) };
        if created != 0
            && std::io::Error::last_os_error().kind() != std::io::ErrorKind::AlreadyExists
        {
            return Err(std::io::Error::last_os_error().into());
        }
        // SAFETY: descriptor/name are valid; the returned owned fd is wrapped once.
        let fd = unsafe {
            libc::openat(
                directory.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        directory = unsafe { std::fs::File::from_raw_fd(fd) };
        mark_download(&directory)?;
    }
    unreachable!()
}

#[cfg(unix)]
fn secure_promote(root: &Path, source: &Path, target: &Path) -> anyhow::Result<()> {
    use std::os::fd::AsRawFd;
    let (source_parent, source_name) = secure_parent(root, source)?;
    let (target_parent, target_name) = secure_parent(root, target)?;
    // Hard-linking preserves sparse boundary data and refuses an existing target.
    // SAFETY: both descriptors and terminated names remain live for each call.
    let result = unsafe {
        nix::libc::linkat(
            source_parent.as_raw_fd(),
            source_name.as_ptr(),
            target_parent.as_raw_fd(),
            target_name.as_ptr(),
            0,
        )
    };
    if result != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    target_parent.sync_all()?;
    let result = unsafe { nix::libc::unlinkat(source_parent.as_raw_fd(), source_name.as_ptr(), 0) };
    if result != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    source_parent.sync_all()?;
    Ok(())
}

#[cfg(unix)]
fn secure_remove(root: &Path, relative: &Path) -> anyhow::Result<()> {
    use nix::libc;
    use std::{
        ffi::CString,
        os::{
            fd::{AsRawFd, FromRawFd},
            unix::{ffi::OsStrExt, fs::OpenOptionsExt},
        },
        path::Component,
    };
    let mut directory = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
        .open(root)?;
    let components: Vec<_> = relative.components().collect();
    anyhow::ensure!(!components.is_empty(), "empty torrent filename");
    for (index, component) in components.iter().enumerate() {
        let Component::Normal(name) = component else {
            anyhow::bail!("unsafe deletion path")
        };
        let name = CString::new(name.as_bytes())?;
        if index + 1 == components.len() {
            let result = unsafe { libc::unlinkat(directory.as_raw_fd(), name.as_ptr(), 0) };
            if result == 0 {
                return Ok(());
            }
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::NotFound {
                return Ok(());
            }
            return Err(error.into());
        }
        let fd = unsafe {
            libc::openat(
                directory.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::NotFound {
                return Ok(());
            }
            return Err(error.into());
        }
        directory = unsafe { std::fs::File::from_raw_fd(fd) };
    }
    Ok(())
}

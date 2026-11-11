//! SeedHop: reopen only files created through this directory capability.
use anyhow::Context;
use std::{
    collections::HashMap,
    ffi::CString,
    fs::File,
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{ffi::OsStrExt, fs::MetadataExt},
    },
    path::{Component, Path, PathBuf},
    sync::{Arc, Mutex},
};

#[derive(Clone, Copy, PartialEq, Eq)]
struct Identity(u64, u64);

fn identity(file: &File) -> std::io::Result<Identity> {
    let metadata = file.metadata()?;
    Ok(Identity(metadata.dev(), metadata.ino()))
}

#[derive(Default)]
struct Records {
    directories: HashMap<PathBuf, Arc<File>>,
    // Keep original inodes alive even if a pathname is removed. A recycled
    // inode number must never become authority to adopt an unrelated file.
    files: HashMap<PathBuf, File>,
}

pub(super) struct PinnedRoot {
    path: PathBuf,
    directory: Arc<File>,
    records: Mutex<Records>,
}

impl PinnedRoot {
    pub(super) fn new(path: PathBuf, directory: File) -> anyhow::Result<Self> {
        let metadata = directory.metadata()?;
        anyhow::ensure!(
            metadata.is_dir()
                && metadata.uid() == unsafe { nix::libc::geteuid() }
                && metadata.mode() & 0o077 == 0,
            "private payload root must be an owner-only directory"
        );
        let root = Self {
            path,
            directory: Arc::new(directory),
            records: Mutex::new(Records::default()),
        };
        root.check_path()?;
        Ok(root)
    }

    pub(super) fn check_path(&self) -> anyhow::Result<()> {
        let metadata = std::fs::symlink_metadata(&self.path)?;
        let owned = self.directory.metadata()?;
        anyhow::ensure!(
            metadata.is_dir()
                && !metadata.file_type().is_symlink()
                && Identity(metadata.dev(), metadata.ino()) == identity(&self.directory)?,
            "private payload directory was replaced or moved"
        );
        anyhow::ensure!(
            owned.uid() == unsafe { nix::libc::geteuid() } && owned.mode() & 0o077 == 0,
            "private payload directory permissions changed"
        );
        Ok(())
    }

    pub(super) fn path(&self) -> &Path {
        &self.path
    }

    pub(super) fn open(&self, relative: &Path) -> anyhow::Result<File> {
        self.check_path()?;
        let components: Vec<_> = relative
            .components()
            .map(|part| match part {
                Component::Normal(name) => Ok(name),
                _ => Err(anyhow::anyhow!("unsafe private payload path")),
            })
            .collect::<anyhow::Result<_>>()?;
        anyhow::ensure!(!components.is_empty(), "empty private payload path");
        let mut records = self.records.lock().unwrap();
        let mut directory = self.directory.clone();
        let mut prefix = PathBuf::new();
        for component in &components[..components.len() - 1] {
            prefix.push(component);
            let name = CString::new(component.as_bytes())?;
            if let Some(owned) = records.directories.get(&prefix) {
                check_child(&directory, &name, owned)?;
                directory = owned.clone();
            } else {
                // Refuse existing subdirectories: this capability never adopts
                // an unknown directory, even when its name looks correct.
                // SAFETY: the directory descriptor and terminated child name are live.
                let result =
                    unsafe { nix::libc::mkdirat(directory.as_raw_fd(), name.as_ptr(), 0o700) };
                if result != 0 {
                    return Err(std::io::Error::last_os_error().into());
                }
                // SAFETY: open relative to the owned parent without following symlinks.
                let fd = unsafe {
                    nix::libc::openat(
                        directory.as_raw_fd(),
                        name.as_ptr(),
                        nix::libc::O_RDONLY
                            | nix::libc::O_DIRECTORY
                            | nix::libc::O_NOFOLLOW
                            | nix::libc::O_CLOEXEC,
                    )
                };
                if fd < 0 {
                    return Err(std::io::Error::last_os_error().into());
                }
                // SAFETY: openat returned a new owned descriptor.
                let child = Arc::new(unsafe { File::from_raw_fd(fd) });
                let metadata = child.metadata()?;
                anyhow::ensure!(
                    metadata.uid() == unsafe { nix::libc::geteuid() }
                        && metadata.mode() & 0o077 == 0,
                    "unsafe private payload directory owner or permissions"
                );
                super::mark_download(&child)?;
                check_child(&directory, &name, &child)?;
                records.directories.insert(prefix.clone(), child.clone());
                directory = child;
            }
        }
        let name = CString::new(components.last().unwrap().as_bytes())?;
        let expected = records.files.get(relative).map(identity).transpose()?;
        let flags = nix::libc::O_RDWR
            | nix::libc::O_NOFOLLOW
            | nix::libc::O_NONBLOCK
            | nix::libc::O_CLOEXEC
            | if expected.is_some() {
                0
            } else {
                nix::libc::O_CREAT | nix::libc::O_EXCL
            };
        // SAFETY: a live parent descriptor and one validated, terminated component.
        let fd = unsafe { nix::libc::openat(directory.as_raw_fd(), name.as_ptr(), flags, 0o600) };
        if fd < 0 {
            return Err(std::io::Error::last_os_error()).context("private payload open refused");
        }
        // SAFETY: openat returned a new owned descriptor.
        let file = unsafe { File::from_raw_fd(fd) };
        let metadata = file.metadata()?;
        anyhow::ensure!(
            metadata.is_file()
                && metadata.nlink() == 1
                && metadata.uid() == unsafe { nix::libc::geteuid() }
                && metadata.mode() & 0o077 == 0,
            "unsafe private payload file"
        );
        let actual = identity(&file)?;
        anyhow::ensure!(
            expected.is_none_or(|expected| expected == actual),
            "private payload file was replaced"
        );
        super::mark_download(&file)?;
        if expected.is_none() {
            records.files.insert(relative.to_owned(), file.try_clone()?);
        }
        Ok(file)
    }
}

fn check_child(parent: &File, name: &CString, owned: &File) -> anyhow::Result<()> {
    let mut named = std::mem::MaybeUninit::<nix::libc::stat>::uninit();
    // SAFETY: the parent/name are live; fstatat initializes named only on success.
    let result = unsafe {
        nix::libc::fstatat(
            parent.as_raw_fd(),
            name.as_ptr(),
            named.as_mut_ptr(),
            nix::libc::AT_SYMLINK_NOFOLLOW,
        )
    };
    if result != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    // SAFETY: fstatat succeeded above.
    let named = unsafe { named.assume_init() };
    let actual = owned.metadata()?;
    anyhow::ensure!(
        named.st_mode & nix::libc::S_IFMT == nix::libc::S_IFDIR
            && named.st_dev as u64 == actual.dev()
            && named.st_ino as u64 == actual.ino(),
        "private payload subdirectory was replaced or moved"
    );
    anyhow::ensure!(
        actual.uid() == unsafe { nix::libc::geteuid() } && actual.mode() & 0o077 == 0,
        "private payload subdirectory permissions changed"
    );
    Ok(())
}

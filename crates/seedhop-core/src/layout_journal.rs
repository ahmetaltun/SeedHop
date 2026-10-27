use crate::{
    CoreError, Result, StorageRecovery, storage,
    storage_receipt::{self as receipt, CopyEvent, FileIdentity, FileStamp},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs::File,
    path::{Path, PathBuf},
};

const NAME: &str = "layout-transaction.json";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct Layout {
    pub root: PathBuf,
    pub names: HashMap<usize, PathBuf>,
}

#[derive(Clone, Serialize, Deserialize)]
struct Entry {
    source: String,
    target: String,
    source_stamp: Option<FileStamp>,
    target_identity: Option<FileIdentity>,
    digest: Option<[u8; 32]>,
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Journal {
    version: u32,
    pub id: String,
    pub torrent_id: String,
    pub original: Layout,
    pub destination: Layout,
    original_identity: FileIdentity,
    destination_identity: FileIdentity,
    entries: Vec<Entry>,
}

fn quarantine(name: &str, id: &str, index: usize, side: &str) -> String {
    Path::new(name)
        .parent()
        .unwrap_or(Path::new(""))
        .join(format!(".seedhop-{id}-{index}-{side}"))
        .to_string_lossy()
        .into_owned()
}

impl Journal {
    pub fn begin(
        state: &Path,
        id: String,
        torrent_id: String,
        original: Layout,
        destination: Layout,
        paths: Vec<(String, String)>,
    ) -> Result<Self> {
        if state.join(NAME).exists() {
            return Err(CoreError::Storage("A previous file change still needs cleanup. Retry cleanup or keep its extra copies before moving more files.".into()));
        }
        let mut entries = Vec::new();
        let state_root = state.canonicalize().map_err(storage::io_error)?;
        let reserved = [
            state_root.join(NAME),
            state_root.join(NAME).with_extension("pending"),
        ];
        for (source, target) in paths {
            if reserved.contains(&original.root.join(&source))
                || reserved.contains(&destination.root.join(&target))
            {
                return Err(CoreError::InvalidInput(
                    "Choose a folder outside SeedHop's library state.".into(),
                ));
            }
            let source_stamp = receipt::open_relative(&original.root, &source)?
                .map(|file| FileStamp::read(&file))
                .transpose()?;
            entries.push(Entry {
                source,
                target,
                source_stamp,
                target_identity: None,
                digest: None,
            });
        }
        let journal = Self {
            version: 1,
            id,
            torrent_id,
            original_identity: receipt::directory_identity(&original.root)?,
            destination_identity: receipt::directory_identity(&destination.root)?,
            original,
            destination,
            entries,
        };
        journal.save(state)?;
        checkpoint("journal");
        Ok(journal)
    }

    fn save(&self, state: &Path) -> Result<()> {
        storage::atomic_write(
            &state.join(NAME),
            &serde_json::to_vec(self).map_err(|error| CoreError::Storage(error.to_string()))?,
        )
    }

    pub fn copied(&mut self, state: &Path, index: usize, event: CopyEvent) -> Result<()> {
        let entry = &mut self.entries[index];
        match event {
            CopyEvent::Created(stamp) => {
                entry.target_identity = Some(stamp.identity);
                self.save(state)?;
                checkpoint("created");
            }
            CopyEvent::Completed { source, digest } => {
                if entry.source_stamp.as_ref() != Some(&source) {
                    return Err(CoreError::Storage("A source file changed while it was being copied. Original files and copies have been preserved.".into()));
                }
                entry.digest = Some(digest);
                self.save(state)?;
                checkpoint("copied");
            }
        }
        Ok(())
    }

    pub fn authority(&self, state: &Path) -> Result<bool> {
        committed(state, self)
    }

    fn validate(&self) -> Result<()> {
        if self.version != 1 || uuid::Uuid::parse_str(&self.id).is_err() {
            return Err(CoreError::CorruptState(
                "Unsupported file-recovery record. Retain it for inspection.".into(),
            ));
        }
        for (layout, identity) in [
            (&self.original, &self.original_identity),
            (&self.destination, &self.destination_identity),
        ] {
            if receipt::directory_identity(&layout.root)? != *identity {
                return Err(CoreError::Storage(format!(
                    "The folder {} is unavailable or has been replaced. Reconnect the original volume and retry cleanup.",
                    layout.root.display()
                )));
            }
        }
        storage::validate_destination(
            &self.original.root,
            &self
                .entries
                .iter()
                .map(|entry| entry.source.clone())
                .chain(
                    self.entries
                        .iter()
                        .enumerate()
                        .map(|(index, entry)| quarantine(&entry.source, &self.id, index, "source")),
                )
                .collect::<Vec<_>>(),
        )?;
        storage::validate_destination(
            &self.destination.root,
            &self
                .entries
                .iter()
                .map(|entry| entry.target.clone())
                .chain(
                    self.entries
                        .iter()
                        .enumerate()
                        .map(|(index, entry)| quarantine(&entry.target, &self.id, index, "target")),
                )
                .collect::<Vec<_>>(),
        )?;
        Ok(())
    }

    fn cleanup(&self, committed: bool) -> Result<()> {
        self.validate()?;
        let mut errors = Vec::new();
        let mut failures = 0;
        for (index, entry) in self.entries.iter().enumerate() {
            let result = if committed {
                self.clean_original(index, entry)
            } else {
                self.clean_target(index, entry)
            };
            if let Err(error) = result {
                failures += 1;
                if errors.len() < 3 {
                    errors.push(error.to_string());
                }
            }
            checkpoint("cleanup");
        }
        if !errors.is_empty() {
            if failures > errors.len() {
                errors.push(format!(
                    "{} additional cleanup paths also need inspection.",
                    failures - errors.len()
                ));
            }
            return Err(CoreError::Storage(errors.join(" ")));
        }
        Ok(())
    }

    fn clean_target(&self, index: usize, entry: &Entry) -> Result<()> {
        let temporary = quarantine(&entry.target, &self.id, index, "target");
        if receipt::open_relative(&self.destination.root, &entry.target)?.is_none()
            && receipt::open_relative(&self.destination.root, &temporary)?.is_none()
        {
            return Ok(());
        }
        let identity = entry.target_identity.as_ref().ok_or_else(|| {
            CoreError::Storage(format!(
                "The copy at {} has uncertain ownership. It has been preserved.",
                self.destination.root.join(&entry.target).display()
            ))
        })?;
        let source =
            receipt::open_relative(&self.original.root, &entry.source)?.ok_or_else(|| {
                CoreError::Storage(
                    "An original file is missing. Its copy has been preserved.".into(),
                )
            })?;
        if entry.source_stamp.as_ref() != Some(&FileStamp::read(&source)?) {
            return Err(CoreError::Storage(
                "An original file changed. Its copies have been preserved.".into(),
            ));
        }
        receipt::remove_checked(
            &self.destination.root,
            &entry.target,
            &temporary,
            identity,
            |file| {
                Ok(
                    entry.source_stamp.as_ref() == Some(&FileStamp::read(&source)?)
                        && receipt::is_prefix(&source, file)?,
                )
            },
        )
    }

    fn clean_original(&self, index: usize, entry: &Entry) -> Result<()> {
        let temporary = quarantine(&entry.source, &self.id, index, "source");
        if receipt::open_relative(&self.original.root, &entry.source)?.is_none()
            && receipt::open_relative(&self.original.root, &temporary)?.is_none()
        {
            return Ok(());
        }
        let stamp = entry.source_stamp.as_ref().ok_or_else(|| {
            CoreError::Storage(
                "An unrecorded original file appeared. It has been preserved.".into(),
            )
        })?;
        let digest = entry.digest.ok_or_else(|| {
            CoreError::Storage(
                "The saved copy has no completed byte check. Original files have been preserved."
                    .into(),
            )
        })?;
        let target =
            receipt::open_relative(&self.destination.root, &entry.target)?.ok_or_else(|| {
                CoreError::Storage(
                    "The new copy is missing. Original files have been preserved.".into(),
                )
            })?;
        if Some(&FileStamp::read(&target)?.identity) != entry.target_identity.as_ref()
            || receipt::fingerprint(&target)? != digest
        {
            return Err(CoreError::Storage(
                "The new copy changed. Original files have been preserved.".into(),
            ));
        }
        receipt::remove_checked(
            &self.original.root,
            &entry.source,
            &temporary,
            &stamp.identity,
            |file| {
                Ok(FileStamp::read(file)? == *stamp
                    && receipt::fingerprint(file)? == digest
                    && receipt::fingerprint(&target)? == digest)
            },
        )
    }

    pub fn complete(&self, state: &Path, committed: bool) -> Result<()> {
        self.cleanup(committed)?;
        remove_record(state, &self.id)
    }
}

fn load(state: &Path) -> Result<Option<Journal>> {
    let Some(file) = receipt::open_relative(state, NAME)? else {
        return Ok(None);
    };
    serde_json::from_reader(file)
        .map(Some)
        .map_err(|error| CoreError::CorruptState(error.to_string()))
}

fn saved_layout(state: &Path, id: &str) -> Result<Layout> {
    let file =
        receipt::open_relative(&state.join("session"), "session.json")?.ok_or_else(|| {
            CoreError::CorruptState("The saved session is missing. Keep all copies.".into())
        })?;
    let session: serde_json::Value = serde_json::from_reader(file)
        .map_err(|error| CoreError::CorruptState(error.to_string()))?;
    // A save may have renamed its database before reporting a directory flush
    // failure. Establish durability before using that layout to delete copies.
    File::open(state.join("session/session.json"))
        .map_err(storage::io_error)?
        .sync_all()
        .map_err(storage::io_error)?;
    File::open(state.join("session"))
        .map_err(storage::io_error)?
        .sync_all()
        .map_err(storage::io_error)?;
    let torrent = session["torrents"]
        .as_object()
        .and_then(|torrents| {
            torrents.values().find(|torrent| {
                torrent["info_hash_v2"]
                    .as_str()
                    .or(torrent["info_hash"].as_str())
                    == Some(id)
            })
        })
        .ok_or_else(|| {
            CoreError::CorruptState(
                "The transfer is absent from the saved session. Keep all copies.".into(),
            )
        })?;
    Ok(Layout {
        root: PathBuf::from(
            torrent["output_folder"]
                .as_str()
                .ok_or_else(|| CoreError::CorruptState("Missing saved destination".into()))?,
        ),
        names: serde_json::from_value(
            torrent
                .get("file_names")
                .cloned()
                .unwrap_or_else(|| serde_json::json!({})),
        )
        .map_err(|error| CoreError::CorruptState(error.to_string()))?,
    })
}

fn committed(state: &Path, journal: &Journal) -> Result<bool> {
    let saved = saved_layout(state, &journal.torrent_id)?;
    if saved == journal.destination {
        return Ok(true);
    }
    if saved == journal.original {
        return Ok(false);
    }
    Err(CoreError::CorruptState(
        "The saved transfer uses a different layout. Keep all copies for inspection.".into(),
    ))
}

fn remove_record(state: &Path, id: &str) -> Result<()> {
    if load(state)?.is_none_or(|journal| journal.id != id) {
        return Err(CoreError::Storage(
            "The recovery record changed. Retry using its current state.".into(),
        ));
    }
    std::fs::remove_file(state.join(NAME)).map_err(storage::io_error)?;
    File::open(state)
        .map_err(storage::io_error)?
        .sync_all()
        .map_err(storage::io_error)
}

pub(crate) fn recover(state: &Path) -> Option<StorageRecovery> {
    let journal = match load(state) {
        Ok(None) => return None,
        Ok(Some(journal)) => journal,
        Err(error) => {
            return Some(StorageRecovery {
                id: "unreadable-recovery-record".into(),
                torrent_id: String::new(),
                active_location: "unknown".into(),
                message: format!(
                    "The file-recovery record could not be read. All copies were retained. {error}"
                ),
                original_directory: state.to_string_lossy().into(),
                destination_directory: state.to_string_lossy().into(),
                can_keep_copies: false,
            });
        }
    };
    let authority = committed(state, &journal);
    let active_location = match &authority {
        Ok(true) => "new",
        Ok(false) => "original",
        Err(_) => "unknown",
    };
    let can_keep_copies =
        authority.is_ok() && journal.version == 1 && uuid::Uuid::parse_str(&journal.id).is_ok();
    let result = authority.and_then(|committed| journal.complete(state, committed));
    if result.is_ok() {
        return None;
    }
    Some(StorageRecovery {
        id: journal.id,
        torrent_id: journal.torrent_id,
        active_location: active_location.into(),
        message: format!(
            "{} Extra copies were retained. Retry cleanup after restoring access, or inspect the folders and keep the extra copies. {}",
            match active_location {
                "new" => "The new location is saved.",
                "original" => "The original location is still active.",
                _ => "The active location could not be established.",
            },
            result.unwrap_err()
        ),
        original_directory: journal.original.root.to_string_lossy().into(),
        destination_directory: journal.destination.root.to_string_lossy().into(),
        can_keep_copies,
    })
}

pub(crate) fn keep_copies(state: &Path, id: &str) -> Result<()> {
    let journal = load(state)?
        .ok_or_else(|| CoreError::InvalidInput("There is no pending cleanup.".into()))?;
    if journal.id != id || journal.version != 1 {
        return Err(CoreError::InvalidInput(
            "The recovery record changed. Refresh and try again.".into(),
        ));
    }
    committed(state, &journal)?;
    remove_record(state, id)
}

pub(crate) struct RecoveryResult {
    pub notice: Option<StorageRecovery>,
    pub active_layout: Option<(String, Layout)>,
}

pub(crate) fn retry(state: &Path, id: &str, keep: bool) -> Result<RecoveryResult> {
    let active = load(state).ok().flatten().and_then(|journal| {
        let committed = committed(state, &journal).ok()?;
        let (layout, identity) = if committed {
            (&journal.destination, &journal.destination_identity)
        } else {
            (&journal.original, &journal.original_identity)
        };
        if receipt::directory_identity(&layout.root).ok().as_ref() != Some(identity) {
            return None;
        }
        Some((journal.torrent_id.clone(), layout.clone()))
    });
    if let Ok(Some(journal)) = load(state)
        && journal.id != id
        && id != "unreadable-recovery-record"
    {
        return Err(CoreError::InvalidInput(
            "The recovery record changed. Refresh and try again.".into(),
        ));
    }
    if keep {
        keep_copies(state, id)?;
        return Ok(RecoveryResult {
            notice: None,
            active_layout: active,
        });
    }
    Ok(RecoveryResult {
        notice: recover(state),
        active_layout: active,
    })
}

/// Refuse writes on a replacement active folder. An unavailable extra-copy
/// folder can wait without preventing the saved active layout from restoring.
pub(crate) fn ensure_active_root(state: &Path) -> Result<()> {
    let Some(journal) = load(state).ok().flatten() else {
        return Ok(());
    };
    if journal.version != 1 {
        return Ok(());
    }
    let Ok(committed) = committed(state, &journal) else {
        return Ok(());
    };
    let (layout, identity) = if committed {
        (&journal.destination, &journal.destination_identity)
    } else {
        (&journal.original, &journal.original_identity)
    };
    if receipt::directory_identity(&layout.root).ok().as_ref() != Some(identity) {
        return Err(CoreError::Storage(format!(
            "The active folder {} is unavailable or has been replaced. Reconnect the original volume before restarting transfers. All copies have been retained.",
            layout.root.display()
        )));
    }
    Ok(())
}

pub(crate) fn checkpoint(stage: &str) {
    #[cfg(feature = "layout-fault-injection")]
    if std::env::var("SEEDHOP_LAYOUT_CHECKPOINT").as_deref() == Ok(stage) {
        use std::io::Write;
        println!("LAYOUT CHECKPOINT {stage}");
        let _ = std::io::stdout().flush();
        loop {
            std::thread::park();
        }
    }
    #[cfg(not(feature = "layout-fault-injection"))]
    let _ = stage;
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        state: tempfile::TempDir,
        source: tempfile::TempDir,
        target: tempfile::TempDir,
        journal: Journal,
    }
    impl Fixture {
        fn new() -> Self {
            let state = tempfile::tempdir().unwrap();
            let source = tempfile::tempdir().unwrap();
            let target = tempfile::tempdir().unwrap();
            std::fs::create_dir(state.path().join("session")).unwrap();
            std::fs::write(source.path().join("file"), vec![7; 2 * 1024 * 1024 + 17]).unwrap();
            let journal = Journal::begin(
                state.path(),
                uuid::Uuid::new_v4().to_string(),
                "a".repeat(40),
                Layout {
                    root: source.path().to_owned(),
                    names: HashMap::new(),
                },
                Layout {
                    root: target.path().to_owned(),
                    names: HashMap::new(),
                },
                vec![("file".into(), "file".into())],
            )
            .unwrap();
            let fixture = Self {
                state,
                source,
                target,
                journal,
            };
            fixture.authority(false);
            fixture
        }
        fn authority(&self, committed: bool) {
            let layout = if committed {
                &self.journal.destination
            } else {
                &self.journal.original
            };
            storage::atomic_write(&self.state.path().join("session/session.json"), &serde_json::to_vec(&serde_json::json!({"torrents":{"0":{"info_hash":self.journal.torrent_id,"output_folder":layout.root,"file_names":layout.names}}})).unwrap()).unwrap();
        }
        fn copy(&mut self, cancel: bool) {
            let result = storage::copy_exclusive(
                self.source.path(),
                "file",
                self.target.path(),
                "file",
                &mut |bytes| {
                    if cancel && bytes > 0 {
                        Err(CoreError::Cancelled)
                    } else {
                        Ok(())
                    }
                },
                &mut |event| self.journal.copied(self.state.path(), 0, event),
            );
            if cancel {
                assert!(matches!(result, Err(CoreError::Cancelled)));
            } else {
                assert!(result.unwrap());
            }
        }
    }

    #[test]
    fn partial_and_completed_copies_roll_back_before_commit_idempotently() {
        for partial in [false, true] {
            let mut fixture = Fixture::new();
            fixture.copy(partial);
            let bytes = std::fs::read(fixture.source.path().join("file")).unwrap();
            assert!(recover(fixture.state.path()).is_none());
            assert!(recover(fixture.state.path()).is_none());
            assert_eq!(
                std::fs::read(fixture.source.path().join("file")).unwrap(),
                bytes
            );
            assert!(!fixture.target.path().join("file").exists());
            assert!(!fixture.state.path().join(NAME).exists());
        }
    }

    #[test]
    fn committed_cleanup_recovers_a_quarantined_source_and_preserves_target() {
        let mut fixture = Fixture::new();
        fixture.copy(false);
        fixture.authority(true);
        let temporary = quarantine("file", &fixture.journal.id, 0, "source");
        std::fs::rename(
            fixture.source.path().join("file"),
            fixture.source.path().join(&temporary),
        )
        .unwrap();
        let bytes = std::fs::read(fixture.target.path().join("file")).unwrap();
        assert!(recover(fixture.state.path()).is_none());
        assert!(!fixture.source.path().join(&temporary).exists());
        assert_eq!(
            std::fs::read(fixture.target.path().join("file")).unwrap(),
            bytes
        );
    }

    #[test]
    fn replaced_and_changed_files_are_retained_and_keep_copies_is_scoped() {
        for committed in [false, true] {
            let mut fixture = Fixture::new();
            fixture.copy(false);
            fixture.authority(committed);
            let root = if committed {
                fixture.source.path()
            } else {
                fixture.target.path()
            };
            std::fs::rename(root.join("file"), root.join("external-backup")).unwrap();
            std::fs::write(root.join("file"), b"external replacement").unwrap();
            let notice = recover(fixture.state.path()).unwrap();
            assert_eq!(
                notice.active_location,
                if committed { "new" } else { "original" }
            );
            assert!(notice.can_keep_copies);
            assert_eq!(
                std::fs::read(root.join("file")).unwrap(),
                b"external replacement"
            );
            assert!(retry(fixture.state.path(), "stale", true).is_err());
            keep_copies(fixture.state.path(), &notice.id).unwrap();
            assert!(recover(fixture.state.path()).is_none());
            assert!(root.join("external-backup").exists());
            assert_eq!(
                std::fs::read(root.join("file")).unwrap(),
                b"external replacement"
            );
        }
    }

    #[test]
    fn changed_copy_can_be_inspected_and_cleanup_retried_without_replacement() {
        let mut fixture = Fixture::new();
        fixture.copy(false);
        std::fs::write(fixture.target.path().join("file"), b"edited").unwrap();
        let notice = recover(fixture.state.path()).unwrap();
        assert_eq!(
            std::fs::read(fixture.target.path().join("file")).unwrap(),
            b"edited"
        );
        assert!(notice.message.contains("original location"));
        std::fs::write(
            fixture.target.path().join("file"),
            std::fs::read(fixture.source.path().join("file")).unwrap(),
        )
        .unwrap();
        assert!(
            retry(fixture.state.path(), &notice.id, false)
                .unwrap()
                .notice
                .is_none()
        );
        assert!(!fixture.target.path().join("file").exists());
    }

    #[test]
    fn unavailable_replaced_roots_symlinks_and_unknown_versions_never_delete_bytes() {
        let mut fixture = Fixture::new();
        fixture.copy(false);
        std::fs::remove_file(fixture.target.path().join("file")).unwrap();
        std::os::unix::fs::symlink(
            fixture.source.path().join("file"),
            fixture.target.path().join("file"),
        )
        .unwrap();
        assert!(recover(fixture.state.path()).is_some());
        assert!(fixture.source.path().join("file").exists());
        std::fs::remove_file(fixture.target.path().join("file")).unwrap();
        fixture.journal.version = 999;
        fixture.journal.save(fixture.state.path()).unwrap();
        let notice = recover(fixture.state.path()).unwrap();
        assert!(!notice.can_keep_copies);
        assert!(keep_copies(fixture.state.path(), &notice.id).is_err());
        fixture.journal.version = 1;
        fixture.journal.save(fixture.state.path()).unwrap();
        let moved = fixture.target.path().with_extension("removed");
        std::fs::rename(fixture.target.path(), &moved).unwrap();
        assert!(recover(fixture.state.path()).is_some());
        std::fs::create_dir(fixture.target.path()).unwrap();
        assert!(
            recover(fixture.state.path())
                .unwrap()
                .message
                .contains("replaced")
        );
        std::fs::remove_dir(fixture.target.path()).unwrap();
        std::fs::rename(moved, fixture.target.path()).unwrap();
        assert!(recover(fixture.state.path()).is_none());
    }

    #[test]
    fn malformed_and_unknown_authority_records_preserve_all_files() {
        let mut fixture = Fixture::new();
        fixture.copy(false);
        storage::atomic_write(
            &fixture.state.path().join("session/session.json"),
            b"{\"torrents\":{}}",
        )
        .unwrap();
        let notice = recover(fixture.state.path()).unwrap();
        assert_eq!(notice.active_location, "unknown");
        assert!(!notice.can_keep_copies);
        assert!(
            fixture.source.path().join("file").exists()
                && fixture.target.path().join("file").exists()
        );
        storage::atomic_write(&fixture.state.path().join(NAME), b"broken").unwrap();
        assert!(!recover(fixture.state.path()).unwrap().can_keep_copies);
    }

    #[test]
    fn layout_targets_cannot_overwrite_the_journal_or_its_pending_save() {
        for target in [NAME, "layout-transaction.pending"] {
            let state = tempfile::tempdir().unwrap();
            let source = tempfile::tempdir().unwrap();
            std::fs::write(source.path().join("file"), b"keep").unwrap();
            let result = Journal::begin(
                state.path(),
                uuid::Uuid::new_v4().to_string(),
                "a".repeat(40),
                Layout {
                    root: source.path().canonicalize().unwrap(),
                    names: HashMap::new(),
                },
                Layout {
                    root: state.path().canonicalize().unwrap(),
                    names: HashMap::new(),
                },
                vec![("file".into(), target.into())],
            );
            assert!(result.is_err());
            assert!(!state.path().join(NAME).exists());
            assert_eq!(std::fs::read(source.path().join("file")).unwrap(), b"keep");
        }
    }
}

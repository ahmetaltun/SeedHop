use crate::{CoreError, Engine, Result, adapter, storage};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Default, Serialize, Deserialize)]
pub(crate) struct QueueConfig {
    pub max_active: u32,
    pub order: Vec<String>,
}
impl QueueConfig {
    pub fn load(root: &Path) -> Result<Self> {
        let path = root.join("queue.json");
        if !path.exists() {
            return Ok(Self::default());
        }
        serde_json::from_slice(&std::fs::read(path).map_err(storage::io_error)?)
            .map_err(|e| CoreError::CorruptState(e.to_string()))
    }
}

impl Engine {
    async fn refresh_storage_recovery(&self) -> Result<()> {
        let state = self.state_directory.clone();
        let recovery = tokio::task::spawn_blocking(move || crate::layout_journal::recover(&state))
            .await
            .map_err(adapter::engine_error)?;
        *self.storage_recovery.lock().unwrap() = recovery;
        Ok(())
    }

    pub async fn resolve_storage_recovery(&self, id: String, keep_copies: bool) -> Result<()> {
        let _operation = self.operations.write().await;
        self.ready()?;
        let notice = self
            .storage_recovery
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| CoreError::Validation {
                code: "input.there_is_no_pending_file_cleanup",
                message: "There is no pending file cleanup.".into(),
            })?;
        if notice.id != id || (keep_copies && !notice.can_keep_copies) {
            return Err(CoreError::Validation {
                code: "input.refresh_the_recovery_notice_and_try_again",
                message: "Refresh the recovery notice and try again.".into(),
            });
        }
        let handle = self.handle(&notice.torrent_id).ok();
        if let Some(handle) = &handle {
            handle.quiesce().map_err(adapter::engine_error)?;
        }
        let state = self.state_directory.clone();
        let result = tokio::task::spawn_blocking(move || {
            crate::layout_journal::retry(&state, &id, keep_copies)
        })
        .await
        .map_err(adapter::engine_error)?;
        match result {
            Ok(result) => {
                *self.storage_recovery.lock().unwrap() = result.notice;
                if let Some((torrent_id, layout)) = result.active_layout {
                    let handle = self.handle(&torrent_id)?;
                    handle
                        .replace_storage_layout(layout.root.clone(), layout.names)
                        .map_err(adapter::engine_error)?;
                    storage::validate_destination(
                        &layout.root,
                        &adapter::record(&handle)
                            .files
                            .into_iter()
                            .map(|file| file.name)
                            .collect::<Vec<_>>(),
                    )?;
                    self.session
                        .force_recheck(&handle)
                        .await
                        .map_err(adapter::engine_error)?;
                } else if let Some(handle) = handle {
                    // Keep uncertain layouts quiesced and visibly in error.
                    handle
                        .replace_storage_layout(handle.output_folder(), handle.file_names())
                        .map_err(adapter::engine_error)?;
                }
                Ok(())
            }
            Err(error) => {
                if let Some(handle) = handle {
                    self.session
                        .set_queued(&handle, handle.is_queued())
                        .await
                        .map_err(adapter::engine_error)?;
                }
                Err(error)
            }
        }
    }

    /// Cancellation must not wait for the mutation lock held by the copy.
    /// IDs originate in the engine, so an old request cannot cancel a retry.
    pub fn cancel_file_operation(&self, operation_id: String) -> Result<bool> {
        self.ready()?;
        let operation = self.file_operation.lock().unwrap().clone();
        Ok(operation.is_some_and(|operation| operation.cancel(&operation_id)))
    }

    pub async fn set_file_priorities(&self, id: String, priorities: Vec<u8>) -> Result<()> {
        let _operation = self.operations.write().await;
        self.ready()?;
        let handle = self.handle(&id)?;
        let count = adapter::record(&handle).files.len();
        if priorities.len() != count || priorities.iter().any(|p| *p > 2) {
            return Err(CoreError::Validation {
                code: "input.choose_low_normal_or_high_for_every_file",
                message: "Choose Low, Normal, or High for every file.".into(),
            });
        }
        self.session
            .update_file_priorities(&handle, priorities)
            .await
            .map_err(adapter::engine_error)
    }

    pub async fn set_queue(&self, max_active: u32) -> Result<()> {
        let _operation = self.operations.write().await;
        self.ready()?;
        let mut config = self.queue.lock().unwrap().clone();
        config.max_active = max_active;
        self.save_queue(config)?;
        self.reconcile_queue().await
    }

    pub async fn move_in_queue(&self, id: String, position: u32) -> Result<()> {
        let _operation = self.operations.write().await;
        self.ready()?;
        self.handle(&id)?;
        let mut config = self.queue.lock().unwrap().clone();
        for torrent in self.snapshot()?.torrents {
            if !config.order.contains(&torrent.id) {
                config.order.push(torrent.id);
            }
        }
        config.order.retain(|item| item != &id);
        config.order.insert(
            (position.saturating_sub(1) as usize).min(config.order.len()),
            id,
        );
        self.save_queue(config)?;
        self.reconcile_queue().await
    }

    pub(crate) fn prepare_top_of_queue(&self, id: &str) -> Result<()> {
        let mut config = self.queue.lock().unwrap().clone();
        let mut handles = self
            .session
            .with_torrents(|items| items.map(|(_, t)| t.clone()).collect::<Vec<_>>());
        handles.sort_by_key(|t| t.id());
        config
            .order
            .retain(|item| item != id && handles.iter().any(|t| adapter::torrent_id(t) == *item));
        for handle in &handles {
            let item = adapter::torrent_id(handle);
            if !config.order.contains(&item) {
                config.order.push(item);
            }
        }
        // Keep current downloads ahead of this admission, so the normal queue
        // reconciler cannot preempt them. Existing queue-move behavior is unchanged.
        let position = config
            .order
            .iter()
            .rposition(|item| {
                handles.iter().any(|t| {
                    adapter::torrent_id(t) == *item
                        && !t.is_paused()
                        && !t.is_queued()
                        && !t.stats().finished
                })
            })
            .map_or(0, |p| p + 1);
        config.order.insert(position, id.to_owned());
        self.save_queue(config)
    }

    fn save_queue(&self, config: QueueConfig) -> Result<()> {
        storage::atomic_write(
            &self.state_directory.join("queue.json"),
            &serde_json::to_vec(&config).map_err(adapter::engine_error)?,
        )?;
        *self.queue.lock().unwrap() = config;
        Ok(())
    }

    pub(crate) async fn reconcile_queue(&self) -> Result<()> {
        let mut handles = self
            .session
            .with_torrents(|items| items.map(|(_, t)| t.clone()).collect::<Vec<_>>());
        // Engine IDs are monotonic and define stable FIFO for newly added items.
        handles.sort_by_key(|t| t.id());
        let mut config = self.queue.lock().unwrap().clone();
        let old = config.order.clone();
        config
            .order
            .retain(|id| handles.iter().any(|t| adapter::torrent_id(t) == *id));
        for handle in &handles {
            let id = adapter::torrent_id(handle);
            if !config.order.contains(&id) {
                config.order.push(id);
            }
        }
        if old != config.order {
            self.save_queue(config.clone())?;
        }
        handles.sort_by_key(|t| {
            config
                .order
                .iter()
                .position(|id| *id == adapter::torrent_id(t))
                .unwrap_or(usize::MAX)
        });
        let suspended = self.schedule_suspended();
        let mut slots = 0;
        let mut changes = Vec::new();
        for handle in handles {
            let stats = handle.stats();
            if matches!(
                stats.state,
                librqbit::TorrentStatsState::Initializing { .. }
            ) {
                let queued = suspended || config.max_active > 0;
                if queued != handle.is_queued() {
                    changes.push((handle, queued));
                }
                continue;
            }
            if matches!(stats.state, librqbit::TorrentStatsState::Error) {
                continue;
            }
            let queued = if suspended {
                true
            } else if handle.is_paused() || stats.finished || config.max_active == 0 {
                false
            } else {
                slots += 1;
                slots > config.max_active
            };
            if queued != handle.is_queued() {
                changes.push((handle, queued));
            }
        }
        // Release slots before starting replacements, including when reordered.
        changes.sort_by_key(|(_, queued)| !queued);
        for (handle, queued) in changes {
            self.session
                .set_queued(&handle, queued)
                .await
                .map_err(adapter::engine_error)?;
        }
        Ok(())
    }

    pub async fn relocate(&self, id: String, destination: String) -> Result<()> {
        let _operation = self.operations.write().await;
        self.ready()?;
        let handle = self.handle(&id)?;
        let target = PathBuf::from(destination)
            .canonicalize()
            .map_err(storage::io_error)?;
        self.change_layout(handle, target, None).await
    }

    pub async fn rename_file(&self, id: String, index: u32, name: String) -> Result<()> {
        let _operation = self.operations.write().await;
        self.ready()?;
        let handle = self.handle(&id)?;
        self.change_layout(handle.clone(), handle.output_folder(), Some((index, name)))
            .await
    }

    async fn change_layout(
        &self,
        handle: std::sync::Arc<librqbit::ManagedTorrent>,
        root: PathBuf,
        rename: Option<(u32, String)>,
    ) -> Result<()> {
        let operation = crate::file_operation::Guard::begin(
            &self.file_operation,
            adapter::torrent_id(&handle),
            if rename.is_some() {
                "rename"
            } else {
                "relocate"
            },
        );
        self.perform_layout(handle, root, rename, operation.control.clone())
            .await
    }

    async fn perform_layout(
        &self,
        handle: std::sync::Arc<librqbit::ManagedTorrent>,
        root: PathBuf,
        rename: Option<(u32, String)>,
        operation: std::sync::Arc<crate::file_operation::Control>,
    ) -> Result<()> {
        if self.storage_recovery.lock().unwrap().is_some() {
            return Err(CoreError::Storage("A previous file change needs attention. Retry cleanup or keep its extra copies before moving more files.".into()));
        }
        let record = adapter::record(&handle);
        let old_root = handle.output_folder();
        let old_names = handle.file_names();
        let mut names = old_names.clone();
        if let Some((index, name)) = &rename {
            if *index as usize >= record.files.len() {
                return Err(CoreError::Validation {
                    code: "input.choose_an_existing_file",
                    message: "Choose an existing file.".into(),
                });
            }
            names.insert(*index as usize, PathBuf::from(name));
        }
        let targets: Vec<String> = record
            .files
            .iter()
            .map(|f| {
                names
                    .get(&(f.index as usize))
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_else(|| f.name.clone())
            })
            .collect();
        storage::validate_destination(
            &old_root,
            &record
                .files
                .iter()
                .map(|f| f.name.clone())
                .collect::<Vec<_>>(),
        )?;
        storage::validate_destination(&root, &targets)?;
        for (i, path) in targets.iter().enumerate() {
            if targets.iter().enumerate().any(|(j, other)| {
                i != j
                    && (Path::new(path) == Path::new(other)
                        || Path::new(path).starts_with(Path::new(other)))
            }) {
                return Err(CoreError::Validation {
                    code: "input.two_files_cannot_share_a_path_or_contain",
                    message: "Two files cannot share a path or contain one another.".into(),
                });
            }
        }
        let changed: Vec<_> = record
            .files
            .iter()
            .zip(&targets)
            .filter(|(f, target)| old_root.join(&f.name) != root.join(target))
            .collect();
        if changed.is_empty() {
            return Ok(());
        }
        for (_, target) in &changed {
            if root.join(target).exists() {
                return Err(CoreError::Validation {
                    code: "input.a_file_already_exists_at_the_new_location",
                    message:
                        "A file already exists at the new location. Choose another name or folder."
                            .into(),
                });
            }
        }
        // Reject insufficient space before changing transfer state or creating
        // targets. Later allocation failures still use the copy rollback path.
        storage::preflight_copy(
            &old_root,
            &changed
                .iter()
                .map(|(file, _)| (file.name.as_str(), file.size))
                .collect::<Vec<_>>(),
            &root,
        )?;
        operation.advance(0)?;
        handle.quiesce().map_err(adapter::engine_error)?;
        // Materialize any legacy filename mapping before recording the old
        // layout. A pre-commit crash must match the saved layout exactly.
        if let Err(error) = self.session.try_update_persistence_metadata(&handle).await {
            let _ = self.session.set_queued(&handle, handle.is_queued()).await;
            return Err(adapter::engine_error(error));
        }
        let copied = tokio::task::spawn_blocking({
            let old_root = old_root.clone();
            let root = root.clone();
            let files: Vec<_> = changed
                .iter()
                .map(|(f, t)| (f.name.clone(), (*t).clone(), f.size))
                .collect();
            let operation = operation.clone();
            let state = self.state_directory.clone();
            let original = crate::layout_journal::Layout { root: old_root.clone(), names: old_names.clone() };
            let destination = crate::layout_journal::Layout { root: root.clone(), names: names.clone() };
            let torrent_id = record.id.clone();
            move || {
                // Re-estimate after quiescing so progress measures the actual
                // bytes copied, independently of the partial-file reservation.
                operation.advance(0)?;
                let total = storage::preflight_copy(
                    &old_root,
                    &files
                        .iter()
                        .map(|(name, _, size)| (name.as_str(), *size))
                        .collect::<Vec<_>>(),
                    &root,
                )?;
                operation.copying(total)?;
                let mut journal = crate::layout_journal::Journal::begin(&state, operation.snapshot().id, torrent_id, original, destination, files.iter().map(|(source, target, _)| (source.clone(), target.clone())).collect())?;
                for (index, (source, target, _)) in files.into_iter().enumerate() {
                    match storage::copy_exclusive(
                        &old_root,
                        &source,
                        &root,
                        &target,
                        &mut |bytes| { if bytes > 0 { crate::layout_journal::checkpoint("copy"); } operation.advance(bytes) },
                        &mut |event| journal.copied(&state, index, event),
                    ) {
                        Ok(_) => {}
                        Err(error) => {
                            journal.complete(&state, false).map_err(|cleanup| CoreError::Storage(format!("{error} Original files are still active. Some copies were retained: {cleanup}")))?;
                            return Err(error);
                        }
                    }
                }
                if let Err(error) = operation.saving() {
                    journal.complete(&state, false).map_err(|cleanup| CoreError::Storage(format!("{error} Original files are still active. Some copies were retained: {cleanup}")))?;
                    return Err(error);
                }
                crate::layout_journal::checkpoint("saving");
                Ok(journal)
            }
        })
        .await
        .map_err(adapter::engine_error)
        .and_then(|result| result);
        let copied = match copied {
            Ok(copied) => copied,
            Err(error) => {
                operation.phase("restoring");
                self.refresh_storage_recovery().await?;
                if let Err(restore) = self.session.set_queued(&handle, handle.is_queued()).await {
                    return Err(CoreError::Storage(format!(
                        "{error} Your original files are unchanged, but resuming the transfer failed: {restore}. Retry Resume."
                    )));
                }
                return Err(error);
            }
        };
        handle
            .replace_storage_layout(root.clone(), names)
            .map_err(adapter::engine_error)?;
        if let Err(error) = self.session.try_update_persistence_metadata(&handle).await {
            operation.phase("restoring");
            if matches!(copied.authority(&self.state_directory), Ok(false)) {
                handle
                    .replace_storage_layout(old_root.clone(), old_names)
                    .map_err(adapter::engine_error)?;
                let _ = self.session.try_update_persistence_metadata(&handle).await;
            }
            // A failed flush may already have renamed the database. Recovery
            // uses its saved layout and retains copies if authority is unclear.
            self.refresh_storage_recovery().await?;
            if self
                .storage_recovery
                .lock()
                .unwrap()
                .as_ref()
                .is_none_or(|notice| notice.active_location != "unknown")
            {
                let _ = self.session.force_recheck(&handle).await;
            }
            let error = adapter::engine_error(error);
            return Err(error);
        }
        crate::layout_journal::checkpoint("commit");
        // The durable new layout is the commit point. Interrupted cleanup may
        // leave extra originals, but can never leave the library without bytes.
        operation.phase("cleaning");
        let state = self.state_directory.clone();
        let cleanup = tokio::task::spawn_blocking(move || copied.complete(&state, true))
            .await
            .map_err(adapter::engine_error)?;
        if cleanup.is_err() {
            self.refresh_storage_recovery().await?;
        }
        operation.phase("verifying");
        self.session
            .force_recheck(&handle)
            .await
            .map_err(adapter::engine_error)?;
        cleanup.map_err(|error| {
            CoreError::Storage(format!(
                "The new location is saved and active. Some original copies were retained: {error}"
            ))
        })
    }
}

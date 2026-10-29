pub mod adapter;
mod connection_settings;
mod diagnostics;
pub use diagnostics::{PeerDetails, TorrentDetails, TrackerDetails};
mod file_operation;
mod layout_journal;
mod management;
mod model;
mod scheduling;
pub use scheduling::{ScheduleConfig, ScheduleRule};
pub mod storage;
mod storage_receipt;
pub mod v2;
use librqbit::{AddTorrent, AddTorrentOptions, Session, SessionOptions, SessionPersistenceConfig};
pub use model::*;
use std::{
    collections::HashMap,
    num::NonZeroU32,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::sync::RwLock;

pub const PROTOCOL_VERSION: u32 = 5;

/// Owns library state. The FFI host retains this lease until its runtime has stopped.
pub struct Engine {
    accounting: Mutex<diagnostics::Accounting>,
    statistics_warning: Mutex<Option<String>>,
    session: Arc<Session>,
    active_peer_encryption: librqbit::EncryptionPolicy,
    connection_settings: Mutex<connection_settings::ConnectionSettings>,
    lease: storage::LibraryLease,
    state_directory: PathBuf,
    instance_id: String,
    operations: RwLock<()>,
    file_operation: Mutex<Option<Arc<file_operation::Control>>>,
    storage_recovery: Mutex<Option<StorageRecovery>>,
    resolving: Mutex<HashMap<String, tokio::sync::watch::Sender<bool>>>,
    stopping: AtomicBool,
    queue: Mutex<management::QueueConfig>,
    scheduling: Mutex<scheduling::Scheduling>,
    scheduling_warning: Mutex<Option<String>>,
}

impl Engine {
    pub async fn open(config: EngineConfig) -> Result<Arc<Self>> {
        Self::open_with_policy_override(config, None).await
    }

    pub async fn open_with_peer_encryption(
        config: EngineConfig,
        peer_encryption: librqbit::EncryptionPolicy,
    ) -> Result<Arc<Self>> {
        Self::open_with_policy_override(config, Some(peer_encryption)).await
    }

    async fn open_with_policy_override(
        config: EngineConfig,
        override_policy: Option<librqbit::EncryptionPolicy>,
    ) -> Result<Arc<Self>> {
        let dir = PathBuf::from(&config.state_directory);
        let lease = storage::LibraryLease::acquire(&dir)?;
        let connection_settings = connection_settings::ConnectionSettings::load(&dir)?;
        let peer_encryption = override_policy.unwrap_or(connection_settings.peer_encryption);
        let downloads = PathBuf::from(&config.download_directory);
        storage::validate_destination(&downloads, &[])?;
        let session_dir = dir.join("session");
        std::fs::create_dir_all(&session_dir).map_err(storage::io_error)?;
        // Preserve unreadable state in place. Never overwrite it with an empty session.
        let session_path = session_dir.join("session.json");
        // Resolve interrupted layouts while the lease is exclusive and before
        // Session restoration can open files or resume any peer writes.
        let storage_recovery = tokio::task::spawn_blocking({
            let dir = dir.clone();
            move || layout_journal::recover(&dir)
        })
        .await
        .map_err(adapter::engine_error)?;
        layout_journal::ensure_active_root(&dir)?;
        if session_path.exists() {
            let data = std::fs::read(&session_path).map_err(storage::io_error)?;
            let value: serde_json::Value = serde_json::from_slice(&data)
                .map_err(|e| CoreError::CorruptState(e.to_string()))?;
            if !value.get("torrents").is_some_and(|t| t.is_object()) {
                return Err(CoreError::CorruptState("Invalid session database".into()));
            }
            // Revalidate saved paths before a session has a chance to resume disk I/O.
            if let Some(torrents) = value["torrents"].as_object() {
                for saved in torrents.values() {
                    let folder = saved["output_folder"]
                        .as_str()
                        .ok_or_else(|| CoreError::CorruptState("Missing destination".into()))?;
                    if !Path::new(folder).is_dir() {
                        return Err(CoreError::Storage(format!(
                            "The saved destination is unavailable: {folder}. Reconnect the drive and retry."
                        )));
                    }
                    let hash = saved["info_hash"].as_str().unwrap_or_default();
                    let metadata = session_dir.join(format!("{hash}.torrent"));
                    if let Ok(bytes) = std::fs::read(metadata) {
                        let preview = adapter::preview(bytes)?;
                        if let Some(expected) = saved["info_hash_v2"].as_str()
                            && expected != preview.id
                        {
                            return Err(CoreError::CorruptState(
                                "Saved v2 identity does not match torrent metadata.".into(),
                            ));
                        }
                        let mut paths: Vec<_> =
                            preview.files.iter().map(|f| f.name.clone()).collect();
                        if saved["info_hash_v2"].is_string() {
                            let version = match saved.get("file_name_mapping_version") {
                                None => 0,
                                Some(value) => value.as_u64().ok_or_else(|| {
                                    CoreError::CorruptState(
                                        "Invalid filename mapping version".into(),
                                    )
                                })?,
                            };
                            match version {
                                0 => {
                                    paths =
                                        v2::Metainfo::legacy_storage_paths(&preview.torrent_bytes)
                                            .map_err(|e| CoreError::CorruptState(e.to_string()))?
                                            .ok_or_else(|| {
                                                CoreError::CorruptState(
                                                    "Saved v2 layout is missing".into(),
                                                )
                                            })?
                                            .into_iter()
                                            .map(|parts| parts.join("/"))
                                            .collect()
                                }
                                1 => {
                                    let names =
                                        saved["file_names"].as_object().ok_or_else(|| {
                                            CoreError::CorruptState(
                                                "Saved filename map is missing".into(),
                                            )
                                        })?;
                                    if names.len() != paths.len()
                                        || !(0..paths.len())
                                            .all(|i| names.contains_key(&i.to_string()))
                                    {
                                        return Err(CoreError::CorruptState(
                                            "Saved filename map is incomplete".into(),
                                        ));
                                    }
                                }
                                _ => {
                                    return Err(CoreError::CorruptState(
                                        "Saved filename mapping version is unsupported".into(),
                                    ));
                                }
                            }
                        }
                        if let Some(names) = saved["file_names"].as_object() {
                            for (index, name) in names {
                                let index: usize = index.parse().map_err(|_| {
                                    CoreError::CorruptState("Invalid renamed file index".into())
                                })?;
                                let path = paths.get_mut(index).ok_or_else(|| {
                                    CoreError::CorruptState("Invalid renamed file index".into())
                                })?;
                                *path = name
                                    .as_str()
                                    .ok_or_else(|| {
                                        CoreError::CorruptState("Invalid renamed file path".into())
                                    })?
                                    .into();
                            }
                        }
                        storage::validate_destination(Path::new(folder), &paths)?;
                    }
                }
            }
        }
        let limits_path = dir.join("limits.json");
        let queue = management::QueueConfig::load(&dir)?;
        let accounting = diagnostics::Accounting::load(&dir)?;
        let schedule = ScheduleConfig::load(&dir)?;
        let limits: [u32; 2] = if limits_path.exists() {
            serde_json::from_slice(&std::fs::read(limits_path).map_err(storage::io_error)?)
                .map_err(|e| CoreError::CorruptState(e.to_string()))?
        } else {
            [0, 0]
        };
        let options = SessionOptions {
            peer_encryption,
            restore_queued: queue.max_active > 0 || schedule.enabled,
            ratelimits: librqbit::limits::LimitsConfig {
                download_bps: NonZeroU32::new(limits[0]),
                upload_bps: NonZeroU32::new(limits[1]),
            },
            dht: if config.allow_network_discovery {
                Some(librqbit::DhtSessionConfig {
                    persistence: None,
                    ..Default::default()
                })
            } else {
                None
            },
            disable_local_service_discovery: true,
            fastresume: false, // Always recheck disk data in this preview; never trust stale piece maps.
            listen: Some(librqbit::ListenerOptions {
                mode: librqbit::ListenerMode::TcpAndUtp,
                listen_addr: (std::net::Ipv6Addr::UNSPECIFIED, config.listen_port).into(),
                ..Default::default()
            }),
            persistence: Some(SessionPersistenceConfig::Json {
                folder: Some(session_dir),
            }),
            client_name_and_version: Some("SeedHop/0.1.0".into()),
            ..Default::default()
        };
        let session = Session::new_with_opts(downloads, options)
            .await
            .map_err(adapter::engine_error)?;
        let engine = Arc::new(Self {
            accounting: Mutex::new(accounting),
            statistics_warning: Mutex::new(None),
            session,
            active_peer_encryption: peer_encryption,
            connection_settings: Mutex::new(connection_settings),
            lease,
            state_directory: dir,
            instance_id: uuid::Uuid::new_v4().to_string(),
            operations: RwLock::new(()),
            file_operation: Mutex::new(None),
            storage_recovery: Mutex::new(storage_recovery),
            resolving: Mutex::new(HashMap::new()),
            stopping: AtomicBool::new(false),
            queue: Mutex::new(queue),
            scheduling_warning: Mutex::new(None),
            scheduling: Mutex::new(scheduling::Scheduling {
                config: schedule,
                normal_limits: limits,
                active: None,
            }),
        });
        engine.reconcile_schedule();
        let scheduling_enabled = engine.scheduling.lock().unwrap().config.enabled;
        if scheduling_enabled {
            // Restored transfers remain queued on failure. Keep the lease and
            // expose a retryable warning instead of abandoning a live session.
            let _ = engine.apply_scheduling().await;
        }
        let weak = Arc::downgrade(&engine);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_millis(250)).await;
                let Some(engine) = weak.upgrade() else { break };
                if engine.stopping.load(Ordering::SeqCst) {
                    break;
                }
                // Metadata inspection is read-only and can wait on peers. Share
                // its lock so calendar transitions still run; file changes and
                // shutdown retain their exclusive operation boundary.
                let _operation = engine.operations.read().await;
                // Retry failed persistence on the next tick. Commands still
                // report their own persistence errors directly to the caller.
                let _ = engine.apply_scheduling().await;
                engine.sample_accounting();
                let saved = engine
                    .accounting
                    .lock()
                    .unwrap()
                    .checkpoint(&engine.state_directory, false);
                *engine.statistics_warning.lock().unwrap() = saved.err().map(|_| "Transfer statistics could not be saved. Check space and access to the library folder. Totals remain in memory until saving succeeds.".into());
            }
        });
        Ok(engine)
    }

    fn ready(&self) -> Result<()> {
        if self.stopping.load(Ordering::SeqCst) {
            Err(CoreError::Stopping)
        } else {
            Ok(())
        }
    }

    pub fn snapshot(&self) -> Result<Snapshot> {
        self.ready()?;
        self.sample_accounting();
        let mut torrents = self
            .session
            .with_torrents(|ts| ts.map(|(_, t)| adapter::record(t)).collect::<Vec<_>>());
        torrents.sort_by_key(|a| a.name.to_lowercase());
        let accounting = self.accounting.lock().unwrap();
        let queue = self.queue.lock().unwrap();
        for torrent in &mut torrents {
            if let Some(totals) = accounting.entries.get(&torrent.id) {
                torrent.network_downloaded_bytes = totals.downloaded;
                torrent.network_uploaded_bytes = totals.uploaded;
                torrent.added_at = totals.added;
                torrent.completed_at = totals.completed;
            }
            torrent.queue_position = queue
                .order
                .iter()
                .position(|id| id == &torrent.id)
                .map(|i| i as u32 + 1)
                .unwrap_or(0);
        }
        let scheduling = self.scheduling.lock().unwrap();
        if scheduling
            .active
            .is_some_and(|i| scheduling.config.rules[i].suspend)
        {
            for torrent in &mut torrents {
                if torrent.state == "queued" {
                    torrent.state = "scheduled".into();
                }
            }
        }
        Ok(Snapshot {
            schedule: scheduling.config.clone(),
            scheduling_warning: self.scheduling_warning.lock().unwrap().clone(),
            schedule_active: scheduling.active.is_some(),
            normal_download_limit: scheduling.normal_limits[0],
            normal_upload_limit: scheduling.normal_limits[1],
            peer_encryption: connection_settings::name(self.active_peer_encryption).into(),
            saved_peer_encryption: connection_settings::name(
                self.connection_settings.lock().unwrap().peer_encryption,
            )
            .into(),
            statistics_warning: self.statistics_warning.lock().unwrap().clone(),
            protocol_version: PROTOCOL_VERSION,
            engine_version: "librqbit 9.0.1 + SeedHop patches".into(),
            instance_id: self.instance_id.clone(),
            listen_port: self.session.listen_addr().map(|a| a.port()).unwrap_or(0),
            download_limit: self
                .session
                .ratelimits
                .get_download_bps()
                .map(|v| v.get())
                .unwrap_or(0),
            upload_limit: self
                .session
                .ratelimits
                .get_upload_bps()
                .map(|v| v.get())
                .unwrap_or(0),
            torrents,
            max_active_downloads: queue.max_active,
            file_operation: self
                .file_operation
                .lock()
                .unwrap()
                .as_ref()
                .map(|operation| operation.snapshot()),
            storage_recovery: self.storage_recovery.lock().unwrap().clone(),
        })
    }

    pub async fn inspect(
        &self,
        request_id: String,
        magnet: Option<String>,
        bytes: Vec<u8>,
    ) -> Result<TorrentPreview> {
        let _operation = self.operations.read().await;
        self.ready()?;
        if magnet.is_none() {
            return adapter::preview(bytes);
        }
        let magnet = magnet.unwrap_or_default();
        let parsed = url::Url::parse(&magnet)
            .map_err(|_| CoreError::InvalidInput("Paste a valid magnet link.".into()))?;
        if parsed.scheme() != "magnet" {
            return Err(CoreError::InvalidInput(
                "This preview accepts magnet links or .torrent files.".into(),
            ));
        }
        if !parsed.query_pairs().any(|(k, v)| {
            k == "xt" && (v.starts_with("urn:btih:") || v.starts_with("urn:btmh:1220"))
        }) {
            return Err(CoreError::Unsupported);
        }
        let (cancel, mut receiver) = tokio::sync::watch::channel(false);
        match self.resolving.lock().unwrap().entry(request_id.clone()) {
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(cancel);
            }
            std::collections::hash_map::Entry::Occupied(_) => {
                return Err(CoreError::InvalidInput(
                    "This request is already resolving.".into(),
                ));
            }
        }
        let result = tokio::select! {
            _ = receiver.changed() => Err(CoreError::Cancelled),
            response = tokio::time::timeout(Duration::from_secs(120), self.session.add_torrent(AddTorrent::from_url(magnet), Some(AddTorrentOptions { list_only: true, ..Default::default() }))) => {
                match response {
                    Ok(Ok(librqbit::AddTorrentResponse::ListOnly(list))) => adapter::preview(list.torrent_bytes.to_vec()),
                    Ok(Ok(_)) => Err(CoreError::InvalidInput("This torrent is already in your library.".into())),
                    Ok(Err(error)) => Err(adapter::engine_error(error)),
                    Err(_) => Err(CoreError::Engine("No peers supplied metadata within two minutes. Check the link and connection, then retry.".into())),
                }
            }
        };
        self.resolving.lock().unwrap().remove(&request_id);
        result
    }

    pub fn cancel(&self, request_id: &str) {
        if let Some(sender) = self.resolving.lock().unwrap().get(request_id) {
            let _ = sender.send(true);
        }
    }

    pub async fn add(
        &self,
        bytes: Vec<u8>,
        destination: String,
        selected: Vec<u32>,
        paused: bool,
        reuse: bool,
    ) -> Result<String> {
        self.add_with_options(
            bytes,
            destination,
            selected,
            TorrentAddOptions {
                paused,
                reuse,
                ..Default::default()
            },
        )
        .await
    }

    pub async fn add_with_options(
        &self,
        bytes: Vec<u8>,
        destination: String,
        selected: Vec<u32>,
        options: TorrentAddOptions,
    ) -> Result<String> {
        let _operation = self.operations.write().await;
        self.ready()?;
        let TorrentAddOptions {
            paused,
            reuse,
            add_to_top,
            sequential,
            first_last_pieces,
        } = options;
        let metadata = adapter::preview(bytes.clone())?;
        if self
            .snapshot()?
            .torrents
            .iter()
            .any(|t| t.id == metadata.id)
        {
            return Err(CoreError::InvalidInput(
                "This torrent is already in your library.".into(),
            ));
        }
        if selected.is_empty() || selected.iter().any(|i| *i as usize >= metadata.files.len()) {
            return Err(CoreError::InvalidInput(
                "Select at least one file to download.".into(),
            ));
        }
        let root = PathBuf::from(destination)
            .canonicalize()
            .map_err(storage::io_error)?;
        // Force one explicit root so preflight validation matches the engine's actual paths.
        storage::validate_destination(
            &root,
            &metadata
                .files
                .iter()
                .map(|f| f.name.clone())
                .collect::<Vec<_>>(),
        )?;
        if !reuse && metadata.files.iter().any(|f| root.join(&f.name).exists()) {
            return Err(CoreError::InvalidInput("Files already exist here. Enable verification of existing files or choose another folder.".into()));
        }
        let queued = self.queue.lock().unwrap().max_active > 0 || self.schedule_suspended();
        // Commit accounting before membership so a statistics write failure cannot
        // report an unsuccessful Add after the torrent has already been added.
        self.accounting
            .lock()
            .unwrap()
            .new_transfer(metadata.id.clone());
        self.accounting
            .lock()
            .unwrap()
            .checkpoint(&self.state_directory, true)?;
        // Save admission order before membership: a failed queue write must never
        // report an unsuccessful Add after the torrent has already been added.
        if add_to_top {
            self.prepare_top_of_queue(&metadata.id)?;
        }
        self.session
            .add_torrent(
                AddTorrent::TorrentFileBytes(bytes.into()),
                Some(AddTorrentOptions {
                    output_folder: Some(root.to_string_lossy().into()),
                    only_files: Some(selected.into_iter().map(|i| i as usize).collect()),
                    paused,
                    queued,
                    sequential,
                    first_last_pieces,
                    overwrite: reuse,
                    ..Default::default()
                }),
            )
            .await
            .map_err(adapter::engine_error)?;
        self.sample_accounting();
        Ok(metadata.id)
    }

    fn handle(&self, id: &str) -> Result<Arc<librqbit::ManagedTorrent>> {
        self.session
            .with_torrents(|torrents| {
                for (_, torrent) in torrents {
                    if adapter::torrent_id(torrent) == id {
                        return Some(torrent.clone());
                    }
                }
                None
            })
            .ok_or_else(|| {
                CoreError::InvalidInput("This torrent is no longer in the library.".into())
            })
    }

    pub async fn set_paused(&self, id: String, paused: bool) -> Result<()> {
        let _operation = self.operations.write().await;
        self.ready()?;
        let handle = self.handle(&id)?;
        let record = adapter::record(&handle);
        if !paused {
            let root = Path::new(&record.destination);
            if !root.is_dir() {
                return Err(CoreError::Storage(format!(
                    "The saved destination is unavailable: {}. Reconnect the drive and retry.",
                    record.destination
                )));
            }
            storage::validate_destination(
                root,
                &record
                    .files
                    .iter()
                    .map(|file| file.name.clone())
                    .collect::<Vec<_>>(),
            )?;
        }
        // A fatal storage error retains the previous running intent. Resume must
        // restart and recheck it even when that intent already says "running".
        let retry_error = !paused && record.state == "error";
        if paused == handle.is_paused() && !retry_error {
            return self
                .session
                .try_update_persistence_metadata(&handle)
                .await
                .map_err(adapter::engine_error);
        }
        if paused {
            self.session.pause(&handle).await
        } else {
            let queued = self.queue.lock().unwrap().max_active > 0 || self.schedule_suspended();
            if queued {
                self.session
                    .set_queued(&handle, true)
                    .await
                    .map_err(adapter::engine_error)?;
            }
            self.session.unpause(&handle).await
        }
        .map_err(adapter::engine_error)?;
        self.reconcile_queue().await
    }

    pub async fn select_files(&self, id: String, indices: Vec<u32>) -> Result<()> {
        let _operation = self.operations.write().await;
        self.ready()?;
        let handle = self.handle(&id)?;
        let count = adapter::record(&handle).files.len();
        if indices.is_empty() || indices.iter().any(|i| *i as usize >= count) {
            return Err(CoreError::InvalidInput("Select valid files.".into()));
        }
        self.session
            .update_only_files(&handle, &indices.into_iter().map(|i| i as usize).collect())
            .await
            .map_err(adapter::engine_error)
    }

    pub async fn force_recheck(&self, id: String) -> Result<()> {
        let _operation = self.operations.write().await;
        self.ready()?;
        let handle = self.handle(&id)?;
        self.validate_recheck_destination(&handle)?;
        self.session
            .force_recheck(&handle)
            .await
            .map_err(adapter::engine_error)
    }

    /// Retrying an error retains pause intent and re-enters queue admission.
    /// A delayed retry for a transfer that has recovered is a harmless no-op.
    pub async fn retry_transfer(&self, id: String) -> Result<()> {
        let _operation = self.operations.write().await;
        self.ready()?;
        let handle = self.handle(&id)?;
        if !matches!(handle.stats().state, librqbit::TorrentStatsState::Error) {
            return Ok(());
        }
        if self
            .storage_recovery
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|notice| notice.torrent_id == id || notice.torrent_id.is_empty())
        {
            return Err(CoreError::Storage(
                "Resolve the retained-file cleanup before retrying this transfer.".into(),
            ));
        }
        self.validate_recheck_destination(&handle)?;
        if !handle.is_paused() && self.queue.lock().unwrap().max_active > 0 {
            self.session
                .set_queued(&handle, true)
                .await
                .map_err(adapter::engine_error)?;
        }
        self.session
            .force_recheck(&handle)
            .await
            .map_err(adapter::engine_error)?;
        self.reconcile_queue().await
    }

    fn validate_recheck_destination(&self, handle: &Arc<librqbit::ManagedTorrent>) -> Result<()> {
        let record = adapter::record(handle);
        let root = Path::new(&record.destination);
        if !root.is_dir() {
            return Err(CoreError::Storage(format!(
                "The saved destination is unavailable: {}. Reconnect the drive and retry.",
                record.destination
            )));
        }
        storage::validate_destination(
            root,
            &record
                .files
                .iter()
                .map(|f| f.name.clone())
                .collect::<Vec<_>>(),
        )?;
        Ok(())
    }

    pub async fn remove(&self, id: String, delete_files: bool) -> Result<()> {
        let _operation = self.operations.write().await;
        self.ready()?;
        if self
            .storage_recovery
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|notice| notice.torrent_id == id)
        {
            return Err(CoreError::Storage(
                "Resolve this transfer's pending file cleanup before removing it.".into(),
            ));
        }
        let handle = self.handle(&id)?;
        let record = adapter::record(&handle);
        storage::validate_destination(
            Path::new(&record.destination),
            &record
                .files
                .iter()
                .map(|f| f.name.clone())
                .collect::<Vec<_>>(),
        )?;
        self.session
            .delete(
                librqbit::api::TorrentIdOrHash::Id(handle.id()),
                delete_files,
            )
            .await
            .map_err(adapter::engine_error)?;
        self.accounting.lock().unwrap().entries.remove(&id);
        let saved = self
            .accounting
            .lock()
            .unwrap()
            .checkpoint(&self.state_directory, true);
        *self.statistics_warning.lock().unwrap() = saved.err().map(|_| "Transfer statistics could not be saved. Check space and access to the library folder. Totals remain in memory until saving succeeds.".into());
        Ok(())
    }

    pub async fn set_limits(&self, down: u32, up: u32) -> Result<()> {
        let _operation = self.operations.write().await;
        self.ready()?;
        let value = serde_json::to_vec(&[down, up]).map_err(adapter::engine_error)?;
        storage::atomic_write(&self.state_directory.join("limits.json"), &value)?;
        self.scheduling.lock().unwrap().normal_limits = [down, up];
        self.reconcile_schedule();
        Ok(())
    }

    pub async fn shutdown(&self) -> Result<()> {
        self.stopping.store(true, Ordering::SeqCst);
        for cancel in self.resolving.lock().unwrap().values() {
            let _ = cancel.send(true);
        }
        let _operation = self.operations.write().await;
        self.session
            .stop_and_wait()
            .await
            .map_err(adapter::engine_error)?;
        self.sample_accounting();
        self.accounting
            .lock()
            .unwrap()
            .checkpoint(&self.state_directory, true)
    }

    pub fn retain_lease(&self) -> &storage::LibraryLease {
        &self.lease
    }
}

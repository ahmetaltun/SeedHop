pub mod adapter;
mod model;
pub mod storage;
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

pub const PROTOCOL_VERSION: u32 = 1;

/// Owns library state. The FFI host retains this lease until its runtime has stopped.
pub struct Engine {
    session: Arc<Session>,
    lease: storage::LibraryLease,
    state_directory: PathBuf,
    instance_id: String,
    operations: RwLock<()>,
    resolving: Mutex<HashMap<String, tokio::sync::watch::Sender<bool>>>,
    stopping: AtomicBool,
}

impl Engine {
    pub async fn open(config: EngineConfig) -> Result<Arc<Self>> {
        let dir = PathBuf::from(&config.state_directory);
        let lease = storage::LibraryLease::acquire(&dir)?;
        let downloads = PathBuf::from(&config.download_directory);
        storage::validate_destination(&downloads, &[])?;
        let session_dir = dir.join("session");
        std::fs::create_dir_all(&session_dir).map_err(storage::io_error)?;
        // Preserve unreadable state in place. Never overwrite it with an empty session.
        let session_path = session_dir.join("session.json");
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
                        storage::validate_destination(
                            Path::new(folder),
                            &preview
                                .files
                                .iter()
                                .map(|f| f.name.clone())
                                .collect::<Vec<_>>(),
                        )?;
                    }
                }
            }
        }
        let limits_path = dir.join("limits.json");
        let limits: [u32; 2] = if limits_path.exists() {
            serde_json::from_slice(&std::fs::read(limits_path).map_err(storage::io_error)?)
                .map_err(|e| CoreError::CorruptState(e.to_string()))?
        } else {
            [0, 0]
        };
        let options = SessionOptions {
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
                listen_addr: ([0, 0, 0, 0], config.listen_port).into(),
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
        Ok(Arc::new(Self {
            session,
            lease,
            state_directory: dir,
            instance_id: uuid::Uuid::new_v4().to_string(),
            operations: RwLock::new(()),
            resolving: Mutex::new(HashMap::new()),
            stopping: AtomicBool::new(false),
        }))
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
        let mut torrents = self
            .session
            .with_torrents(|ts| ts.map(|(_, t)| adapter::record(t)).collect::<Vec<_>>());
        torrents.sort_by_key(|a| a.name.to_lowercase());
        Ok(Snapshot {
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
        if !parsed
            .query_pairs()
            .any(|(k, v)| k == "xt" && v.starts_with("urn:btih:"))
        {
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
        let _operation = self.operations.write().await;
        self.ready()?;
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
        self.session
            .add_torrent(
                AddTorrent::TorrentFileBytes(bytes.into()),
                Some(AddTorrentOptions {
                    output_folder: Some(root.to_string_lossy().into()),
                    only_files: Some(selected.into_iter().map(|i| i as usize).collect()),
                    paused,
                    overwrite: reuse,
                    ..Default::default()
                }),
            )
            .await
            .map_err(adapter::engine_error)?;
        Ok(metadata.id)
    }

    fn handle(&self, id: &str) -> Result<Arc<librqbit::ManagedTorrent>> {
        let hash = id
            .parse()
            .map_err(|_| CoreError::InvalidInput("Invalid torrent identifier.".into()))?;
        self.session
            .get(librqbit::api::TorrentIdOrHash::Hash(hash))
            .ok_or_else(|| {
                CoreError::InvalidInput("This torrent is no longer in the library.".into())
            })
    }

    pub async fn set_paused(&self, id: String, paused: bool) -> Result<()> {
        let _operation = self.operations.write().await;
        self.ready()?;
        let handle = self.handle(&id)?;
        if paused == handle.is_paused() {
            return self
                .session
                .try_update_persistence_metadata(&handle)
                .await
                .map_err(adapter::engine_error);
        }
        if paused {
            self.session.pause(&handle).await
        } else {
            self.session.unpause(&handle).await
        }
        .map_err(adapter::engine_error)
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

    pub async fn remove(&self, id: String, delete_files: bool) -> Result<()> {
        let _operation = self.operations.write().await;
        self.ready()?;
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
            .map_err(adapter::engine_error)
    }

    pub async fn set_limits(&self, down: u32, up: u32) -> Result<()> {
        let _operation = self.operations.write().await;
        self.ready()?;
        let value = serde_json::to_vec(&[down, up]).map_err(adapter::engine_error)?;
        storage::atomic_write(&self.state_directory.join("limits.json"), &value)?;
        self.session
            .ratelimits
            .set_download_bps(NonZeroU32::new(down));
        self.session.ratelimits.set_upload_bps(NonZeroU32::new(up));
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
            .map_err(adapter::engine_error)
    }

    pub fn retain_lease(&self) -> &storage::LibraryLease {
        &self.lease
    }
}

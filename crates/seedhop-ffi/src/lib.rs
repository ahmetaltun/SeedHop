use seedhop_core::{Engine, EngineConfig};
use std::sync::{Arc, Mutex};
uniffi::setup_scaffolding!();

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum BridgeError {
    #[error("{message}")]
    Operation { message: String },
}
fn error(e: impl std::fmt::Display) -> BridgeError {
    BridgeError::Operation {
        message: e.to_string(),
    }
}

#[derive(uniffi::Record)]
pub struct TransferFile {
    pub index: u32,
    pub name: String,
    pub size: u64,
    pub downloaded: u64,
    pub selected: bool,
    pub priority: u8,
}
impl From<seedhop_core::FileRecord> for TransferFile {
    fn from(f: seedhop_core::FileRecord) -> Self {
        Self {
            index: f.index,
            name: f.name,
            size: f.size,
            downloaded: f.downloaded,
            selected: f.selected,
            priority: f.priority,
        }
    }
}
#[derive(uniffi::Record)]
pub struct Transfer {
    pub id: String,
    pub name: String,
    pub state: String,
    pub total_bytes: u64,
    pub downloaded_bytes: u64,
    pub uploaded_bytes: u64,
    pub download_rate: u64,
    pub upload_rate: u64,
    pub peers: u32,
    pub destination: String,
    pub files: Vec<TransferFile>,
    pub trackers: Vec<String>,
    pub error: Option<String>,
    pub queue_position: u32,
    pub paused_intent: bool,
}
impl From<seedhop_core::TorrentRecord> for Transfer {
    fn from(t: seedhop_core::TorrentRecord) -> Self {
        Self {
            id: t.id,
            name: t.name,
            state: t.state,
            total_bytes: t.total_bytes,
            downloaded_bytes: t.downloaded_bytes,
            uploaded_bytes: t.uploaded_bytes,
            download_rate: t.download_rate,
            upload_rate: t.upload_rate,
            peers: t.peers,
            destination: t.destination,
            files: t.files.into_iter().map(Into::into).collect(),
            trackers: t.trackers,
            error: t.error,
            queue_position: t.queue_position,
            paused_intent: t.paused_intent,
        }
    }
}
#[derive(uniffi::Record)]
pub struct EngineSnapshot {
    pub protocol_version: u32,
    pub engine_version: String,
    pub instance_id: String,
    pub listen_port: u16,
    pub download_limit: u32,
    pub upload_limit: u32,
    pub max_active_downloads: u32,
    pub torrents: Vec<Transfer>,
}
#[derive(uniffi::Record)]
pub struct MetadataPreview {
    pub id: String,
    pub name: String,
    pub total_bytes: u64,
    pub files: Vec<TransferFile>,
    pub torrent_bytes: Vec<u8>,
}

#[derive(uniffi::Object)]
pub struct EngineBridge {
    core: Mutex<Option<Arc<Engine>>>,
    runtime: Mutex<Option<Arc<tokio::runtime::Runtime>>>,
}

impl EngineBridge {
    fn handles(&self) -> Result<(Arc<Engine>, Arc<tokio::runtime::Runtime>), BridgeError> {
        let core = self
            .core
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| error("The engine has stopped."))?;
        let rt = self
            .runtime
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| error("The engine has stopped."))?;
        Ok((core, rt))
    }
}

// Synchronous FFI calls run exclusively on the host's worker queues. The XPC API
// and Swift client remain async; the Rust reactor is owned by this object.
#[uniffi::export]
impl EngineBridge {
    #[uniffi::constructor]
    pub fn new(
        state_directory: String,
        download_directory: String,
        discovery: bool,
    ) -> Result<Arc<Self>, BridgeError> {
        let runtime = Arc::new(
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .worker_threads(2)
                .build()
                .map_err(error)?,
        );
        let core = runtime
            .block_on(Engine::open(EngineConfig {
                state_directory,
                download_directory,
                allow_network_discovery: discovery,
                listen_port: 0,
            }))
            .map_err(error)?;
        Ok(Arc::new(Self {
            core: Mutex::new(Some(core)),
            runtime: Mutex::new(Some(runtime)),
        }))
    }
    pub fn snapshot(&self) -> Result<EngineSnapshot, BridgeError> {
        let (core, _) = self.handles()?;
        let s = core.snapshot().map_err(error)?;
        Ok(EngineSnapshot {
            protocol_version: s.protocol_version,
            engine_version: s.engine_version,
            instance_id: s.instance_id,
            listen_port: s.listen_port,
            download_limit: s.download_limit,
            upload_limit: s.upload_limit,
            torrents: s.torrents.into_iter().map(Into::into).collect(),
            max_active_downloads: s.max_active_downloads,
        })
    }
    pub fn inspect(
        &self,
        request_id: String,
        magnet: Option<String>,
        bytes: Vec<u8>,
    ) -> Result<MetadataPreview, BridgeError> {
        let (core, rt) = self.handles()?;
        let p = rt
            .block_on(core.inspect(request_id, magnet, bytes))
            .map_err(error)?;
        Ok(MetadataPreview {
            id: p.id,
            name: p.name,
            total_bytes: p.total_bytes,
            files: p.files.into_iter().map(Into::into).collect(),
            torrent_bytes: p.torrent_bytes,
        })
    }
    pub fn cancel(&self, request_id: String) -> Result<(), BridgeError> {
        self.handles()?.0.cancel(&request_id);
        Ok(())
    }
    pub fn add(
        &self,
        bytes: Vec<u8>,
        destination: String,
        selected: Vec<u32>,
        paused: bool,
        reuse: bool,
    ) -> Result<String, BridgeError> {
        let (core, rt) = self.handles()?;
        rt.block_on(core.add(bytes, destination, selected, paused, reuse))
            .map_err(error)
    }
    pub fn set_paused(&self, id: String, paused: bool) -> Result<(), BridgeError> {
        let (core, rt) = self.handles()?;
        rt.block_on(core.set_paused(id, paused)).map_err(error)
    }
    pub fn select_files(&self, id: String, indices: Vec<u32>) -> Result<(), BridgeError> {
        let (core, rt) = self.handles()?;
        rt.block_on(core.select_files(id, indices)).map_err(error)
    }
    pub fn force_recheck(&self, id: String) -> Result<(), BridgeError> {
        let (core, rt) = self.handles()?;
        rt.block_on(core.force_recheck(id)).map_err(error)
    }
    pub fn set_file_priorities(&self, id: String, priorities: Vec<u8>) -> Result<(), BridgeError> {
        let (core, rt) = self.handles()?;
        rt.block_on(core.set_file_priorities(id, priorities))
            .map_err(error)
    }
    pub fn set_queue(&self, max_active: u32) -> Result<(), BridgeError> {
        let (core, rt) = self.handles()?;
        rt.block_on(core.set_queue(max_active)).map_err(error)
    }
    pub fn move_in_queue(&self, id: String, position: u32) -> Result<(), BridgeError> {
        let (core, rt) = self.handles()?;
        rt.block_on(core.move_in_queue(id, position)).map_err(error)
    }
    pub fn relocate(&self, id: String, destination: String) -> Result<(), BridgeError> {
        let (core, rt) = self.handles()?;
        rt.block_on(core.relocate(id, destination)).map_err(error)
    }
    pub fn rename_file(&self, id: String, index: u32, name: String) -> Result<(), BridgeError> {
        let (core, rt) = self.handles()?;
        rt.block_on(core.rename_file(id, index, name))
            .map_err(error)
    }
    pub fn remove(&self, id: String, delete_files: bool) -> Result<(), BridgeError> {
        let (core, rt) = self.handles()?;
        rt.block_on(core.remove(id, delete_files)).map_err(error)
    }
    pub fn set_limits(&self, download: u32, upload: u32) -> Result<(), BridgeError> {
        let (core, rt) = self.handles()?;
        rt.block_on(core.set_limits(download, upload))
            .map_err(error)
    }
    pub fn shutdown(&self) -> Result<(), BridgeError> {
        let (core, rt) = self.handles()?;
        rt.block_on(core.shutdown()).map_err(error)?;
        self.core.lock().unwrap().take();
        self.runtime.lock().unwrap().take();
        // Each operation owns a runtime reference; retain the lease until those
        // references have drained, then synchronously tear down every reactor task.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while Arc::strong_count(&rt) > 1 {
            if std::time::Instant::now() >= deadline {
                return Err(error("An operation did not finish during shutdown."));
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let runtime = Arc::try_unwrap(rt).map_err(|_| error("The runtime is still in use."))?;
        // Runtime::drop waits for blocking disk work; no timeout that could release
        // ownership while another thread still has a writable descriptor.
        drop(runtime);
        drop(core);
        Ok(())
    }
}

#[uniffi::export]
pub fn protocol_version() -> u32 {
    seedhop_core::PROTOCOL_VERSION
}

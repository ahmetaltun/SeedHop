use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("Another SeedHop engine is already using this library.")]
    Busy,
    #[error("{0}")]
    InvalidInput(String),
    #[error("This torrent format is not supported. Use a BitTorrent v1, v2, or hybrid torrent.")]
    Unsupported,
    #[error("The operation was cancelled.")]
    Cancelled,
    #[error("The engine is stopping. Try again once it reconnects.")]
    Stopping,
    #[error("The saved library could not be read. Your files have been preserved: {0}")]
    CorruptState(String),
    #[error("{0}")]
    Storage(String),
    #[error("{0}")]
    Engine(String),
}

pub type Result<T> = std::result::Result<T, CoreError>;

#[derive(Clone, Copy, Debug, Default)]
pub struct TorrentAddOptions {
    pub paused: bool,
    pub reuse: bool,
    pub add_to_top: bool,
    pub sequential: bool,
    pub first_last_pieces: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FileRecord {
    pub index: u32,
    pub name: String,
    pub size: u64,
    pub downloaded: u64,
    pub selected: bool,
    pub priority: u8,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TorrentRecord {
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
    pub files: Vec<FileRecord>,
    pub trackers: Vec<String>,
    pub error: Option<String>,
    pub error_kind: Option<String>,
    pub queue_position: u32,
    pub paused_intent: bool,
    pub network_downloaded_bytes: Option<u64>,
    pub network_uploaded_bytes: Option<u64>,
    pub added_at: Option<u64>,
    pub completed_at: Option<u64>,
    pub eta_seconds: Option<u64>,
    pub selected_bytes: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileOperation {
    pub id: String,
    pub torrent_id: String,
    pub kind: String,
    pub phase: String,
    pub completed_bytes: u64,
    pub total_bytes: u64,
    pub cancel_requested: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageRecovery {
    pub id: String,
    pub torrent_id: String,
    pub active_location: String,
    pub message: String,
    pub original_directory: String,
    pub destination_directory: String,
    pub can_keep_copies: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub schedule: crate::ScheduleConfig,
    pub schedule_active: bool,
    pub scheduling_warning: Option<String>,
    pub normal_download_limit: u32,
    pub normal_upload_limit: u32,
    pub alternative_download_limit: u32,
    pub alternative_upload_limit: u32,
    pub alternative_limits_enabled: bool,
    pub statistics_warning: Option<String>,
    pub peer_encryption: String,
    pub saved_peer_encryption: String,
    pub protocol_version: u32,
    pub engine_version: String,
    pub instance_id: String,
    pub listen_port: u16,
    pub download_limit: u32,
    pub upload_limit: u32,
    pub max_active_downloads: u32,
    pub torrents: Vec<TorrentRecord>,
    pub file_operation: Option<FileOperation>,
    pub storage_recovery: Option<StorageRecovery>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TorrentPreview {
    pub id: String,
    pub name: String,
    pub total_bytes: u64,
    pub files: Vec<FileRecord>,
    pub torrent_bytes: Vec<u8>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineConfig {
    pub state_directory: String,
    pub download_directory: String,
    pub allow_network_discovery: bool,
    pub listen_port: u16,
}

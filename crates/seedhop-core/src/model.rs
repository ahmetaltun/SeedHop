use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("Another SeedHop engine is already using this library.")]
    Busy,
    #[error("{0}")]
    InvalidInput(String),
    #[error("This torrent format is not supported by this preview. Use a BitTorrent v1 torrent.")]
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

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FileRecord {
    pub index: u32,
    pub name: String,
    pub size: u64,
    pub downloaded: u64,
    pub selected: bool,
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
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub protocol_version: u32,
    pub engine_version: String,
    pub instance_id: String,
    pub listen_port: u16,
    pub download_limit: u32,
    pub upload_limit: u32,
    pub torrents: Vec<TorrentRecord>,
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

use seedhop_core::{Engine, EngineConfig, PrivateEngineConfig, PrivateProtection};
use std::future::Future;
use std::sync::{Arc, Mutex};
uniffi::setup_scaffolding!();

#[derive(Debug, Clone, serde::Serialize, uniffi::Enum)]
#[serde(tag = "type", content = "value", rename_all = "camelCase")]
pub enum NoticeArgument {
    Text(String),
    Number(u64),
}

#[derive(Debug, Clone, serde::Serialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct UserNotice {
    pub code: String,
    pub arguments: Vec<NoticeArgument>,
    pub diagnostic: Option<String>,
}
impl UserNotice {
    fn new(code: &str, diagnostic: Option<String>) -> Self {
        Self {
            code: code.into(),
            arguments: vec![],
            diagnostic,
        }
    }
}

fn transfer_notice(detail: Option<&str>, kind: Option<&str>) -> Option<UserNotice> {
    detail.map(|detail| {
        UserNotice::new(
            match kind {
                Some("storage_space") => "storage.space",
                Some("storage_permission") => "storage.permission",
                Some("storage_missing") => "storage.missing",
                Some("storage") => "storage.failed",
                _ => "engine.failed",
            },
            Some(detail.to_owned()),
        )
    })
}

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum BridgeError {
    #[error("The operation was cancelled.")]
    Cancelled,
    #[error("{message}")]
    Operation { message: String, notice: UserNotice },
}
fn layout_error(error_value: seedhop_core::CoreError) -> BridgeError {
    match error_value {
        seedhop_core::CoreError::Cancelled => BridgeError::Cancelled,
        other => error(other),
    }
}
fn error(e: impl std::fmt::Display + std::any::Any) -> BridgeError {
    if matches!(
        (&e as &dyn std::any::Any).downcast_ref::<seedhop_core::CoreError>(),
        Some(seedhop_core::CoreError::Cancelled)
    ) {
        return BridgeError::Cancelled;
    }
    let code = (&e as &dyn std::any::Any)
        .downcast_ref::<seedhop_core::CoreError>()
        .map(seedhop_core::CoreError::message_code)
        .unwrap_or("engine.failed");
    let mut notice = UserNotice::new(code, Some(e.to_string()));
    if code == "input.use_at_most_schedule_windows" {
        notice.arguments.push(NoticeArgument::Number(32));
    }
    BridgeError::Operation {
        message: e.to_string(),
        notice,
    }
}

#[derive(uniffi::Record)]
pub struct TorrentAddOptions {
    pub paused: bool,
    pub reuse: bool,
    pub add_to_top: bool,
    pub sequential: bool,
    pub first_last_pieces: bool,
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
    pub can_reannounce: bool,
    pub error: Option<String>,
    pub error_kind: Option<String>,
    pub error_message: Option<UserNotice>,
    pub queue_position: u32,
    pub paused_intent: bool,
    pub force_started: bool,
}
impl From<seedhop_core::TorrentRecord> for Transfer {
    fn from(t: seedhop_core::TorrentRecord) -> Self {
        Self {
            error_message: transfer_notice(t.error.as_deref(), t.error_kind.as_deref()),
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
            can_reannounce: t.can_reannounce,
            error: t.error,
            error_kind: t.error_kind,
            queue_position: t.queue_position,
            paused_intent: t.paused_intent,
            force_started: t.force_started,
        }
    }
}
#[derive(uniffi::Record)]
pub struct FileOperationProgress {
    pub id: String,
    pub torrent_id: String,
    pub kind: String,
    pub phase: String,
    pub completed_bytes: u64,
    pub total_bytes: u64,
    pub cancel_requested: bool,
}
impl From<seedhop_core::FileOperation> for FileOperationProgress {
    fn from(operation: seedhop_core::FileOperation) -> Self {
        Self {
            id: operation.id,
            torrent_id: operation.torrent_id,
            kind: operation.kind,
            phase: operation.phase,
            completed_bytes: operation.completed_bytes,
            total_bytes: operation.total_bytes,
            cancel_requested: operation.cancel_requested,
        }
    }
}

#[derive(uniffi::Record)]
pub struct StorageRecoveryNotice {
    pub id: String,
    pub torrent_id: String,
    pub active_location: String,
    pub message: String,
    pub original_directory: String,
    pub destination_directory: String,
    pub can_keep_copies: bool,
    pub message_descriptor: UserNotice,
}
impl From<seedhop_core::StorageRecovery> for StorageRecoveryNotice {
    fn from(notice: seedhop_core::StorageRecovery) -> Self {
        Self {
            message_descriptor: recovery_notice(&notice),
            id: notice.id,
            torrent_id: notice.torrent_id,
            active_location: notice.active_location,
            message: notice.message,
            original_directory: notice.original_directory,
            destination_directory: notice.destination_directory,
            can_keep_copies: notice.can_keep_copies,
        }
    }
}

fn recovery_notice(notice: &seedhop_core::StorageRecovery) -> UserNotice {
    UserNotice::new(
        match notice.active_location.as_str() {
            "new" => "recovery.new",
            "original" => "recovery.original",
            _ => "recovery.unknown",
        },
        Some(notice.message.clone()),
    )
}

#[derive(uniffi::Record)]
pub struct ScheduleWindow {
    pub days: u8,
    pub start_minute: u16,
    pub end_minute: u16,
    pub download_limit: u32,
    pub upload_limit: u32,
    pub suspend: bool,
}
impl From<seedhop_core::ScheduleRule> for ScheduleWindow {
    fn from(r: seedhop_core::ScheduleRule) -> Self {
        Self {
            days: r.days,
            start_minute: r.start_minute,
            end_minute: r.end_minute,
            download_limit: r.download_limit,
            upload_limit: r.upload_limit,
            suspend: r.suspend,
        }
    }
}
impl From<ScheduleWindow> for seedhop_core::ScheduleRule {
    fn from(r: ScheduleWindow) -> Self {
        Self {
            days: r.days,
            start_minute: r.start_minute,
            end_minute: r.end_minute,
            download_limit: r.download_limit,
            upload_limit: r.upload_limit,
            suspend: r.suspend,
        }
    }
}

#[derive(uniffi::Record)]
pub struct EngineSnapshot {
    pub statistics_message: Option<UserNotice>,
    pub scheduling_message: Option<UserNotice>,
    pub schedule_enabled: bool,
    pub schedule_rules: Vec<ScheduleWindow>,
    pub schedule_active: bool,
    pub scheduling_warning: Option<String>,
    pub normal_download_limit: u32,
    pub normal_upload_limit: u32,
    pub alternative_download_limit: u32,
    pub alternative_upload_limit: u32,
    pub alternative_limits_enabled: bool,
    pub network_blocked: bool,
    pub network_interface: Option<String>,
    pub peer_encryption: String,
    pub saved_peer_encryption: String,
    pub protocol_version: u32,
    pub engine_version: String,
    pub instance_id: String,
    pub listen_port: u16,
    pub download_limit: u32,
    pub upload_limit: u32,
    pub max_active_downloads: u32,
    pub torrents: Vec<Transfer>,
    pub file_operation: Option<FileOperationProgress>,
    pub storage_recovery: Option<StorageRecoveryNotice>,
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
    fn launch(
        start: impl Future<Output = seedhop_core::Result<Arc<Engine>>> + Send + 'static,
    ) -> Result<Arc<Self>, BridgeError> {
        let runtime = Arc::new(
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .worker_threads(2)
                .build()
                .map_err(error)?,
        );
        // Start on a Tokio worker, away from the smaller macOS dispatch stack.
        let core = runtime
            .block_on(runtime.spawn(start))
            .map_err(error)?
            .map_err(error)?;
        Ok(Arc::new(Self {
            core: Mutex::new(Some(core)),
            runtime: Mutex::new(Some(runtime)),
        }))
    }

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
        allow_local_network: bool,
    ) -> Result<Arc<Self>, BridgeError> {
        let config = EngineConfig {
            state_directory,
            download_directory,
            allow_network_discovery: discovery,
            allow_local_network,
            listen_port: 0,
        };
        Self::launch(Engine::open(config))
    }

    #[uniffi::constructor]
    pub fn new_private(
        download_directory: String,
        protection: String,
        discovery: bool,
    ) -> Result<Arc<Self>, BridgeError> {
        let settings: PrivateProtection = serde_json::from_str(&protection).map_err(error)?;
        Self::launch(Engine::open_private(PrivateEngineConfig {
            download_directory,
            network_policy: settings.network_policy,
            peer_encryption: settings.peer_encryption,
            allow_network_discovery: discovery,
        }))
    }

    pub fn private_protection(&self) -> Result<String, BridgeError> {
        let (core, runtime) = self.handles()?;
        let settings = runtime.block_on(core.private_protection()).map_err(error)?;
        serde_json::to_string(&settings).map_err(error)
    }
    pub fn network_info(&self) -> Result<String, BridgeError> {
        self.handles()?.0.network_info().map_err(error)
    }
    pub fn set_network_policy(&self, policy: String) -> Result<(), BridgeError> {
        let (core, runtime) = self.handles()?;
        runtime
            .block_on(core.set_network_policy(policy))
            .map_err(error)
    }
    pub fn set_peer_encryption(&self, policy: String) -> Result<(), BridgeError> {
        let (core, runtime) = self.handles()?;
        runtime
            .block_on(core.set_peer_encryption(policy))
            .map_err(error)
    }
    pub fn snapshot_json(&self) -> Result<String, BridgeError> {
        let snapshot = self.handles()?.0.snapshot().map_err(error)?;
        let mut value = serde_json::to_value(&snapshot).map_err(error)?;
        value["statisticsMessage"] = serde_json::to_value(
            snapshot
                .statistics_warning
                .as_ref()
                .map(|detail| UserNotice::new("statistics.save_failed", Some(detail.clone()))),
        )
        .map_err(error)?;
        value["schedulingMessage"] = serde_json::to_value(
            snapshot
                .scheduling_warning
                .as_ref()
                .map(|detail| UserNotice::new("schedule.apply_failed", Some(detail.clone()))),
        )
        .map_err(error)?;
        for (record, torrent) in value["torrents"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .zip(&snapshot.torrents)
        {
            record["errorMessage"] = serde_json::to_value(transfer_notice(
                torrent.error.as_deref(),
                torrent.error_kind.as_deref(),
            ))
            .map_err(error)?;
        }
        if let Some(notice) = snapshot.storage_recovery {
            value["storageRecovery"]["messageDescriptor"] =
                serde_json::to_value(recovery_notice(&notice)).map_err(error)?;
        }
        serde_json::to_string(&value).map_err(error)
    }
    pub fn torrent_details(
        &self,
        id: String,
        section: String,
        all_peers: bool,
    ) -> Result<String, BridgeError> {
        serde_json::to_string(
            &self
                .handles()?
                .0
                .torrent_details(id, section, all_peers)
                .map_err(error)?,
        )
        .map_err(error)
    }
    pub fn snapshot(&self) -> Result<EngineSnapshot, BridgeError> {
        let (core, _) = self.handles()?;
        let s = core.snapshot().map_err(error)?;
        Ok(EngineSnapshot {
            statistics_message: s
                .statistics_warning
                .as_ref()
                .map(|detail| UserNotice::new("statistics.save_failed", Some(detail.clone()))),
            scheduling_message: s
                .scheduling_warning
                .as_ref()
                .map(|detail| UserNotice::new("schedule.apply_failed", Some(detail.clone()))),
            schedule_enabled: s.schedule.enabled,
            schedule_rules: s.schedule.rules.into_iter().map(Into::into).collect(),
            schedule_active: s.schedule_active,
            scheduling_warning: s.scheduling_warning,
            normal_download_limit: s.normal_download_limit,
            normal_upload_limit: s.normal_upload_limit,
            alternative_download_limit: s.alternative_download_limit,
            alternative_upload_limit: s.alternative_upload_limit,
            alternative_limits_enabled: s.alternative_limits_enabled,
            network_blocked: s.network_blocked,
            network_interface: s.network_interface,
            peer_encryption: s.peer_encryption,
            saved_peer_encryption: s.saved_peer_encryption,
            protocol_version: s.protocol_version,
            engine_version: s.engine_version,
            instance_id: s.instance_id,
            listen_port: s.listen_port,
            download_limit: s.download_limit,
            upload_limit: s.upload_limit,
            torrents: s.torrents.into_iter().map(Into::into).collect(),
            max_active_downloads: s.max_active_downloads,
            file_operation: s.file_operation.map(Into::into),
            storage_recovery: s.storage_recovery.map(Into::into),
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
    pub fn add_with_options(
        &self,
        bytes: Vec<u8>,
        destination: String,
        selected: Vec<u32>,
        options: TorrentAddOptions,
    ) -> Result<String, BridgeError> {
        let (core, rt) = self.handles()?;
        rt.block_on(core.import_with_options(
            bytes,
            destination,
            selected,
            seedhop_core::TorrentAddOptions {
                paused: options.paused,
                reuse: options.reuse,
                add_to_top: options.add_to_top,
                sequential: options.sequential,
                first_last_pieces: options.first_last_pieces,
            },
        ))
        .map_err(error)
    }
    pub fn set_paused(&self, id: String, paused: bool) -> Result<(), BridgeError> {
        let (core, rt) = self.handles()?;
        rt.block_on(core.set_paused(id, paused)).map_err(error)
    }
    pub fn set_force_started(&self, id: String, enabled: bool) -> Result<(), BridgeError> {
        let (core, rt) = self.handles()?;
        rt.block_on(core.set_force_started(id, enabled))
            .map_err(error)
    }
    pub fn select_files(&self, id: String, indices: Vec<u32>) -> Result<(), BridgeError> {
        let (core, rt) = self.handles()?;
        rt.block_on(core.select_files(id, indices)).map_err(error)
    }
    pub fn force_reannounce(
        &self,
        id: String,
        target_ids: Option<Vec<String>>,
        instance_id: String,
    ) -> Result<String, BridgeError> {
        let (core, rt) = self.handles()?;
        let summary = rt
            .block_on(core.force_reannounce(id, target_ids, instance_id))
            .map_err(error)?;
        serde_json::to_string(&summary).map_err(error)
    }
    pub fn force_recheck(&self, id: String) -> Result<(), BridgeError> {
        let (core, rt) = self.handles()?;
        rt.block_on(core.force_recheck(id)).map_err(error)
    }
    pub fn clear_activity(&self) -> Result<(), BridgeError> {
        let (core, rt) = self.handles()?;
        rt.block_on(core.clear_activity()).map_err(error)
    }
    pub fn statistics_preferences(&self) -> Result<String, BridgeError> {
        self.handles()?.0.statistics_preferences().map_err(error)
    }
    pub fn set_statistics_retention(&self, remember: bool) -> Result<String, BridgeError> {
        let (core, rt) = self.handles()?;
        rt.block_on(core.set_statistics_retention(remember))
            .map_err(error)
    }
    pub fn retry_transfer(&self, id: String) -> Result<(), BridgeError> {
        let (core, rt) = self.handles()?;
        rt.block_on(core.retry_transfer(id)).map_err(error)
    }
    pub fn set_piece_preferences(
        &self,
        id: String,
        sequential: Option<bool>,
        first_last_pieces: Option<bool>,
    ) -> Result<(), BridgeError> {
        let (core, rt) = self.handles()?;
        rt.block_on(core.set_piece_preferences(id, sequential, first_last_pieces))
            .map_err(error)
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
        rt.block_on(core.relocate(id, destination))
            .map_err(layout_error)
    }
    pub fn rename_file(&self, id: String, index: u32, name: String) -> Result<(), BridgeError> {
        let (core, rt) = self.handles()?;
        rt.block_on(core.rename_file(id, index, name))
            .map_err(layout_error)
    }
    pub fn cancel_file_operation(&self, operation_id: String) -> Result<bool, BridgeError> {
        let (core, _) = self.handles()?;
        core.cancel_file_operation(operation_id).map_err(error)
    }
    pub fn resolve_storage_recovery(
        &self,
        id: String,
        keep_copies: bool,
    ) -> Result<(), BridgeError> {
        let (core, rt) = self.handles()?;
        rt.block_on(core.resolve_storage_recovery(id, keep_copies))
            .map_err(error)
    }
    pub fn remove(&self, id: String, delete_files: bool) -> Result<(), BridgeError> {
        let (core, rt) = self.handles()?;
        rt.block_on(core.remove(id, delete_files)).map_err(error)
    }
    pub fn set_schedule(
        &self,
        enabled: bool,
        rules: Vec<ScheduleWindow>,
    ) -> Result<(), BridgeError> {
        let (core, rt) = self.handles()?;
        rt.block_on(core.set_schedule(seedhop_core::ScheduleConfig {
            enabled,
            rules: rules.into_iter().map(Into::into).collect(),
        }))
        .map_err(error)
    }
    pub fn set_alternative_limits(
        &self,
        download: u32,
        upload: u32,
        enabled: bool,
    ) -> Result<(), BridgeError> {
        let (core, rt) = self.handles()?;
        rt.block_on(core.set_alternative_limits(download, upload, enabled))
            .map_err(error)
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
pub fn offline_private_protection(state_directory: String) -> Result<String, BridgeError> {
    let settings = seedhop_core::offline_private_protection(std::path::Path::new(&state_directory))
        .map_err(error)?;
    serde_json::to_string(&settings).map_err(error)
}

#[uniffi::export]
pub fn protocol_version() -> u32 {
    seedhop_core::PROTOCOL_VERSION
}

#[cfg(test)]
mod tests {
    #[test]
    fn structured_errors_keep_codes_diagnostics_and_cancellation_distinct() {
        use super::*;
        let failure = error(seedhop_core::CoreError::Validation {
            code: "input.use_at_most_schedule_windows",
            message: "Use at most 32 schedule windows.".into(),
        });
        let BridgeError::Operation { notice, .. } = failure else {
            panic!("missing structured failure")
        };
        assert_eq!(notice.code, "input.use_at_most_schedule_windows");
        assert!(matches!(
            notice.arguments.as_slice(),
            [NoticeArgument::Number(32)]
        ));
        assert_eq!(
            notice.diagnostic.as_deref(),
            Some("Use at most 32 schedule windows.")
        );
        assert!(matches!(
            error(seedhop_core::CoreError::Cancelled),
            BridgeError::Cancelled
        ));
        let BridgeError::Operation { notice, .. } = error(std::io::Error::from_raw_os_error(13))
        else {
            panic!("missing OS diagnostic")
        };
        assert_eq!(notice.code, "engine.failed");
        assert!(notice.diagnostic.is_some());
    }

    #[test]
    fn message_json_preserves_unicode_and_unsigned_argument_types() {
        let notice = super::UserNotice {
            code: "future.message".into(),
            arguments: vec![
                super::NoticeArgument::Text("文件/résumé.txt".into()),
                super::NoticeArgument::Number(u64::MAX),
            ],
            diagnostic: Some("untranslated peer detail".into()),
        };
        let value = serde_json::to_value(notice).unwrap();
        assert_eq!(value["arguments"][0]["type"], "text");
        assert_eq!(value["arguments"][0]["value"], "文件/résumé.txt");
        assert_eq!(value["arguments"][1]["type"], "number");
        assert_eq!(value["arguments"][1]["value"].as_u64(), Some(u64::MAX));
    }

    #[test]
    fn recovery_guidance_uses_authority_not_diagnostic_wording() {
        let mut recovery = seedhop_core::StorageRecovery {
            id: "recovery".into(),
            torrent_id: "torrent".into(),
            active_location: "original".into(),
            message: "arbitrary OS detail".into(),
            original_directory: "/old".into(),
            destination_directory: "/new".into(),
            can_keep_copies: true,
        };
        assert_eq!(super::recovery_notice(&recovery).code, "recovery.original");
        recovery.active_location = "new".into();
        assert_eq!(super::recovery_notice(&recovery).code, "recovery.new");
        recovery.active_location = "unknown".into();
        assert_eq!(super::recovery_notice(&recovery).code, "recovery.unknown");
    }
    #[test]
    fn force_start_crosses_typed_and_json_snapshots() {
        let state = tempfile::tempdir().unwrap();
        let downloads = tempfile::tempdir().unwrap();
        let bridge = super::EngineBridge::new(
            state.path().display().to_string(),
            downloads.path().display().to_string(),
            false,
            true,
        )
        .unwrap();
        let id = bridge
            .add(
                seedhop_core::adapter::create_fixture("forced", b"content"),
                downloads.path().display().to_string(),
                vec![0],
                true,
                false,
            )
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while bridge.snapshot().unwrap().torrents[0].state == "checking" {
            assert!(
                std::time::Instant::now() < deadline,
                "initial check did not finish"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        bridge.set_force_started(id.clone(), true).unwrap();
        let transfer = &bridge.snapshot().unwrap().torrents[0];
        assert!(transfer.force_started && !transfer.paused_intent);
        assert!(
            !transfer.can_reannounce,
            "Trackerless transfer exposed reannounce"
        );
        let instance = bridge.snapshot().unwrap().instance_id;
        assert!(
            matches!(bridge.force_reannounce(id.clone(), None, "retired".into()),
            Err(super::BridgeError::Operation { notice, .. }) if notice.code == "input.reannounce_stale")
        );
        assert!(
            matches!(bridge.force_reannounce(id.clone(), Some(vec![]), instance.clone()),
            Err(super::BridgeError::Operation { notice, .. }) if notice.code == "input.reannounce_empty")
        );
        assert!(
            matches!(bridge.force_reannounce(id.clone(), None, instance),
            Err(super::BridgeError::Operation { notice, .. }) if notice.code == "input.reannounce_unavailable")
        );
        let json: serde_json::Value =
            serde_json::from_str(&bridge.snapshot_json().unwrap()).unwrap();
        assert_eq!(json["torrents"][0]["forceStarted"], true);
        assert_eq!(json["torrents"][0]["canReannounce"], false);
        assert_eq!(json["protocolVersion"], seedhop_core::PROTOCOL_VERSION);
        bridge.set_paused(id, false).unwrap();
        assert!(!bridge.snapshot().unwrap().torrents[0].force_started);
        bridge.shutdown().unwrap();
    }

    #[test]
    fn bridge_initializes_utp_and_shuts_down_from_a_small_host_stack() {
        let state = tempfile::tempdir().unwrap();
        let downloads = tempfile::tempdir().unwrap();
        std::thread::Builder::new()
            .name("small-ffi-host".into())
            .stack_size(512 * 1024)
            .spawn(move || {
                let bridge = super::EngineBridge::new(
                    state.path().to_string_lossy().into_owned(),
                    downloads.path().to_string_lossy().into_owned(),
                    false,
                    true,
                )
                .unwrap();
                let snapshot = bridge.snapshot().unwrap();
                assert!(snapshot.listen_port > 0);
                assert!(snapshot.torrents.is_empty());
                let json: serde_json::Value = serde_json::from_str(&bridge.snapshot_json().unwrap()).unwrap();
                assert_eq!(json["protocolVersion"], seedhop_core::PROTOCOL_VERSION);
                assert!(json.get("statisticsMessage").unwrap().is_null());
                assert!(json.get("schedulingMessage").unwrap().is_null());
                let invalid = match bridge.inspect("localization".into(),Some("not a magnet".into()),vec![]) { Err(error) => error, Ok(_) => panic!("invalid magnet accepted") };
                assert!(matches!(invalid, super::BridgeError::Operation { notice, .. } if notice.code == "input.paste_a_valid_magnet_link"));
                bridge.shutdown().unwrap();
            })
            .unwrap()
            .join()
            .unwrap();
    }
}

#[uniffi::export]
pub fn network_preferences(state_directory: String) -> Result<String, BridgeError> {
    seedhop_core::network_preferences(std::path::Path::new(&state_directory)).map_err(error)
}
#[uniffi::export]
pub fn statistics_preferences(state_directory: String) -> Result<String, BridgeError> {
    seedhop_core::statistics_preferences(std::path::Path::new(&state_directory)).map_err(error)
}
#[uniffi::export]
pub fn save_statistics_preferences(
    state_directory: String,
    remember: bool,
) -> Result<String, BridgeError> {
    seedhop_core::save_statistics_preferences(std::path::Path::new(&state_directory), remember)
        .map_err(error)
}
#[uniffi::export]
pub fn save_network_preferences(
    state_directory: String,
    policy: String,
) -> Result<(), BridgeError> {
    seedhop_core::save_network_preferences(std::path::Path::new(&state_directory), &policy)
        .map_err(error)
}

use crate::{CoreError, FileRecord, Result, TorrentPreview, TorrentRecord};
use librqbit::{ManagedTorrent, TorrentStatsState};
use sha1::{Digest, Sha1};
use std::collections::HashSet;

pub fn engine_error(error: impl std::fmt::Display) -> CoreError {
    CoreError::Engine(error.to_string())
}

pub fn preview(bytes: Vec<u8>) -> Result<TorrentPreview> {
    if bytes.len() > 32 * 1024 * 1024 {
        return Err(CoreError::Validation {
            code: "input.the_torrent_metadata_exceeds_mb",
            message: "The torrent metadata exceeds 32 MB.".into(),
        });
    }
    let meta =
        librqbit::torrent_from_bytes(&bytes).map_err(|e| CoreError::InvalidInput(e.to_string()))?;
    let id = meta
        .info
        .data
        .v2
        .as_ref()
        .map(|v| v.info_hash().iter().map(|b| format!("{b:02x}")).collect())
        .unwrap_or_else(|| meta.info_hash.as_string());
    let info = meta.info.data.validate().map_err(engine_error)?;
    let files: Vec<_> = info
        .iter_file_details()
        .enumerate()
        .map(|(index, f)| FileRecord {
            index: index as u32,
            name: f.filename.to_string(),
            size: f.len,
            downloaded: 0,
            selected: true,
            priority: 1,
        })
        .collect();
    if files.is_empty() {
        return Err(CoreError::Unsupported);
    }
    Ok(TorrentPreview {
        id,
        name: info
            .name()
            .map(|v| v.into_owned())
            .unwrap_or_else(|| "Untitled torrent".into()),
        total_bytes: files.iter().map(|f| f.size).sum(),
        files,
        torrent_bytes: bytes,
    })
}

pub fn record(torrent: &ManagedTorrent) -> TorrentRecord {
    let stats = torrent.stats();
    let down = stats
        .live
        .as_ref()
        .map(|s| s.download_speed.as_bytes())
        .unwrap_or(0);
    let up = stats
        .live
        .as_ref()
        .map(|s| s.upload_speed.as_bytes())
        .unwrap_or(0);
    let state = match stats.state {
        TorrentStatsState::Initializing { .. } => "checking",
        TorrentStatsState::Paused if torrent.is_queued() && !torrent.is_paused() => "queued",
        TorrentStatsState::Paused => "paused",
        TorrentStatsState::Error => "error",
        TorrentStatsState::Live if stats.finished => "seeding",
        TorrentStatsState::Live if down == 0 => "waiting",
        TorrentStatsState::Live => "downloading",
    };
    // Membership is read once per file; a vector scan makes large selections quadratic.
    let selected = torrent
        .only_files()
        .map(|indices| indices.into_iter().collect::<HashSet<_>>());
    let names = torrent.file_names();
    let priorities = torrent.file_priorities();
    let files = torrent
        .with_metadata(|meta| {
            meta.info
                .iter_file_details()
                .enumerate()
                .map(|(i, file)| FileRecord {
                    index: i as u32,
                    name: names
                        .get(&i)
                        .map(|p| p.to_string_lossy().into_owned())
                        .unwrap_or_else(|| file.filename.to_string()),
                    size: file.len,
                    downloaded: stats.file_progress.get(i).copied().unwrap_or(0),
                    selected: selected.as_ref().is_none_or(|s| s.contains(&i)),
                    priority: priorities.get(i).copied().unwrap_or(1),
                })
                .collect()
        })
        .unwrap_or_default();
    let peers = stats
        .live
        .as_ref()
        .map(|v| v.snapshot.peer_stats.live)
        .unwrap_or(0);
    let error_kind = torrent.with_state(|state| match state {
        librqbit::ManagedTorrentState::Error(error) => {
            Some(classify_error(error.as_ref()).to_owned())
        }
        _ => None,
    });
    TorrentRecord {
        id: torrent_id(torrent),
        name: torrent
            .name()
            .unwrap_or_else(|| "Resolving metadata…".into()),
        state: state.into(),
        total_bytes: stats.total_bytes,
        downloaded_bytes: stats.progress_bytes,
        uploaded_bytes: stats.uploaded_bytes,
        download_rate: down,
        upload_rate: up,
        peers,
        destination: torrent.output_folder().to_string_lossy().into_owned(),
        files,
        trackers: torrent
            .shared()
            .trackers
            .iter()
            .map(|u| u.to_string())
            .collect(),
        error: stats.error,
        error_kind,
        queue_position: 0,
        paused_intent: torrent.is_paused(),
        network_downloaded_bytes: None,
        network_uploaded_bytes: None,
        added_at: None,
        completed_at: None,
        eta_seconds: stats
            .live
            .as_ref()
            .and_then(|s| s.time_remaining.as_ref())
            .and_then(|v| serde_json::to_value(v).ok())
            .and_then(|v| v["duration"]["secs"].as_u64()),
        selected_bytes: torrent
            .with_metadata(|m| {
                m.info
                    .iter_file_details()
                    .enumerate()
                    .filter(|(i, _)| selected.as_ref().is_none_or(|s| s.contains(i)))
                    .map(|(_, f)| f.len)
                    .sum()
            })
            .unwrap_or(0),
    }
}

// Fatal torrent errors originate in initialization or file writes. Inspect typed
// causes, never localized text; network timeouts and unknown causes stay generic.
fn classify_error(mut error: &(dyn std::error::Error + 'static)) -> &'static str {
    loop {
        let io = error.downcast_ref::<std::io::Error>();
        // rqbit uses nix pwritev on Unix; its typed Errno is not an io::Error.
        let errno = io.and_then(std::io::Error::raw_os_error).or_else(|| {
            error
                .downcast_ref::<nix::errno::Errno>()
                .map(|errno| *errno as i32)
        });
        match errno {
            Some(libc::ENOSPC | libc::EDQUOT) => return "storage_space",
            Some(libc::EROFS | libc::EACCES | libc::EPERM) => return "storage_permission",
            Some(libc::ENOENT | libc::ENOTDIR | libc::ENODEV | libc::ESTALE) => {
                return "storage_missing";
            }
            Some(libc::EIO | libc::EFBIG) => return "storage",
            _ => {}
        }
        if let Some(io) = io {
            match io.kind() {
                std::io::ErrorKind::PermissionDenied => return "storage_permission",
                std::io::ErrorKind::NotFound => return "storage_missing",
                std::io::ErrorKind::WriteZero => return "storage",
                _ => {}
            }
        }
        let Some(source) = error.source() else {
            return "transfer";
        };
        error = source;
    }
}

/// Small deterministic fixture creator; independent clients can verify the output.
pub fn create_fixture(name: &str, content: &[u8]) -> Vec<u8> {
    let mut bytes = format!(
        "d4:infod6:lengthi{}e4:name{}:{}12:piece lengthi16384e6:pieces{}:",
        content.len(),
        name.len(),
        name,
        content.chunks(16384).count() * 20
    )
    .into_bytes();
    for piece in content.chunks(16384) {
        bytes.extend_from_slice(&Sha1::digest(piece));
    }
    bytes.extend_from_slice(b"ee");
    bytes
}

/// Application identity retains all 256 bits; the engine's 20-byte key is wire-only.
pub fn torrent_id(torrent: &ManagedTorrent) -> String {
    torrent
        .with_metadata(|meta| {
            meta.info.info().v2.as_ref().map(|v| {
                v.info_hash()
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect::<String>()
            })
        })
        .ok()
        .flatten()
        .unwrap_or_else(|| torrent.info_hash().as_string())
}

#[cfg(test)]
mod status_tests {
    use super::*;
    #[test]
    fn storage_causes_survive_context_and_network_errors_stay_generic() {
        for (errno, expected) in [
            (libc::ENOSPC, "storage_space"),
            (libc::EDQUOT, "storage_space"),
            (libc::EACCES, "storage_permission"),
            (libc::EROFS, "storage_permission"),
            (libc::ENOENT, "storage_missing"),
            (libc::EIO, "storage"),
        ] {
            let error = anyhow::Error::new(std::io::Error::from_raw_os_error(errno))
                .context("file operation failed");
            assert_eq!(classify_error(error.as_ref()), expected);
            let unix = anyhow::Error::new(nix::errno::Errno::from_raw(errno))
                .context("error calling pwritev")
                .context("error writing chunk");
            assert_eq!(classify_error(unix.as_ref()), expected);
        }
        assert_eq!(
            classify_error(&std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "network timeout"
            )),
            "transfer"
        );
        assert_eq!(
            classify_error(&std::io::Error::other(
                "disk full words without a typed cause"
            )),
            "transfer"
        );
    }
}

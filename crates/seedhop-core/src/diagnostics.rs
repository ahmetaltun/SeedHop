//! Engine-owned accounting and read-only, selection-scoped diagnostics.
use crate::{Engine, Result, adapter, storage};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    path::Path,
    sync::atomic::Ordering,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[derive(Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Totals {
    pub downloaded: Option<u64>,
    pub uploaded: Option<u64>,
    pub added: Option<u64>,
    pub completed: Option<u64>,
    pub seeding_seconds: Option<u64>,
    #[serde(skip)]
    session_downloaded: u64,
    #[serde(skip)]
    session_uploaded: u64,
    #[serde(skip)]
    baseline: (u64, u64),
    #[serde(skip)]
    last: Option<Instant>,
    #[serde(skip)]
    seeding: bool,
    #[serde(skip)]
    seeding_fraction: f64,
}
#[derive(Default)]
pub(crate) struct Accounting {
    pub entries: HashMap<String, Totals>,
    last_checkpoint: Option<Instant>,
}
impl Accounting {
    pub fn load(dir: &Path) -> Result<Self> {
        let path = dir.join("statistics.json");
        let entries = if path.exists() {
            serde_json::from_slice(&std::fs::read(path).map_err(storage::io_error)?)
                .map_err(|e| crate::CoreError::CorruptState(format!("Statistics: {e}")))?
        } else {
            HashMap::new()
        };
        Ok(Self {
            entries,
            last_checkpoint: None,
        })
    }
    pub fn new_transfer(&mut self, id: String) {
        self.entries.insert(
            id,
            Totals {
                downloaded: Some(0),
                uploaded: Some(0),
                added: Some(now()),
                seeding_seconds: Some(0),
                ..Default::default()
            },
        );
    }
    pub fn checkpoint(&mut self, dir: &Path, force: bool) -> Result<()> {
        if !force
            && self
                .last_checkpoint
                .is_some_and(|t| t.elapsed().as_secs() < 30)
        {
            return Ok(());
        }
        let result = storage::atomic_write(
            &dir.join("statistics.json"),
            &serde_json::to_vec(&self.entries).map_err(adapter::engine_error)?,
        );
        // A failed forced save also invalidates the normal checkpoint timer.
        // Maintenance must retry instead of clearing the warning on a skipped save.
        self.last_checkpoint = result.as_ref().ok().map(|_| Instant::now());
        result
    }
    pub fn sample(&mut self, id: String, down: u64, up: u64, seeding: bool, complete: bool) {
        let t = self.entries.entry(id).or_default();
        let delta = (
            down.saturating_sub(t.baseline.0),
            up.saturating_sub(t.baseline.1),
        );
        // Shared counters are monotonic for the lifetime of this engine. A stale
        // observation must not lower the baseline and count the same bytes twice.
        t.baseline = (down.max(t.baseline.0), up.max(t.baseline.1));
        t.session_downloaded = t.session_downloaded.saturating_add(delta.0);
        t.session_uploaded = t.session_uploaded.saturating_add(delta.1);
        if let Some(total) = t.downloaded.as_mut() {
            *total = total.saturating_add(delta.0);
        }
        if let Some(total) = t.uploaded.as_mut() {
            *total = total.saturating_add(delta.1);
        }
        let instant = Instant::now();
        if t.seeding
            && let Some(last) = t.last
        {
            t.seeding_fraction += instant.duration_since(last).as_secs_f64();
            let whole = t.seeding_fraction as u64;
            t.seeding_fraction -= whole as f64;
            if let Some(seconds) = t.seeding_seconds.as_mut() {
                *seconds = seconds.saturating_add(whole);
            }
        }
        t.last = Some(instant);
        t.seeding = seeding;
        if complete && t.added.is_some() && t.completed.is_none() {
            t.completed = Some(now());
        }
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PeerDetails {
    pub id: String,
    pub endpoint: String,
    pub client: Option<String>,
    pub state: String,
    pub transport: Option<String>,
    pub encrypted: Option<bool>,
    pub incoming: Option<bool>,
    pub downloaded: u64,
    pub uploaded: u64,
    pub interested: Option<bool>,
    pub available_pieces: Option<usize>,
    pub pending_requests: Option<usize>,
    pub errors: u32,
    pub pieces_stolen: u32,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackerDetails {
    pub id: String,
    pub address: String,
    pub endpoint: Option<String>,
    pub hash: String,
    pub state: String,
    pub message: Option<String>,
    pub last_announce: Option<u64>,
    pub next_announce: Option<u64>,
    pub peers: Option<u64>,
    pub seeds: Option<u64>,
    pub leechers: Option<u64>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticEvent {
    pub sequence: u64,
    pub timestamp: u64,
    pub message: String,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TorrentDetails {
    pub torrent_id: String,
    pub instance_id: String,
    pub observed_at: u64,
    pub section: String,
    pub format: String,
    pub private: bool,
    pub v1_hash: Option<String>,
    pub v2_hash: Option<String>,
    pub dht: bool,
    pub pex: bool,
    pub local_discovery: bool,
    pub session_downloaded: u64,
    pub session_uploaded: u64,
    pub seeding_seconds: Option<u64>,
    pub peers: Vec<PeerDetails>,
    pub trackers: Vec<TrackerDetails>,
    pub events: Vec<DiagnosticEvent>,
}

impl Engine {
    pub(crate) fn sample_accounting(&self) {
        // Serialize collection and application so concurrent readers cannot
        // commit an older counter sample after a newer one.
        let mut accounting = self.accounting.lock().unwrap();
        let samples = self.session.with_torrents(|torrents| {
            torrents
                .map(|(_, torrent)| {
                    let stats = torrent.stats();
                    (
                        adapter::torrent_id(torrent),
                        torrent
                            .shared()
                            .payload_counters
                            .downloaded
                            .load(Ordering::Relaxed),
                        torrent
                            .shared()
                            .payload_counters
                            .uploaded
                            .load(Ordering::Relaxed),
                        matches!(stats.state, librqbit::TorrentStatsState::Live) && stats.finished,
                        stats.finished,
                    )
                })
                .collect::<Vec<_>>()
        });
        for (id, down, up, seeding, complete) in samples {
            accounting.sample(id, down, up, seeding, complete);
        }
    }
    pub fn torrent_details(
        &self,
        id: String,
        section: String,
        all_peers: bool,
    ) -> Result<TorrentDetails> {
        self.ready()?;
        if !["general", "files", "peers", "trackers", "activity"].contains(&section.as_str()) {
            return Err(crate::CoreError::InvalidInput(
                "Unknown details section".into(),
            ));
        }
        let torrent = self.handle(&id)?;
        self.sample_accounting();
        let (format, private, v1_hash, v2_hash) = torrent
            .with_metadata(|m| {
                let info = m.info.info();
                let v2 = info.v2.as_ref();
                (
                    if v2.is_some_and(|v| v.is_hybrid()) {
                        "hybrid"
                    } else if v2.is_some() {
                        "v2"
                    } else {
                        "v1"
                    }
                    .to_owned(),
                    info.private,
                    if v2.is_none() || v2.is_some_and(|v| v.is_hybrid()) {
                        Some(m.v1_info_hash.as_string())
                    } else {
                        None
                    },
                    v2.map(|v| {
                        v.info_hash()
                            .iter()
                            .map(|b| format!("{b:02x}"))
                            .collect::<String>()
                    }),
                )
            })
            .map_err(adapter::engine_error)?;
        let peers = if section == "peers" {
            torrent
                .live()
                .map(|live| {
                    live.per_peer_stats_snapshot(librqbit::PeerStatsFilter {
                        state: if all_peers {
                            librqbit::PeerStatsFilterState::All
                        } else {
                            Default::default()
                        },
                    })
                    .peers
                    .into_iter()
                    .map(|(endpoint, p)| PeerDetails {
                        id: format!(
                            "{}:{}",
                            endpoint,
                            p.connection_id
                                .map(|v| v.to_string())
                                .unwrap_or_else(|| "attempt".into())
                        ),
                        endpoint,
                        client: p.client_name,
                        state: p.state.into(),
                        transport: p.conn_kind.map(|k| k.to_string()),
                        encrypted: p.encrypted,
                        incoming: p.incoming,
                        downloaded: p.counters.fetched_bytes,
                        uploaded: p.counters.uploaded_bytes,
                        interested: p.interested,
                        available_pieces: p.available_pieces,
                        pending_requests: p.pending_requests,
                        errors: p.counters.errors,
                        pieces_stolen: p.counters.times_stolen_from_me,
                    })
                    .collect()
                })
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let trackers = if section == "trackers" || section == "activity" {
            let observations = torrent.shared().tracker_observations.read();
            let mut records: Vec<_> = observations
                .iter()
                .map(|(key, (hash, o))| TrackerDetails {
                    id: tracker_identity(key),
                    address: redact_tracker(&o.url),
                    endpoint: o.endpoint.clone(),
                    hash: hash.clone(),
                    state: o.state.clone(),
                    message: o.message.as_ref().map(|m| sanitize_message(m, &o.url)),
                    last_announce: o.last_announce,
                    next_announce: o.next_announce,
                    peers: o.peers,
                    seeds: o.seeds,
                    leechers: o.leechers,
                })
                .collect();
            for url in &torrent.shared().trackers {
                if !observations.values().any(|(_, o)| o.url == url.as_str()) {
                    records.push(TrackerDetails {
                        id: tracker_identity(&format!("uncontacted:{}", url.as_str())),
                        address: redact_tracker(url.as_str()),
                        endpoint: None,
                        hash: torrent.info_hash().as_string(),
                        state: "not contacted".into(),
                        message: None,
                        last_announce: None,
                        next_announce: None,
                        peers: None,
                        seeds: None,
                        leechers: None,
                    });
                }
            }
            records.sort_by(|a, b| a.address.cmp(&b.address).then(a.id.cmp(&b.id)));
            records
        } else {
            Vec::new()
        };
        let accounting = self.accounting.lock().unwrap();
        let t = accounting.entries.get(&id).ok_or_else(|| {
            crate::CoreError::InvalidInput("This torrent is no longer in the library.".into())
        })?;
        let events = if section == "activity" {
            torrent
                .shared()
                .tracker_events
                .lock()
                .iter()
                .map(|(sequence, timestamp, url, state)| DiagnosticEvent {
                    sequence: *sequence,
                    timestamp: *timestamp,
                    message: format!("Tracker {}: {}", redact_tracker(url), state),
                })
                .collect()
        } else {
            Vec::new()
        };
        Ok(TorrentDetails {
            torrent_id: id,
            instance_id: self.instance_id.clone(),
            observed_at: now(),
            section,
            format,
            private,
            v1_hash,
            v2_hash,
            dht: !private && self.session.get_dht().is_some(),
            pex: !private,
            local_discovery: false,
            session_downloaded: t.session_downloaded,
            session_uploaded: t.session_uploaded,
            seeding_seconds: t.seeding_seconds,
            peers,
            trackers,
            events,
        })
    }
}

pub fn redact_tracker(raw: &str) -> String {
    url::Url::parse(raw)
        .ok()
        .and_then(|u| {
            Some(format!(
                "{}://{}{}",
                u.scheme(),
                u.host()?,
                u.port().map(|p| format!(":{p}")).unwrap_or_default()
            ))
        })
        .unwrap_or_else(|| "Tracker".into())
}
fn tracker_identity(raw: &str) -> String {
    format!("{:x}", Sha256::digest(raw.as_bytes()))
}
fn sanitize_message(message: &str, raw: &str) -> String {
    // Remote tracker strings are untrusted; never forward credentials, paths or arbitrary URLs.
    let lower = message.to_ascii_lowercase();
    let exposes_secret = url::Url::parse(raw).ok().is_some_and(|url| {
        let mut fragments = vec![
            url.username().to_owned(),
            url.password().unwrap_or_default().to_owned(),
        ];
        if let Some(segments) = url.path_segments() {
            fragments.extend(segments.map(str::to_owned));
        }
        fragments.extend(url.query_pairs().map(|(_, value)| value.into_owned()));
        fragments.iter().any(|fragment| {
            (!fragment.is_empty() && lower.contains(&fragment.to_ascii_lowercase()))
                || url::form_urlencoded::parse(fragment.as_bytes()).any(|(key, value)| {
                    [key, value]
                        .iter()
                        .any(|part| !part.is_empty() && lower.contains(&part.to_ascii_lowercase()))
                })
        })
    });
    if lower.contains("http")
        || lower.contains("udp:")
        || message.contains('/')
        || message.contains('?')
        || message.contains('@')
        || exposes_secret
    {
        "Tracker returned a message (private details omitted)".into()
    } else {
        message
            .chars()
            .filter(|c| !c.is_control())
            .take(240)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn payload_counts_are_not_content_progress_and_do_not_double_count() {
        let mut a = Accounting::default();
        a.new_transfer("new".into());
        a.sample("new".into(), 0, 0, false, true);
        assert_eq!(a.entries["new"].downloaded, Some(0));
        a.sample("new".into(), 100, 50, false, false);
        a.sample("new".into(), 100, 50, false, false);
        assert_eq!(a.entries["new"].downloaded, Some(100));
        a.sample("old".into(), 100, 50, false, true);
        assert_eq!(a.entries["old"].downloaded, None);
        assert_eq!(a.entries["old"].completed, None);
    }
    #[test]
    fn checkpoint_restart_keeps_totals_and_resets_session_baselines() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = Accounting::default();
        a.new_transfer("x".into());
        a.sample("x".into(), 100, 40, false, false);
        a.checkpoint(dir.path(), true).unwrap();
        let mut b = Accounting::load(dir.path()).unwrap();
        b.sample("x".into(), 20, 5, false, false);
        assert_eq!(b.entries["x"].downloaded, Some(120));
        assert_eq!(b.entries["x"].session_downloaded, 20);
    }
    #[test]
    fn failed_checkpoint_preserves_memory_and_retries() {
        let dir = tempfile::tempdir().unwrap();
        let mut accounting = Accounting::default();
        accounting.new_transfer("x".into());
        accounting.sample("x".into(), 100, 25, false, false);
        accounting.checkpoint(dir.path(), true).unwrap();
        std::fs::create_dir(dir.path().join("statistics.pending")).unwrap();
        assert!(accounting.checkpoint(dir.path(), true).is_err());
        assert!(accounting.last_checkpoint.is_none());
        assert_eq!(accounting.entries["x"].downloaded, Some(100));
        std::fs::remove_dir(dir.path().join("statistics.pending")).unwrap();
        accounting.checkpoint(dir.path(), false).unwrap();
        assert_eq!(
            Accounting::load(dir.path()).unwrap().entries["x"].uploaded,
            Some(25)
        );
    }
    #[test]
    fn tracker_credentials_are_not_presented() {
        assert_eq!(
            redact_tracker("https://user:secret@example.com/passkey/announce?token=secret"),
            "https://example.com"
        );
        assert!(
            !sanitize_message("failed https://x/secret", "https://x/secret").contains("secret")
        );
    }
    #[test]
    fn stale_counter_samples_never_lower_baselines() {
        let mut a = Accounting::default();
        a.new_transfer("x".into());
        for (down, up) in [(100, 50), (200, 100), (100, 50), (200, 100)] {
            a.sample("x".into(), down, up, false, false);
        }
        assert_eq!(a.entries["x"].downloaded, Some(200));
        assert_eq!(a.entries["x"].uploaded, Some(100));
        assert_eq!(a.entries["x"].session_downloaded, 200);
    }
    #[test]
    fn tracker_messages_redact_standalone_credential_fragments() {
        let raw = "https://user:password@tracker.example/pathpass/announce?token=querypass";
        for secret in ["user", "password", "pathpass", "querypass"] {
            let message = format!("Invalid credential {secret}");
            let safe = sanitize_message(&message, raw);
            assert!(!safe.contains(secret), "exposed {secret}");
        }
        assert!(
            !sanitize_message(
                "Rejected decoded secret",
                "https://tracker.example/decoded%20secret/announce"
            )
            .contains("decoded secret")
        );
        assert_eq!(
            sanitize_message("Temporarily unavailable", raw),
            "Temporarily unavailable"
        );
    }
    #[test]
    fn tracker_identity_is_opaque_and_distinguishes_same_host_urls() {
        let first = "https://tracker.example/secret-one/announce";
        let second = "https://tracker.example/secret-two/announce";
        assert_eq!(redact_tracker(first), redact_tracker(second));
        assert_ne!(tracker_identity(first), tracker_identity(second));
        assert_eq!(tracker_identity(first), tracker_identity(first));
        assert_eq!(tracker_identity(first).len(), 64);
        assert!(!tracker_identity(first).contains("secret"));
        assert!(!tracker_identity(&format!("hash|{first}|127.0.0.1:80")).contains("127.0.0.1"));
    }
}

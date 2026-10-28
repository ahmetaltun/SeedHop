use std::{collections::HashMap, sync::atomic::Ordering};

use serde::{Deserialize, Serialize};

use crate::{
    stream_connect::ConnectionKind,
    torrent_state::live::peer::{Peer, PeerState},
};

#[derive(Serialize, Deserialize)]
pub struct PeerCounters {
    pub incoming_connections: u32,
    pub fetched_bytes: u64,
    pub uploaded_bytes: u64,
    pub total_time_connecting_ms: u64,
    pub connection_attempts: u32,
    pub connections: u32,
    pub errors: u32,
    pub fetched_chunks: u32,
    pub downloaded_and_checked_pieces: u32,
    pub total_piece_download_ms: u64,
    pub times_stolen_from_me: u32,
    pub times_i_stole: u32,
}

#[derive(Serialize)]
pub struct PeerStats {
    pub counters: PeerCounters,
    pub state: &'static str,
    pub conn_kind: Option<ConnectionKind>,
    pub client_name: Option<String>,
    pub connection_id: Option<u64>,
    pub encrypted: Option<bool>,
    pub incoming: Option<bool>,
}

impl From<&super::atomic::PeerCountersAtomic> for PeerCounters {
    fn from(counters: &super::atomic::PeerCountersAtomic) -> Self {
        Self {
            incoming_connections: counters.incoming_connections.load(Ordering::Relaxed),
            fetched_bytes: counters.fetched_bytes.load(Ordering::Relaxed),
            uploaded_bytes: counters.uploaded_bytes.load(Ordering::Relaxed),
            total_time_connecting_ms: counters.total_time_connecting_ms.load(Ordering::Relaxed),
            connection_attempts: counters
                .outgoing_connection_attempts
                .load(Ordering::Relaxed),
            connections: counters.outgoing_connections.load(Ordering::Relaxed),
            errors: counters.errors.load(Ordering::Relaxed),
            fetched_chunks: counters.fetched_chunks.load(Ordering::Relaxed),
            downloaded_and_checked_pieces: counters
                .downloaded_and_checked_pieces
                .load(Ordering::Relaxed),
            total_piece_download_ms: counters.total_piece_download_ms.load(Ordering::Relaxed),
            times_i_stole: counters.times_i_stole.load(Ordering::Relaxed),
            times_stolen_from_me: counters.times_stolen_from_me.load(Ordering::Relaxed),
        }
    }
}

impl From<&Peer> for PeerStats {
    fn from(peer: &Peer) -> Self {
        let state = peer.get_state();
        Self {
            counters: peer.stats.counters.as_ref().into(),
            state: state.name(),
            conn_kind: match state {
                PeerState::Live(l) => Some(l.connection_kind),
                _ => None,
            },
            client_name: match state {
                PeerState::Live(l) => l.client_name.clone(),
                _ => None,
            },
            connection_id: match state {
                PeerState::Live(l) => Some(l.connection_id),
                _ => None,
            },
            encrypted: match state {
                PeerState::Live(l) => l.encrypted,
                _ => None,
            },
            incoming: match state {
                PeerState::Live(l) => Some(l.incoming),
                _ => None,
            },
        }
    }
}

#[derive(Serialize)]
pub struct PeerStatsSnapshot {
    pub peers: HashMap<String, PeerStats>,
}

#[derive(Clone, Copy, Default, Deserialize)]
pub enum PeerStatsFilterState {
    #[serde(rename = "all")]
    All,
    #[default]
    #[serde(rename = "live")]
    Live,
}

impl PeerStatsFilterState {
    pub(crate) fn matches(&self, s: &PeerState) -> bool {
        matches!((self, s), (Self::All, _) | (Self::Live, PeerState::Live(_)))
    }
}

#[derive(Default, Deserialize)]
pub struct PeerStatsFilter {
    #[serde(default)]
    pub state: PeerStatsFilterState,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::torrent_state::live::peer::LivePeerState;
    use librqbit_core::hash_id::Id20;

    #[test]
    fn connection_identity_and_encryption_follow_live_generation() {
        let address = "127.0.0.1:1234".parse().unwrap();
        let mut peer = Peer::new_with_outgoing_address(address, Id20::new([0; 20]));
        let unknown = PeerStats::from(&peer);
        assert!(unknown.connection_id.is_none());
        assert!(unknown.encrypted.is_none());
        assert!(unknown.incoming.is_none());

        let (tx, _) = tokio::sync::mpsc::unbounded_channel();
        let mut first = LivePeerState::new(Id20::new([1; 20]), tx, true, ConnectionKind::Tcp, true);
        first.encrypted = Some(false);
        peer.state = PeerState::Live(first);
        let first = PeerStats::from(&peer);
        assert_eq!(first.encrypted, Some(false));
        assert_eq!(first.incoming, Some(true));

        let (tx, _) = tokio::sync::mpsc::unbounded_channel();
        let mut next =
            LivePeerState::new(Id20::new([1; 20]), tx, false, ConnectionKind::Utp, false);
        next.encrypted = Some(true);
        peer.state = PeerState::Live(next);
        let next = PeerStats::from(&peer);
        assert_ne!(first.connection_id, next.connection_id);
        assert_eq!(next.encrypted, Some(true));
        assert_eq!(next.incoming, Some(false));

        peer.state = PeerState::Dead;
        let dead = PeerStats::from(&peer);
        assert!(dead.connection_id.is_none());
        assert!(dead.encrypted.is_none());
    }
}

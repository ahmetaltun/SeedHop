mod cancelled_requests;
pub mod stats;

use cancelled_requests::CancelledRequests;
use std::collections::{HashMap, hash_map::Entry};
use std::net::SocketAddr;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use librqbit_core::hash_id::Id20;
use librqbit_core::lengths::{ChunkInfo, ValidPieceIndex};
use peer_binary_protocol::{Message, Request};

use tokio::sync::{
    Notify,
    mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel},
};
use tracing::debug;

use crate::peer_connection::WriterRequest;
use crate::stream_connect::ConnectionKind;
use crate::type_aliases::BF;

use super::PeerStates;

pub(crate) type InflightRequest = ChunkInfo;
pub(crate) type PeerRx = UnboundedReceiver<WriterRequest>;
pub(crate) type PeerTx = UnboundedSender<WriterRequest>;

static NEXT_CONNECTION_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Debug)]
pub(crate) struct Peer {
    pub addr: SocketAddr,
    state: PeerState,
    pub stats: stats::atomic::PeerStats,
    pub outgoing_address: Option<SocketAddr>,
    pub outgoing_info_hash: Option<Id20>,
    pub tried_alternate_hash: bool,
}

impl Peer {
    pub fn new_live_for_incoming_connection(
        addr: SocketAddr,
        peer_id: Id20,
        tx: PeerTx,
        counters: &PeerStates,
        connection_kind: ConnectionKind,
    ) -> Self {
        let state = PeerState::Live(LivePeerState::new(peer_id, tx, true, connection_kind, true));
        for counter in [&counters.session_stats, &counters.stats] {
            counter.inc(&state);
        }
        Self {
            addr,
            state,
            stats: Default::default(),
            outgoing_address: None,
            outgoing_info_hash: None,
            tried_alternate_hash: false,
        }
    }

    pub fn new_with_outgoing_address(addr: SocketAddr, info_hash: Id20) -> Self {
        Self {
            addr,
            outgoing_address: Some(addr),
            outgoing_info_hash: Some(info_hash),
            tried_alternate_hash: false,
            stats: Default::default(),
            state: Default::default(),
        }
    }

    pub(crate) fn reconnect_not_needed_peer(
        &mut self,
        counters: &PeerStates,
    ) -> Option<SocketAddr> {
        if let PeerState::NotNeeded = self.get_state() {
            match self.outgoing_address {
                None => None,
                Some(socket_addr) if self.addr == socket_addr => {
                    self.set_state(PeerState::Queued, counters);
                    Some(socket_addr)
                }
                Some(socket_addr) => {
                    debug!(
                        peer = %self.addr,
                        outgoing_addr = %socket_addr,
                        "peer will by retried on different address",
                    );
                    Some(socket_addr)
                }
            }
        } else {
            None
        }
    }
}

#[derive(Debug, Default)]
pub(crate) enum PeerState {
    #[default]
    // Will be tried to be connected as soon as possible.
    Queued,
    Connecting(PeerTx),
    Live(LivePeerState),
    // There was an error, and it's waiting for exponential backoff.
    Dead,
    // We don't need to do anything with the peer any longer.
    // The peer has the full torrent, and we have the full torrent, so no need
    // to keep talking to it.
    NotNeeded,
}

impl std::fmt::Display for PeerState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name())
    }
}

impl PeerState {
    pub fn name(&self) -> &'static str {
        match self {
            PeerState::Queued => "queued",
            PeerState::Connecting(_) => "connecting",
            PeerState::Live(_) => "live",
            PeerState::Dead => "dead",
            PeerState::NotNeeded => "not needed",
        }
    }

    pub fn take_live_no_counters(self) -> Option<LivePeerState> {
        match self {
            PeerState::Live(l) => Some(l),
            _ => None,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum IncomingConnectionResult {
    #[error("peer already active")]
    AlreadyActive,
}

impl Peer {
    pub fn get_state(&self) -> &PeerState {
        &self.state
    }

    pub fn take_state(&mut self, counters: &PeerStates) -> PeerState {
        self.set_state(Default::default(), counters)
    }

    pub fn destroy(self, counters: &PeerStates) {
        for counter in [&counters.session_stats, &counters.stats] {
            counter.dec(&self.state);
        }
        if let (Some(addr), PeerState::Live(..)) = (self.outgoing_address, &self.state) {
            counters.live_outgoing_peers.write().remove(&addr);
        }
    }

    pub fn set_state(&mut self, new: PeerState, counters: &PeerStates) -> PeerState {
        for counter in [&counters.session_stats, &counters.stats] {
            counter.incdec(&self.state, &new);
        }
        if let Some(addr) = self.outgoing_address {
            if matches!(&self.state, PeerState::Live(..)) {
                counters.live_outgoing_peers.write().remove(&addr);
            }
            if matches!(&new, PeerState::Live(..))
                && self
                    .stats
                    .counters
                    .outgoing_connections
                    .load(Ordering::Relaxed)
                    > 0
            {
                counters.live_outgoing_peers.write().insert(addr);
            }
        }

        std::mem::replace(&mut self.state, new)
    }

    pub fn get_live(&self) -> Option<&LivePeerState> {
        match &self.state {
            PeerState::Live(l) => Some(l),
            _ => None,
        }
    }

    pub fn get_live_mut(&mut self) -> Option<&mut LivePeerState> {
        match &mut self.state {
            PeerState::Live(l) => Some(l),
            _ => None,
        }
    }

    pub fn idle_to_connecting(&mut self, counters: &PeerStates) -> Option<(PeerRx, PeerTx)> {
        match &self.state {
            PeerState::Queued | PeerState::NotNeeded => {
                let (tx, rx) = unbounded_channel();
                let tx_2 = tx.clone();
                self.set_state(PeerState::Connecting(tx), counters);
                Some((rx, tx_2))
            }
            _ => None,
        }
    }

    pub fn incoming_connection(
        &mut self,
        peer_id: Id20,
        tx: PeerTx,
        counters: &PeerStates,
        connection_kind: ConnectionKind,
    ) -> Result<(), IncomingConnectionResult> {
        if matches!(&self.state, PeerState::Connecting(..) | PeerState::Live(..)) {
            return Err(IncomingConnectionResult::AlreadyActive);
        }
        match self.take_state(counters) {
            PeerState::Queued | PeerState::Dead | PeerState::NotNeeded => {
                self.set_state(
                    PeerState::Live(LivePeerState::new(peer_id, tx, true, connection_kind, true)),
                    counters,
                );
            }
            PeerState::Connecting(..) | PeerState::Live(..) => unreachable!(),
        }
        Ok(())
    }

    pub fn connecting_to_live(
        &mut self,
        peer_id: Id20,
        counters: &PeerStates,
        conn_kind: ConnectionKind,
    ) -> Option<&mut LivePeerState> {
        if let PeerState::Connecting(_) = &self.state {
            let tx = match self.take_state(counters) {
                PeerState::Connecting(tx) => tx,
                _ => unreachable!(),
            };
            self.set_state(
                PeerState::Live(LivePeerState::new(peer_id, tx, false, conn_kind, false)),
                counters,
            );
            self.get_live_mut()
        } else {
            None
        }
    }

    pub fn set_not_needed(&mut self, counters: &PeerStates) -> PeerState {
        self.set_state(PeerState::NotNeeded, counters)
    }
}

#[derive(Debug)]
pub(crate) struct LivePeerState {
    pub connection_id: u64,
    pub encrypted: Option<bool>,
    pub incoming: bool,
    #[allow(dead_code)]
    peer_id: Id20,

    pub client_name: Option<String>,

    pub peer_interested: bool,

    // This is used to track the pieces the peer has.
    pub bitfield: BF,

    // When the peer sends us data this is used to track if we asked for it.
    inflight_requests: HashMap<InflightRequest, u32>,

    // Cancel does not remove the remote's obligation to answer. Track exact
    // requests (including repeated cancels), never a tolerance for arbitrary data.
    late_cancelled_requests: CancelledRequests<InflightRequest>,

    request_slots_changed: Arc<Notify>,

    // The main channel to send requests to peer.
    pub tx: PeerTx,

    pub connection_kind: ConnectionKind,
}

impl LivePeerState {
    pub fn new(
        peer_id: Id20,
        tx: PeerTx,
        initial_interested: bool,
        connection_kind: ConnectionKind,
        incoming: bool,
    ) -> Self {
        LivePeerState {
            connection_id: NEXT_CONNECTION_ID.fetch_add(1, Ordering::Relaxed),
            encrypted: None,
            incoming,
            peer_id,
            client_name: None,
            peer_interested: initial_interested,
            bitfield: BF::default(),
            inflight_requests: Default::default(),
            late_cancelled_requests: Default::default(),
            request_slots_changed: Default::default(),
            tx,
            connection_kind,
        }
    }

    pub fn has_full_torrent(&self, total_pieces: usize) -> bool {
        self.bitfield.get(0..total_pieces).is_some_and(|s| s.all())
    }

    pub fn request_slots_changed(&self) -> Arc<Notify> {
        self.request_slots_changed.clone()
    }

    pub fn requested_inflight_count(&self) -> usize {
        self.inflight_requests.len()
    }

    pub fn add_inflight_request(&mut self, chunk: ChunkInfo, wire_length: u32) -> bool {
        match self.inflight_requests.entry(chunk) {
            Entry::Occupied(_) => false,
            Entry::Vacant(entry) => {
                entry.insert(wire_length);
                true
            }
        }
    }

    pub fn remove_inflight_request(&mut self, chunk: &ChunkInfo) -> RemoveInflightRequestResult {
        if self.inflight_requests.remove(chunk).is_some() {
            self.request_slots_changed.notify_waiters();
            return RemoveInflightRequestResult::Expected;
        }

        if self.late_cancelled_requests.remove(chunk) {
            RemoveInflightRequestResult::LateCanceled
        } else {
            RemoveInflightRequestResult::Unexpected
        }
    }

    pub fn cancel_inflight_requests_for_piece(&mut self, piece: ValidPieceIndex) {
        let tx = &self.tx;
        let cancelled = &mut self.late_cancelled_requests;
        let before = self.inflight_requests.len();
        self.inflight_requests.retain(|req, wire_length| {
            if req.piece_index == piece {
                let _ = tx.send(WriterRequest::Message(Message::Cancel(Request {
                    index: piece.get(),
                    begin: req.offset,
                    length: *wire_length,
                })));
                if !cancelled.insert(*req) {
                    // An unresponsive peer must not grow this ledger forever.
                    let _ = tx.send(WriterRequest::Disconnect(Err(anyhow::anyhow!(
                        "too many unanswered cancelled requests"
                    ))));
                }
                false
            } else {
                true
            }
        });

        if self.inflight_requests.len() != before {
            self.request_slots_changed.notify_waiters();
        }
    }

    pub fn inflight_requests(&self) -> impl Iterator<Item = &InflightRequest> {
        self.inflight_requests.keys()
    }

    pub fn inflight_requests_debug(&self) -> &HashMap<InflightRequest, u32> {
        &self.inflight_requests
    }
}

pub(crate) enum RemoveInflightRequestResult {
    Expected,
    LateCanceled,
    Unexpected,
}

use std::{collections::HashSet, net::SocketAddr, sync::Arc};

use dashmap::DashMap;
use librqbit_core::lengths::ValidPieceIndex;
use parking_lot::RwLock;

use crate::{
    Error,
    torrent_state::utils::{TimedExistence, atomic_inc},
    type_aliases::{BF, PeerHandle},
};

use self::stats::{AggregatePeerStats, AggregatePeerStatsAtomic};

use super::peer::{LivePeerState, Peer, PeerRx, PeerState, PeerTx};

pub mod stats;

pub(crate) struct PeerStates {
    pub session_stats: Arc<AggregatePeerStatsAtomic>,

    // This keeps track of live addresses we connected to, for PEX.
    pub live_outgoing_peers: RwLock<HashSet<PeerHandle>>,
    pub stats: AggregatePeerStatsAtomic,
    pub states: DashMap<PeerHandle, Peer>,
    pub piece_availability: RwLock<Vec<u32>>,
}

impl Drop for PeerStates {
    fn drop(&mut self) {
        for (_, p) in std::mem::take(&mut self.states).into_iter() {
            p.destroy(self);
        }
    }
}

impl PeerStates {
    // Call with the peer locked. Never acquire another peer lock while holding
    // availability: piece acquisition already holds its requesting peer lock.
    pub fn replace_availability(&self, old: Option<&BF>, new: Option<&BF>) {
        let mut counts = self.piece_availability.write();
        let len = counts.len();
        if let Some(old) = old {
            for index in old.iter_ones().take_while(|i| *i < len) {
                counts[index] = counts[index].saturating_sub(1);
            }
        }
        if let Some(new) = new {
            for index in new.iter_ones().take_while(|i| *i < len) {
                counts[index] += 1;
            }
        }
    }

    pub fn set_piece_available(&self, live: &mut LivePeerState, index: usize, value: bool) {
        if let Some(mut bit) = live.bitfield.get_mut(index) {
            if *bit != value {
                let mut counts = self.piece_availability.write();
                if let Some(count) = counts.get_mut(index) {
                    if value {
                        *count += 1;
                    } else {
                        *count = count.saturating_sub(1);
                    }
                }
                *bit = value;
            }
        }
    }

    pub fn stats(&self) -> AggregatePeerStats {
        self.stats.snapshot()
    }

    pub fn add_if_not_seen(
        &self,
        addr: SocketAddr,
        info_hash: librqbit_core::hash_id::Id20,
    ) -> Option<PeerHandle> {
        use dashmap::mapref::entry::Entry;
        match self.states.entry(addr) {
            Entry::Occupied(_) => None,
            Entry::Vacant(vac) => {
                vac.insert(Peer::new_with_outgoing_address(addr, info_hash));
                atomic_inc(&self.stats.queued);
                atomic_inc(&self.session_stats.queued);

                atomic_inc(&self.stats.seen);
                atomic_inc(&self.session_stats.seen);
                Some(addr)
            }
        }
    }
    pub fn with_peer<R>(&self, addr: PeerHandle, f: impl FnOnce(&Peer) -> R) -> Option<R> {
        self.states.get(&addr).map(|e| f(e.value()))
    }

    pub fn with_peer_mut<R>(
        &self,
        addr: PeerHandle,
        reason: &'static str,
        f: impl FnOnce(&mut Peer) -> R,
    ) -> Option<R> {
        use crate::torrent_state::utils::timeit;
        timeit(reason, || self.states.get_mut(&addr))
            .map(|e| f(TimedExistence::new(e, reason).value_mut()))
    }

    pub fn with_live<R>(&self, addr: PeerHandle, f: impl FnOnce(&LivePeerState) -> R) -> Option<R> {
        self.with_peer(addr, |peer| peer.get_live().map(f))
            .flatten()
    }

    pub fn with_live_mut<R>(
        &self,
        addr: PeerHandle,
        reason: &'static str,
        f: impl FnOnce(&mut LivePeerState) -> R,
    ) -> Option<R> {
        self.with_peer_mut(addr, reason, |peer| peer.get_live_mut().map(f))
            .flatten()
    }

    pub fn drop_peer(&self, handle: PeerHandle) -> Option<Peer> {
        let p = self.states.remove(&handle).map(|r| r.1)?;
        let s = p.get_state();
        self.replace_availability(p.get_live().map(|live| &live.bitfield), None);
        self.stats.dec(s);
        self.session_stats.dec(s);

        Some(p)
    }

    pub fn is_peer_not_interested_and_has_full_torrent(
        &self,
        handle: PeerHandle,
        total_pieces: usize,
    ) -> bool {
        self.with_live(handle, |live| {
            !live.peer_interested && live.has_full_torrent(total_pieces)
        })
        .unwrap_or(false)
    }

    pub fn mark_peer_interested(&self, handle: PeerHandle, is_interested: bool) -> Option<bool> {
        self.with_live_mut(handle, "mark_peer_interested", |live| {
            let prev = live.peer_interested;
            live.peer_interested = is_interested;
            prev
        })
    }

    pub fn update_bitfield(&self, handle: PeerHandle, bitfield: BF) -> Option<()> {
        self.with_live_mut(handle, "update_bitfield", |live| {
            self.replace_availability(Some(&live.bitfield), Some(&bitfield));
            live.bitfield = bitfield;
        })
    }

    pub fn mark_peer_connecting(&self, h: PeerHandle) -> crate::Result<(PeerRx, PeerTx)> {
        let rx = self
            .with_peer_mut(h, "mark_peer_connecting", |peer| {
                peer.idle_to_connecting(self)
                    .ok_or(Error::BugInvalidPeerState)
            })
            .ok_or(Error::BugPeerNotFound)??;
        Ok(rx)
    }

    pub fn reset_peer_backoff(&self, handle: PeerHandle) {
        self.with_peer_mut(handle, "reset_peer_backoff", |p| {
            p.stats.reset_backoff();
        });
    }

    pub fn mark_peer_not_needed(&self, handle: PeerHandle) -> Option<PeerState> {
        let prev = self.with_peer_mut(handle, "mark_peer_not_needed", |peer| {
            peer.set_not_needed(self)
        })?;
        Some(prev)
    }

    pub(crate) fn on_steal(
        &self,
        from_peer: SocketAddr,
        to_peer: SocketAddr,
        stolen_idx: ValidPieceIndex,
    ) {
        self.with_peer(to_peer, |p| {
            atomic_inc(&p.stats.counters.times_i_stole);
        });
        self.with_peer(from_peer, |p| {
            atomic_inc(&p.stats.counters.times_stolen_from_me);
        });
        self.stats.inc_steals();
        self.session_stats.inc_steals();

        self.with_live_mut(from_peer, "send_cancellations", |live| {
            live.cancel_inflight_requests_for_piece(stolen_idx);
        });
    }
}

#[cfg(test)]
mod availability_tests {
    use super::*;
    use crate::stream_connect::ConnectionKind;
    use librqbit_core::Id20;

    #[test]
    fn availability_tracks_duplicates_rejects_replacements_and_disconnects() {
        let peers = PeerStates {
            session_stats: Default::default(),
            live_outgoing_peers: Default::default(),
            stats: Default::default(),
            states: Default::default(),
            piece_availability: RwLock::new(vec![0; 4]),
        };
        let addr: SocketAddr = "127.0.0.1:1234".parse().unwrap();
        let (tx, _) = tokio::sync::mpsc::unbounded_channel();
        peers.states.insert(
            addr,
            Peer::new_live_for_incoming_connection(
                addr,
                Id20::new([0; 20]),
                tx,
                &peers,
                ConnectionKind::Tcp,
            ),
        );
        let bf = |byte| BF::from_boxed_slice(vec![byte].into_boxed_slice());
        peers.update_bitfield(addr, bf(0b11000000));
        peers.with_live_mut(addr, "test", |live| {
            peers.set_piece_available(live, 1, true); // Duplicate HAVE.
            peers.set_piece_available(live, 2, true);
            peers.set_piece_available(live, 0, false); // Reject.
            peers.set_piece_available(live, 0, false);
        });
        assert_eq!(*peers.piece_availability.read(), vec![0, 1, 1, 0]);
        peers.update_bitfield(addr, bf(0b10010000));
        assert_eq!(*peers.piece_availability.read(), vec![1, 0, 0, 1]);
        let second: SocketAddr = "127.0.0.1:1235".parse().unwrap();
        let (tx, _) = tokio::sync::mpsc::unbounded_channel();
        peers.states.insert(
            second,
            Peer::new_live_for_incoming_connection(
                second,
                Id20::new([1; 20]),
                tx,
                &peers,
                ConnectionKind::Tcp,
            ),
        );
        peers.update_bitfield(second, bf(0b10000000));
        assert_eq!(*peers.piece_availability.read(), vec![2, 0, 0, 1]);
        peers.with_peer_mut(addr, "test", |peer| {
            peer.take_state(&peers);
        });
        assert_eq!(*peers.piece_availability.read(), vec![1, 0, 0, 0]);
        peers.drop_peer(addr);
        peers.drop_peer(second);
        assert_eq!(*peers.piece_availability.read(), vec![0; 4]);
    }
}

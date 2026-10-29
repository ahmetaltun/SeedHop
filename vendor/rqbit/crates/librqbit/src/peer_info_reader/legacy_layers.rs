//! Derive hybrid Merkle layers from legacy payload without creating files.
use crate::{
    peer_connection::{
        PeerConnection, PeerConnectionHandler, PeerConnectionOptions, WriterRequest,
    },
    spawn_utils::BlockingSpawner,
    stream_connect::{ConnectionKind, StreamConnector},
    type_aliases::PeerStream,
};
use anyhow::Context;
use buffers::{ByteBuf, ByteBufOwned};
use futures::{StreamExt, stream::FuturesUnordered};
use librqbit_core::{
    Id20,
    lengths::{ChunkInfo, ValidPieceIndex},
    v2::{LegacyPieceLayers, Metainfo, V2Error, sha256},
};
use parking_lot::Mutex;
use peer_binary_protocol::{
    Handshake, Message, Request,
    extended::{ExtendedMessage, handshake::ExtendedHandshake, ut_metadata::UtMetadata},
};
use sha1w::{ISha1, Sha1};
use std::{
    collections::{HashSet, VecDeque},
    net::SocketAddr,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::{Notify, watch};
use tracing::debug;

struct State {
    layers: Option<LegacyPieceLayers>,
    needed: VecDeque<u32>,
    claimed: HashSet<u32>,
    fatal: Option<V2Error>,
}
struct Shared {
    state: Mutex<State>,
    pieces: ByteBufOwned,
    piece_count: usize,
    changed: watch::Sender<u64>,
}
impl Shared {
    fn new(info: &[u8], pieces: ByteBufOwned) -> anyhow::Result<Arc<Self>> {
        let layers =
            LegacyPieceLayers::from_info(info)?.context("legacy provider supplied v1 metadata")?;
        let piece_count = layers.metainfo().piece_count() as usize;
        anyhow::ensure!(
            pieces.as_ref().len() == piece_count * 20,
            "legacy piece commitments have wrong length"
        );
        let needed = layers.needed_pieces().into();
        let (changed, _) = watch::channel(0);
        Ok(Arc::new(Self {
            state: Mutex::new(State {
                layers: Some(layers),
                needed,
                claimed: HashSet::new(),
                fatal: None,
            }),
            pieces,
            piece_count,
            changed,
        }))
    }
    fn poke(&self) {
        self.changed
            .send_modify(|epoch| *epoch = epoch.wrapping_add(1));
    }
    fn complete(&self) -> anyhow::Result<bool> {
        let state = self.state.lock();
        if let Some(error) = &state.fatal {
            return Err(error.clone().into());
        }
        Ok(state
            .layers
            .as_ref()
            .is_none_or(|layers| layers.is_complete()))
    }
    fn useful(&self, available: &[u8]) -> anyhow::Result<bool> {
        let state = self.state.lock();
        if let Some(error) = &state.fatal {
            return Err(error.clone().into());
        }
        let Some(layers) = state.layers.as_ref() else {
            return Ok(false);
        };
        // A piece claimed by another peer is still missing until authenticated.
        for &canonical in state.needed.iter().chain(&state.claimed) {
            if layers
                .candidate_pieces(canonical)?
                .into_iter()
                .any(|index| available[index as usize / 8] & (128 >> (index % 8)) != 0)
            {
                return Ok(true);
            }
        }
        Ok(false)
    }
    fn claim(&self, available: &[u8]) -> anyhow::Result<Option<Active>> {
        let mut state = self.state.lock();
        if let Some(error) = &state.fatal {
            return Err(error.clone().into());
        }
        let Some(layers) = state.layers.as_ref() else {
            return Ok(None);
        };
        let mut choice = None;
        for (position, &canonical) in state.needed.iter().enumerate() {
            if let Some(index) = layers
                .candidate_pieces(canonical)?
                .into_iter()
                .find(|&index| available[index as usize / 8] & (128 >> (index % 8)) != 0)
            {
                choice = Some((position, canonical, index));
                break;
            }
        }
        let Some((position, canonical, index)) = choice else {
            return Ok(None);
        };
        let meta = layers.metainfo();
        let real_size = meta.piece_size(u64::from(index))? as u32;
        let wire_size = meta
            .hybrid_piece_size(u64::from(index))
            .context("missing legacy piece size")?;
        let expected = self.pieces.as_ref()[index as usize * 20..index as usize * 20 + 20]
            .try_into()
            .unwrap();
        state.needed.remove(position);
        assert!(state.claimed.insert(canonical));
        Ok(Some(Active {
            canonical,
            index,
            real_size,
            wire_size,
            begin: 0,
            expected,
            sha1: Sha1::new(),
            blocks: Vec::new(),
        }))
    }
    fn release(&self, canonical: u32) {
        let mut state = self.state.lock();
        if state.claimed.remove(&canonical) && state.layers.is_some() && state.fatal.is_none() {
            state.needed.push_back(canonical);
        }
        drop(state);
        self.poke();
    }
    fn accept(&self, active: &mut Active) -> anyhow::Result<()> {
        let mut state = self.state.lock();
        let result = state
            .layers
            .as_mut()
            .context("legacy layers already completed")?
            .complete_piece(active.index, std::mem::take(&mut active.blocks));
        if let Err(error) = result {
            if error == V2Error::HybridHashMismatch {
                state.fatal = Some(error.clone());
            }
            drop(state);
            self.poke();
            return Err(error.into());
        }
        state.claimed.remove(&active.canonical);
        drop(state);
        self.poke();
        Ok(())
    }
}
struct Active {
    canonical: u32,
    index: u32,
    real_size: u32,
    wire_size: u32,
    begin: u32,
    expected: [u8; 20],
    sha1: Sha1,
    blocks: Vec<[u8; 32]>,
}
impl Active {
    fn request(&self) -> Request {
        Request::new(
            self.index,
            self.begin,
            (self.wire_size - self.begin).min(16384),
        )
    }
}
struct Local {
    available: Vec<u8>,
    unchoked: bool,
    interested: bool,
    saw_message: bool,
    active: Option<Active>,
    last_progress: Instant,
}
struct ClaimGuard {
    local: Arc<Mutex<Local>>,
    shared: Arc<Shared>,
}
impl Drop for ClaimGuard {
    fn drop(&mut self) {
        let active = self.local.lock().active.take();
        if let Some(active) = active {
            self.shared.release(active.canonical);
        }
    }
}
#[derive(Debug, thiserror::Error)]
enum PeerError {
    #[error("legacy layer peer choked its pending request")]
    Choked,
    #[error("legacy layer peer made no payload progress for 20 seconds")]
    Idle,
}
struct Handler {
    local: Arc<Mutex<Local>>,
    shared: Arc<Shared>,
    tx: tokio::sync::mpsc::UnboundedSender<WriterRequest>,
    client: String,
    reply_slots: Arc<tokio::sync::Semaphore>,
    local_changed: Arc<Notify>,
}
impl PeerConnectionHandler for Handler {
    fn update_my_extended_handshake(
        &self,
        handshake: &mut ExtendedHandshake<ByteBuf>,
    ) -> anyhow::Result<()> {
        // Acquisition connections do not participate in peer exchange.
        handshake.m.ut_pex = None;
        Ok(())
    }

    fn should_unchoke(&self) -> bool {
        false
    }
    fn should_send_bitfield(&self) -> bool {
        false
    }
    fn serialize_bitfield_message_to_buf(&self, _: &mut [u8]) -> anyhow::Result<usize> {
        Ok(0)
    }
    fn on_handshake(&self, _: Handshake, _: ConnectionKind) -> anyhow::Result<()> {
        Ok(())
    }
    async fn on_extended_handshake(
        &self,
        _: &ExtendedHandshake<ByteBuf<'_>>,
    ) -> anyhow::Result<()> {
        Ok(())
    }
    async fn on_received_message(&self, message: Message<'_>) -> anyhow::Result<()> {
        if let Message::Extended(ExtendedMessage::UtMetadata(UtMetadata::Request(index))) = message
        {
            let permit = self.reply_slots.clone().acquire_owned().await?;
            self.tx
                .send(WriterRequest::UtMetadata(UtMetadata::Reject(index), permit))?;
            return Ok(());
        }
        let mut local = self.local.lock();
        if matches!(message, Message::Bitfield(_)) && local.saw_message {
            return Err(V2Error::Invalid("legacy layer bitfield is not first").into());
        }
        if !matches!(message, Message::Extended(_)) {
            local.saw_message = true;
        }
        let payload_progress = matches!(&message, Message::Piece(_));
        match message {
            Message::Bitfield(bits) => {
                let count = self.shared.piece_count;
                if bits.0.len() != count.div_ceil(8)
                    || (!count.is_multiple_of(8)
                        && bits
                            .0
                            .last()
                            .is_some_and(|last| last & ((1 << (8 - count % 8)) - 1) != 0))
                {
                    return Err(
                        V2Error::Invalid("invalid legacy layer availability bitfield").into(),
                    );
                }
                local.available.copy_from_slice(bits.0);
            }
            Message::Have(index) => {
                if index as usize >= self.shared.piece_count {
                    return Err(V2Error::Invalid("legacy layer HAVE out of range").into());
                }
                let byte = &mut local.available[index as usize / 8];
                let mask = 128 >> (index % 8);
                if *byte & mask != 0 {
                    return Ok(());
                }
                *byte |= mask;
            }
            Message::Unchoke => {
                if local.unchoked {
                    return Ok(());
                }
                local.unchoked = true;
            }
            Message::Choke => {
                if !local.unchoked && local.active.is_none() {
                    return Ok(());
                }
                local.unchoked = false;
                if local.active.is_some() {
                    return Err(PeerError::Choked.into());
                }
            }
            Message::Piece(piece) => {
                let active = local
                    .active
                    .as_mut()
                    .ok_or(V2Error::Invalid("unsolicited legacy layer payload"))?;
                let request = active.request();
                if piece.index != request.index
                    || piece.begin != request.begin
                    || piece.len() != request.length as usize
                {
                    return Err(V2Error::Invalid("uncorrelated legacy layer payload").into());
                }
                let (first, second) = piece.data();
                let mut block = [0; 16384];
                block[..first.len()].copy_from_slice(first);
                block[first.len()..piece.len()].copy_from_slice(second);
                active.sha1.update(&block[..piece.len()]);
                let real = active
                    .real_size
                    .saturating_sub(active.begin)
                    .min(request.length) as usize;
                if block[real..piece.len()].iter().any(|byte| *byte != 0) {
                    return Err(V2Error::Invalid("nonzero legacy layer padding").into());
                }
                if real != 0 {
                    active.blocks.push(sha256(&block[..real]));
                }
                active.begin += request.length;
                if active.begin == active.wire_size {
                    let sha1 = std::mem::replace(&mut active.sha1, Sha1::new()).finish();
                    if sha1 != active.expected {
                        return Err(V2Error::LegacyHashMismatch.into());
                    }
                    self.shared.accept(active)?;
                    local.active = None;
                } else {
                    self.tx
                        .send(WriterRequest::Message(Message::Request(active.request())))?;
                }
                local.last_progress = Instant::now();
            }
            Message::Reject(_) => {
                return Err(
                    V2Error::Invalid("legacy peer rejected a layer payload request").into(),
                );
            }
            Message::Hashes { .. }
            | Message::HashRequest(_)
            | Message::HashReject(_)
            | Message::Extended(ExtendedMessage::UtMetadata(UtMetadata::Data(_))) => {
                return Err(V2Error::Invalid("unsolicited legacy layer metadata").into());
            }
            _ => return Ok(()),
        }
        drop(local);
        // Per-block progress updates the deadline without waking every worker.
        // Shared acceptance/release already wakes peers whose useful set changed.
        if !payload_progress {
            self.local_changed.notify_one();
        }
        Ok(())
    }
    fn should_transmit_have(&self, _: ValidPieceIndex) -> bool {
        false
    }
    fn on_uploaded_bytes(&self, _: u32) {}
    fn read_chunk(&self, _: &ChunkInfo, _: &mut [u8]) -> anyhow::Result<()> {
        anyhow::bail!("legacy layer fetcher does not advertise payload")
    }
    fn client_name_and_version(&self) -> &str {
        &self.client
    }
}

async fn read_peer(
    addr: SocketAddr,
    peer_id: Id20,
    info_hash: Id20,
    shared: Arc<Shared>,
    options: Option<PeerConnectionOptions>,
    connector: Arc<StreamConnector>,
    client: String,
) -> anyhow::Result<()> {
    let local = Arc::new(Mutex::new(Local {
        available: vec![0; shared.piece_count.div_ceil(8)],
        unchoked: false,
        interested: false,
        saw_message: false,
        active: None,
        last_progress: Instant::now(),
    }));
    let _guard = ClaimGuard {
        local: local.clone(),
        shared: shared.clone(),
    };
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let local_changed = Arc::new(Notify::new());
    let handler = Handler {
        local: local.clone(),
        shared: shared.clone(),
        tx: tx.clone(),
        client,
        reply_slots: Arc::new(tokio::sync::Semaphore::new(32)),
        local_changed: local_changed.clone(),
    };
    let connection = PeerConnection::new(
        addr,
        info_hash,
        peer_id,
        handler,
        options,
        BlockingSpawner::new(1),
        connector,
    );
    let (_, brx) = tokio::sync::broadcast::channel(1);
    let scheduler = async {
        let mut changes = shared.changed.subscribe();
        loop {
            changes.borrow_and_update();
            if shared.complete()? {
                return Ok::<(), anyhow::Error>(());
            }
            let deadline = {
                let mut local = local.lock();
                let interested = local.active.is_some() || shared.useful(&local.available)?;
                if local.interested != interested {
                    tx.send(WriterRequest::Message(if interested {
                        Message::Interested
                    } else {
                        Message::NotInterested
                    }))?;
                    local.interested = interested;
                }
                if interested
                    && local.unchoked
                    && local.active.is_none()
                    && let Some(active) = shared.claim(&local.available)?
                {
                    tx.send(WriterRequest::Message(Message::Request(active.request())))?;
                    local.active = Some(active);
                    local.last_progress = Instant::now();
                }
                local.last_progress + Duration::from_secs(20)
            };
            tokio::select! {
                result = changes.changed() => { result?; },
                _ = local_changed.notified() => {},
                _ = tokio::time::sleep_until(deadline.into()) => {
                    if local.lock().last_progress + Duration::from_secs(20) <= Instant::now() {
                        return Err(PeerError::Idle.into());
                    }
                },
            }
        }
    };
    tokio::select! {
        result = scheduler => result,
        result = connection.manage_peer_outgoing(rx, brx) => result.map_err(Into::into),
    }
}

pub(crate) async fn read_from_peers(
    peer_id: Id20,
    info_hash: Id20,
    info: ByteBufOwned,
    pieces: ByteBufOwned,
    mut peers: PeerStream,
    options: Option<PeerConnectionOptions>,
    connector: Arc<StreamConnector>,
    client: String,
) -> anyhow::Result<(Metainfo, HashSet<SocketAddr>)> {
    let shared = Shared::new(info.as_ref(), pieces)?;
    let mut changes = shared.changed.subscribe();
    let mut seen = HashSet::new();
    let mut pending = VecDeque::new();
    let mut jobs = FuturesUnordered::new();
    let mut ended = false;
    loop {
        changes.borrow_and_update();
        if shared.complete()? {
            let layers = shared
                .state
                .lock()
                .layers
                .take()
                .context("legacy layers already consumed")?;
            let metainfo = layers.finish()?;
            debug!(
                pieces = shared.piece_count,
                "legacy payload Merkle layers authenticated"
            );
            return Ok((metainfo, seen));
        }
        while jobs.len() < 8 && !pending.is_empty() {
            let addr = pending.pop_front().unwrap();
            let shared = shared.clone();
            let connector = connector.clone();
            let client = client.clone();
            jobs.push(async move {
                for attempt in 0..3 {
                    let result = read_peer(
                        addr,
                        peer_id,
                        info_hash,
                        shared.clone(),
                        options,
                        connector.clone(),
                        client.clone(),
                    )
                    .await;
                    let retryable = result.as_ref().err().is_some_and(|error| {
                        error.chain().any(|cause| cause.is::<PeerError>())
                            || crate::dht_utils::transient_peer_failure(error)
                    });
                    if attempt == 2 || !retryable {
                        return result;
                    }
                    tokio::time::sleep(Duration::from_millis(250 * (attempt + 1))).await;
                }
                unreachable!()
            });
        }
        if ended && jobs.is_empty() && pending.is_empty() {
            anyhow::bail!("legacy peer stream exhausted before piece layers completed");
        }
        tokio::select! {
            result = jobs.next(), if !jobs.is_empty() => if let Some(Err(error)) = result { debug!(%error, "legacy layer peer failed"); },
            result = changes.changed() => { result?; },
            addr = peers.next(), if !ended => match addr {
                Some(addr) if seen.len() < 4096 && pending.len() < 1024 && seen.insert(addr) => pending.push_back(addr),
                Some(_) => {},
                None => ended = true,
            }
        }
    }
}

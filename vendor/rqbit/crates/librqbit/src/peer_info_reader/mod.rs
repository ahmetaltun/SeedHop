use std::{net::SocketAddr, sync::Arc};

use bencode::from_bytes;
use buffers::{ByteBuf, ByteBufOwned};
use bytes::Bytes;
use librqbit_core::{
    constants::CHUNK_SIZE,
    hash_id::{Id20, Id32},
    lengths::{ChunkInfo, last_element_size},
    torrent_metainfo::TorrentMetaV1Info,
};
use parking_lot::{Mutex, RwLock};
use peer_binary_protocol::{
    Handshake, Message,
    extended::{
        ExtendedMessage,
        handshake::ExtendedHandshake,
        ut_metadata::{UtMetadata, UtMetadataData},
    },
};
use sha1w::{ISha1, Sha1};
use tokio::sync::mpsc::UnboundedSender;
use tracing::trace;

use crate::{
    peer_connection::{
        PeerConnection, PeerConnectionHandler, PeerConnectionOptions, WriterRequest,
    },
    spawn_utils::BlockingSpawner,
    stream_connect::{ConnectionKind, StreamConnector},
};

pub(crate) mod legacy_layers;

#[derive(Clone, Copy, Debug)]
pub(crate) struct PrivateMetadata {
    pub source: SocketAddr,
    pub dictionary_hash: Id32,
}
pub(crate) struct MetadataPrivacy {
    state: tokio::sync::watch::Sender<Option<PrivateMetadata>>,
    trusted_peers: Vec<SocketAddr>,
}
impl MetadataPrivacy {
    pub fn new() -> Arc<Self> {
        Self::with_trusted_peers(Vec::new())
    }
    pub fn with_trusted_peers(trusted_peers: Vec<SocketAddr>) -> Arc<Self> {
        let (state, _) = tokio::sync::watch::channel(None);
        Arc::new(Self {
            state,
            trusted_peers,
        })
    }
    pub fn private_initial(&self, notice: PrivateMetadata) -> Vec<SocketAddr> {
        let mut peers = self.trusted_peers.clone();
        if !peers.contains(&notice.source) {
            peers.push(notice.source);
        }
        peers
    }
    pub fn current(&self) -> Option<PrivateMetadata> {
        *self.state.borrow()
    }
    pub fn subscribe(&self) -> tokio::sync::watch::Receiver<Option<PrivateMetadata>> {
        self.state.subscribe()
    }
    fn authenticated(&self, source: SocketAddr, dictionary: &[u8]) {
        let notice = PrivateMetadata {
            source,
            dictionary_hash: Id32::new(librqbit_core::v2::sha256(dictionary)),
        };
        self.state.send_if_modified(|state| {
            if state.is_none() {
                *state = Some(notice);
                true
            } else {
                false
            }
        });
    }
}

/// The negotiated wire identity and all full magnet commitments are distinct.
/// Legacy metadata can be constrained by a known v2 hash without using v2 wire.
#[derive(Clone, Copy, Debug)]
pub(crate) struct MetadataIdentity {
    pub wire_hash: Id20,
    pub v1: Option<Id20>,
    pub v2: Option<Id32>,
    pub v2_wire: bool,
    dictionary_pin: Option<Id32>,
}
impl MetadataIdentity {
    pub fn legacy(hash: Id20, required_v2: Option<Id32>) -> Self {
        Self {
            wire_hash: hash,
            v1: Some(hash),
            v2: required_v2,
            v2_wire: false,
            dictionary_pin: None,
        }
    }
    pub fn v2(hash: Id32, required_v1: Option<Id20>) -> Self {
        Self {
            wire_hash: hash.truncate_for_dht(),
            v1: required_v1,
            v2: Some(hash),
            v2_wire: true,
            dictionary_pin: None,
        }
    }
    pub fn with_dictionary_pin(mut self, hash: Id32) -> Self {
        self.dictionary_pin = Some(hash);
        self
    }
}
#[derive(Debug, thiserror::Error)]
enum MetadataIdentityError {
    #[error("metadata does not match the magnet's SHA-1 commitment")]
    Sha1,
    #[error("metadata does not match the magnet's SHA-256 commitment")]
    Sha256,
    #[error("a dual-hash magnet must describe a hybrid torrent")]
    NotHybrid,
    #[error("metadata changed after the private dictionary was authenticated")]
    PrivateDictionary,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum MetadataTransferError {
    #[error("metadata size {0} is outside 1..=32 MiB")]
    SizeOutOfBounds(u32),
    #[error("metadata total_size changed from {expected} to {received}")]
    SizeChanged { expected: u32, received: u32 },
    #[error("peer rejected required metadata piece {0}")]
    Rejected(u32),
    #[error("metadata peer stopped making useful progress")]
    NoProgress,
    #[error("metadata peer exceeded the 120 second attempt deadline")]
    Deadline,
}

pub(crate) async fn read_metainfo_from_peer(
    addr: SocketAddr,
    peer_id: Id20,
    identity: MetadataIdentity,
    peer_connection_options: Option<PeerConnectionOptions>,
    spawner: BlockingSpawner,
    connector: Arc<StreamConnector>,
    client_name_and_version: String,
    privacy: Arc<MetadataPrivacy>,
) -> anyhow::Result<TorrentAndInfoBytes> {
    // Metadata probes must advance through dead endpoints quickly. These
    // limits do not change payload connections or the requested encryption policy.
    let mut peer_connection_options = peer_connection_options.unwrap_or_default();
    let probe_timeout = std::time::Duration::from_secs(3);
    peer_connection_options.connect_timeout = Some(
        peer_connection_options
            .connect_timeout
            .unwrap_or(probe_timeout)
            .min(probe_timeout),
    );
    peer_connection_options.read_write_timeout = Some(
        peer_connection_options
            .read_write_timeout
            .unwrap_or(probe_timeout)
            .min(probe_timeout),
    );
    let (result_tx, result_rx) = tokio::sync::oneshot::channel::<
        Result<(TorrentMetaV1Info<ByteBufOwned>, ByteBufOwned), bencode::DeserializeError>,
    >();
    let (writer_tx, writer_rx) = tokio::sync::mpsc::unbounded_channel::<WriterRequest>();
    let (progress, mut progress_rx) = tokio::sync::watch::channel(Some((
        tokio::time::Instant::now(),
        std::time::Duration::from_secs(15),
    )));
    let handler = Handler {
        progress,
        metadata_slots: connector.metadata_slots.clone(),
        metadata_permit: Mutex::new(None),
        addr,
        identity,
        handshake_hash: Mutex::new(None),
        layers: Mutex::new(None),
        pending_info: Mutex::new(None),
        writer_tx,
        reply_slots: Arc::new(tokio::sync::Semaphore::new(32)),
        result_tx: Mutex::new(Some(result_tx)),
        locked: RwLock::new(None),
        client_name_and_version: client_name_and_version.clone(),
        privacy,
    };
    let connection = PeerConnection::new(
        addr,
        identity.wire_hash,
        peer_id,
        handler,
        Some(peer_connection_options),
        spawner.clone(),
        connector.clone(),
    );

    let result_reader = result_rx;
    let (_, brx) = tokio::sync::broadcast::channel(1);
    let connection_runner = async move { connection.manage_peer_outgoing(writer_rx, brx).await };

    let watchdog = async move {
        loop {
            let progress = *progress_rx.borrow_and_update();
            match progress {
                // Waiting for the shared dictionary budget is not peer inactivity.
                None => {
                    if progress_rx.changed().await.is_err() {
                        return;
                    }
                }
                Some((last, budget)) => {
                    let deadline = last + budget;
                    match tokio::time::timeout_at(deadline, progress_rx.changed()).await {
                        Ok(Err(_)) => return,
                        Err(_) if *progress_rx.borrow() == Some((last, budget)) => return,
                        _ => {}
                    }
                }
            }
        }
    };
    let result = tokio::select! {
        // A peer may close immediately after its final authenticated response.
        // Prefer a completed result over an EOF observed in the same poll.
        biased;
        result = result_reader => result??,
        _ = watchdog => return Err(MetadataTransferError::NoProgress.into()),
        _ = tokio::time::sleep(std::time::Duration::from_secs(120)) => return Err(MetadataTransferError::Deadline.into()),
        whatever = connection_runner => match whatever {
            Ok(_) => anyhow::bail!("connection runner completed first"),
            Err(e) => return Err(e.into())
        }
    };
    // A legacy provider need not support hash messages or the v2 identity.
    // The session discovers proof providers after authenticating this info.
    Ok(result)
}

#[derive(Default)]
struct HandlerLocked {
    metadata_size: u32,
    total_pieces: usize,
    buffer: Vec<u8>,
    received_pieces: Vec<bool>,
}

impl HandlerLocked {
    fn new(metadata_size: u32) -> anyhow::Result<Self> {
        if metadata_size == 0 || metadata_size > 32 * 1024 * 1024 {
            return Err(MetadataTransferError::SizeOutOfBounds(metadata_size).into());
        }
        let buffer = vec![0u8; metadata_size as usize];
        let total_pieces: usize = (metadata_size as u64)
            .div_ceil(CHUNK_SIZE as u64)
            .try_into()?;
        let received_pieces = vec![false; total_pieces];
        Ok(Self {
            metadata_size,
            received_pieces,
            buffer,
            total_pieces,
        })
    }
    fn piece_size(&self, index: u32) -> usize {
        if index as usize == self.total_pieces - 1 {
            last_element_size(self.metadata_size as u64, CHUNK_SIZE as u64)
                .try_into()
                .unwrap()
        } else {
            CHUNK_SIZE as usize
        }
    }
    fn record_piece(
        &mut self,
        d: &UtMetadataData<ByteBuf>,
        identity: MetadataIdentity,
    ) -> anyhow::Result<bool> {
        if d.total_size() != self.metadata_size {
            return Err(MetadataTransferError::SizeChanged {
                expected: self.metadata_size,
                received: d.total_size(),
            }
            .into());
        }
        let piece = d.piece();
        if piece as usize >= self.total_pieces {
            anyhow::bail!("wrong index");
        }
        let offset = (piece * CHUNK_SIZE) as usize;
        let size = self.piece_size(piece);
        if d.len() != size {
            anyhow::bail!(
                "expected length of piece {} to be {}, but got {}",
                piece,
                size,
                d.len()
            );
        }
        if self.received_pieces[piece as usize] {
            anyhow::bail!("already received piece {}", piece);
        }
        d.copy_to_slice(&mut self.buffer[offset..offset + d.len()]);
        self.received_pieces[piece as usize] = true;

        if self.received_pieces.iter().all(|p| *p) {
            // check metadata
            let mut hash = Sha1::new();
            hash.update(&self.buffer);
            if identity
                .v1
                .is_some_and(|expected| hash.finish() != expected.0)
            {
                return Err(MetadataIdentityError::Sha1.into());
            }
            if identity
                .v2
                .is_some_and(|expected| librqbit_core::v2::sha256(&self.buffer) != expected.0)
            {
                return Err(MetadataIdentityError::Sha256.into());
            }
            if identity
                .dictionary_pin
                .is_some_and(|expected| librqbit_core::v2::sha256(&self.buffer) != expected.0)
            {
                return Err(MetadataIdentityError::PrivateDictionary.into());
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

pub type TorrentAndInfoBytes = (TorrentMetaV1Info<ByteBufOwned>, ByteBufOwned);

struct Handler {
    progress: tokio::sync::watch::Sender<Option<(tokio::time::Instant, std::time::Duration)>>,
    metadata_slots: Arc<tokio::sync::Semaphore>,
    metadata_permit: Mutex<Option<tokio::sync::OwnedSemaphorePermit>>,
    privacy: Arc<MetadataPrivacy>,
    addr: SocketAddr,
    identity: MetadataIdentity,
    handshake_hash: Mutex<Option<Id20>>,
    layers: Mutex<Option<librqbit_core::v2::PendingMetainfo>>,
    pending_info: Mutex<Option<TorrentAndInfoBytes>>,
    writer_tx: UnboundedSender<WriterRequest>,
    reply_slots: Arc<tokio::sync::Semaphore>,
    result_tx: Mutex<
        Option<
            tokio::sync::oneshot::Sender<Result<TorrentAndInfoBytes, bencode::DeserializeError>>,
        >,
    >,
    locked: RwLock<Option<HandlerLocked>>,
    client_name_and_version: String,
}

impl Handler {
    fn complete_info(&self, info: TorrentAndInfoBytes) -> anyhow::Result<()> {
        self.result_tx
            .lock()
            .take()
            .ok_or_else(|| anyhow::anyhow!("metadata already complete"))?
            .send(Ok(info))
            .map_err(|_| anyhow::anyhow!("metadata consumer closed"))
    }
    fn request_next_layer(&self) -> anyhow::Result<()> {
        let next = self.layers.lock().as_ref().and_then(|p| p.next_request());
        if let Some(request) = next {
            self.writer_tx
                .send(WriterRequest::Message(Message::HashRequest(request)))?;
        } else {
            let pending = self
                .layers
                .lock()
                .take()
                .ok_or_else(|| anyhow::anyhow!("no pending metadata"))?;
            let meta = pending.finish()?;
            let mut info = self
                .pending_info
                .lock()
                .take()
                .ok_or_else(|| anyhow::anyhow!("missing info dictionary"))?;
            info.0.v2 = Some(Arc::new(meta));
            self.complete_info(info)?;
        }
        Ok(())
    }
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

    // A hybrid peer can answer metadata discovery with its v2 wire identity.
    // No content is accepted until the original full SHA-1 commitment and this
    // candidate's truncated SHA-256 identity both match the received info bytes.
    fn accepts_metadata_hash_upgrade(&self, hash: Id20) -> bool {
        !self.identity.v2_wire
            && self
                .identity
                .v2
                .is_none_or(|expected| hash == expected.truncate_for_dht())
    }

    fn is_v2(&self) -> bool {
        self.identity.v2_wire
    }
    fn should_unchoke(&self) -> bool {
        false
    }
    fn should_send_bitfield(&self) -> bool {
        false
    }

    fn serialize_bitfield_message_to_buf(&self, _buf: &mut [u8]) -> anyhow::Result<usize> {
        Ok(0)
    }

    fn on_connected(&self, _connection_time: std::time::Duration) {
        // Encryption and TCP/uTP fallback share the negotiation budget. The
        // shorter useful-data watchdog starts once the BT handshake is valid.
        self.progress.send_replace(Some((
            tokio::time::Instant::now(),
            std::time::Duration::from_secs(15),
        )));
    }

    fn on_handshake(&self, handshake: Handshake, _kind: ConnectionKind) -> anyhow::Result<()> {
        self.progress.send_replace(Some((
            tokio::time::Instant::now(),
            std::time::Duration::from_secs(5),
        )));
        *self.handshake_hash.lock() = Some(handshake.info_hash);
        if !handshake.supports_extended() {
            anyhow::bail!(
                "this peer does not support extended handshaking, which is a prerequisite to download metadata"
            )
        }
        Ok(())
    }

    async fn on_received_message(&self, msg: Message<'_>) -> anyhow::Result<()> {
        trace!("{}: received message: {:?}", self.addr, msg);

        // Metadata discovery advertises no payload or hash availability. It is
        // still a v2 peer connection, so requests need an explicit response.
        match msg {
            Message::Extended(ExtendedMessage::UtMetadata(UtMetadata::Request(index))) => {
                let permit = self.reply_slots.clone().acquire_owned().await?;
                self.writer_tx
                    .send(WriterRequest::UtMetadata(UtMetadata::Reject(index), permit))?;
                return Ok(());
            }
            Message::Extended(ExtendedMessage::UtMetadata(UtMetadata::Reject(index))) => {
                return Err(MetadataTransferError::Rejected(index).into());
            }
            Message::HashRequest(request) => {
                let permit = self.reply_slots.clone().acquire_owned().await?;
                self.writer_tx.send(WriterRequest::BoundedMessage(
                    Message::HashReject(request),
                    permit,
                ))?;
                return Ok(());
            }
            Message::Request(request) if self.identity.v2_wire => {
                let permit = self.reply_slots.clone().acquire_owned().await?;
                self.writer_tx.send(WriterRequest::BoundedMessage(
                    Message::Reject(request),
                    permit,
                ))?;
                return Ok(());
            }
            Message::Piece(_) if self.identity.v2_wire => {
                anyhow::bail!("unsolicited payload during metadata discovery");
            }
            _ => {}
        }

        if let Message::Hashes {
            request,
            ref hashes,
        } = msg
        {
            self.layers
                .lock()
                .as_mut()
                .ok_or_else(|| anyhow::anyhow!("unsolicited hashes"))?
                .accept(request, hashes)?;
            self.progress.send_replace(Some((
                tokio::time::Instant::now(),
                std::time::Duration::from_secs(5),
            )));
            self.request_next_layer()?;
            return Ok(());
        }
        if let Message::HashReject(_) = msg {
            anyhow::bail!("peer rejected required Merkle hashes");
        }
        if let Message::Extended(ExtendedMessage::UtMetadata(UtMetadata::Data(utdata))) = msg {
            let piece_ready = self
                .locked
                .write()
                .as_mut()
                .ok_or_else(|| {
                    anyhow::anyhow!("metadata data received after info dictionary completed")
                })?
                .record_piece(&utdata, self.identity)?;
            self.progress.send_replace(Some((
                tokio::time::Instant::now(),
                std::time::Duration::from_secs(5),
            )));
            if piece_ready {
                let buf = Bytes::from(self.locked.write().take().unwrap().buffer);
                librqbit_core::v2::Metainfo::check_info_version(&buf)?;
                let info = from_bytes::<TorrentMetaV1Info<ByteBuf>>(&buf)
                    .map(|i| {
                        use clone_to_owned::CloneToOwned;
                        i.clone_to_owned(Some(&buf))
                    })
                    .map_err(|e| {
                        trace!("error deserializing TorrentMetaV1Info: {e:#}");
                        e.into_kind()
                    })
                    .map(|i| (i, ByteBufOwned(buf)));

                let info = info?;
                if let Some(candidate) = *self.handshake_hash.lock()
                    && candidate != self.identity.wire_hash
                {
                    anyhow::ensure!(
                        candidate.0 == librqbit_core::v2::sha256(info.1.as_ref())[..20],
                        "hybrid metadata handshake identity mismatch"
                    );
                }
                if info.0.private {
                    self.privacy.authenticated(self.addr, info.1.as_ref());
                }
                if self.identity.v2_wire {
                    let pending =
                        librqbit_core::v2::PendingMetainfo::from_info(info.1.as_ref())?
                            .ok_or_else(|| anyhow::anyhow!("v2 magnet supplied v1 metadata"))?;
                    if Some(Id32::new(pending.info_hash())) != self.identity.v2 {
                        anyhow::bail!("v2 metadata identity mismatch");
                    }
                    if self.identity.v1.is_some() && !pending.is_hybrid() {
                        return Err(MetadataIdentityError::NotHybrid.into());
                    }
                    *self.pending_info.lock() = Some(info);
                    *self.layers.lock() = Some(pending);
                    self.request_next_layer()?;
                } else {
                    self.complete_info(info)?;
                }
            }
        }
        Ok(())
    }

    fn on_uploaded_bytes(&self, _bytes: u32) {}

    fn read_chunk(&self, _chunk: &ChunkInfo, _buf: &mut [u8]) -> anyhow::Result<()> {
        anyhow::bail!("the peer is not supposed to be requesting chunks")
    }

    async fn on_extended_handshake(
        &self,
        extended_handshake: &ExtendedHandshake<ByteBuf<'_>>,
    ) -> anyhow::Result<()> {
        // BEP 10 allows later extension-handshake updates. They must not
        // allocate a second dictionary or replay all outstanding requests.
        if let Some(current) = self.locked.read().as_ref() {
            if let Some(received) = extended_handshake.metadata_size {
                if received != current.metadata_size {
                    return Err(MetadataTransferError::SizeChanged {
                        expected: current.metadata_size,
                        received,
                    }
                    .into());
                }
            }
            return Ok(());
        }
        if self.pending_info.lock().is_some() {
            return Ok(());
        }
        let metadata_size = match extended_handshake.metadata_size {
            Some(metadata_size) => metadata_size,
            None => anyhow::bail!("peer does not have metadata_size"),
        };

        if extended_handshake.m.ut_metadata.is_none_or(|id| id == 0) {
            anyhow::bail!("peer does not support ut_metadata");
        }

        if metadata_size == 0 || metadata_size > 32 * 1024 * 1024 {
            return Err(MetadataTransferError::SizeOutOfBounds(metadata_size).into());
        }
        // Only peers advertising usable metadata reserve dictionary memory.
        // Other socket probes can continue while all four dictionaries are busy.
        self.progress.send_replace(None);
        let permit = self.metadata_slots.clone().acquire_owned().await?;
        *self.metadata_permit.lock() = Some(permit);
        self.progress.send_replace(Some((
            tokio::time::Instant::now(),
            std::time::Duration::from_secs(5),
        )));

        // Metadata and Merkle-hash requests are independent of payload
        // choking. This connection has no payload interest or availability.
        let inner = HandlerLocked::new(metadata_size)?;
        let total_pieces = inner.total_pieces;

        self.locked.write().replace(inner);

        for i in 0..total_pieces {
            self.writer_tx
                .send(WriterRequest::Message(Message::Extended(
                    ExtendedMessage::UtMetadata(UtMetadata::Request(i.try_into()?)),
                )))?;
        }
        Ok(())
    }

    fn should_transmit_have(&self, _id: librqbit_core::lengths::ValidPieceIndex) -> bool {
        false
    }

    fn client_name_and_version(&self) -> &str {
        &self.client_name_and_version
    }
}

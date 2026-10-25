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

pub(crate) async fn read_metainfo_from_peer(
    addr: SocketAddr,
    peer_id: Id20,
    info_hash: Id20,
    v2_hash: Option<Id32>,
    peer_connection_options: Option<PeerConnectionOptions>,
    spawner: BlockingSpawner,
    connector: Arc<StreamConnector>,
    client_name_and_version: String,
) -> anyhow::Result<TorrentAndInfoBytes> {
    let (result_tx, result_rx) = tokio::sync::oneshot::channel::<
        Result<(TorrentMetaV1Info<ByteBufOwned>, ByteBufOwned), bencode::DeserializeError>,
    >();
    let (writer_tx, writer_rx) = tokio::sync::mpsc::unbounded_channel::<WriterRequest>();
    let handler = Handler {
        addr,
        info_hash,
        v2_hash,
        handshake_hash: Mutex::new(None),
        layers: Mutex::new(None),
        pending_info: Mutex::new(None),
        writer_tx,
        result_tx: Mutex::new(Some(result_tx)),
        locked: RwLock::new(None),
        client_name_and_version: client_name_and_version.clone(),
    };
    let connection = PeerConnection::new(
        addr,
        info_hash,
        peer_id,
        handler,
        peer_connection_options,
        spawner.clone(),
        connector.clone(),
    );

    let result_reader = result_rx;
    let (_, brx) = tokio::sync::broadcast::channel(1);
    let connection_runner = async move { connection.manage_peer_outgoing(writer_rx, brx).await };

    let result = tokio::select! {
        result = result_reader => result??,
        whatever = connection_runner => match whatever {
            Ok(_) => anyhow::bail!("connection runner completed first"),
            Err(e) => return Err(e.into())
        }
    };
    // A btih-only hybrid magnet first authenticates SHA-1, then reconnects using
    // its committed SHA-256 identity to fetch v2 Merkle layers.
    if v2_hash.is_none() {
        if let Some(pending) = librqbit_core::v2::PendingMetainfo::from_info(result.1.as_ref())? {
            let full = Id32::new(pending.info_hash());
            return Box::pin(read_metainfo_from_peer(
                addr,
                peer_id,
                full.truncate_for_dht(),
                Some(full),
                peer_connection_options,
                spawner,
                connector,
                client_name_and_version,
            ))
            .await;
        }
    }
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
        if metadata_size > 32 * 1024 * 1024 {
            anyhow::bail!("metadata size {} is too big", metadata_size);
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
        info_hash: &Id20,
        v2_hash: Option<Id32>,
    ) -> anyhow::Result<bool> {
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
            let matches = if let Some(expected) = v2_hash {
                librqbit_core::v2::sha256(&self.buffer) == expected.0
            } else {
                hash.finish() == info_hash.0
            };
            if !matches {
                anyhow::bail!("info checksum invalid");
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

pub type TorrentAndInfoBytes = (TorrentMetaV1Info<ByteBufOwned>, ByteBufOwned);

struct Handler {
    addr: SocketAddr,
    info_hash: Id20,
    v2_hash: Option<Id32>,
    handshake_hash: Mutex<Option<Id20>>,
    layers: Mutex<Option<librqbit_core::v2::PendingMetainfo>>,
    pending_info: Mutex<Option<TorrentAndInfoBytes>>,
    writer_tx: UnboundedSender<WriterRequest>,
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
    // A hybrid peer can answer metadata discovery with its v2 wire identity.
    // No content is accepted until the original full SHA-1 commitment and this
    // candidate's truncated SHA-256 identity both match the received info bytes.
    fn accepts_metadata_hash_upgrade(&self, _hash: Id20) -> bool {
        self.v2_hash.is_none()
    }

    fn is_v2(&self) -> bool {
        self.v2_hash.is_some()
    }
    fn should_send_bitfield(&self) -> bool {
        false
    }

    fn serialize_bitfield_message_to_buf(&self, _buf: &mut [u8]) -> anyhow::Result<usize> {
        Ok(0)
    }

    fn on_handshake(&self, handshake: Handshake, _kind: ConnectionKind) -> anyhow::Result<()> {
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
                .record_piece(&utdata, &self.info_hash, self.v2_hash)?;
            if piece_ready {
                let buf = Bytes::from(self.locked.write().take().unwrap().buffer);
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
                    && candidate != self.info_hash
                {
                    anyhow::ensure!(
                        candidate.0 == librqbit_core::v2::sha256(info.1.as_ref())[..20],
                        "hybrid metadata handshake identity mismatch"
                    );
                }
                if self.v2_hash.is_some() {
                    let pending =
                        librqbit_core::v2::PendingMetainfo::from_info(info.1.as_ref())?
                            .ok_or_else(|| anyhow::anyhow!("v2 magnet supplied v1 metadata"))?;
                    if Some(Id32::new(pending.info_hash())) != self.v2_hash {
                        anyhow::bail!("v2 metadata identity mismatch");
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

    fn on_extended_handshake(
        &self,
        extended_handshake: &ExtendedHandshake<ByteBuf>,
    ) -> anyhow::Result<()> {
        let metadata_size = match extended_handshake.metadata_size {
            Some(metadata_size) => metadata_size,
            None => anyhow::bail!("peer does not have metadata_size"),
        };

        if extended_handshake.m.ut_metadata.is_none() {
            anyhow::bail!("peer does not support ut_metadata");
        }

        self.writer_tx
            .send(WriterRequest::Message(Message::Unchoke))?;
        self.writer_tx
            .send(WriterRequest::Message(Message::Interested))?;

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

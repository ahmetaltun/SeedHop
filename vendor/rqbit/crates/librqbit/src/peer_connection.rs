use std::{
    net::SocketAddr,
    sync::Arc,
    time::{Duration, Instant},
};

use crate::{Error, Result, session::CheckedIncomingConnection, stream_connect::ConnectionKind};
use buffers::{ByteBuf, ByteBufOwned};
use librqbit_core::{
    hash_id::Id20,
    lengths::{ChunkInfo, ValidPieceIndex},
    peer_id::try_decode_peer_id,
};
use parking_lot::RwLock;
use peer_binary_protocol::{
    Handshake, MAX_MSG_LEN, Message,
    extended::{
        ExtendedMessage, PeerExtendedMessageIds, handshake::ExtendedHandshake,
        ut_metadata::UtMetadata, ut_pex::UtPex,
    },
    serialize_piece_preamble,
};
use serde::{Deserialize, Serialize};
use serde_with::serde_as;
use tokio::time::timeout;
use tracing::{Instrument, debug, trace, trace_span};

use crate::{
    read_buf::ReadBuf,
    spawn_utils::BlockingSpawner,
    stream_connect::StreamConnector,
    type_aliases::{BoxAsyncReadVectored, BoxAsyncWrite},
    vectored_traits::AsyncReadVectoredIntoCompat,
};

pub trait PeerConnectionHandler {
    fn accepts_metadata_hash_upgrade(&self, _hash: Id20) -> bool {
        false
    }
    fn is_v2(&self) -> bool {
        false
    }
    fn supports_v2(&self) -> bool {
        self.is_v2()
    }
    fn on_connected(&self, _connection_time: Duration) {}
    fn should_send_bitfield(&self) -> bool;
    fn should_unchoke(&self) -> bool {
        true
    }
    fn serialize_bitfield_message_to_buf(&self, buf: &mut [u8]) -> anyhow::Result<usize>;
    fn on_encryption(&self, _encrypted: bool) {}
    fn on_handshake(&self, handshake: Handshake, ckind: ConnectionKind) -> anyhow::Result<()>;
    fn on_extended_handshake(
        &self,
        extended_handshake: &ExtendedHandshake<ByteBuf>,
    ) -> anyhow::Result<()>;
    async fn on_received_message(&self, msg: Message<'_>) -> anyhow::Result<()>;
    fn should_transmit_have(&self, id: ValidPieceIndex) -> bool;
    fn on_uploaded_bytes(&self, bytes: u32);
    fn read_chunk(&self, chunk: &ChunkInfo, buf: &mut [u8]) -> anyhow::Result<()>;
    fn update_my_extended_handshake(
        &self,
        _handshake: &mut ExtendedHandshake<ByteBuf>,
    ) -> anyhow::Result<()> {
        Ok(())
    }
    fn client_name_and_version(&self) -> &str {
        crate::client_name_and_version()
    }
}

#[derive(Debug)]
pub enum WriterRequest {
    Message(Message<'static>),
    UtPex(UtPex<ByteBufOwned>),
    BoundedMessage(Message<'static>, tokio::sync::OwnedSemaphorePermit),
    UtMetadata(UtMetadata<ByteBufOwned>, tokio::sync::OwnedSemaphorePermit),
    ReadChunkRequest(ChunkInfo, tokio::sync::OwnedSemaphorePermit),
    Disconnect(anyhow::Result<()>),
}

#[serde_as]
#[derive(Default, Debug, Copy, Clone, Serialize, Deserialize, Eq, PartialEq)]
pub struct PeerConnectionOptions {
    #[serde_as(as = "Option<serde_with::DurationSeconds>")]
    pub connect_timeout: Option<Duration>,

    #[serde_as(as = "Option<serde_with::DurationSeconds>")]
    pub read_write_timeout: Option<Duration>,

    #[serde_as(as = "Option<serde_with::DurationSeconds>")]
    pub keep_alive_interval: Option<Duration>,
}

pub(crate) struct PeerConnection<H> {
    handler: H,
    addr: SocketAddr,
    info_hash: Id20,
    peer_id: Id20,
    options: PeerConnectionOptions,
    spawner: BlockingSpawner,
    connector: Arc<StreamConnector>,
}

#[cfg(not(feature = "miri"))]
pub(crate) async fn with_timeout<T>(
    name: &'static str,
    timeout_value: Duration,
    fut: impl std::future::Future<Output = Result<T>>,
) -> crate::Result<T> {
    match timeout(timeout_value, fut).await {
        Ok(v) => v,
        Err(_) => Err(Error::Timeout(name)),
    }
}

#[cfg(feature = "miri")]
pub(crate) async fn with_timeout<T>(
    _name: &'static str,
    _timeout_value: Duration,
    fut: impl std::future::Future<Output = Result<T>>,
) -> crate::Result<T> {
    fut.await
}

struct ManagePeerArgs {
    handshake_supports_extended: bool,
    read_buf: ReadBuf,
    write_buf: Box<[u8; MAX_MSG_LEN]>,
    read: BoxAsyncReadVectored,
    write: BoxAsyncWrite,
    outgoing_chan: tokio::sync::mpsc::UnboundedReceiver<WriterRequest>,
    have_broadcast: tokio::sync::broadcast::Receiver<ValidPieceIndex>,
}

impl<H: PeerConnectionHandler> PeerConnection<H> {
    pub fn new(
        addr: SocketAddr,
        info_hash: Id20,
        peer_id: Id20,
        handler: H,
        options: Option<PeerConnectionOptions>,
        spawner: BlockingSpawner,
        connector: Arc<StreamConnector>,
    ) -> Self {
        PeerConnection {
            handler,
            addr,
            info_hash,
            peer_id,
            spawner,
            options: options.unwrap_or_default(),
            connector,
        }
    }

    // By the time this is called:
    // read_buf should start with valuable data. The handshake should be removed from it.
    pub async fn manage_peer_incoming(
        &self,
        outgoing_chan: tokio::sync::mpsc::UnboundedReceiver<WriterRequest>,
        mut incoming: CheckedIncomingConnection,
        have_broadcast: tokio::sync::broadcast::Receiver<ValidPieceIndex>,
    ) -> Result<()> {
        use tokio::io::AsyncWriteExt;

        let rwtimeout = self
            .options
            .read_write_timeout
            .unwrap_or_else(|| Duration::from_secs(10));

        if incoming.handshake.info_hash != self.info_hash {
            return Err(Error::WrongInfoHash);
        }

        if incoming.handshake.peer_id == self.peer_id {
            return Err(Error::ConnectingToOurselves);
        }

        trace!(
            "incoming connection: id={:?}",
            try_decode_peer_id(incoming.handshake.peer_id)
        );

        let mut write_buf = Box::new([0u8; MAX_MSG_LEN]);
        let handshake =
            Handshake::new(self.info_hash, self.peer_id).with_v2(self.handler.supports_v2());
        let hlen = handshake.serialize_unchecked_len(&mut *write_buf);
        with_timeout("writing handshake", rwtimeout, async {
            incoming
                .writer
                .write_all(&write_buf[..hlen])
                .await
                .map_err(Error::WriteHandshake)?;
            incoming.writer.flush().await.map_err(Error::Write)
        })
        .await?;

        let handshake_supports_extended = incoming.handshake.supports_extended();

        self.handler
            .on_handshake(handshake, incoming.kind)
            .map_err(Error::Anyhow)?;
        self.handler.on_encryption(incoming.encrypted);

        self.manage_peer(ManagePeerArgs {
            handshake_supports_extended,
            read_buf: incoming.read_buf,
            write_buf,
            read: incoming.reader,
            write: incoming.writer,
            outgoing_chan,
            have_broadcast,
        })
        .await
    }

    async fn try_outgoing_connection(
        &self,
        ckind: ConnectionKind,
        mut read: BoxAsyncReadVectored,
        mut write: BoxAsyncWrite,
        encrypted: bool,
        rwtimeout: Duration,
    ) -> Result<(
        ConnectionKind,
        BoxAsyncReadVectored,
        BoxAsyncWrite,
        ReadBuf,
        Handshake,
        bool,
    )> {
        use tokio::io::AsyncWriteExt;
        let handshake =
            Handshake::new(self.info_hash, self.peer_id).with_v2(self.handler.supports_v2());
        let mut bytes = [0; 68];
        let _ = handshake.serialize_unchecked_len(&mut bytes);
        let mut payload_encrypted = false;
        if encrypted {
            let negotiated = with_timeout("MSE negotiation", rwtimeout, async {
                let _permit = self
                    .connector
                    .outgoing_crypto_semaphore
                    .acquire()
                    .await
                    .map_err(|_| Error::TorrentIsNotLive)?;
                Ok(crate::mse::outgoing(
                    read,
                    write,
                    self.info_hash.0,
                    &bytes,
                    self.connector.encryption_policy,
                )
                .await?)
            })
            .await?;
            payload_encrypted = negotiated.encrypted;
            debug!(kind=%ckind, rc4=negotiated.encrypted, "MSE outgoing negotiation completed");
            read = Box::new(negotiated.read.into_vectored_compat());
            write = Box::new(negotiated.write);
        } else {
            with_timeout("writing handshake", rwtimeout, async {
                write
                    .write_all(&bytes)
                    .await
                    .map_err(Error::WriteHandshake)?;
                write.flush().await.map_err(Error::Write)
            })
            .await?;
        }
        let mut read_buf = ReadBuf::new();
        let h = read_buf.read_handshake(&mut read, rwtimeout).await?;
        if h.info_hash != self.info_hash && !self.handler.accepts_metadata_hash_upgrade(h.info_hash)
        {
            return Err(Error::WrongInfoHash);
        }
        if h.peer_id == self.peer_id {
            return Err(Error::ConnectingToOurselves);
        }
        Ok((ckind, read, write, read_buf, h, payload_encrypted))
    }

    pub async fn manage_peer_outgoing(
        &self,
        outgoing_chan: tokio::sync::mpsc::UnboundedReceiver<WriterRequest>,
        have_broadcast: tokio::sync::broadcast::Receiver<ValidPieceIndex>,
    ) -> Result<()> {
        let rwtimeout = self
            .options
            .read_write_timeout
            .unwrap_or_else(|| Duration::from_secs(10));
        let connect_timeout = self
            .options
            .connect_timeout
            .unwrap_or_else(|| Duration::from_secs(10));
        let now = Instant::now();
        let initial = with_timeout(
            "connecting",
            connect_timeout,
            self.connector.connect(self.addr),
        )
        .await?;
        let initial_kind = initial.0;
        let mut initial = Some(initial);
        let mut connected = None;
        let mut last_error = None;
        let transports =
            if matches!(initial_kind, ConnectionKind::Tcp) && self.connector.can_connect_utp() {
                2
            } else {
                1
            };
        'transport: for transport in 0..transports {
            let kind = if transport == 0 {
                initial_kind
            } else {
                ConnectionKind::Utp
            };
            for &encrypted in self.connector.encryption_policy.attempts() {
                let connection = match initial.take() {
                    Some(stream) => Ok(stream),
                    None => {
                        with_timeout(
                            "reconnecting",
                            connect_timeout,
                            self.connector.reconnect(kind, self.addr),
                        )
                        .await
                    }
                };
                let (actual_kind, read, write) = match connection {
                    Ok(stream) => stream,
                    Err(error) => {
                        last_error = Some(error);
                        continue;
                    }
                };
                match self
                    .try_outgoing_connection(actual_kind, read, write, encrypted, rwtimeout)
                    .await
                {
                    Ok(stream) => {
                        connected = Some(stream);
                        break 'transport;
                    }
                    Err(error @ (Error::WrongInfoHash | Error::ConnectingToOurselves)) => {
                        return Err(error);
                    }
                    Err(error) => {
                        debug!(kind=%actual_kind, encrypted, ?error, "peer handshake attempt failed");
                        last_error = Some(error);
                    }
                }
            }
        }
        let (ckind, read, write, read_buf, h, payload_encrypted) =
            connected.ok_or_else(|| last_error.unwrap_or(Error::ConnectDisabled))?;
        let write_buf = Box::new([0u8; MAX_MSG_LEN]);
        self.handler.on_connected(now.elapsed());

        async move {
            let handshake_supports_extended = h.supports_extended();
            trace!(
                peer_id=?h.peer_id,
                decoded_id=?try_decode_peer_id(h.peer_id),
                "connected",
            );
            self.handler.on_handshake(h, ckind).map_err(Error::Anyhow)?;
            self.handler.on_encryption(payload_encrypted);

            self.manage_peer(ManagePeerArgs {
                handshake_supports_extended,
                read_buf,
                write_buf,
                read,
                write,
                outgoing_chan,
                have_broadcast,
            })
            .await
        }
        .instrument(trace_span!("", kind=%ckind))
        .await
    }

    async fn manage_peer(&self, args: ManagePeerArgs) -> Result<()> {
        let ManagePeerArgs {
            handshake_supports_extended,
            mut read_buf,
            mut write_buf,
            mut read,
            mut write,
            mut outgoing_chan,
            mut have_broadcast,
        } = args;

        use tokio::io::AsyncWriteExt;

        let rwtimeout = self
            .options
            .read_write_timeout
            .unwrap_or_else(|| Duration::from_secs(10));

        let extended_handshake: RwLock<Option<PeerExtendedMessageIds>> = RwLock::new(None);
        let extended_handshake_ref = &extended_handshake;
        let supports_extended = handshake_supports_extended;

        if supports_extended {
            let mut my_extended = ExtendedHandshake::new();
            my_extended.v = Some(ByteBuf(self.handler.client_name_and_version().as_bytes()));
            my_extended.yourip = Some(self.addr.ip().into());
            self.handler
                .update_my_extended_handshake(&mut my_extended)
                .map_err(Error::Anyhow)?;
            let my_extended = Message::Extended(ExtendedMessage::Handshake(my_extended));
            trace!("sending extended handshake: {:?}", &my_extended);
            let esz = my_extended.serialize(&mut *write_buf, &Default::default)?;
            with_timeout("writing extended handshake", rwtimeout, async {
                write
                    .write_all(&write_buf[..esz])
                    .await
                    .map_err(Error::Write)?;
                write.flush().await.map_err(Error::Write)
            })
            .await?;
        }

        let writer = async move {
            let keep_alive_interval = self
                .options
                .keep_alive_interval
                .unwrap_or_else(|| Duration::from_secs(120));

            if self.handler.should_send_bitfield() {
                let len = self
                    .handler
                    .serialize_bitfield_message_to_buf(&mut *write_buf)
                    .map_err(Error::Anyhow)?;
                with_timeout("writing bitfield", rwtimeout, async {
                    write
                        .write_all(&write_buf[..len])
                        .await
                        .map_err(Error::Write)?;
                    write.flush().await.map_err(Error::Write)
                })
                .await?;
                trace!("sent bitfield");
            }

            if self.handler.should_unchoke() {
                let len = Message::Unchoke.serialize(&mut *write_buf, &Default::default)?;
                with_timeout("writing", rwtimeout, async {
                    write
                        .write_all(&write_buf[..len])
                        .await
                        .map_err(Error::Write)?;
                    write.flush().await.map_err(Error::Write)
                })
                .await?;
                trace!("sent unchoke");
            }

            let mut broadcast_closed = false;

            loop {
                let req = loop {
                    break tokio::select! {
                        r = have_broadcast.recv(), if !broadcast_closed => match r {
                            Ok(id) => {
                                if self.handler.should_transmit_have(id) {
                                     WriterRequest::Message(Message::Have(id.get()))
                                } else {
                                    continue
                                }
                            },
                            Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                                broadcast_closed = true;
                                debug!("broadcast channel closed, will not poll it anymore");
                                continue
                            },
                            _ => continue
                        },
                        r = timeout(keep_alive_interval, outgoing_chan.recv()) => match r {
                            Ok(Some(msg)) => msg,
                            Ok(None) => {
                                return Err(Error::TorrentIsNotLive);
                            }
                            Err(_) => WriterRequest::Message(Message::KeepAlive),
                        }
                    };
                };

                tokio::task::yield_now().await;

                let mut uploaded_add = None;
                let mut reply_permit = None;

                trace!("about to send: {:?}", &req);
                let ext_msg_ids = &|| {
                    extended_handshake_ref
                        .read()
                        .as_ref()
                        .map(|e| *e)
                        .unwrap_or_default()
                };

                let len = match req {
                    WriterRequest::Message(msg) => msg.serialize(&mut *write_buf, ext_msg_ids)?,
                    WriterRequest::BoundedMessage(msg, permit) => {
                        reply_permit = Some(permit);
                        msg.serialize(&mut *write_buf, ext_msg_ids)?
                    }
                    WriterRequest::UtMetadata(utm, permit) => {
                        reply_permit = Some(permit);
                        if ext_msg_ids().ut_metadata.is_none_or(|id| id == 0) {
                            continue;
                        }
                        Message::Extended(ExtendedMessage::UtMetadata(utm.as_borrowed()))
                            .serialize(&mut *write_buf, ext_msg_ids)?
                    }
                    WriterRequest::UtPex(ut_pex) => {
                        if ext_msg_ids().ut_pex.is_none_or(|id| id == 0) {
                            continue;
                        }
                        Message::Extended(ExtendedMessage::UtPex(ut_pex.as_borrowed()))
                            .serialize(&mut *write_buf, ext_msg_ids)?
                    }
                    WriterRequest::ReadChunkRequest(chunk, permit) => {
                        reply_permit = Some(permit);
                        #[allow(unused_mut)]
                        let mut skip_reading_for_e2e_tests = false;

                        #[cfg(test)]
                        {
                            use tracing::warn;
                            // This is poor-mans fault injection for running e2e tests.
                            use crate::tests::test_util::TestPeerMetadata;
                            let tpm = TestPeerMetadata::from_peer_id(self.peer_id);
                            use rand::RngExt;
                            if rand::rng().random_bool(tpm.disconnect_probability()) {
                                return Err(Error::TestDisconnect);
                            }

                            #[allow(clippy::cast_possible_truncation)]
                            let sleep_ms = (rand::rng().random::<f64>()
                                * (tpm.max_random_sleep_ms as f64))
                                as u64;
                            if sleep_ms > 0 {
                                tokio::time::sleep(Duration::from_millis(sleep_ms)).await;
                            }

                            if rand::rng().random_bool(tpm.bad_data_probability()) {
                                warn!(
                                    "will NOT actually read the data to simulate a malicious peer that sends garbage"
                                );
                                write_buf.fill(0);
                                skip_reading_for_e2e_tests = true;
                            }
                        }

                        // this whole section is an optimization
                        let preamble_len = serialize_piece_preamble(&chunk, &mut *write_buf);
                        let full_len = preamble_len + chunk.size as usize;
                        if !skip_reading_for_e2e_tests {
                            self.spawner
                                .block_in_place_with_semaphore(|| {
                                    self.handler
                                        .read_chunk(&chunk, &mut write_buf[preamble_len..full_len])
                                })
                                .await
                                .map_err(Error::ReadChunk)?;
                        }

                        uploaded_add = Some(chunk.size);
                        full_len
                    }
                    WriterRequest::Disconnect(res) => {
                        trace!("disconnect requested, closing writer");
                        match res {
                            Ok(()) => return Err(Error::Disconnect),
                            Err(e) => return Err(Error::DisconnectWithSource(e)),
                        }
                    }
                };

                with_timeout("writing", rwtimeout, async {
                    write
                        .write_all(&write_buf[..len])
                        .await
                        .map_err(Error::Write)?;
                    write.flush().await.map_err(Error::Write)
                })
                .await?;

                // Keep response capacity charged through socket backpressure.
                drop(reply_permit);

                if let Some(uploaded_add) = uploaded_add {
                    self.handler.on_uploaded_bytes(uploaded_add)
                }
            }

            // For type inference.
            #[allow(unreachable_code)]
            Ok::<_, Error>(())
        };

        let reader = async move {
            loop {
                let message = read_buf.read_message(&mut read, rwtimeout).await?;
                trace!("received: {:?}", &message);

                tokio::task::yield_now().await;

                if let Message::Extended(ExtendedMessage::Handshake(h)) = &message {
                    extended_handshake_ref
                        .write()
                        .get_or_insert_with(Default::default)
                        .apply_update(h.peer_extended_messages());
                    self.handler
                        .on_extended_handshake(h)
                        .map_err(Error::Anyhow)?;
                } else {
                    self.handler
                        .on_received_message(message)
                        .await
                        .map_err(Error::Anyhow)?;
                }
            }

            // For type inference.
            #[allow(unreachable_code)]
            Ok::<_, Error>(())
        };

        tokio::select! {
            r = reader => {
                if let Err(e) = r.as_ref() {
                    trace!("reader finished with error: {e:#}");
                } else {
                    trace!("reader finished without error");
                }
                r
            }
            r = writer => {
                if let Err(e) = r.as_ref() {
                    trace!("writer finished with error: {e:#}");
                } else {
                    trace!("writer finished without error");
                }
                r
            }
        }
    }
}

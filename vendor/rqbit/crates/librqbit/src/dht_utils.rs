use std::{
    collections::{HashMap, HashSet, VecDeque},
    net::SocketAddr,
    sync::Arc,
};

use anyhow::Context;
use buffers::ByteBufOwned;
use futures::{
    FutureExt, StreamExt,
    future::{AbortHandle, Abortable},
    stream::FuturesUnordered,
};
use librqbit_core::torrent_metainfo::TorrentMetaV1Info;
use tracing::{Instrument, debug, debug_span};

use crate::peer_info_reader::{
    MetadataIdentity, MetadataPrivacy, MetadataTransferError, PrivateMetadata,
};
use crate::type_aliases::PeerStream;
use crate::{
    peer_connection::PeerConnectionOptions, peer_info_reader, spawn_utils::BlockingSpawner,
    stream_connect::StreamConnector,
};
use librqbit_core::hash_id::Id20;

pub(crate) fn transient_peer_failure(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause
            .downcast_ref::<MetadataTransferError>()
            .is_some_and(|error| matches!(error, MetadataTransferError::NoProgress))
            || cause.downcast_ref::<crate::Error>().is_some_and(|error| {
                matches!(error, crate::Error::Encryption(crate::mse::Error::Io(_)))
                    || matches!(
                        error,
                        crate::Error::TcpConnect(_)
                            | crate::Error::UtpConnect(_)
                            | crate::Error::SocksConnect(_)
                            | crate::Error::Connect { .. }
                            | crate::Error::WriteHandshake(_)
                            | crate::Error::ReadHandshake(_)
                            | crate::Error::Timeout(_)
                            | crate::Error::PeerDisconnectedReadingHandshake
                            | crate::Error::PeerDisconnected
                            | crate::Error::Read(_)
                            | crate::Error::Write(_)
                    )
            })
    })
}

#[derive(Debug)]
#[allow(clippy::large_enum_variant)]
pub enum ReadMetainfoResult<Rx> {
    Found {
        source: SocketAddr,
        info: TorrentMetaV1Info<ByteBufOwned>,
        info_bytes: ByteBufOwned,
        rx: Rx,
        seen: HashSet<SocketAddr>,
    },
    ChannelClosed {
        #[allow(dead_code)]
        seen: HashSet<SocketAddr>,
    },
}

pub(crate) struct MetadataDiscovery {
    pub privacy: Arc<MetadataPrivacy>,
    pub private_peers: Arc<dyn Fn(PrivateMetadata) -> PeerStream + Send + Sync>,
}

pub async fn read_metainfo_from_peer_receiver(
    peer_id: Id20,
    identity: MetadataIdentity,
    initial_addrs: Vec<SocketAddr>,
    addrs_stream: PeerStream,
    peer_connection_options: Option<PeerConnectionOptions>,
    connector: Arc<StreamConnector>,
    client_name_and_version: String,
    discovery: Option<MetadataDiscovery>,
) -> ReadMetainfoResult<PeerStream> {
    let mut seen = HashSet::<SocketAddr>::new();
    let mut addrs = futures::stream::iter(initial_addrs)
        .chain(addrs_stream)
        .boxed();
    let mut seen_order = VecDeque::new();
    let privacy = discovery
        .as_ref()
        .map(|d| d.privacy.clone())
        .unwrap_or_else(MetadataPrivacy::new);
    let mut privacy_changes = privacy.subscribe();
    let mut private_applied = false;

    let read_info_guarded = |addr, identity| {
        let connector = connector.clone();
        let client_name_and_version = client_name_and_version.clone();
        let privacy = privacy.clone();
        async move {
            let token = connector.metadata_probes.acquire().await?;
            let ret = peer_info_reader::read_metainfo_from_peer(
                addr,
                peer_id,
                identity,
                peer_connection_options,
                // This shouldn't be called anyway as we aren't reading/writing to disk, so it's
                // ok not to use a shared one.
                BlockingSpawner::new(1),
                connector.clone(),
                client_name_and_version.clone(),
                privacy.clone(),
            )
            .instrument(debug_span!("read_metainfo_from_peer", ?addr))
            .await;
            drop(token);
            let ret = ret.with_context(|| format!("error reading metainfo from {addr}"));
            ret.map(|(info, info_bytes)| (addr, info, info_bytes))
        }
    };

    let mut unordered = FuturesUnordered::new();
    let mut handles = HashMap::<SocketAddr, AbortHandle>::new();
    let wrap = |addr, identity, attempt| {
        let (handle, registration) = AbortHandle::new_pair();
        let future = Abortable::new(read_info_guarded(addr, identity), registration);
        (handle, async move { (addr, attempt, future.await) }.boxed())
    };

    // Keep tracker/DHT streams moving while connection attempts wait. Fresh
    // endpoints take priority over retries; both queues stay bounded.
    let mut pending = VecDeque::new();
    let mut retries = VecDeque::new();
    let mut addrs_completed = false;
    let mut pending_result: Option<(SocketAddr, TorrentMetaV1Info<ByteBufOwned>, ByteBufOwned)> =
        None;

    loop {
        privacy_changes.borrow_and_update();
        if !private_applied && let Some(notice) = privacy.current() {
            // The authenticated provider already knows this dictionary. Stop
            // public discovery and all unrelated in-flight peer attempts now,
            // rather than waiting for that provider's Merkle proofs to finish.
            for (addr, handle) in &handles {
                if *addr != notice.source {
                    handle.abort();
                }
            }
            pending.retain(|addr| *addr == notice.source);
            retries.retain(|(addr, _)| *addr == notice.source);
            seen.retain(|addr| *addr == notice.source);
            seen_order.clear();
            if seen.contains(&notice.source) {
                seen_order.push_back(notice.source);
            }
            addrs = discovery
                .as_ref()
                .map(|d| (d.private_peers)(notice))
                .unwrap_or_else(|| futures::stream::iter([notice.source]).boxed());
            addrs_completed = false;
            private_applied = true;
            debug!(source = ?notice.source, "private dictionary authenticated; public metadata discovery stopped");
        }
        if let Some((source, info, info_bytes)) = pending_result.take() {
            if let Some(notice) = privacy.current() {
                if librqbit_core::v2::sha256(info_bytes.as_ref()) != notice.dictionary_hash.0 {
                    debug!(
                        ?source,
                        "discarding metadata that differs from the authenticated private dictionary"
                    );
                    continue;
                }
                if !private_applied {
                    pending_result = Some((source, info, info_bytes));
                    continue;
                }
                seen.insert(source);
            }
            return ReadMetainfoResult::Found {
                source,
                info,
                info_bytes,
                seen,
                rx: addrs,
            };
        }
        if addrs_completed && unordered.is_empty() && pending.is_empty() && retries.is_empty() {
            return ReadMetainfoResult::ChannelClosed { seen };
        }

        tokio::select! {
            biased;
            changed = privacy_changes.changed() => { if changed.is_err() { unreachable!("privacy sender is retained by this lookup"); } },
            done = unordered.next(), if !unordered.is_empty() => {
                match done {
                    Some((addr, attempt, result)) => {
                        handles.remove(&addr);
                        match result {
                            Ok(Ok(result)) => pending_result = Some(result),
                            Ok(Err(error)) => {
                                if attempt < 2 && transient_peer_failure(&error) && retries.len() < 64
                                    && privacy.current().is_none_or(|notice| addr == notice.source)
                                {
                                    retries.push_back((addr, attempt + 1));
                                }
                                debug!("{:#}", error);
                            },
                            Err(_) => debug!(?addr, "unrelated public metadata attempt cancelled"),
                        }
                    },
                    None => unreachable!()
                }
            }

            next_addr = addrs.next(), if !addrs_completed && pending.len() < 64 => {
                match next_addr {
                    Some(addr) => {
                        if !seen.contains(&addr) {
                            // Bound remembered endpoints without permanently
                            // excluding fresh candidates after an address flood.
                            if seen.len() >= 4096 {
                                while let Some(old) = seen_order.pop_front() {
                                    if handles.contains_key(&old) { seen_order.push_back(old); continue; }
                                    seen.remove(&old); break;
                                }
                            }
                            seen.insert(addr);
                            seen_order.push_back(addr);
                            pending.push_back(addr);
                        }
                        continue;
                    },
                    None => {
                        addrs_completed = true;
                    },
                }
            }

            _ = std::future::ready(()), if unordered.len() < 64 && (!pending.is_empty() || !retries.is_empty()) => {
                let (addr, attempt) = pending.pop_front().map(|addr| (addr, 0))
                    .or_else(|| retries.pop_front()).unwrap();
                if handles.contains_key(&addr) { continue; }
                let identity = privacy.current().map(|notice| identity.with_dictionary_pin(notice.dictionary_hash)).unwrap_or(identity);
                let (handle, future) = wrap(addr, identity, attempt);
                handles.insert(addr, handle);
                unordered.push(future);
            }
        };
    }
}

#[cfg(test)]
mod tests {
    use dht::{DhtBuilder, Id20};
    use librqbit_core::peer_id::generate_peer_id;

    use super::*;
    use std::{
        str::FromStr,
        sync::{Arc, Once},
    };

    static LOG_INIT: Once = Once::new();

    fn init_logging() {
        #[allow(unused_must_use)]
        LOG_INIT.call_once(|| {
            // pretty_env_logger::try_init();
        })
    }

    #[tokio::test]
    #[ignore]
    async fn read_metainfo_from_dht() {
        init_logging();

        let info_hash = Id20::from_str("cab507494d02ebb1178b38f2e9d7be299c86b862").unwrap();
        let dht = DhtBuilder::new().await.unwrap();

        let peer_rx = dht.get_peers(info_hash, None);
        let peer_id = generate_peer_id(b"-xx1234-");
        match read_metainfo_from_peer_receiver(
            peer_id,
            MetadataIdentity::legacy(info_hash, None),
            Vec::new(),
            peer_rx.boxed(),
            None,
            Arc::new(StreamConnector::new(Default::default()).await.unwrap()),
            crate::client_name_and_version().to_owned(),
            None,
        )
        .await
        {
            ReadMetainfoResult::Found { info, .. } => dbg!(info),
            ReadMetainfoResult::ChannelClosed { .. } => todo!("should not have happened"),
        };
    }
}

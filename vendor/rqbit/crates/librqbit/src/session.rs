use std::{
    borrow::Cow,
    collections::{HashMap, HashSet},
    io::Read,
    net::SocketAddr,
    path::{Component, Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use crate::{
    ApiError, CreateTorrentOptions, FileInfos, ManagedTorrent, ManagedTorrentShared,
    api::TorrentIdOrHash,
    api_error::WithStatus,
    bitv_factory::{BitVFactory, NonPersistentBitVFactory},
    create_torrent,
    create_torrent_file::CreateTorrentResult,
    dht_utils::{MetadataDiscovery, ReadMetainfoResult, read_metainfo_from_peer_receiver},
    ip_ranges::IpRanges,
    limits::{Limits, LimitsConfig},
    listen::{Accept, ListenerOptions},
    merge_streams::merge_streams,
    peer_connection::PeerConnectionOptions,
    peer_info_reader::{MetadataIdentity, MetadataPrivacy},
    read_buf::ReadBuf,
    session_persistence::{SessionPersistenceStore, json::JsonSessionPersistenceStore},
    session_stats::SessionStats,
    spawn_utils::BlockingSpawner,
    storage::{
        BoxStorageFactory, StorageFactoryExt, TorrentStorage, filesystem::FilesystemStorageFactory,
    },
    stream_connect::{
        ConnectionKind, ConnectionOptions, SocksProxyConfig, StreamConnector, StreamConnectorArgs,
    },
    torrent_state::{
        ManagedTorrentHandle, ManagedTorrentLocked, ManagedTorrentOptions, ManagedTorrentState,
        TorrentMetadata, TorrentStateLive, initializing::TorrentStateInitializing,
    },
    type_aliases::{
        BoxAsyncReadVectored, BoxAsyncWrite, PeerStream, TorrentPeer, TorrentPeerStream,
    },
};
use anyhow::{Context, bail};
use arc_swap::ArcSwapOption;
use bencode::bencode_serialize_to_writer;
use buffers::{ByteBuf, ByteBufOwned};
use bytes::Bytes;
use clone_to_owned::CloneToOwned;
use dht::{Dht, DhtBuilder, DhtConfig, DhtPersistenceConfig, Id20, PersistentDht, dht_listen_addr};
use futures::{
    FutureExt, Stream, StreamExt, TryFutureExt,
    future::BoxFuture,
    stream::{BoxStream, FuturesUnordered},
};
use http::StatusCode;
use itertools::Itertools;
use librqbit_core::{
    crate_version,
    directories::get_configuration_directory,
    magnet::Magnet,
    peer_id::generate_azereus_style,
    spawn_utils::spawn_with_cancel,
    torrent_metainfo::{TorrentMetaV1Owned, ValidatedTorrentMetaV1Info},
};
use librqbit_lsd::{LocalServiceDiscovery, LocalServiceDiscoveryOptions};
use librqbit_utp::BindDevice;
use parking_lot::RwLock;
use peer_binary_protocol::Handshake;
use serde::{Deserialize, Serialize};
use tokio::sync::Notify;
use tokio_util::sync::{CancellationToken, DropGuard};
use tracing::{Instrument, debug, debug_span, error, info, trace, warn};
use tracker_comms::{TrackerComms, UdpTrackerClient};

pub const SUPPORTED_SCHEMES: [&str; 3] = ["http:", "https:", "magnet:"];

pub type TorrentId = usize;

struct ParsedTorrentFile {
    meta: TorrentMetaV1Owned,
    torrent_bytes: Bytes,
}

fn torrent_from_bytes(bytes: Bytes) -> anyhow::Result<ParsedTorrentFile> {
    trace!(
        "all fields in torrent: {:#?}",
        bencode::dyn_from_bytes::<ByteBuf>(&bytes)
    );
    let parsed = librqbit_core::torrent_metainfo::torrent_from_bytes(&bytes)?;
    Ok(ParsedTorrentFile {
        meta: parsed.clone_to_owned(Some(&bytes)),
        torrent_bytes: bytes,
    })
}

#[derive(Default)]
pub struct SessionDatabase {
    torrents: HashMap<TorrentId, ManagedTorrentHandle>,
}

impl SessionDatabase {
    fn add_torrent(&mut self, torrent: ManagedTorrentHandle, id: TorrentId) {
        self.torrents.insert(id, torrent);
    }
}

pub struct Session {
    // Core state and services
    pub(crate) db: RwLock<SessionDatabase>,
    next_id: AtomicUsize,
    pub(crate) bitv_factory: Arc<dyn BitVFactory>,
    spawner: BlockingSpawner,

    // Network
    peer_id: Id20,
    announce_port: Option<u16>,
    listen_addr: Option<SocketAddr>,
    dht: Option<Dht>,
    pub(crate) connector: Arc<StreamConnector>,
    reqwest_client: reqwest::Client,
    udp_tracker_client: UdpTrackerClient,
    disable_trackers: bool,

    // Lifecycle management
    cancellation_token: CancellationToken,
    // SeedHop: await session-owned tasks instead of guessing at shutdown latency.
    shutdown_tasks: parking_lot::Mutex<Vec<tokio::task::JoinHandle<()>>>,
    shutdown_task_failed: std::sync::atomic::AtomicBool,
    _cancellation_token_drop_guard: DropGuard,

    // Runtime settings
    output_folder: PathBuf,
    peer_opts: PeerConnectionOptions,
    default_storage_factory: Option<BoxStorageFactory>,
    persistence: Option<Arc<dyn SessionPersistenceStore>>,
    trackers: HashSet<url::Url>,

    lsd: Option<LocalServiceDiscovery>,

    // Limits and throttling
    pub(crate) concurrent_initialize_semaphore: Arc<tokio::sync::Semaphore>,
    pub ratelimits: Limits,

    pub blocklist: IpRanges,
    pub allowlist: Option<IpRanges>,

    // Monitoring / tracing / logging
    pub(crate) stats: Arc<SessionStats>,
    root_span: Option<tracing::Span>,

    // Feature flags
    #[cfg(feature = "disable-upload")]
    _disable_upload: bool,
    pub ipv4_only: bool,
    pub peer_limit: Option<usize>,
    client_name_and_version: String,
}

async fn torrent_from_url(
    reqwest_client: &reqwest::Client,
    url: &str,
) -> anyhow::Result<ParsedTorrentFile> {
    let response = reqwest_client
        .get(url)
        .send()
        .await
        .context("error downloading torrent metadata")?;
    if !response.status().is_success() {
        bail!("GET {} returned {}", url, response.status())
    }
    let b = response
        .bytes()
        .await
        .with_context(|| format!("error reading response body from {url}"))?;
    torrent_from_bytes(b).context("error decoding torrent")
}

fn compute_only_files_regex<ByteBuf: AsRef<[u8]>>(
    torrent: &ValidatedTorrentMetaV1Info<ByteBuf>,
    filename_re: &str,
) -> anyhow::Result<Vec<usize>> {
    let filename_re = regex::Regex::new(filename_re).context("filename regex is incorrect")?;
    let mut only_files = Vec::new();
    for (idx, fd) in torrent.iter_file_details().enumerate() {
        let full_path = fd.filename.to_pathbuf();
        if filename_re.is_match(full_path.to_str().unwrap()) {
            only_files.push(idx);
        }
    }
    if only_files.is_empty() {
        bail!("none of the filenames match the given regex")
    }
    Ok(only_files)
}

fn compute_only_files(
    info: &ValidatedTorrentMetaV1Info<ByteBufOwned>,
    only_files: Option<Vec<usize>>,
    only_files_regex: Option<String>,
    list_only: bool,
) -> anyhow::Result<Option<Vec<usize>>> {
    match (only_files, only_files_regex) {
        (Some(_), Some(_)) => {
            bail!("only_files and only_files_regex are mutually exclusive");
        }
        (Some(only_files), None) => {
            let total_files = info.iter_file_lengths().count();
            for id in only_files.iter().copied() {
                if id >= total_files {
                    bail!("file id {} is out of range", id);
                }
            }
            Ok(Some(only_files))
        }
        (None, Some(filename_re)) => {
            let only_files = compute_only_files_regex(info, &filename_re)?;
            for (idx, fd) in info.iter_file_details().enumerate() {
                if !only_files.contains(&idx) {
                    continue;
                }
                if !list_only {
                    info!(filename=?fd.filename, "will download");
                }
            }
            Ok(Some(only_files))
        }
        (None, None) => Ok(None),
    }
}

fn merge_two_optional_streams<T>(
    s1: Option<impl Stream<Item = T> + Unpin + Send + 'static>,
    s2: Option<impl Stream<Item = T> + Unpin + Send + 'static>,
) -> Option<BoxStream<'static, T>> {
    match (s1, s2) {
        (Some(s1), None) => Some(Box::pin(s1)),
        (None, Some(s2)) => Some(Box::pin(s2)),
        (Some(s1), Some(s2)) => Some(Box::pin(merge_streams(s1, s2))),
        (None, None) => None,
    }
}

/// Options for adding new torrents to the session.
//
// Serialize/deserialize is for Tauri.
#[derive(Default, Serialize, Deserialize)]
pub struct AddTorrentOptions {
    /// Start in paused state.
    #[serde(default)]
    pub paused: bool,
    #[serde(default)]
    pub queued: bool,
    #[serde(default)]
    pub sequential: bool,
    #[serde(default)]
    pub first_last_pieces: bool,
    #[serde(default)]
    pub file_names: std::collections::HashMap<usize, PathBuf>,
    #[serde(default)]
    pub file_priorities: Vec<u8>,
    /// A regex to only download files matching it.
    pub only_files_regex: Option<String>,
    /// An explicit list of file IDs to download.
    /// To see the file indices, run with "list_only".
    pub only_files: Option<Vec<usize>>,
    /// Allow writing on top of existing files, including when resuming a torrent.
    /// You probably want to set it, however for safety it's not default.
    ///
    /// Even when all the torrent pieces have been written, `overwrite` needs to
    /// be enabled in order to resume/seed the torrent.
    #[serde(default)]
    pub overwrite: bool,
    /// Only list the files in the torrent without starting it.
    #[serde(default)]
    pub list_only: bool,
    /// The output folder for the torrent. If not set, the session's default one will be used.
    pub output_folder: Option<String>,
    /// Sub-folder within session's default output folder. Will error if "output_folder" if also set.
    /// By default, multi-torrent files are downloaded to a sub-folder.
    pub sub_folder: Option<String>,
    /// Peer connection options, timeouts etc. If not set, session's defaults will be used.
    pub peer_opts: Option<PeerConnectionOptions>,

    /// Force a refresh interval for polling trackers.
    pub force_tracker_interval: Option<Duration>,

    #[serde(default)]
    pub disable_trackers: bool,

    #[serde(default)]
    pub ratelimits: LimitsConfig,

    /// Initial peers to start of with.
    pub initial_peers: Option<Vec<SocketAddr>>,

    /// Max concurrent connected peers.
    pub peer_limit: Option<usize>,

    /// This is used to restore the session from serialized state.
    pub preferred_id: Option<usize>,

    #[serde(skip)]
    pub storage_factory: Option<BoxStorageFactory>,

    // Custom trackers
    pub trackers: Option<Vec<String>>,
}

pub struct ListOnlyResponse {
    pub info_hash: Id20,
    pub info: ValidatedTorrentMetaV1Info<ByteBufOwned>,
    pub only_files: Option<Vec<usize>>,
    pub output_folder: PathBuf,
    pub seen_peers: Vec<SocketAddr>,
    pub torrent_bytes: Bytes,
}

#[allow(clippy::large_enum_variant)]
pub enum AddTorrentResponse {
    AlreadyManaged(TorrentId, ManagedTorrentHandle),
    ListOnly(ListOnlyResponse),
    Added(TorrentId, ManagedTorrentHandle),
}

impl AddTorrentResponse {
    pub fn into_handle(self) -> Option<ManagedTorrentHandle> {
        match self {
            Self::AlreadyManaged(_, handle) => Some(handle),
            Self::ListOnly(_) => None,
            Self::Added(_, handle) => Some(handle),
        }
    }
}

pub fn read_local_file_including_stdin(filename: &str) -> anyhow::Result<Vec<u8>> {
    let mut buf = Vec::new();
    if filename == "-" {
        std::io::stdin()
            .read_to_end(&mut buf)
            .context("error reading stdin")?;
    } else {
        std::fs::File::open(filename)
            .context("error opening")?
            .read_to_end(&mut buf)
            .context("error reading")?;
    }
    Ok(buf)
}

pub enum AddTorrent<'a> {
    Url(Cow<'a, str>),
    TorrentFileBytes(Bytes),
}

impl<'a> AddTorrent<'a> {
    // Don't call this from HTTP API.
    #[inline(never)]
    pub fn from_cli_argument(path: &'a str) -> anyhow::Result<Self> {
        if SUPPORTED_SCHEMES.iter().any(|s| path.starts_with(s)) {
            return Ok(Self::Url(Cow::Borrowed(path)));
        }
        if path.len() == 40 && !Path::new(path).exists() && Magnet::parse(path).is_ok() {
            return Ok(Self::Url(Cow::Borrowed(path)));
        }
        Self::from_local_filename(path)
    }

    pub fn from_url(url: impl Into<Cow<'a, str>>) -> Self {
        Self::Url(url.into())
    }

    pub fn from_bytes(bytes: impl Into<Bytes>) -> Self {
        Self::TorrentFileBytes(bytes.into())
    }

    // Don't call this from HTTP API.
    #[inline(never)]
    pub fn from_local_filename(filename: &str) -> anyhow::Result<Self> {
        let file = read_local_file_including_stdin(filename)
            .with_context(|| format!("error reading local file {filename:?}"))?;
        Ok(Self::TorrentFileBytes(file.into()))
    }

    pub fn into_bytes(self) -> Bytes {
        match self {
            Self::Url(s) => s.into_owned().into_bytes().into(),
            Self::TorrentFileBytes(b) => b,
        }
    }
}

pub enum SessionPersistenceConfig {
    /// The filename for persistence. By default uses an OS-specific folder.
    Json { folder: Option<PathBuf> },
    #[cfg(feature = "postgres")]
    Postgres { connection_string: String },
}

impl SessionPersistenceConfig {
    pub fn default_json_persistence_folder() -> anyhow::Result<PathBuf> {
        let dir = get_configuration_directory("session")?;
        Ok(dir.data_dir().to_owned())
    }
}

/// Configuration for the DHT subsystem.
/// Set to `None` in `SessionOptions::dht` to disable DHT entirely.
pub struct DhtSessionConfig {
    /// Bootstrap nodes (host:port or ip:port). Uses built-in defaults if None.
    pub bootstrap_addrs: Option<Vec<String>>,
    /// The DHT listen port. Priority: this explicit port -> persisted port
    /// (when persistence is enabled) -> random. The bind IP is derived from
    /// `SessionOptions::ipv4_only` (`0.0.0.0` if true, `[::]` otherwise).
    /// Use `SessionOptions::bind_device_name` to scope the bind to a specific
    /// network interface.
    pub port: Option<u16>,
    /// Persistence behavior. If None, persistence is disabled.
    pub persistence: Option<DhtPersistenceConfig>,
}

impl Default for DhtSessionConfig {
    fn default() -> Self {
        Self {
            bootstrap_addrs: None,
            port: None,
            persistence: Some(DhtPersistenceConfig::default()),
        }
    }
}

pub struct SessionOptions {
    pub peer_encryption: crate::EncryptionPolicy,
    pub restore_queued: bool,
    /// DHT configuration. Set to None to disable DHT entirely.
    /// Defaults to DHT enabled with persistence.
    pub dht: Option<DhtSessionConfig>,

    /// What network device to bind to for DHT, BT-UDP, BT-TCP, trackers and LSD.
    /// On OSX will use IP(V6)_BOUND_IF, on Linux will use SO_BINDTODEVICE.
    pub bind_device_name: Option<String>,

    /// Disable tracker communication
    pub disable_trackers: bool,

    /// Enable fastresume, to restore state quickly after restart.
    pub fastresume: bool,

    /// Turn on to dump session contents into a file periodically, so that on next start
    /// all remembered torrents will continue where they left off.
    pub persistence: Option<SessionPersistenceConfig>,

    /// The peer ID to use. If not specified, a random one will be generated.
    pub peer_id: Option<Id20>,

    /// Options for listening on TCP and/or uTP for incoming connections.
    pub listen: Option<ListenerOptions>,
    /// Options for connecting to peers (for outgiong connections).
    pub connect: Option<ConnectionOptions>,

    pub default_storage_factory: Option<BoxStorageFactory>,

    pub cancellation_token: Option<CancellationToken>,

    /// how many concurrent torrent initializations can happen
    pub concurrent_init_limit: Option<usize>,

    /// How many blocking threads does the tokio runtime have.
    /// Will limit blocking work to that number to avoid starving the runtime.
    pub runtime_worker_threads: Option<usize>,

    /// the root span to use. If not set will be None.
    pub root_span: Option<tracing::Span>,

    pub ratelimits: LimitsConfig,

    pub blocklist_url: Option<String>,
    pub allowlist_url: Option<String>,

    // The list of tracker URLs to always use for each torrent.
    pub trackers: HashSet<url::Url>,

    /// Default peer limit per torrent.
    pub peer_limit: Option<usize>,

    #[cfg(feature = "disable-upload")]
    pub disable_upload: bool,

    /// Disable LSD multicast
    pub disable_local_service_discovery: bool,

    /// Force IPv4 only.
    pub ipv4_only: bool,

    /// Override the client name and version used in User-Agent headers and
    /// peer extended handshakes. Defaults to "rqbit X.Y.Z".
    pub client_name_and_version: Option<String>,
}

impl Default for SessionOptions {
    fn default() -> Self {
        Self {
            peer_encryption: Default::default(),
            restore_queued: false,
            dht: Some(DhtSessionConfig::default()),
            bind_device_name: None,
            disable_trackers: false,
            fastresume: false,
            persistence: None,
            peer_id: None,
            listen: None,
            connect: None,
            default_storage_factory: None,
            cancellation_token: None,
            concurrent_init_limit: None,
            runtime_worker_threads: None,
            root_span: None,
            ratelimits: LimitsConfig::default(),
            blocklist_url: None,
            allowlist_url: None,
            trackers: HashSet::new(),
            peer_limit: None,
            #[cfg(feature = "disable-upload")]
            disable_upload: false,
            disable_local_service_discovery: false,
            ipv4_only: false,
            client_name_and_version: None,
        }
    }
}

fn torrent_file_from_info_bytes(
    info_bytes: &[u8],
    trackers: &[url::Url],
    v2: Option<&librqbit_core::v2::Metainfo>,
) -> anyhow::Result<Bytes> {
    #[derive(Serialize)]
    struct Tmp<'a> {
        announce: &'a str,
        #[serde(rename = "announce-list")]
        announce_list: &'a [&'a [url::Url]],
        info: bencode::raw_value::RawValue<&'a [u8]>,
        #[serde(rename = "piece layers", skip_serializing_if = "Option::is_none")]
        layers: Option<std::collections::BTreeMap<buffers::ByteBufOwned, buffers::ByteBufOwned>>,
    }

    let mut w = Vec::new();
    let v = Tmp {
        info: bencode::raw_value::RawValue(info_bytes),
        layers: v2.map(|v| {
            v.piece_layers()
                .into_iter()
                .map(|(key, value)| {
                    (
                        buffers::ByteBufOwned(Bytes::copy_from_slice(&key)),
                        buffers::ByteBufOwned(value.into()),
                    )
                })
                .collect()
        }),
        announce: trackers.first().map(|s| s.as_str()).unwrap_or(""),
        announce_list: &[trackers],
    };
    bencode_serialize_to_writer(&v, &mut w)?;
    Ok(w.into())
}

pub(crate) struct CheckedIncomingConnection {
    pub kind: ConnectionKind,
    pub encrypted: bool,
    pub addr: SocketAddr,
    pub reader: BoxAsyncReadVectored,
    pub writer: BoxAsyncWrite,
    pub read_buf: ReadBuf,
    pub handshake: Handshake,
}

struct InternalAddResult {
    info_hash: Id20,
    v2_hash: Option<librqbit_core::Id32>,
    magnet_v1_hash: Option<Id20>,
    metadata: Option<TorrentMetadata>,
    trackers: Vec<url::Url>,
    name: Option<String>,
}

impl Session {
    /// Create a new session with default options.
    /// The passed in folder will be used as a default unless overridden per torrent.
    /// It will run a DHT server/client, a TCP listener and .
    #[inline(never)]
    pub fn new(default_output_folder: PathBuf) -> BoxFuture<'static, anyhow::Result<Arc<Self>>> {
        Self::new_with_opts(default_output_folder, SessionOptions::default())
    }

    pub fn cancellation_token(&self) -> &CancellationToken {
        &self.cancellation_token
    }

    pub fn client_name_and_version(&self) -> &str {
        &self.client_name_and_version
    }

    /// Create a new session with options.
    #[inline(never)]
    pub fn new_with_opts(
        default_output_folder: PathBuf,
        mut opts: SessionOptions,
    ) -> BoxFuture<'static, anyhow::Result<Arc<Self>>> {
        async move {
            let restore_queued = opts.restore_queued;
            let peer_id = opts
                .peer_id
                .unwrap_or_else(|| generate_azereus_style(*b"rQ", crate_version!()));
            let token = opts.cancellation_token.take().unwrap_or_default();

            #[cfg(feature = "disable-upload")]
            if opts.disable_upload {
                warn!("uploading disabled");
            }

            let bind_device = match opts.bind_device_name.as_ref() {
                Some(name) => Some(
                    BindDevice::new_from_name(name)
                        .with_context(|| format!("error creating bind device {name}"))?,
                ),
                None => None,
            };

            let listen_result = if let Some(listen_opts) = opts.listen.take() {
                Some(
                    listen_opts
                        .start(
                            opts.root_span.as_ref().and_then(|s| s.id()),
                            token.child_token(),
                            bind_device.as_ref(),
                        )
                        .await
                        .context("error starting listeners")?,
                )
            } else {
                None
            };

            let dht = if let Some(dht_config) = opts.dht.take() {
                let dht = if let Some(persistence_config) = dht_config.persistence {
                    PersistentDht::create(
                        persistence_config,
                        dht_config.port,
                        opts.ipv4_only,
                        dht_config.bootstrap_addrs,
                        Some(token.clone()),
                        bind_device.as_ref(),
                    )
                    .await
                    .context("error initializing persistent DHT")?
                } else {
                    let listen_addr = dht_listen_addr(dht_config.port, None, opts.ipv4_only);
                    DhtBuilder::with_config(DhtConfig {
                        bootstrap_addrs: dht_config.bootstrap_addrs,
                        cancellation_token: Some(token.child_token()),
                        bind_device: bind_device.as_ref(),
                        listen_addr: Some(listen_addr),
                        ..Default::default()
                    })
                    .await
                    .context("error initializing DHT")?
                };

                Some(dht)
            } else {
                None
            };
            let peer_opts = opts
                .connect
                .as_ref()
                .and_then(|p| p.peer_opts)
                .unwrap_or_default();

            async fn persistence_factory(
                opts: &SessionOptions,
                spawner: BlockingSpawner,
            ) -> anyhow::Result<(
                Option<Arc<dyn SessionPersistenceStore>>,
                Arc<dyn BitVFactory>,
            )> {
                macro_rules! make_result {
                    ($store:expr) => {
                        if opts.fastresume {
                            Ok((Some($store.clone()), $store))
                        } else {
                            Ok((Some($store), Arc::new(NonPersistentBitVFactory {})))
                        }
                    };
                }

                match &opts.persistence {
                    Some(SessionPersistenceConfig::Json { folder }) => {
                        let folder = match folder.as_ref() {
                            Some(f) => f.clone(),
                            None => SessionPersistenceConfig::default_json_persistence_folder()?,
                        };

                        let s = Arc::new(
                            JsonSessionPersistenceStore::new(folder, spawner)
                                .await
                                .context("error initializing JsonSessionPersistenceStore")?,
                        );

                        make_result!(s)
                    }
                    #[cfg(feature = "postgres")]
                    Some(SessionPersistenceConfig::Postgres { connection_string }) => {
                        use crate::session_persistence::postgres::PostgresSessionStorage;
                        let p = Arc::new(PostgresSessionStorage::new(connection_string).await?);
                        make_result!(p)
                    }
                    None => Ok((None, Arc::new(NonPersistentBitVFactory {}))),
                }
            }

            const DEFAULT_BLOCKING_THREADS_IF_NOT_SET: usize = 8;
            let spawner = BlockingSpawner::new(
                opts.runtime_worker_threads
                    .unwrap_or(DEFAULT_BLOCKING_THREADS_IF_NOT_SET),
            );

            let (persistence, bitv_factory) = persistence_factory(&opts, spawner.clone())
                .await
                .context("error initializing session persistence store")?;

            let proxy_url = opts.connect.as_ref().and_then(|s| s.proxy_url.as_ref());
            let proxy_config = match proxy_url {
                Some(pu) => Some(
                    SocksProxyConfig::parse(pu)
                        .with_context(|| format!("error parsing proxy url {pu}"))?,
                ),
                None => None,
            };

            let client_name_and_version = opts
                .client_name_and_version
                .unwrap_or_else(|| crate::client_name_and_version().to_owned());

            let reqwest_client = {
                let builder = if let Some(proxy_url) = proxy_url {
                    let proxy = reqwest::Proxy::all(proxy_url)
                        .context("error creating socks5 proxy for HTTP")?;
                    reqwest::Client::builder().proxy(proxy)
                } else {
                    #[allow(unused_mut)]
                    let mut b = reqwest::Client::builder();
                    #[cfg(not(windows))]
                    if let Some(bd) = opts.bind_device_name.as_ref() {
                        b = b.interface(bd);
                    }
                    b
                };

                builder
                    .user_agent(&client_name_and_version)
                    .build()
                    .context("error building HTTP(S) client")?
            };

            let stream_connector = Arc::new(
                StreamConnector::new(StreamConnectorArgs {
                    encryption_policy: opts.peer_encryption,
                    enable_tcp: opts.connect.as_ref().map(|c| c.enable_tcp).unwrap_or(true),
                    socks_proxy_config: proxy_config,
                    utp_socket: listen_result.as_ref().and_then(|l| l.utp_socket.clone()),
                    bind_device: bind_device.clone(),
                    ipv4_only: opts.ipv4_only,
                })
                .await
                .context("error creating stream connector")?,
            );

            let blocklist = if let Some(blocklist_url) = opts.blocklist_url {
                info!(url = blocklist_url, "loading p2p blocklist");
                let bl = IpRanges::load_from_url(&blocklist_url)
                    .await
                    .with_context(|| format!("error reading blocklist from {blocklist_url}"))?;
                info!(len = bl.len(), "loaded blocklist");
                bl
            } else {
                IpRanges::default()
            };

            let allowlist = if let Some(allowlist_url) = opts.allowlist_url {
                info!(url = allowlist_url, "loading p2p allowlist");
                let al = IpRanges::load_from_url(&allowlist_url)
                    .await
                    .with_context(|| format!("error reading allowlist from {allowlist_url}"))?;
                info!(len = al.len(), "loaded allowlist");
                Some(al)
            } else {
                None
            };

            let udp_tracker_client = UdpTrackerClient::new(token.clone(), bind_device.as_ref())
                .await
                .context("error creating UDP tracker client")?;

            let lsd = {
                if opts.disable_local_service_discovery {
                    None
                } else {
                    LocalServiceDiscovery::new(LocalServiceDiscoveryOptions {
                        cancel_token: token.clone(),
                        bind_device: bind_device.as_ref(),
                        ..Default::default()
                    })
                    .await
                    .inspect_err(|e| warn!("error starting local service discovery: {e:#}"))
                    .ok()
                }
            };

            let session = Arc::new(Self {
                persistence,
                bitv_factory,
                peer_id,
                dht,
                peer_opts,
                spawner: spawner.clone(),
                output_folder: default_output_folder,
                next_id: AtomicUsize::new(0),
                db: RwLock::new(Default::default()),
                _cancellation_token_drop_guard: token.clone().drop_guard(),
                cancellation_token: token,
                shutdown_tasks: parking_lot::Mutex::new(Vec::new()),
                shutdown_task_failed: std::sync::atomic::AtomicBool::new(false),
                announce_port: listen_result.as_ref().and_then(|l| l.announce_port),
                listen_addr: listen_result.as_ref().map(|l| l.addr),
                default_storage_factory: opts.default_storage_factory,
                reqwest_client,
                connector: stream_connector,
                root_span: opts.root_span,
                stats: Arc::new(SessionStats::new()),
                concurrent_initialize_semaphore: Arc::new(tokio::sync::Semaphore::new(
                    opts.concurrent_init_limit.unwrap_or(3),
                )),
                udp_tracker_client,
                ratelimits: Limits::new(opts.ratelimits),
                ipv4_only: opts.ipv4_only,
                trackers: opts.trackers,
                disable_trackers: opts.disable_trackers,
                peer_limit: opts.peer_limit,
                client_name_and_version,

                #[cfg(feature = "disable-upload")]
                _disable_upload: opts.disable_upload,
                blocklist,
                allowlist,
                lsd,
            });

            if let Some(mut listen) = listen_result {
                if let Some(tcp) = listen.tcp_socket.take() {
                    let max_pending_incoming_handshake_checks =
                        listen.max_pending_incoming_handshake_checks;
                    session.spawn(
                        debug_span!(parent: session.rs(), "tcp_listen", addr = ?listen.addr),
                        "tcp_listen",
                        {
                            let this = session.clone();
                            async move {
                                this.task_listener(tcp, max_pending_incoming_handshake_checks)
                                    .await
                            }
                        },
                    );
                }
                if let Some(utp) = listen.utp_socket.take() {
                    let max_pending_incoming_handshake_checks =
                        listen.max_pending_incoming_handshake_checks;
                    session.spawn(
                        debug_span!(parent: session.rs(), "utp_listen", addr = ?listen.addr),
                        "utp_listen",
                        {
                            let this = session.clone();
                            async move {
                                this.task_listener(utp, max_pending_incoming_handshake_checks)
                                    .await
                            }
                        },
                    );
                }
                if listen.enable_upnp_port_forwarding
                    && let Some(announce_port) = listen.announce_port
                {
                    info!(port = announce_port, "starting UPnP port forwarder");
                    let bind_device = bind_device.clone();
                    session.spawn(
                        debug_span!(parent: session.rs(), "upnp_forward", port = announce_port),
                        "upnp_forward",
                        Self::task_upnp_port_forwarder(announce_port, bind_device),
                    );
                }
            }

            if let Some(persistence) = session.persistence.as_ref() {
                info!("will use {persistence:?} for session persistence");

                let mut ps = persistence.stream_all().await?;
                let mut added_all = false;
                let mut futs = FuturesUnordered::new();

                while !added_all || !futs.is_empty() {
                    // NOTE: this closure exists purely to workaround rustfmt screwing up when inlining it.
                    let add_torrent_span = |info_hash: &Id20| -> tracing::Span {
                        debug_span!(parent: session.rs(), "add_torrent", info_hash=?info_hash)
                    };
                    tokio::select! {
                        Some(res) = futs.next(), if !futs.is_empty() => {
                            if let Err(e) = res {
                                error!("error adding torrent to session: {e:#}");
                            }
                        }
                        st = ps.next(), if !added_all => {
                            match st {
                                Some(st) => {
                                    let (id, st) = st?;
                                    let span = add_torrent_span(st.info_hash());
                                    let (add_torrent, mut opts) = st.into_add_torrent()?;
                                    opts.preferred_id = Some(id);
                                    opts.queued = opts.queued || restore_queued;
                                    let fut = session.add_torrent(add_torrent, Some(opts));
                                    let fut = fut.instrument(span);
                                    futs.push(fut);
                                },
                                None => added_all = true
                            };
                        }
                    };
                }
            }

            session.start_speed_estimator_updater();

            Ok(session)
        }
        .boxed()
    }

    async fn check_incoming_connection(
        self: Arc<Self>,
        addr: SocketAddr,
        kind: ConnectionKind,
        mut reader: BoxAsyncReadVectored,
        writer: BoxAsyncWrite,
    ) -> anyhow::Result<(Arc<TorrentStateLive>, CheckedIncomingConnection)> {
        let rwtimeout = self
            .peer_opts
            .read_write_timeout
            .unwrap_or_else(|| Duration::from_secs(10));

        let incoming_ip = addr.ip();
        if self.blocklist.has(incoming_ip) {
            self.stats
                .counters
                .blocked_incoming
                .fetch_add(1, Ordering::Relaxed);
            bail!("Incoming ip {incoming_ip} is in blocklist");
        }
        if self.allowlist.as_ref().is_some_and(|l| !l.has(incoming_ip)) {
            self.stats
                .counters
                .blocked_incoming
                .fetch_add(1, Ordering::Relaxed);
            bail!("Incoming ip {incoming_ip} is not in allowlist");
        }

        use crate::vectored_traits::AsyncReadVectoredIntoCompat;
        use tokio::io::AsyncReadExt;
        let policy = self.connector.encryption_policy;
        let negotiated = tokio::time::timeout(rwtimeout, async {
            let mut first = [0; 20];
            reader.read_exact(&mut first).await?;
            let _permit = if first != *crate::mse::PLAIN_PREFIX {
                Some(self.connector.incoming_crypto_semaphore.acquire().await?)
            } else {
                None
            };
            crate::mse::incoming_after_prefix(
                reader,
                writer,
                first,
                |identifier| {
                    for torrent in self.db.read().torrents.values() {
                        let key = torrent.info_hash().0;
                        if crate::mse::stream_key_hash(&key) == identifier {
                            return Some(key);
                        }
                        if let Ok(Some(key)) = torrent.with_metadata(|m| {
                            m.info
                                .info()
                                .v2
                                .as_ref()
                                .filter(|v| v.is_hybrid())
                                .map(|_| m.v1_info_hash.0)
                        }) && crate::mse::stream_key_hash(&key) == identifier
                        {
                            return Some(key);
                        }
                    }
                    None
                },
                policy,
            )
            .await
            .map_err(anyhow::Error::from)
        })
        .await
        .context("incoming MSE/peer handshake timed out")??;
        let stream_key = negotiated.info_hash;
        let encrypted = negotiated.encrypted;
        if stream_key.is_some() {
            debug!(
                rc4 = negotiated.encrypted,
                "MSE incoming negotiation completed"
            );
        }
        let mut reader: BoxAsyncReadVectored = Box::new(negotiated.read.into_vectored_compat());
        let writer: BoxAsyncWrite = Box::new(negotiated.write);
        let mut read_buf = ReadBuf::new();
        let mut h = read_buf
            .read_handshake(&mut reader, rwtimeout)
            .await
            .context("error reading handshake")?;
        trace!("received handshake from {addr}: {:?}", h);
        if let Some(key) = stream_key {
            anyhow::ensure!(
                h.info_hash.0 == key,
                "MSE stream key and BitTorrent handshake identify different torrents"
            );
        }

        if h.peer_id == self.peer_id {
            bail!("seems like we are connecting to ourselves, ignoring");
        }

        let (id, torrent) = self
            .db
            .read()
            .torrents
            .iter()
            .find(|(_, t)| {
                t.info_hash() == h.info_hash
                    || t.with_metadata(|m| {
                        m.info.info().v2.as_ref().is_some_and(|v| v.is_hybrid())
                            && m.v1_info_hash == h.info_hash
                    })
                    .unwrap_or(false)
            })
            .map(|(id, t)| (*id, t.clone()))
            .with_context(|| format!("didn't find a matching torrent {:?}", h.info_hash))?;

        // A v1 hybrid handshake may explicitly offer a BEP 52 upgrade.
        // Normalize only after matching both the v1 identity and the upgrade bit.
        if h.reserved & 0x10 != 0 {
            h.info_hash = torrent.info_hash();
        }
        let live = torrent
            .live_wait_initializing(Duration::from_secs(5))
            .await
            .with_context(|| format!("torrent {id} is not live, ignoring connection"))?;

        Ok((
            live,
            CheckedIncomingConnection {
                encrypted,
                addr,
                reader,
                writer,
                kind,
                handshake: h,
                read_buf,
            },
        ))
    }

    async fn task_listener<A: Accept>(
        self: Arc<Self>,
        l: A,
        max_pending_incoming_handshake_checks: usize,
    ) -> anyhow::Result<()> {
        let mut futs = FuturesUnordered::new();
        let session = Arc::downgrade(&self);
        drop(self);

        loop {
            tokio::select! {
                r = l.accept(), if futs.len() < max_pending_incoming_handshake_checks => {
                    match r {
                        Ok((addr, (read, write))) => {
                            trace!("accepted connection from {addr}");
                            let session = session.upgrade().context("session is dead")?;
                            let span = debug_span!(parent: session.rs(), "incoming", addr=%addr);
                            futs.push(
                                session.check_incoming_connection(addr, A::KIND, Box::new(read), Box::new(write))
                                    .map_err(|e| {
                                        debug!("error checking incoming connection: {e:#}");
                                        e
                                    })
                                    .instrument(span)
                            );
                        }
                        Err(e) => {
                            warn!("error accepting: {e:#}");
                            // Whatever is the reason, ensure we are not stuck trying to
                            // accept indefinitely.
                            tokio::time::sleep(Duration::from_secs(10)).await;
                            continue
                        }
                    }
                },
                Some(Ok((live, checked))) = futs.next(), if !futs.is_empty() => {
                    let (addr, kind) = (checked.addr, checked.kind);
                    if let Err(e) = live.add_incoming_peer(checked) {
                        warn!(?addr, ?kind, "error handing over incoming connection: {e:#}");
                    }
                },
            }
        }
    }

    async fn task_upnp_port_forwarder(
        port: u16,
        bind_device: Option<BindDevice>,
    ) -> anyhow::Result<()> {
        let pf = librqbit_upnp::UpnpPortForwarder::new(vec![port], None, bind_device)?;
        pf.run_forever().await
    }

    pub fn get_dht(&self) -> Option<&Dht> {
        self.dht.as_ref()
    }

    fn merge_peer_opts(&self, other: Option<PeerConnectionOptions>) -> PeerConnectionOptions {
        let other = match other {
            Some(o) => o,
            None => self.peer_opts,
        };
        PeerConnectionOptions {
            connect_timeout: other.connect_timeout.or(self.peer_opts.connect_timeout),
            read_write_timeout: other
                .read_write_timeout
                .or(self.peer_opts.read_write_timeout),
            keep_alive_interval: other
                .keep_alive_interval
                .or(self.peer_opts.keep_alive_interval),
        }
    }

    /// Spawn a task in the context of the session.
    #[track_caller]
    pub fn spawn(
        &self,
        span: tracing::Span,
        name: impl Into<Cow<'static, str>>,
        fut: impl std::future::Future<Output = anyhow::Result<()>> + Send + 'static,
    ) {
        let task = spawn_with_cancel(span, name, self.cancellation_token.clone(), fut);
        self.track_task(task);
    }

    /// Retain task ownership until shutdown, including live torrent generations
    /// that have since been paused, rechecked, or removed from the database.
    pub(crate) fn track_task(&self, task: tokio::task::JoinHandle<()>) {
        let mut tasks = self.shutdown_tasks.lock();
        if tasks.len() >= 128 {
            let mut pending = Vec::with_capacity(tasks.len());
            for existing in tasks.drain(..) {
                if existing.is_finished() {
                    if let Some(Err(_)) = existing.now_or_never() {
                        self.shutdown_task_failed.store(true, Ordering::Relaxed);
                    }
                } else {
                    pending.push(existing);
                }
            }
            *tasks = pending;
        }
        tasks.push(task);
    }

    pub(crate) fn rs(&self) -> Option<tracing::Id> {
        self.root_span.as_ref().and_then(|s| s.id())
    }

    /// Stop the session and all managed tasks.
    pub async fn stop(&self) {
        if let Err(error) = self.stop_and_wait().await {
            tracing::error!(?error, "session shutdown failed");
        }
    }

    /// SeedHop: cancellation barrier for all tasks spawned through this session.
    /// The caller must also tear down its Tokio runtime before releasing its disk lock.
    pub async fn stop_and_wait(&self) -> anyhow::Result<()> {
        let torrents = self
            .db
            .read()
            .torrents
            .values()
            .cloned()
            .collect::<Vec<_>>();
        for torrent in torrents {
            if let Err(e) = torrent.pause() {
                debug!("error pausing torrent: {e:#}");
            }
        }
        self.cancellation_token.cancel();
        // Joining a parent can register further child tasks. Drain until all
        // generations have completed, never holding the registry lock over await.
        loop {
            let tasks = std::mem::take(&mut *self.shutdown_tasks.lock());
            if tasks.is_empty() {
                break;
            }
            for task in tasks {
                if task.await.is_err() {
                    self.shutdown_task_failed.store(true, Ordering::Relaxed);
                }
            }
        }
        anyhow::ensure!(
            !self.shutdown_task_failed.load(Ordering::Relaxed),
            "session task panicked during shutdown"
        );
        Ok(())
    }

    /// Run a callback given the currently managed torrents.
    pub fn with_torrents<R>(
        &self,
        callback: impl Fn(&mut dyn Iterator<Item = (TorrentId, &ManagedTorrentHandle)>) -> R,
    ) -> R {
        callback(&mut self.db.read().torrents.iter().map(|(id, t)| (*id, t)))
    }

    /// Add a torrent to the session.
    #[inline(never)]
    pub fn add_torrent<'a>(
        self: &'a Arc<Self>,
        add: AddTorrent<'a>,
        opts: Option<AddTorrentOptions>,
    ) -> BoxFuture<'a, anyhow::Result<AddTorrentResponse>> {
        async move {
            let mut opts = opts.unwrap_or_default();
            let add_res = match add {
                AddTorrent::Url(magnet) if magnet.starts_with("magnet:") || magnet.len() == 40 => {
                    let magnet = Magnet::parse(&magnet)
                        .context("provided path is not a valid magnet URL")?;
                    let v2_hash = magnet.as_id32();
                    let magnet_v1_hash = magnet.as_id20();
                    let info_hash = v2_hash
                        .map(|h| h.truncate_for_dht())
                        .or(magnet_v1_hash)
                        .context("magnet has no supported info hash")?;
                    if let Some(so) = magnet.get_select_only() {
                        // Only overwrite opts.only_files if user didn't specify
                        if opts.only_files.is_none() {
                            opts.only_files = Some(so);
                        }
                    }

                    InternalAddResult {
                        info_hash,
                        v2_hash,
                        magnet_v1_hash,
                        trackers: magnet
                            .trackers
                            .into_iter()
                            .filter_map(|t| url::Url::parse(&t).ok())
                            .collect(),
                        metadata: None,
                        name: magnet.name,
                    }
                }
                other => {
                    let torrent = match other {
                        AddTorrent::Url(url)
                            if url.starts_with("http://") || url.starts_with("https://") =>
                        {
                            torrent_from_url(&self.reqwest_client, &url).await?
                        }
                        AddTorrent::Url(url) => {
                            bail!(
                                "unsupported URL {:?}. Supporting magnet:, http:, and https",
                                url
                            )
                        }
                        AddTorrent::TorrentFileBytes(bytes) => {
                            torrent_from_bytes(bytes).context("error decoding torrent")?
                        }
                    };

                    let mut trackers = torrent
                        .meta
                        .iter_announce()
                        .unique()
                        .filter_map(|tracker| match std::str::from_utf8(tracker.as_ref()) {
                            Ok(url) => Some(url.to_owned()),
                            Err(_) => {
                                warn!("cannot parse tracker url as utf-8, ignoring");
                                None
                            }
                        })
                        .collect::<Vec<_>>();
                    if let Some(custom_trackers) = opts.trackers.clone() {
                        trackers.extend(custom_trackers);
                    }

                    InternalAddResult {
                        info_hash: torrent.meta.info_hash,
                        magnet_v1_hash: None,
                        v2_hash: torrent
                            .meta
                            .info
                            .data
                            .v2
                            .as_ref()
                            .map(|v| librqbit_core::Id32::new(v.info_hash())),
                        metadata: Some(TorrentMetadata::new(
                            torrent.meta.info.data.validate()?,
                            torrent.torrent_bytes,
                            torrent.meta.info.raw_bytes.0,
                        )?),
                        trackers: trackers
                            .iter()
                            .filter_map(|t| url::Url::parse(t).ok())
                            .collect(),
                        name: None,
                    }
                }
            };

            self.add_torrent_internal(add_res, opts).await
        }
        .instrument(debug_span!(parent: self.rs(), "add_torrent"))
        .boxed()
    }

    fn get_default_subfolder_for_torrent(
        &self,
        info: &ValidatedTorrentMetaV1Info<ByteBufOwned>,
        magnet_name: Option<&str>,
    ) -> anyhow::Result<Option<PathBuf>> {
        let files = info
            .iter_file_details()
            .map(|fd| Ok((fd.filename.to_pathbuf(), fd.len)))
            .collect::<anyhow::Result<Vec<(PathBuf, u64)>>>()?;
        if files.len() < 2 {
            return Ok(None);
        }

        if let Some(v2) = &info.info().v2 {
            return Ok(Some(PathBuf::from(v2.suggested_root_name())));
        }

        fn check_valid(pb: &Path) -> anyhow::Result<()> {
            if pb.components().any(|x| !matches!(x, Component::Normal(_))) {
                bail!("path traversal in torrent name detected")
            }
            Ok(())
        }

        if let Some(name) = info.name()
            && !name.is_empty()
        {
            let pb = PathBuf::from(name.as_ref());
            check_valid(&pb)?;
            return Ok(Some(pb));
        };
        if let Some(name) = magnet_name {
            let pb = PathBuf::from(name);
            check_valid(&pb)?;
            return Ok(Some(pb));
        }
        // Let the subfolder name be the longest filename
        let longest = files
            .iter()
            .max_by_key(|(_, l)| l)
            .unwrap()
            .0
            .file_stem()
            .context("can't determine longest filename")?;
        Ok::<_, anyhow::Error>(Some(PathBuf::from(longest)))
    }

    async fn add_torrent_internal(
        self: &Arc<Self>,
        add_res: InternalAddResult,
        mut opts: AddTorrentOptions,
    ) -> anyhow::Result<AddTorrentResponse> {
        let InternalAddResult {
            mut info_hash,
            v2_hash,
            magnet_v1_hash,
            metadata,
            mut trackers,
            name,
        } = add_res;

        let was_magnet = metadata.is_none();
        let private = metadata.as_ref().is_some_and(|m| m.info.info().private);

        let make_peer_rx = || {
            self.make_peer_rx(
                info_hash,
                trackers.clone(),
                !opts.paused && !opts.list_only,
                opts.force_tracker_interval,
                opts.initial_peers.clone().unwrap_or_default(),
                private,
            )
        };

        let mut seen_peers = Vec::new();

        let (metadata, mut peer_rx) = {
            match metadata {
                Some(metadata) => {
                    let mut peer_rx = None;
                    if !opts.paused && !opts.list_only {
                        peer_rx = make_peer_rx();
                    }
                    (metadata, peer_rx)
                }
                None => {
                    let peer_rx = make_peer_rx().context(
                        "no known way to resolve peers (no DHT, no trackers, no initial_peers)",
                    )?;
                    let resolved_magnet = self
                        .resolve_magnet(
                            info_hash,
                            v2_hash,
                            magnet_v1_hash,
                            peer_rx,
                            &trackers,
                            &opts,
                        )
                        .await?;
                    info_hash = resolved_magnet.discovery_hash;

                    // Add back seen_peers into the peer stream, as we consumed some peers
                    // while resolving the magnet.
                    seen_peers = resolved_magnet.seen_peers.clone();
                    let peer_rx = Some(
                        merge_streams(
                            resolved_magnet.peer_rx,
                            futures::stream::iter(resolved_magnet.seen_peers),
                        )
                        .boxed(),
                    );
                    (resolved_magnet.metadata, peer_rx)
                }
            }
        };

        // Retain the private tracker choice before converting it to an
        // unordered set. Both hybrid swarms, resume and reconnect must keep
        // using the same first tracker rather than choosing a different one.
        if metadata.info.info().private {
            trackers.truncate(1);
        }

        // A private flag learned from magnet metadata must also stop the
        // pre-metadata public discovery stream before live transfer begins.
        if was_magnet && metadata.info.info().private {
            peer_rx = None;
            if !opts.paused && !opts.list_only {
                peer_rx = self.make_peer_rx(
                    info_hash,
                    trackers.clone(),
                    true,
                    opts.force_tracker_interval,
                    opts.initial_peers.clone().unwrap_or_default(),
                    true,
                );
            }
        }
        let discovery_hash = info_hash;
        if let Some(v2) = &metadata.info.info().v2 {
            info_hash = librqbit_core::Id32::new(v2.info_hash()).truncate_for_dht();
        }
        trace!("Torrent metadata: {:#?}", &metadata.info.info());

        let only_files = compute_only_files(
            &metadata.info,
            opts.only_files,
            opts.only_files_regex,
            opts.list_only,
        )?;

        let output_folder = match (opts.output_folder, opts.sub_folder) {
            (None, None) => self.output_folder.join(
                self.get_default_subfolder_for_torrent(&metadata.info, name.as_deref())?
                    .unwrap_or_default(),
            ),
            (Some(o), None) => PathBuf::from(o),
            (Some(_), Some(_)) => {
                bail!("you can't provide both output_folder and sub_folder")
            }
            (None, Some(s)) => self.output_folder.join(s),
        };

        if opts.list_only {
            return Ok(AddTorrentResponse::ListOnly(ListOnlyResponse {
                info_hash,
                info: metadata.info,
                only_files,
                output_folder,
                seen_peers,
                torrent_bytes: metadata.torrent_bytes,
            }));
        }

        // Pin every v2 storage path, including unchanged names, so later
        // sanitizer versions cannot move a restored transfer implicitly.
        if let Some(v2) = &metadata.info.info().v2 {
            anyhow::ensure!(
                opts.file_names
                    .keys()
                    .all(|index| *index < v2.files().len()),
                "filename map contains an invalid file index"
            );
            for (index, file) in metadata.info.iter_file_details().enumerate() {
                opts.file_names
                    .entry(index)
                    .or_insert_with(|| file.filename.to_pathbuf());
            }
        }

        let storage_factory = opts
            .storage_factory
            .take()
            .or_else(|| self.default_storage_factory.as_ref().map(|f| f.clone_box()))
            .unwrap_or_else(|| FilesystemStorageFactory::default().boxed());

        let id = if let Some(id) = opts.preferred_id {
            id
        } else if let Some(p) = self.persistence.as_ref() {
            p.next_id().await?
        } else {
            self.next_id
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        };

        let _permit = self.spawner.semaphore().acquire_owned().await?;

        let (managed_torrent, metadata) = {
            let mut g = self.db.write();
            if let Some((id, handle)) = g.torrents.iter().find_map(|(eid, t)| {
                if t.info_hash() == info_hash || *eid == id {
                    Some((*eid, t.clone()))
                } else {
                    None
                }
            }) {
                let existing = handle
                    .with_metadata(|m| m.info.info().v2.as_ref().map(|v| v.info_hash()))
                    .ok()
                    .flatten();
                let incoming = metadata.info.info().v2.as_ref().map(|v| v.info_hash());
                anyhow::ensure!(
                    existing == incoming,
                    "truncated wire identity collision; full torrent identities differ"
                );
                return Ok(AddTorrentResponse::AlreadyManaged(id, handle));
            }

            let span = debug_span!(parent: self.rs(), "torrent", id);
            let peer_opts = self.merge_peer_opts(opts.peer_opts);
            let metadata = Arc::new(metadata);
            let minfo = Arc::new(ManagedTorrentShared {
                payload_counters: Default::default(),
                tracker_observations: Default::default(),
                tracker_event_sequence: Default::default(),
                tracker_events: Default::default(),
                id,
                span,
                info_hash,
                trackers: trackers.into_iter().collect(),
                spawner: self.spawner.clone(),
                peer_id: self.peer_id,
                storage_factory,
                options: ManagedTorrentOptions {
                    force_tracker_interval: opts.force_tracker_interval,
                    peer_connect_timeout: peer_opts.connect_timeout,
                    peer_read_write_timeout: peer_opts.read_write_timeout,
                    allow_overwrite: std::sync::atomic::AtomicBool::new(opts.overwrite),
                    output_folder: RwLock::new(output_folder),
                    file_names: RwLock::new(opts.file_names.clone()),
                    file_priorities: RwLock::new(opts.file_priorities.clone()),
                    sequential: opts.sequential,
                    first_last_pieces: opts.first_last_pieces,
                    ratelimits: opts.ratelimits,
                    initial_peers: opts.initial_peers.clone().unwrap_or_default(),
                    peer_limit: opts.peer_limit.or(self.peer_limit),
                    #[cfg(feature = "disable-upload")]
                    _disable_upload: self._disable_upload,
                },
                connector: self.connector.clone(),
                session: Arc::downgrade(self),
                magnet_name: name,
                client_name_and_version: self.client_name_and_version.clone(),
            });

            let initializing = Arc::new(TorrentStateInitializing::new(
                minfo.clone(),
                metadata.clone(),
                only_files.clone(),
                self.spawner
                    .block_in_place(|| minfo.storage_factory.create_and_init(&minfo, &metadata))?,
                false,
            ));
            let handle = Arc::new(ManagedTorrent {
                locked: RwLock::new(ManagedTorrentLocked {
                    paused: opts.paused,
                    queued: opts.queued,
                    state: ManagedTorrentState::Initializing(initializing),
                    only_files,
                }),
                state_change_notify: Notify::new(),
                shared: minfo,
                metadata: ArcSwapOption::new(Some(metadata.clone())),
            });

            g.add_torrent(handle.clone(), id);
            (handle, metadata)
        };

        if let Some(p) = self.persistence.as_ref()
            && let Err(e) = p.store(id, &managed_torrent).await
        {
            self.db.write().torrents.remove(&id);
            return Err(e);
        }

        let _e = managed_torrent.shared.span.clone().entered();
        managed_torrent
            .shared
            .options
            .allow_overwrite
            .store(true, std::sync::atomic::Ordering::Relaxed);

        let v1_hash = metadata.v1_info_hash;
        let hybrid = metadata
            .info
            .info()
            .v2
            .as_ref()
            .is_some_and(|v| v.is_hybrid());
        let initial_peers = managed_torrent.shared.options.initial_peers.clone();
        let mut peer_rx: Option<TorrentPeerStream> = peer_rx.map(|peers| {
            peers
                .map(move |addr| TorrentPeer {
                    addr,
                    info_hash: if hybrid && initial_peers.contains(&addr) {
                        v1_hash
                    } else {
                        discovery_hash
                    },
                })
                .boxed()
        });
        if hybrid && !opts.paused {
            let other_hash = if discovery_hash == info_hash {
                v1_hash
            } else {
                info_hash
            };
            let other = self
                .make_peer_rx(
                    other_hash,
                    managed_torrent.shared.trackers.iter().cloned().collect(),
                    true,
                    managed_torrent.shared.options.force_tracker_interval,
                    vec![],
                    metadata.info.info().private,
                )
                .map(|peers| {
                    peers
                        .map(move |addr| TorrentPeer {
                            addr,
                            info_hash: other_hash,
                        })
                        .boxed()
                });
            peer_rx = merge_two_optional_streams(peer_rx, other).map(|peers| peers.boxed());
        }
        managed_torrent
            .start(peer_rx, opts.paused)
            .context("error starting torrent")?;

        if let Some(name) = metadata.info.name() {
            info!(?name, "added torrent");
        }

        Ok(AddTorrentResponse::Added(id, managed_torrent))
    }

    pub fn get(&self, id: TorrentIdOrHash) -> Option<ManagedTorrentHandle> {
        match id {
            TorrentIdOrHash::Id(id) => self.db.read().torrents.get(&id).cloned(),
            TorrentIdOrHash::Hash(id) => self.db.read().torrents.iter().find_map(|(_, v)| {
                if v.info_hash() == id {
                    Some(v.clone())
                } else {
                    None
                }
            }),
        }
    }

    pub async fn delete(&self, id: TorrentIdOrHash, delete_files: bool) -> anyhow::Result<()> {
        let id = match id {
            TorrentIdOrHash::Id(id) => id,
            TorrentIdOrHash::Hash(h) => self
                .db
                .read()
                .torrents
                .values()
                .find_map(|v| {
                    if v.info_hash() == h {
                        Some(v.id())
                    } else {
                        None
                    }
                })
                .context("no such torrent in db")?,
        };
        let removed = self
            .db
            .write()
            .torrents
            .remove(&id)
            .with_context(|| format!("torrent with id {id} did not exist"))?;

        if let Err(e) = removed.pause() {
            debug!("error pausing torrent before deletion: {e:#}")
        }

        let metadata = removed.metadata.load_full().expect("TODO");

        let storage = removed
            .with_state_mut(|s| match s.take() {
                ManagedTorrentState::Initializing(p) => p.files.take().ok(),
                ManagedTorrentState::Paused(p) => Some(p.files),
                ManagedTorrentState::Live(l) => l
                    .pause()
                    // inspect_err not available in 1.75
                    .map_err(|e| {
                        warn!(?id, "error pausing torrent: {e:#}");
                        e
                    })
                    .ok()
                    .map(|p| p.files),
                _ => None,
            })
            .map(Ok)
            .unwrap_or_else(|| {
                removed
                    .shared
                    .storage_factory
                    .create(removed.shared(), &metadata)
            });

        if let Some(p) = self.persistence.as_ref() {
            if let Err(e) = p.delete(id).await {
                return Err(e).context("error deleting torrent from persistence database");
            } else {
                debug!(?id, "deleted torrent from persistence database")
            }
        }

        match (storage, delete_files) {
            (Err(e), true) => return Err(e).context("torrent deleted, but could not delete files"),
            (Ok(storage), true) => {
                debug!("will delete files");
                let mut infos = metadata.file_infos.clone();
                for (index, name) in removed.file_names() {
                    if let Some(info) = infos.get_mut(index) {
                        info.relative_filename = name;
                    }
                }
                remove_files_and_dirs(&infos, &storage)?;
                // SeedHop: explicit destinations belong to the user, not to the torrent.
                // Remove only empty directories inside the torrent, never the selected root.
            }
            (_, false) => {
                debug!("not deleting files")
            }
        };

        info!(id, "deleted torrent");
        Ok(())
    }

    pub(crate) fn make_peer_rx_managed_torrent(
        self: &Arc<Self>,
        t: &Arc<ManagedTorrent>,
        announce: bool,
    ) -> Option<TorrentPeerStream> {
        let is_private = t.with_metadata(|m| m.info.info().private).unwrap_or(false);
        let hybrid_hash = t
            .with_metadata(|m| {
                m.info
                    .info()
                    .v2
                    .as_ref()
                    .filter(|v| v.is_hybrid())
                    .map(|_| m.v1_info_hash)
            })
            .ok()
            .flatten();
        let canonical_hash = t.info_hash();
        let canonical = self
            .make_peer_rx(
                t.info_hash(),
                t.shared().trackers.iter().cloned().collect(),
                announce,
                t.shared().options.force_tracker_interval,
                if hybrid_hash.is_some() {
                    vec![]
                } else {
                    t.shared().options.initial_peers.clone()
                },
                is_private,
            )
            .map(|peers| {
                peers
                    .map(move |addr| TorrentPeer {
                        addr,
                        info_hash: canonical_hash,
                    })
                    .boxed()
            });
        let legacy = hybrid_hash.and_then(|info_hash| {
            self.make_peer_rx(
                info_hash,
                t.shared().trackers.iter().cloned().collect(),
                announce,
                t.shared().options.force_tracker_interval,
                t.shared().options.initial_peers.clone(),
                is_private,
            )
            .map(|peers| {
                peers
                    .map(move |addr| TorrentPeer { addr, info_hash })
                    .boxed()
            })
        });
        merge_two_optional_streams(canonical, legacy).map(|peers| peers.boxed())
    }

    // Get a peer stream from both DHT and trackers.
    fn make_peer_rx(
        self: &Arc<Self>,
        info_hash: Id20,
        mut trackers: Vec<url::Url>,
        announce: bool,
        force_tracker_interval: Option<Duration>,
        initial_peers: Vec<SocketAddr>,
        is_private: bool,
    ) -> Option<PeerStream> {
        let dht_rx = if is_private {
            None
        } else {
            self.dht.as_ref().map(|dht| {
                dht.get_peers(info_hash, if announce { self.announce_port } else { None })
            })
        };

        let lsd_rx = if is_private {
            None
        } else {
            self.lsd.as_ref().map(|lsd| {
                lsd.announce(info_hash, if announce { self.announce_port } else { None })
            })
        };

        if self.disable_trackers {
            trackers.clear();
        }

        if is_private {
            if trackers.len() > 1 {
                warn!(
                    ?info_hash,
                    "private trackers are not fully implemented, so using only the first tracker"
                );
                trackers.truncate(1);
            }
        } else if !self.disable_trackers {
            trackers.extend(self.trackers.iter().cloned());
        }

        let tracker_rx_stats = PeerRxTorrentInfo {
            info_hash,
            session: self.clone(),
        };
        let tracker_rx = TrackerComms::start(
            info_hash,
            self.peer_id,
            trackers.into_iter().collect(),
            Box::new(tracker_rx_stats),
            force_tracker_interval,
            self.announce_port().unwrap_or(4240),
            self.reqwest_client.clone(),
            self.udp_tracker_client.clone(),
        );

        let initial_peers_rx = if initial_peers.is_empty() {
            None
        } else {
            Some(futures::stream::iter(initial_peers))
        };
        merge_two_optional_streams(
            merge_two_optional_streams(
                merge_two_optional_streams(dht_rx, tracker_rx),
                initial_peers_rx,
            ),
            lsd_rx,
        )
    }

    pub async fn try_update_persistence_metadata(
        &self,
        handle: &ManagedTorrentHandle,
    ) -> anyhow::Result<()> {
        if let Some(p) = self.persistence.as_ref() {
            p.update_metadata(handle.id(), handle).await?;
        }
        Ok(())
    }

    pub async fn pause(&self, handle: &ManagedTorrentHandle) -> anyhow::Result<()> {
        handle.pause()?;
        self.try_update_persistence_metadata(handle).await?;
        Ok(())
    }

    pub async fn force_recheck(
        self: &Arc<Self>,
        handle: &ManagedTorrentHandle,
    ) -> anyhow::Result<()> {
        // Verify persistence first. Rechecking never changes selection or pause
        // intent and never removes/re-adds library membership.
        self.try_update_persistence_metadata(handle).await?;
        let paused = handle.prepare_recheck()?;
        let peers = if paused {
            None
        } else {
            self.make_peer_rx_managed_torrent(handle, true)
        };
        handle.start(peers, paused)
    }

    pub async fn set_queued(
        self: &Arc<Self>,
        handle: &ManagedTorrentHandle,
        queued: bool,
    ) -> anyhow::Result<()> {
        let old = handle.is_queued();
        handle.set_queued(queued)?;
        if let Err(error) = self.try_update_persistence_metadata(handle).await {
            handle.set_queued(old)?;
            if !old && !handle.is_paused() {
                handle.start(self.make_peer_rx_managed_torrent(handle, true), false)?;
            }
            return Err(error);
        }
        if !queued && !handle.is_paused() {
            handle.start(self.make_peer_rx_managed_torrent(handle, true), false)?;
        }
        Ok(())
    }

    pub async fn update_file_priorities(
        self: &Arc<Self>,
        handle: &ManagedTorrentHandle,
        priorities: Vec<u8>,
    ) -> anyhow::Result<()> {
        let old = handle.file_priorities();
        handle.quiesce()?;
        handle.replace_file_priorities(priorities)?;
        let result = self.try_update_persistence_metadata(handle).await;
        if result.is_err() {
            *handle.shared.options.file_priorities.write() = old;
        }
        handle.start(
            self.make_peer_rx_managed_torrent(handle, true),
            handle.is_paused(),
        )?;
        result
    }

    pub async fn unpause(self: &Arc<Self>, handle: &ManagedTorrentHandle) -> anyhow::Result<()> {
        let peer_rx = self.make_peer_rx_managed_torrent(handle, true);
        handle.start(peer_rx, false)?;
        self.try_update_persistence_metadata(handle).await?;
        Ok(())
    }

    pub async fn update_only_files(
        self: &Arc<Self>,
        handle: &ManagedTorrentHandle,
        only_files: &HashSet<usize>,
    ) -> anyhow::Result<()> {
        handle.update_only_files(only_files)?;
        self.try_update_persistence_metadata(handle).await?;
        Ok(())
    }

    pub fn listen_addr(&self) -> Option<SocketAddr> {
        self.listen_addr
    }

    pub fn announce_port(&self) -> Option<u16> {
        self.announce_port
    }

    async fn resolve_magnet(
        self: &Arc<Self>,
        info_hash: Id20,
        v2_hash: Option<librqbit_core::Id32>,
        v1_hash: Option<Id20>,
        peer_rx: PeerStream,
        trackers: &[url::Url],
        opts: &AddTorrentOptions,
    ) -> anyhow::Result<ResolveMagnetResult> {
        let identity = match v2_hash {
            Some(hash) => MetadataIdentity::v2(hash, v1_hash),
            None => MetadataIdentity::legacy(info_hash, None),
        };
        let privacy =
            MetadataPrivacy::with_trusted_peers(opts.initial_peers.clone().unwrap_or_default());
        let preferred =
            self.resolve_magnet_one(identity, peer_rx, trackers, opts.peer_opts, privacy.clone());
        let (Some(v1), Some(v2)) = (v1_hash, v2_hash) else {
            return preferred.await;
        };
        let Some(legacy_peers) = self.make_peer_rx(
            v1,
            trackers.to_vec(),
            !opts.paused && !opts.list_only,
            opts.force_tracker_interval,
            opts.initial_peers.clone().unwrap_or_default(),
            false,
        ) else {
            return preferred.await;
        };
        let alternate = self.resolve_magnet_one(
            MetadataIdentity::legacy(v1, Some(v2)),
            legacy_peers,
            trackers,
            opts.peer_opts,
            privacy,
        );
        tokio::pin!(preferred, alternate);
        let fatal = |error: &anyhow::Error| {
            error.chain().any(|cause| {
                cause.downcast_ref::<librqbit_core::v2::V2Error>()
                    == Some(&librqbit_core::v2::V2Error::HybridHashMismatch)
            })
        };
        tokio::select! {
            result = &mut preferred => match result {
                Ok(result) => Ok(result),
                Err(error) if fatal(&error) => Err(error),
                Err(error) => { debug!(%error, "v2 magnet metadata lookup ended; waiting for legacy swarm"); alternate.await }
            },
            result = &mut alternate => match result {
                Ok(result) => Ok(result),
                Err(error) if fatal(&error) => Err(error),
                Err(error) => { debug!(%error, "legacy magnet metadata lookup ended; waiting for v2 swarm"); preferred.await }
            },
        }
    }

    fn metadata_discovery(
        self: &Arc<Self>,
        identity: MetadataIdentity,
        trackers: &[url::Url],
        privacy: Arc<MetadataPrivacy>,
    ) -> MetadataDiscovery {
        let session = self.clone();
        let trackers = trackers.to_vec();
        let trusted = privacy.clone();
        MetadataDiscovery {
            privacy,
            private_peers: Arc::new(move |notice| {
                session
                    .make_peer_rx(
                        identity.wire_hash,
                        trackers.clone(),
                        false,
                        None,
                        trusted.private_initial(notice),
                        true,
                    )
                    .unwrap_or_else(|| futures::stream::empty().boxed())
            }),
        }
    }

    async fn resolve_magnet_one(
        self: &Arc<Self>,
        identity: MetadataIdentity,
        peer_rx: PeerStream,
        trackers: &[url::Url],
        peer_opts: Option<PeerConnectionOptions>,
        privacy: Arc<MetadataPrivacy>,
    ) -> anyhow::Result<ResolveMagnetResult> {
        let info_hash = identity.wire_hash;
        match read_metainfo_from_peer_receiver(
            self.peer_id,
            identity,
            Default::default(),
            peer_rx,
            Some(self.merge_peer_opts(peer_opts)),
            self.connector.clone(),
            self.client_name_and_version.clone(),
            Some(self.metadata_discovery(identity, trackers, privacy.clone())),
        )
        .await
        {
            ReadMetainfoResult::Found {
                source,
                mut info,
                mut info_bytes,
                mut rx,
                mut seen,
            } => {
                // The SHA-1-authenticated info dictionary commits to the full
                // v2 identity, but its provider may be strictly v1-only. Use a
                // separate v2 discovery stream for the missing Merkle layers.
                let legacy_pending = if identity.v2_wire {
                    None
                } else {
                    librqbit_core::v2::PendingMetainfo::from_info(info_bytes.as_ref())?
                };
                if !identity.v2_wire && identity.v2.is_some() {
                    anyhow::ensure!(
                        legacy_pending
                            .as_ref()
                            .is_some_and(|pending| pending.is_hybrid()),
                        "a dual-hash magnet must describe a hybrid torrent"
                    );
                }
                if let Some(pending) = legacy_pending {
                    if pending.next_request().is_none() {
                        // Files no larger than a piece need no external layer.
                        info.v2 = Some(Arc::new(pending.finish()?));
                    } else {
                        let full = librqbit_core::Id32::new(pending.info_hash());
                        drop(pending);
                        let private = info.private;
                        if private {
                            // Stop pre-metadata public discovery immediately.
                            // Only the authenticated provider already knows
                            // this dictionary; discard other public candidates.
                            rx = futures::stream::empty().boxed();
                            seen = std::collections::HashSet::from([source]);
                        }
                        let proofs = self
                            .make_peer_rx(
                                full.truncate_for_dht(),
                                trackers.to_vec(),
                                false,
                                None,
                                seen.iter().copied().collect(),
                                private,
                            )
                            .context("no way to discover v2 Merkle proof providers")?;
                        let proof_lookup = read_metainfo_from_peer_receiver(
                            self.peer_id,
                            MetadataIdentity::v2(full, Some(info_hash)),
                            vec![],
                            proofs,
                            Some(self.merge_peer_opts(peer_opts)),
                            self.connector.clone(),
                            self.client_name_and_version.clone(),
                            Some(self.metadata_discovery(
                                MetadataIdentity::v2(full, Some(info_hash)),
                                trackers,
                                privacy.clone(),
                            )),
                        );
                        let legacy_initial = seen.iter().copied().collect();
                        let legacy_info = info_bytes.clone();
                        let legacy_pieces = info.pieces.clone();
                        let legacy_privacy = privacy.clone();
                        let legacy_lookup = async move {
                            // Prefer cheap Merkle proofs. If no v2 provider
                            // answers promptly, derive them from authenticated
                            // legacy payload without allocating transfer files.
                            tokio::time::sleep(Duration::from_secs(2)).await;
                            let (private, legacy_initial) =
                                if let Some(notice) = legacy_privacy.current() {
                                    anyhow::ensure!(
                                        librqbit_core::v2::sha256(legacy_info.as_ref())
                                            == notice.dictionary_hash.0,
                                        "metadata changed after private dictionary authentication"
                                    );
                                    (true, legacy_privacy.private_initial(notice))
                                } else {
                                    (private, legacy_initial)
                                };
                            let peers = self
                                .make_peer_rx(
                                    info_hash,
                                    trackers.to_vec(),
                                    false,
                                    None,
                                    legacy_initial,
                                    private,
                                )
                                .context(
                                    "no legacy peers available for Merkle layer acquisition",
                                )?;
                            crate::peer_info_reader::legacy_layers::read_from_peers(
                                self.peer_id,
                                info_hash,
                                legacy_info,
                                legacy_pieces,
                                peers,
                                Some(self.merge_peer_opts(peer_opts)),
                                self.connector.clone(),
                                self.client_name_and_version.clone(),
                            )
                            .await
                        };
                        tokio::pin!(proof_lookup, legacy_lookup);
                        let result = tokio::select! {
                            result = &mut proof_lookup => match result {
                                ReadMetainfoResult::Found { .. } => Some(result),
                                ReadMetainfoResult::ChannelClosed { .. } => {
                                    let (layers, peers) = legacy_lookup.await?;
                                    info.v2 = Some(Arc::new(layers));
                                    seen.extend(peers);
                                    None
                                }
                            },
                            result = &mut legacy_lookup => match result {
                                Ok((layers, peers)) => {
                                    info.v2 = Some(Arc::new(layers));
                                    seen.extend(peers);
                                    None
                                }
                                Err(error) => {
                                    if error.chain().any(|cause| cause.downcast_ref::<librqbit_core::v2::V2Error>() == Some(&librqbit_core::v2::V2Error::HybridHashMismatch)) {
                                        return Err(error);
                                    }
                                    debug!(%error, "legacy layer acquisition ended; waiting for a v2 proof provider");
                                    Some(proof_lookup.await)
                                }
                            }
                        };
                        if let Some(result) = result {
                            match result {
                                ReadMetainfoResult::Found {
                                    info: verified,
                                    info_bytes: verified_bytes,
                                    seen: proof_peers,
                                    ..
                                } => {
                                    anyhow::ensure!(
                                        verified_bytes.as_ref() == info_bytes.as_ref(),
                                        "v2 proof source changed the authenticated info dictionary"
                                    );
                                    info = verified;
                                    info_bytes = verified_bytes;
                                    seen.extend(proof_peers);
                                }
                                ReadMetainfoResult::ChannelClosed { .. } => {
                                    bail!("no peer supplied the required v2 Merkle layers")
                                }
                            }
                        }
                    }
                }
                trace!(?info, "received result from DHT");
                let torrent_bytes = torrent_file_from_info_bytes(
                    info_bytes.as_ref(),
                    trackers,
                    info.v2.as_deref(),
                )?;
                let info = info.validate()?;
                let metadata = TorrentMetadata::new(info, torrent_bytes, info_bytes.0)?;
                debug!(wire_hash = %info_hash.as_string(), wire_v2 = identity.v2_wire, "magnet metadata authenticated");
                Ok(ResolveMagnetResult {
                    discovery_hash: info_hash,
                    metadata,
                    peer_rx: rx,
                    seen_peers: {
                        let seen = seen.into_iter().collect_vec();
                        for peer in &seen {
                            trace!(?peer, "seen")
                        }
                        seen
                    },
                })
            }
            ReadMetainfoResult::ChannelClosed { .. } => {
                bail!("input address stream exhausted, no way to discover torrent metainfo")
            }
        }
    }

    pub async fn create_and_serve_torrent(
        self: &Arc<Self>,
        path: &Path,
        opts: CreateTorrentOptions<'_>,
    ) -> Result<(CreateTorrentResult, ManagedTorrentHandle), ApiError> {
        if !path.exists() {
            return Err(ApiError::from((
                StatusCode::BAD_REQUEST,
                "path doesn't exist",
            )));
        }

        let torrent = create_torrent(path, opts, &self.spawner)
            .await
            .with_status(StatusCode::BAD_REQUEST)?;

        let bytes = torrent.as_bytes()?;

        let handle = self
            .add_torrent(
                AddTorrent::TorrentFileBytes(bytes.clone()),
                Some(AddTorrentOptions {
                    paused: false,
                    overwrite: true,
                    output_folder: Some(
                        torrent
                            .output_folder
                            .to_str()
                            .context("invalid utf-8")?
                            .to_owned(),
                    ),
                    ..Default::default()
                }),
            )
            .await?
            .into_handle()
            .context("error adding to session")?;

        Ok((torrent, handle))
    }
}

pub(crate) struct ResolveMagnetResult {
    pub discovery_hash: Id20,
    pub metadata: TorrentMetadata,
    pub peer_rx: PeerStream,
    pub seen_peers: Vec<SocketAddr>,
}

fn remove_files_and_dirs(infos: &FileInfos, files: &dyn TorrentStorage) -> anyhow::Result<()> {
    let mut failure = None;
    let mut all_dirs = HashSet::new();
    for (id, fi) in infos.iter().enumerate() {
        if fi.attrs.padding {
            continue;
        }
        let mut fname = &*fi.relative_filename;
        if let Err(e) = files.remove_file(id, fname) {
            warn!(?fi.relative_filename, error=?e, "could not delete file");
            failure.get_or_insert(e);
        } else {
            debug!(?fi.relative_filename, "deleted the file")
        }
        while let Some(parent) = fname.parent() {
            if parent != Path::new("") {
                all_dirs.insert(parent);
            }
            fname = parent;
        }
    }

    let all_dirs = {
        let mut v = all_dirs.into_iter().collect::<Vec<_>>();
        v.sort_unstable_by_key(|p| std::cmp::Reverse(p.as_os_str().len()));
        v
    };
    for dir in all_dirs {
        if let Err(e) = files.remove_directory_if_empty(dir) {
            warn!("error removing {dir:?}: {e:#}");
        } else {
            debug!("removed {dir:?}")
        }
    }
    if let Some(error) = failure {
        return Err(error).context("transfer removed, but some files could not be deleted");
    }
    Ok(())
}

// Ad adapter for converting stats into the format that tracker_comms accepts.
struct PeerRxTorrentInfo {
    info_hash: Id20,
    session: Arc<Session>,
}

impl tracker_comms::TorrentStatsProvider for PeerRxTorrentInfo {
    fn observe(&self, observation: tracker_comms::TrackerObservation) {
        self.session.with_torrents(|torrents| {
            for (_, mt) in torrents {
                let matches = mt.info_hash() == self.info_hash
                    || mt
                        .with_metadata(|m| m.v1_info_hash == self.info_hash)
                        .unwrap_or(false);
                if matches {
                    let key = format!(
                        "{}|{}|{}",
                        self.info_hash.as_string(),
                        observation.url,
                        observation.endpoint.as_deref().unwrap_or("")
                    );
                    let mut observations = mt.shared().tracker_observations.write();
                    // A resolved UDP endpoint supersedes its DNS-only placeholder.
                    if observation.endpoint.is_some() {
                        let root = format!("{}|{}|", self.info_hash.as_string(), observation.url);
                        observations.remove(&root);
                    }
                    let mut value = observation.clone();
                    if value.state == "contacting" || value.state == "resolving" {
                        if let Some((_, prior)) = observations.get(&key) {
                            value.last_announce = prior.last_announce;
                            value.message = prior.message.clone();
                            value.peers = prior.peers;
                            value.seeds = prior.seeds;
                            value.leechers = prior.leechers;
                        }
                    }
                    if matches!(value.state.as_str(), "error" | "warning") {
                        let sequence = mt
                            .shared()
                            .tracker_event_sequence
                            .fetch_add(1, Ordering::Relaxed)
                            + 1;
                        let timestamp = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs();
                        let mut events = mt.shared().tracker_events.lock();
                        events.push_back((
                            sequence,
                            timestamp,
                            value.url.clone(),
                            value.state.clone(),
                        ));
                        while events.len() > 200 {
                            events.pop_front();
                        }
                    }
                    observations.insert(key, (self.info_hash.as_string(), value));
                    break;
                }
            }
        });
    }
    fn get(&self) -> tracker_comms::TrackerCommsStats {
        let mt = self.session.with_torrents(|torrents| {
            for (_, mt) in torrents {
                if mt.info_hash() == self.info_hash
                    || mt
                        .with_metadata(|m| {
                            m.v1_info_hash == self.info_hash
                                && m.info.info().v2.as_ref().is_some_and(|v| v.is_hybrid())
                        })
                        .unwrap_or(false)
                {
                    return Some(mt.clone());
                }
            }
            None
        });
        let mt = match mt {
            Some(mt) => mt,
            None => {
                trace!(info_hash=?self.info_hash, "can't find torrent in the session, using default stats");
                return Default::default();
            }
        };
        let stats = mt.stats();
        let legacy_bytes = mt
            .with_metadata(|metadata| {
                if metadata.v1_info_hash != self.info_hash {
                    return None;
                }
                let v2 = metadata.info.info().v2.as_ref()?;
                let total = v2.hybrid_total_bytes()?;
                let count = |chunks: &crate::chunk_tracker::ChunkTracker| {
                    metadata
                        .lengths()
                        .iter_piece_infos()
                        .filter(|p| chunks.get_have_pieces().as_slice()[p.piece_index.get_usize()])
                        .map(|p| {
                            v2.hybrid_piece_size(p.piece_index.get() as u64)
                                .unwrap_or(0) as u64
                        })
                        .sum::<u64>()
                };
                let downloaded = match &mt.locked.read().state {
                    ManagedTorrentState::Live(live) => live
                        .lock_read("legacy tracker stats")
                        .get_chunks()
                        .map(count)
                        .unwrap_or(0),
                    ManagedTorrentState::Paused(paused) => count(&paused.chunk_tracker),
                    _ => 0,
                };
                Some((downloaded, total))
            })
            .ok()
            .flatten();

        use crate::torrent_state::stats::TorrentStatsState as TS;
        use tracker_comms::TrackerCommsStatsState as S;

        tracker_comms::TrackerCommsStats {
            downloaded_bytes: legacy_bytes.map(|v| v.0).unwrap_or(stats.progress_bytes),
            total_bytes: legacy_bytes.map(|v| v.1).unwrap_or(stats.total_bytes),
            uploaded_bytes: stats.uploaded_bytes,
            torrent_state: match stats.state {
                TS::Initializing { .. } => S::Initializing,
                TS::Live => S::Live,
                TS::Paused => S::Paused,
                TS::Error => S::None,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use buffers::ByteBuf;
    use itertools::Itertools;
    use librqbit_core::torrent_metainfo::{TorrentMetaV1, torrent_from_bytes};

    use super::torrent_file_from_info_bytes;

    #[test]
    fn test_torrent_file_from_info_and_bytes() {
        fn get_trackers(info: &TorrentMetaV1<ByteBuf>) -> Vec<url::Url> {
            info.iter_announce()
                .filter_map(|t| std::str::from_utf8(t.as_ref()).ok().map(|t| t.to_owned()))
                .filter_map(|t| t.parse().ok())
                .collect_vec()
        }

        let orig_full_torrent =
            include_bytes!("../resources/ubuntu-21.04-desktop-amd64.iso.torrent");
        let parsed = torrent_from_bytes(&orig_full_torrent[..]).unwrap();
        let parsed_trackers = get_trackers(&parsed);

        let generated_torrent =
            torrent_file_from_info_bytes(parsed.info.raw_bytes.as_ref(), &parsed_trackers, None)
                .unwrap();
        let generated_parsed = torrent_from_bytes(generated_torrent.as_ref()).unwrap();
        assert_eq!(parsed.info_hash, generated_parsed.info_hash);
        assert_eq!(parsed.info, generated_parsed.info);
        assert_eq!(parsed_trackers, get_trackers(&generated_parsed));
    }
}

use seedhop_core::{Engine, EngineConfig, TorrentRecord, adapter};
use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::watch,
    task::JoinHandle,
};

pub fn config(state: &Path, downloads: &Path) -> EngineConfig {
    EngineConfig {
        state_directory: state.to_string_lossy().into(),
        download_directory: downloads.to_string_lossy().into(),
        allow_network_discovery: false,
        listen_port: 0,
    }
}

pub async fn wait_for(
    engine: &Engine,
    predicate: impl Fn(&TorrentRecord) -> bool,
) -> TorrentRecord {
    tokio::time::timeout(Duration::from_secs(45), async {
        loop {
            let item = engine.snapshot().unwrap().torrents.remove(0);
            if predicate(&item) {
                return item;
            }
            assert_ne!(item.state, "error", "unexpected transfer failure: {item:?}");
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out: {:?}", engine.snapshot()))
}

/// A real engine seed behind a switchable TCP relay and HTTP tracker.
/// Dropping this guard also aborts accepted connections, including on test failure.
#[allow(dead_code)]
pub struct Swarm {
    pub seed: Arc<Engine>,
    pub metadata: Vec<u8>,
    pub announce: String,
    pub content: Vec<u8>,
    pub online: watch::Sender<bool>,
    pub offline_tracker_requests: Arc<AtomicUsize>,
    pub corrupt: Arc<AtomicBool>,
    pub corrupt_blocks: Arc<AtomicUsize>,
    pub corrupt_proofs: Arc<AtomicBool>,
    pub corrupted_proofs: Arc<AtomicUsize>,
    tasks: Vec<JoinHandle<()>>,
    _state: tempfile::TempDir,
    _files: tempfile::TempDir,
}

impl Swarm {
    pub async fn new(size: usize) -> Self {
        Self::with_metadata(size, |content| {
            adapter::create_fixture("payload.bin", content)
        })
        .await
    }

    pub async fn with_metadata(size: usize, make_metadata: impl FnOnce(&[u8]) -> Vec<u8>) -> Self {
        let state = tempfile::tempdir().unwrap();
        let files = tempfile::tempdir().unwrap();
        let content: Vec<u8> = (0..size).map(|i| (i % 251) as u8).collect();
        std::fs::write(files.path().join("payload.bin"), &content).unwrap();
        let seed = Engine::open(config(state.path(), files.path()))
            .await
            .unwrap();
        let seed_port = seed.snapshot().unwrap().listen_port;
        let relay = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let relay_port = relay.local_addr().unwrap().port();
        let tracker = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let announce = format!("http://{}/announce", tracker.local_addr().unwrap());
        let mut metadata = format!("d8:announce{}:{}", announce.len(), announce).into_bytes();
        metadata.extend_from_slice(&make_metadata(&content)[1..]);
        let (online, receiver) = watch::channel(true);
        let relay_online = receiver.clone();
        let corrupt = Arc::new(AtomicBool::new(false));
        let corrupt_blocks = Arc::new(AtomicUsize::new(0));
        let corrupt_proofs = Arc::new(AtomicBool::new(false));
        let corrupted_proofs = Arc::new(AtomicUsize::new(0));
        let relay_proofs = corrupt_proofs.clone();
        let relay_proof_count = corrupted_proofs.clone();
        let corrupt_relay = corrupt.clone();
        let corrupt_count = corrupt_blocks.clone();
        let relay_task = tokio::spawn(async move {
            let mut connections = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    accepted = relay.accept() => {
                        let (mut client, _) = accepted.unwrap();
                        let mut online = relay_online.clone();
                        if !*online.borrow_and_update() { continue; }
                        let corrupt = corrupt_relay.clone();
                        let count = corrupt_count.clone();
                        let proofs = relay_proofs.clone();
                        let proof_count = relay_proof_count.clone();
                        connections.spawn(async move {
                            tokio::select! {
                                _ = online.changed() => {},
                                _ = async {
                                    if let Ok(mut peer) = TcpStream::connect(("127.0.0.1", seed_port)).await {
                                        let (mut client_rx, mut client_tx) = client.split();
                                        let (mut peer_rx, mut peer_tx) = peer.split();
                                        let downstream = async {
                                            let mut handshake = [0; 68];
                                            peer_rx.read_exact(&mut handshake).await?;
                                            client_tx.write_all(&handshake).await?;
                                            loop {
                                                let length = peer_rx.read_u32().await?;
                                                if length > 1024 * 1024 { return Err::<(), _>(std::io::Error::other("oversized relay frame")); }
                                                let mut frame = vec![0; length as usize];
                                                peer_rx.read_exact(&mut frame).await?;
                                                let damaged = frame.first() == Some(&7) && frame.len() > 9 && corrupt.load(Ordering::SeqCst);
                                                let damaged_proof = frame.first() == Some(&22) && frame.len() > 49 && proofs.load(Ordering::SeqCst);
                                                if damaged { frame[9] ^= 1; }
                                                if damaged_proof { frame[49] ^= 1; }
                                                client_tx.write_u32(length).await?;
                                                client_tx.write_all(&frame).await?;
                                                if damaged { count.fetch_add(1, Ordering::SeqCst); }
                                                if damaged_proof { proof_count.fetch_add(1, Ordering::SeqCst); }
                                            }
                                        };
                                        tokio::select! {
                                            _ = tokio::io::copy(&mut client_rx, &mut peer_tx) => {},
                                            _ = downstream => {},
                                        }
                                    }
                                } => {}
                            }
                        });
                    },
                    _ = connections.join_next(), if !connections.is_empty() => {}
                }
            }
        });
        let offline_tracker_requests = Arc::new(AtomicUsize::new(0));
        let offline_requests = offline_tracker_requests.clone();
        let tracker_task = tokio::spawn(async move {
            loop {
                let (mut socket, _) = tracker.accept().await.unwrap();
                let mut request = [0; 8192];
                let count = tokio::time::timeout(Duration::from_secs(2), socket.read(&mut request))
                    .await
                    .unwrap()
                    .unwrap();
                let request = String::from_utf8_lossy(&request[..count]);
                let is_seed = request
                    .split(['?', '&', ' '])
                    .any(|part| part == format!("port={seed_port}"));
                if !*receiver.borrow() {
                    if !is_seed {
                        offline_requests.fetch_add(1, Ordering::SeqCst);
                    }
                    continue;
                }
                let mut body = if is_seed {
                    b"d8:intervali1e5:peers0:".to_vec()
                } else {
                    b"d8:intervali1e5:peers6:".to_vec()
                };
                if !is_seed {
                    body.extend_from_slice(&[127, 0, 0, 1]);
                    body.extend_from_slice(&relay_port.to_be_bytes());
                }
                body.push(b'e');
                let header = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = socket.write_all(header.as_bytes()).await;
                let _ = socket.write_all(&body).await;
            }
        });
        seed.add(
            metadata.clone(),
            files.path().to_string_lossy().into(),
            vec![0],
            false,
            true,
        )
        .await
        .unwrap();
        wait_for(&seed, |item| item.state == "seeding").await;
        Self {
            seed,
            metadata,
            announce,
            content,
            online,
            offline_tracker_requests,
            corrupt,
            corrupt_blocks,
            corrupt_proofs,
            corrupted_proofs,
            tasks: vec![relay_task, tracker_task],
            _state: state,
            _files: files,
        }
    }
}

impl Drop for Swarm {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}

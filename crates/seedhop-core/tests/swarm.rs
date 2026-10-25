use seedhop_core::{Engine, EngineConfig};
use sha1::{Digest, Sha1};
use std::{sync::Arc, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

fn config(state: &std::path::Path, downloads: &std::path::Path) -> EngineConfig {
    EngineConfig {
        state_directory: state.to_string_lossy().into(),
        download_directory: downloads.to_string_lossy().into(),
        allow_network_discovery: false,
        listen_port: 0,
    }
}

fn multifile(announce: &str, files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = format!("d8:announce{}:{}4:infod5:filesl", announce.len(), announce).into_bytes();
    let mut content = Vec::new();
    for (name, bytes) in files {
        out.extend_from_slice(format!("d6:lengthi{}e4:pathl", bytes.len()).as_bytes());
        for component in name.split('/') {
            out.extend_from_slice(format!("{}:{}", component.len(), component).as_bytes());
        }
        out.extend_from_slice(b"ee");
        content.extend_from_slice(bytes);
    }
    out.extend_from_slice(b"e4:name7:fixture12:piece lengthi16384e6:pieces");
    out.extend_from_slice(format!("{}:", content.chunks(16384).count() * 20).as_bytes());
    for piece in content.chunks(16384) {
        out.extend_from_slice(&Sha1::digest(piece));
    }
    out.extend_from_slice(b"ee");
    out
}

async fn wait_finished(engine: &Arc<Engine>) {
    tokio::time::timeout(Duration::from_secs(12), async {
        loop {
            let snapshot = engine.snapshot().unwrap();
            assert!(
                snapshot.torrents.iter().all(|t| t.state != "error"),
                "{snapshot:?}"
            );
            if snapshot.torrents.iter().all(|t| t.state == "seeding") {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("local swarm did not complete");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn multifile_download_verifies_bytes_and_respects_paused_start() {
    let seed_state = tempfile::tempdir().unwrap();
    let seed_files = tempfile::tempdir().unwrap();
    let down_state = tempfile::tempdir().unwrap();
    let down_files = tempfile::tempdir().unwrap();
    let first: Vec<u8> = (0..256 * 1024).map(|i| (i % 251) as u8).collect();
    let second: Vec<u8> = (0..100_001).map(|i| (i % 173) as u8).collect();
    std::fs::create_dir(seed_files.path().join("docs")).unwrap();
    std::fs::write(seed_files.path().join("first.bin"), &first).unwrap();
    std::fs::write(seed_files.path().join("docs/second.bin"), &second).unwrap();
    let tracker = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let announce = format!("http://{}/announce", tracker.local_addr().unwrap());
    let bytes = multifile(
        &announce,
        &[("first.bin", &first), ("docs/second.bin", &second)],
    );
    let seed = Engine::open(config(seed_state.path(), seed_files.path()))
        .await
        .unwrap();
    let seed_port = seed.snapshot().unwrap().listen_port;
    let relay = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let relay_port = relay.local_addr().unwrap().port();
    let requests = Arc::new(std::sync::Mutex::new(Vec::<u32>::new()));
    let relay_task = tokio::spawn({
        let requests = requests.clone();
        async move {
            loop {
                let (socket, _) = relay.accept().await.unwrap();
                let requests = requests.clone();
                tokio::spawn(async move {
                    let seed =
                        tokio::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, seed_port))
                            .await
                            .unwrap();
                    let (mut down_read, mut down_write) = socket.into_split();
                    let (mut seed_read, mut seed_write) = seed.into_split();
                    let forward = async {
                        let mut handshake = [0; 68];
                        down_read.read_exact(&mut handshake).await?;
                        seed_write.write_all(&handshake).await?;
                        loop {
                            let length = down_read.read_u32().await?;
                            if length > 1024 * 1024 {
                                return Err::<(), std::io::Error>(std::io::Error::other(
                                    "Oversized fixture message",
                                ));
                            }
                            let mut message = vec![0; length as usize];
                            down_read.read_exact(&mut message).await?;
                            if message.len() == 13 && message[0] == 6 {
                                requests
                                    .lock()
                                    .unwrap()
                                    .push(u32::from_be_bytes(message[1..5].try_into().unwrap()));
                            }
                            seed_write.write_u32(length).await?;
                            seed_write.write_all(&message).await?;
                        }
                    };
                    tokio::select! { _ = forward => {}, _ = tokio::io::copy(&mut seed_read,&mut down_write) => {} }
                });
            }
        }
    });
    let tracker_task = tokio::spawn(async move {
        loop {
            let (mut socket, _) = tracker.accept().await.unwrap();
            let mut request = [0; 8192];
            let _ = socket.read(&mut request).await;
            let mut body = b"d8:intervali1e5:peers6:".to_vec();
            body.extend_from_slice(&[127, 0, 0, 1]);
            body.extend_from_slice(&relay_port.to_be_bytes());
            body.push(b'e');
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = socket.write_all(header.as_bytes()).await;
            let _ = socket.write_all(&body).await;
        }
    });
    seed.add(
        bytes.clone(),
        seed_files.path().to_string_lossy().into(),
        vec![0, 1],
        false,
        true,
    )
    .await
    .unwrap();
    wait_finished(&seed).await;
    let down = Engine::open(config(down_state.path(), down_files.path()))
        .await
        .unwrap();
    let id = down
        .add(
            bytes,
            down_files.path().to_string_lossy().into(),
            vec![0, 1],
            true,
            false,
        )
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(down.snapshot().unwrap().torrents[0].downloaded_bytes, 0);
    down.set_file_priorities(id.clone(), vec![2, 0])
        .await
        .unwrap();
    down.set_paused(id, false).await.unwrap();
    wait_finished(&down).await;
    // The default alphabetical order starts docs/second.bin. High priority must
    // instead cause the first wire Request to target first.bin (pieces 0..16).
    assert!(*requests.lock().unwrap().first().unwrap() < 16);
    assert_eq!(
        std::fs::read(down_files.path().join("first.bin")).unwrap(),
        first
    );
    assert_eq!(
        std::fs::read(down_files.path().join("docs/second.bin")).unwrap(),
        second
    );
    assert_eq!(down.snapshot().unwrap().torrents[0].files.len(), 2);
    down.shutdown().await.unwrap();
    seed.shutdown().await.unwrap();
    tracker_task.abort();
    relay_task.abort();
}

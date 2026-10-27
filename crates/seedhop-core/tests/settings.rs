use seedhop_core::{Engine, EngineConfig, adapter};
use std::{path::Path, time::Duration};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn config(state: &Path, files: &Path) -> EngineConfig {
    EngineConfig {
        state_directory: state.display().to_string(),
        download_directory: files.display().to_string(),
        allow_network_discovery: false,
        listen_port: 0,
    }
}
async fn settled(engine: &Engine) {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let snapshot = engine.snapshot().unwrap();
            let assigned = snapshot
                .torrents
                .iter()
                .all(|t| t.state != "checking" && t.queue_position > 0);
            let wants_running = snapshot
                .torrents
                .iter()
                .any(|t| !t.paused_intent && t.downloaded_bytes < t.total_bytes);
            let admitted = snapshot
                .torrents
                .iter()
                .any(|t| t.state == "waiting" || t.state == "downloading");
            if assigned && (!wants_running || admitted) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn saved_policies_apply_only_after_restart_and_preserve_transfer_choices() {
    let state = tempfile::tempdir().unwrap();
    let files = tempfile::tempdir().unwrap();
    let cfg = config(state.path(), files.path());
    let mut engine = Engine::open(cfg.clone()).await.unwrap();
    assert_eq!(engine.snapshot().unwrap().peer_encryption, "allow");
    for (name, paused) in [("running", false), ("paused", true), ("queued", false)] {
        let id = engine
            .add(
                adapter::create_fixture(name, b"missing"),
                cfg.download_directory.clone(),
                vec![0],
                paused,
                true,
            )
            .await
            .unwrap();
        settled(&engine).await;
        engine.set_file_priorities(id, vec![2]).await.unwrap();
    }
    engine.set_queue(1).await.unwrap();
    settled(&engine).await;
    let before = engine.snapshot().unwrap();
    for policy in ["require", "prefer", "disabled", "allow"] {
        let active = engine.snapshot().unwrap().peer_encryption;
        engine.set_peer_encryption(policy.into()).await.unwrap();
        let saved = engine.snapshot().unwrap();
        assert_eq!(saved.peer_encryption, active);
        assert_eq!(saved.saved_peer_encryption, policy);
        engine.shutdown().await.unwrap();
        drop(engine);
        engine = Engine::open(cfg.clone()).await.unwrap();
        settled(&engine).await;
        let restored = engine.snapshot().unwrap();
        assert_eq!(restored.peer_encryption, policy);
        assert_eq!(restored.saved_peer_encryption, policy);
        assert_eq!(restored.max_active_downloads, 1);
        assert_eq!(restored.torrents, before.torrents);
    }
    engine.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn invalid_or_failed_saves_preserve_the_active_policy_and_remain_retryable() {
    let state = tempfile::tempdir().unwrap();
    let files = tempfile::tempdir().unwrap();
    let cfg = config(state.path(), files.path());
    let engine = Engine::open(cfg.clone()).await.unwrap();
    engine.set_peer_encryption("prefer".into()).await.unwrap();
    let saved = std::fs::read(state.path().join("connection-settings.json")).unwrap();
    assert!(engine.set_peer_encryption("unknown".into()).await.is_err());
    std::fs::create_dir(state.path().join("connection-settings.pending")).unwrap();
    assert!(engine.set_peer_encryption("require".into()).await.is_err());
    assert_eq!(engine.snapshot().unwrap().peer_encryption, "allow");
    assert_eq!(engine.snapshot().unwrap().saved_peer_encryption, "prefer");
    assert_eq!(
        std::fs::read(state.path().join("connection-settings.json")).unwrap(),
        saved
    );
    std::fs::remove_dir(state.path().join("connection-settings.pending")).unwrap();
    engine.set_peer_encryption("require".into()).await.unwrap();
    assert_eq!(engine.snapshot().unwrap().saved_peer_encryption, "require");
    engine.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unreadable_unknown_or_malformed_settings_are_preserved_at_startup() {
    for bytes in [
        b"{".as_slice(),
        br#"{"version":2,"peer_encryption":"allow"}"#,
        br#"{"version":1,"peer_encryption":"unknown"}"#,
    ] {
        let state = tempfile::tempdir().unwrap();
        let files = tempfile::tempdir().unwrap();
        let path = state.path().join("connection-settings.json");
        std::fs::write(&path, bytes).unwrap();
        assert!(
            Engine::open(config(state.path(), files.path()))
                .await
                .is_err()
        );
        assert_eq!(std::fs::read(path).unwrap(), bytes);
    }
    let state = tempfile::tempdir().unwrap();
    let files = tempfile::tempdir().unwrap();
    std::fs::create_dir(state.path().join("connection-settings.json")).unwrap();
    assert!(
        Engine::open(config(state.path(), files.path()))
            .await
            .is_err()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn persisted_required_policy_reaches_the_real_peer_listener() {
    use librqbit::mse::{self, EncryptionPolicy};
    let state = tempfile::tempdir().unwrap();
    let files = tempfile::tempdir().unwrap();
    let cfg = config(state.path(), files.path());
    let engine = Engine::open(cfg.clone()).await.unwrap();
    let id = engine
        .add(
            adapter::create_fixture("data", b"content"),
            cfg.download_directory.clone(),
            vec![0],
            false,
            true,
        )
        .await
        .unwrap();
    settled(&engine).await;
    let mut key = [0; 20];
    for (index, slot) in key.iter_mut().enumerate() {
        *slot = u8::from_str_radix(&id[index * 2..index * 2 + 2], 16).unwrap();
    }
    let mut handshake = vec![19];
    handshake.extend_from_slice(b"BitTorrent protocol");
    handshake.extend([0; 8]);
    handshake.extend(key);
    handshake.extend_from_slice(b"-SHtest-123456789012");
    engine.set_peer_encryption("require".into()).await.unwrap();
    // Saving must not silently alter the existing listener's policy.
    let mut peer =
        tokio::net::TcpStream::connect(("127.0.0.1", engine.snapshot().unwrap().listen_port))
            .await
            .unwrap();
    peer.write_all(&handshake).await.unwrap();
    let mut prefix = [0; 20];
    tokio::time::timeout(Duration::from_secs(5), peer.read_exact(&mut prefix))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&prefix, mse::PLAIN_PREFIX);
    drop(peer);
    engine.shutdown().await.unwrap();
    drop(engine);
    let engine = Engine::open(cfg).await.unwrap();
    settled(&engine).await;
    let port = engine.snapshot().unwrap().listen_port;
    let mut bare = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .unwrap();
    bare.write_all(&handshake).await.unwrap();
    let result = tokio::time::timeout(Duration::from_secs(5), bare.read(&mut [0; 1]))
        .await
        .unwrap();
    assert!(matches!(result, Ok(0)) || result.is_err());
    let peer = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .unwrap();
    let (read, write) = peer.into_split();
    let mut encrypted = mse::outgoing(read, write, key, &handshake, EncryptionPolicy::Require)
        .await
        .unwrap();
    assert!(encrypted.encrypted);
    tokio::time::timeout(
        Duration::from_secs(5),
        encrypted.read.read_exact(&mut prefix),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(&prefix, mse::PLAIN_PREFIX);
    engine.shutdown().await.unwrap();
}

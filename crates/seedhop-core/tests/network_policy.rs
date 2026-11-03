use seedhop_core::{
    Engine, EngineConfig, NetworkPolicy, adapter, network_interfaces, save_network_preferences,
};
use std::{path::Path, time::Duration};

fn config(state: &Path, files: &Path) -> EngineConfig {
    EngineConfig {
        state_directory: state.display().to_string(),
        download_directory: files.display().to_string(),
        allow_network_discovery: false,
        allow_local_network: true,
        listen_port: 0,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn network_change_drains_connections_preserves_intent_and_restores() {
    let state = tempfile::tempdir().unwrap();
    let files = tempfile::tempdir().unwrap();
    let cfg = config(state.path(), files.path());
    let engine = Engine::open(cfg.clone()).await.unwrap();
    for (name, paused) in [("active", false), ("paused", true)] {
        engine
            .add(
                adapter::create_fixture(name, b"not downloaded"),
                cfg.download_directory.clone(),
                vec![0],
                paused,
                true,
            )
            .await
            .unwrap();
    }
    tokio::time::sleep(Duration::from_millis(250)).await;
    let before = engine.snapshot().unwrap();
    let policy = NetworkPolicy::default();
    engine
        .set_network_policy(serde_json::to_string(&policy).unwrap())
        .await
        .unwrap();
    assert!(engine.snapshot().unwrap().network_blocked);
    assert!(
        engine
            .inspect(
                "blocked".into(),
                Some("magnet:?xt=urn:btih:0000000000000000000000000000000000000000".into()),
                vec![]
            )
            .await
            .is_err()
    );
    // A drained listener must not accept new peer connections.
    assert!(
        tokio::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, before.listen_port))
            .await
            .is_err()
    );
    engine.shutdown().await.unwrap();
    drop(engine);
    let restored = Engine::open(cfg).await.unwrap();
    let after = restored.snapshot().unwrap();
    assert!(!after.network_blocked);
    for torrent in before.torrents {
        let other = after.torrents.iter().find(|t| t.id == torrent.id).unwrap();
        assert_eq!(torrent.paused_intent, other.paused_intent);
        assert_eq!(torrent.files, other.files);
    }
    restored.shutdown().await.unwrap();
}

#[tokio::test]
async fn unknown_or_missing_interface_never_falls_back_or_overwrites_policy() {
    let state = tempfile::tempdir().unwrap();
    let files = tempfile::tempdir().unwrap();
    let interface = network_interfaces().unwrap().into_iter().next().unwrap();
    let mut policy = NetworkPolicy {
        version: 1,
        interface: Some(interface),
        dns_server: Some("1.1.1.1".parse().unwrap()),
        allow_local_network: true,
    };
    policy.interface.as_mut().unwrap().index = u32::MAX;
    let bytes = serde_json::to_vec(&policy).unwrap();
    std::fs::write(state.path().join("network-policy.json"), &bytes).unwrap();
    assert!(
        Engine::open(config(state.path(), files.path()))
            .await
            .is_err()
    );
    assert_eq!(
        std::fs::read(state.path().join("network-policy.json")).unwrap(),
        bytes
    );
    // Offline recovery is explicit and guarded by the library lease.
    save_network_preferences(
        state.path(),
        &serde_json::to_string(&NetworkPolicy::default()).unwrap(),
    )
    .unwrap();
    let engine = Engine::open(config(state.path(), files.path()))
        .await
        .unwrap();
    assert!(save_network_preferences(state.path(), "{}").is_err());
    engine.shutdown().await.unwrap();
}

#[test]
fn corrupt_unknown_and_incomplete_policy_never_defaults_to_direct() {
    let state = tempfile::tempdir().unwrap();
    for json in [
        "{",
        "{}",
        r#"{"version":2,"interface":null,"dnsServer":null}"#,
        r#"{"version":1,"interface":null,"dnsServer":"1.1.1.1"}"#,
        r#"{"version":1,"interface":null,"dnsServer":null,"typo":true}"#,
    ] {
        std::fs::write(state.path().join("network-policy.json"), json).unwrap();
        assert!(NetworkPolicy::load(state.path()).is_err());
    }
}

#[tokio::test]
async fn protection_save_blocks_legacy_readers_and_partial_save_cannot_start_directly() {
    let state = tempfile::tempdir().unwrap();
    let files = tempfile::tempdir().unwrap();
    save_network_preferences(
        state.path(),
        &serde_json::to_string(&NetworkPolicy::default()).unwrap(),
    )
    .unwrap();
    let path = state.path().join("connection-settings.json");
    let legacy: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(legacy["version"], 2); // Previous clients accept only version 1.
    let engine = Engine::open(config(state.path(), files.path()))
        .await
        .unwrap();
    engine.set_peer_encryption("require".into()).await.unwrap();
    engine.shutdown().await.unwrap();
    drop(engine);
    let saved: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(saved["version"], 2);
    let mut pending = saved.clone();
    pending["network_policy_pending"] = true.into();
    std::fs::write(&path, serde_json::to_vec(&pending).unwrap()).unwrap();
    assert!(
        Engine::open(config(state.path(), files.path()))
            .await
            .is_err()
    );
    std::fs::write(&path, serde_json::to_vec(&saved).unwrap()).unwrap();
    std::fs::remove_file(state.path().join("network-policy.json")).unwrap();
    assert!(
        Engine::open(config(state.path(), files.path()))
            .await
            .is_err()
    );
    let info: serde_json::Value =
        serde_json::from_str(&seedhop_core::network_preferences(state.path()).unwrap()).unwrap();
    assert!(info["blocked"].as_bool().unwrap());
    // Explicit offline recovery creates a complete policy before any new session.
    save_network_preferences(
        state.path(),
        &serde_json::to_string(&NetworkPolicy::default()).unwrap(),
    )
    .unwrap();
    let restored = Engine::open(config(state.path(), files.path()))
        .await
        .unwrap();
    assert_eq!(restored.snapshot().unwrap().peer_encryption, "require");
    restored.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn local_tracker_requests_require_explicit_permission() {
    for allow in [false, true] {
        let state = tempfile::tempdir().unwrap();
        let files = tempfile::tempdir().unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/announce", listener.local_addr().unwrap());
        let fixture = adapter::create_fixture("payload", b"missing data");
        let mut torrent = format!("d8:announce{}:{}", url.len(), url).into_bytes();
        torrent.extend_from_slice(&fixture[1..]);
        let mut cfg = config(state.path(), files.path());
        cfg.allow_local_network = false;
        if allow {
            let policy = NetworkPolicy {
                allow_local_network: true,
                ..Default::default()
            };
            save_network_preferences(state.path(), &serde_json::to_string(&policy).unwrap())
                .unwrap();
        }
        let engine = Engine::open(cfg.clone()).await.unwrap();
        engine
            .add(torrent, cfg.download_directory, vec![0], false, true)
            .await
            .unwrap();
        let connection = tokio::time::timeout(Duration::from_millis(500), listener.accept()).await;
        assert_eq!(connection.is_ok(), allow);
        drop(connection);
        engine.shutdown().await.unwrap();
    }
}

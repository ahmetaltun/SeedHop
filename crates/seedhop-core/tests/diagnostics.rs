use seedhop_core::{Engine, EngineConfig, adapter};
use std::time::Duration;

#[tokio::test]
async fn diagnostics_preserve_unknown_history_and_reused_files_are_not_network_traffic() {
    let state = tempfile::tempdir().unwrap();
    let files = tempfile::tempdir().unwrap();
    let config = EngineConfig {
        state_directory: state.path().to_string_lossy().into(),
        download_directory: files.path().to_string_lossy().into(),
        allow_network_discovery: false,
        listen_port: 0,
    };
    let bytes = adapter::create_fixture("existing.bin", b"already on disk");
    std::fs::write(files.path().join("existing.bin"), b"already on disk").unwrap();
    let engine = Engine::open(config.clone()).await.unwrap();
    let id = engine
        .add(
            bytes,
            config.download_directory.clone(),
            vec![0],
            true,
            true,
        )
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if engine.snapshot().unwrap().torrents[0].state == "paused" {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    let summary = engine.snapshot().unwrap().torrents.remove(0);
    assert_eq!(summary.downloaded_bytes, 15);
    assert_eq!(summary.network_downloaded_bytes, Some(0));
    assert_eq!(summary.network_uploaded_bytes, Some(0));
    let details = engine
        .torrent_details(id.clone(), "general".into(), false)
        .unwrap();
    assert_eq!(details.format, "v1");
    assert_eq!(details.v1_hash.as_deref(), Some(id.as_str()));
    assert!(details.v2_hash.is_none());
    assert!(!details.dht && !details.local_discovery);
    assert!(details.peers.is_empty() && details.trackers.is_empty());
    assert!(
        engine
            .torrent_details(id.clone(), "wrong".into(), false)
            .is_err()
    );
    engine.shutdown().await.unwrap();
    drop(engine);
    std::fs::remove_file(state.path().join("statistics.json")).unwrap();
    let restored = Engine::open(config).await.unwrap();
    let legacy = restored.snapshot().unwrap().torrents.remove(0);
    assert_eq!(legacy.network_downloaded_bytes, None);
    assert_eq!(legacy.network_uploaded_bytes, None);
    assert_eq!(legacy.added_at, None);
    assert_eq!(legacy.completed_at, None);
    restored.remove(id.clone(), false).await.unwrap();
    assert!(
        restored
            .torrent_details(id, "general".into(), false)
            .is_err()
    );
    restored.shutdown().await.unwrap();
}

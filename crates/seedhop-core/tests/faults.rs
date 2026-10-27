mod support;

use seedhop_core::{CoreError, Engine, adapter};
use std::{sync::atomic::Ordering, time::Duration};
use support::{Swarm, config, wait_for};
#[path = "support/volume.rs"]
mod volume;
use volume::{fault_volume, fill_volume, free_volume};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn unavailable_destination_preserves_library_and_recovers_after_reconnection() {
    let state = tempfile::tempdir().unwrap();
    let root = tempfile::tempdir().unwrap();
    let downloads = root.path().join("drive");
    let disconnected = root.path().join("disconnected");
    std::fs::create_dir(&downloads).unwrap();
    let cfg = config(state.path(), &downloads);
    let swarm = Swarm::new(256 * 1024).await;
    let engine = Engine::open(cfg.clone()).await.unwrap();
    let id = engine
        .add(
            swarm.metadata.clone(),
            cfg.download_directory.clone(),
            vec![0],
            true,
            false,
        )
        .await
        .unwrap();
    wait_for(&engine, |item| item.state == "paused").await;
    let session_path = state.path().join("session/session.json");
    let saved = std::fs::read(&session_path).unwrap();
    std::fs::rename(&downloads, &disconnected).unwrap();
    assert!(
        engine.set_paused(id.clone(), false).await.is_err(),
        "Resume must reject a missing destination"
    );
    assert_eq!(engine.snapshot().unwrap().torrents[0].state, "paused");
    assert_eq!(std::fs::read(&session_path).unwrap(), saved);
    assert!(
        !downloads.exists(),
        "a missing drive must never be recreated"
    );
    engine.shutdown().await.unwrap();
    drop(engine);

    // Keep the default directory available so the saved per-transfer path is checked.
    let mut cfg = cfg;
    cfg.download_directory = root.path().to_string_lossy().into();
    let error = Engine::open(cfg.clone())
        .await
        .err()
        .expect("restore should fail");
    assert!(
        matches!(&error, CoreError::Storage(message) if message.contains("Reconnect the drive")),
        "{error}"
    );
    assert_eq!(std::fs::read(&session_path).unwrap(), saved);
    assert!(!downloads.exists());
    println!(
        "Unavailable destination: Resume and restore rejected; saved session bytes unchanged."
    );
    std::fs::rename(&disconnected, &downloads).unwrap();
    let engine = Engine::open(cfg).await.unwrap();
    wait_for(&engine, |item| item.state == "paused").await;
    assert_eq!(engine.snapshot().unwrap().torrents[0].id, id);
    engine.set_paused(id, false).await.unwrap();
    wait_for(&engine, |item| item.state == "seeding").await;
    assert_eq!(
        std::fs::read(downloads.join("payload.bin")).unwrap(),
        swarm.content
    );
    println!("Reconnected destination: original transfer restored and 262144 bytes verified.");
    engine.shutdown().await.unwrap();
    swarm.seed.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn interrupted_peer_and_tracker_connections_recover_without_user_resume() {
    let state = tempfile::tempdir().unwrap();
    let downloads = tempfile::tempdir().unwrap();
    let swarm = Swarm::new(4 * 1024 * 1024).await;
    let cfg = config(state.path(), downloads.path());
    let engine = Engine::open(cfg.clone()).await.unwrap();
    engine.set_limits(512 * 1024, 0).await.unwrap();
    engine
        .add(
            swarm.metadata.clone(),
            cfg.download_directory,
            vec![0],
            false,
            false,
        )
        .await
        .unwrap();
    let before = wait_for(&engine, |item| item.downloaded_bytes >= 64 * 1024).await;
    assert!(before.downloaded_bytes < before.total_bytes);
    swarm.online.send(false).unwrap();
    let stalled = wait_for(&engine, |item| item.peers == 0).await;
    assert!(stalled.downloaded_bytes > 0 && stalled.downloaded_bytes < stalled.total_bytes);
    // Keep the outage until at least one real tracker announce also fails.
    tokio::time::timeout(Duration::from_secs(15), async {
        while swarm.offline_tracker_requests.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("tracker was not exercised during the outage");
    let offline = engine.snapshot().unwrap().torrents.remove(0);
    assert_eq!(offline.downloaded_bytes, stalled.downloaded_bytes);
    assert!(offline.error.is_none());
    assert_ne!(offline.state, "paused");
    println!(
        "Connections interrupted: stalled at {} bytes, {} tracker requests rejected.",
        offline.downloaded_bytes,
        swarm.offline_tracker_requests.load(Ordering::SeqCst)
    );
    swarm.online.send(true).unwrap();
    // No pause/resume or restart: the existing transfer must reconnect itself.
    wait_for(&engine, |item| item.state == "seeding").await;
    assert_eq!(
        std::fs::read(downloads.path().join("payload.bin")).unwrap(),
        swarm.content
    );
    println!("Connections restored: automatic recovery and 4194304 bytes verified.");
    engine.shutdown().await.unwrap();
    swarm.seed.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failed_add_to_missing_destination_is_retryable() {
    let state = tempfile::tempdir().unwrap();
    let downloads = tempfile::tempdir().unwrap();
    let destination = downloads.path().join("missing");
    let engine = Engine::open(config(state.path(), downloads.path()))
        .await
        .unwrap();
    let metadata = adapter::create_fixture("retry.bin", b"retry");
    assert!(
        engine
            .add(
                metadata.clone(),
                destination.to_string_lossy().into(),
                vec![0],
                true,
                false
            )
            .await
            .is_err()
    );
    assert!(engine.snapshot().unwrap().torrents.is_empty());
    assert!(!destination.exists());
    std::fs::create_dir(&destination).unwrap();
    engine
        .add(
            metadata,
            destination.to_string_lossy().into(),
            vec![0],
            true,
            false,
        )
        .await
        .unwrap();
    wait_for(&engine, |item| item.state == "paused").await;
    engine.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires a disposable mounted disk image; run python3 scripts/faults.py"]
async fn real_disk_full_payload_and_state_writes_recover() {
    let volume = fault_volume();
    let disk = tempfile::tempdir_in(volume).unwrap();
    let state = tempfile::tempdir().unwrap();
    let downloads = disk.path().join("downloads");
    std::fs::create_dir(&downloads).unwrap();
    // Precreate an empty file so add can open it even when directory allocation is exhausted.
    std::fs::File::create(downloads.join("payload.bin")).unwrap();
    let swarm = Swarm::new(2 * 1024 * 1024).await;
    let cfg = config(state.path(), &downloads);
    let engine = Engine::open(cfg.clone()).await.unwrap();
    let filler = disk.path().join("filler");
    fill_volume(&filler);
    let id = engine
        .add(
            swarm.metadata.clone(),
            cfg.download_directory,
            vec![0],
            false,
            true,
        )
        .await
        .unwrap();
    let failed = wait_for(&engine, |item| item.state == "error").await;
    let error = failed
        .error
        .as_deref()
        .expect("disk failure must reach snapshots");
    assert!(
        error.contains("No space left") || error.contains(&format!("os error {}", libc::ENOSPC)),
        "{error}"
    );
    assert!(failed.downloaded_bytes < failed.total_bytes);
    println!("Real payload ENOSPC reached snapshot: {error}");
    free_volume(&filler);
    engine.set_paused(id.clone(), false).await.unwrap();
    wait_for(&engine, |item| item.state == "seeding").await;
    assert_eq!(
        std::fs::read(downloads.join("payload.bin")).unwrap(),
        swarm.content
    );
    assert_eq!(engine.snapshot().unwrap().torrents[0].id, id);
    println!("Space freed: Resume recovered the original transfer and 2097152 bytes verified.");
    engine.shutdown().await.unwrap();
    swarm.seed.shutdown().await.unwrap();

    // A separate state directory on the same bounded volume exercises durable command failure.
    let state = disk.path().join("state");
    let downloads = tempfile::tempdir().unwrap();
    let cfg = config(&state, downloads.path());
    let engine = Engine::open(cfg.clone()).await.unwrap();
    engine.set_limits(123_000, 45_000).await.unwrap();
    let saved = std::fs::read(state.join("limits.json")).unwrap();
    fill_volume(&filler);
    let error = engine.set_limits(999_000, 888_000).await.unwrap_err();
    assert!(matches!(error, CoreError::Storage(_)));
    assert_eq!(std::fs::read(state.join("limits.json")).unwrap(), saved);
    assert_eq!(engine.snapshot().unwrap().download_limit, 123_000);
    println!("Real state ENOSPC reached caller: {error}; saved and active limits unchanged.");
    free_volume(&filler);
    engine.set_limits(999_000, 888_000).await.unwrap();
    engine.shutdown().await.unwrap();
    drop(engine);
    let engine = Engine::open(cfg).await.unwrap();
    let snapshot = engine.snapshot().unwrap();
    assert_eq!(
        (snapshot.download_limit, snapshot.upload_limit),
        (999_000, 888_000)
    );
    println!("Space freed: state command retry succeeded and limits survived restart.");
    engine.shutdown().await.unwrap();
}

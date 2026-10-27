mod support;
#[path = "support/volume.rs"]
mod volume;
use seedhop_core::{Engine, EngineConfig, TorrentRecord, adapter};
use std::{path::Path, time::Duration};
use support::Swarm;
use volume::{fault_volume, fill_volume, free_volume};

fn config(state: &Path, files: &Path) -> EngineConfig {
    EngineConfig {
        state_directory: state.to_string_lossy().into(),
        download_directory: files.to_string_lossy().into(),
        allow_network_discovery: false,
        listen_port: 0,
    }
}
async fn wait(engine: &Engine, id: &str, state: &str) -> TorrentRecord {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let item = engine
                .snapshot()
                .unwrap()
                .torrents
                .into_iter()
                .find(|item| item.id == id)
                .unwrap();
            if item.state == state {
                return item;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out waiting for {state}: {:?}", engine.snapshot()))
}

/// The runner serializes these tests on its bounded, disposable mounted volume.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires scripts/faults.py and a disposable disk image"]
async fn real_disk_full_retry_preserves_intent_queue_choices_and_destination() {
    let volume = fault_volume();
    for paused in [true, false] {
        let disk = tempfile::tempdir_in(&volume).unwrap();
        let files = disk.path().join("downloads");
        std::fs::create_dir(&files).unwrap();
        std::fs::File::create(files.join("payload.bin")).unwrap();
        let state = tempfile::tempdir().unwrap();
        let cfg = config(state.path(), &files);
        let engine = Engine::open(cfg.clone()).await.unwrap();
        let swarm = Swarm::new(2 * 1024 * 1024).await;
        let filler = disk.path().join("filler");
        let id = engine
            .add(
                swarm.metadata.clone(),
                cfg.download_directory.clone(),
                vec![0],
                true,
                true,
            )
            .await
            .unwrap();
        wait(&engine, &id, "paused").await;
        engine
            .set_file_priorities(id.clone(), vec![2])
            .await
            .unwrap();
        // Initial checking may allocate the full file on HFS+. Drop those extents
        // before filling the volume so the next peer write must allocate space.
        let payload = std::fs::OpenOptions::new()
            .write(true)
            .open(files.join("payload.bin"))
            .unwrap();
        payload.set_len(0).unwrap();
        payload.sync_all().unwrap();
        drop(payload);
        fill_volume(&filler);
        engine.set_paused(id.clone(), false).await.unwrap();
        let failed = wait(&engine, &id, "error").await;
        assert_eq!(failed.error_kind.as_deref(), Some("storage_space"));
        if paused {
            engine.set_paused(id.clone(), true).await.unwrap();
        }
        let before = engine.snapshot().unwrap().torrents.remove(0);
        let retained = disk.path().join("retained");
        std::fs::rename(&files, &retained).unwrap();
        let error = engine
            .retry_transfer(id.clone())
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("Reconnect the drive"));
        assert!(!files.exists());
        assert_eq!(engine.snapshot().unwrap().torrents[0], before);
        std::fs::rename(&retained, &files).unwrap();
        free_volume(&filler);
        // Another transfer owns the only queue slot before the failed one retries.
        engine.set_queue(1).await.unwrap();
        let other = engine
            .add(
                adapter::create_fixture("other.bin", b"missing"),
                cfg.download_directory.clone(),
                vec![0],
                false,
                true,
            )
            .await
            .unwrap();
        engine.move_in_queue(other.clone(), 1).await.unwrap();
        wait(&engine, &other, "waiting").await;
        engine.retry_transfer(id.clone()).await.unwrap();
        let expected = if paused { "paused" } else { "queued" };
        let recovered = wait(&engine, &id, expected).await;
        assert_eq!(recovered.paused_intent, paused);
        assert_eq!(recovered.id, before.id);
        assert_eq!(recovered.destination, before.destination);
        assert_eq!(
            recovered
                .files
                .iter()
                .map(|f| (f.index, f.selected, f.priority))
                .collect::<Vec<_>>(),
            before
                .files
                .iter()
                .map(|f| (f.index, f.selected, f.priority))
                .collect::<Vec<_>>()
        );
        assert!(recovered.error.is_none() && recovered.error_kind.is_none());
        engine.retry_transfer(id.clone()).await.unwrap(); // Stale actions do not restart healthy transfers.
        assert_eq!(
            engine
                .snapshot()
                .unwrap()
                .torrents
                .iter()
                .find(|item| item.id == id)
                .unwrap()
                .state,
            expected
        );
        engine.shutdown().await.unwrap();
        drop(engine);
        let engine = Engine::open(cfg).await.unwrap();
        let restored = wait(&engine, &id, expected).await;
        assert_eq!(restored.paused_intent, paused);
        engine.set_paused(other, true).await.unwrap();
        if paused {
            engine.set_paused(id.clone(), false).await.unwrap();
        }
        wait(&engine, &id, "seeding").await;
        assert_eq!(
            std::fs::read(files.join("payload.bin")).unwrap(),
            swarm.content
        );
        engine.shutdown().await.unwrap();
        swarm.seed.shutdown().await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn delayed_retry_leaves_a_healthy_paused_transfer_unchanged() {
    let state = tempfile::tempdir().unwrap();
    let files = tempfile::tempdir().unwrap();
    let cfg = config(state.path(), files.path());
    let engine = Engine::open(cfg.clone()).await.unwrap();
    let id = engine
        .add(
            adapter::create_fixture("data.bin", b"content"),
            cfg.download_directory,
            vec![0],
            true,
            true,
        )
        .await
        .unwrap();
    wait(&engine, &id, "paused").await;
    engine
        .set_file_priorities(id.clone(), vec![2])
        .await
        .unwrap();
    let before = wait(&engine, &id, "paused").await;
    engine.retry_transfer(id).await.unwrap();
    assert_eq!(engine.snapshot().unwrap().torrents[0], before);
    engine.shutdown().await.unwrap();
}

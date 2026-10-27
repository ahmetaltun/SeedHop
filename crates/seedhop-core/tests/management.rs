use seedhop_core::{Engine, EngineConfig, Snapshot, adapter};
use std::{path::Path, time::Duration};

fn config(state: &Path, files: &Path) -> EngineConfig {
    EngineConfig {
        state_directory: state.to_string_lossy().into(),
        download_directory: files.to_string_lossy().into(),
        allow_network_discovery: false,
        listen_port: 0,
    }
}
async fn wait(engine: &Engine, predicate: impl Fn(&Snapshot) -> bool) -> Snapshot {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let snapshot = engine.snapshot().unwrap();
            if predicate(&snapshot) {
                return snapshot;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out: {:?}", engine.snapshot()))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn copying_can_be_cancelled_without_losing_running_paused_or_queued_intent() {
    use seedhop_core::CoreError;
    for intent in ["paused", "waiting", "queued"] {
        let state = tempfile::tempdir().unwrap();
        let files = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        let cfg = config(state.path(), files.path());
        let engine = Engine::open(cfg.clone()).await.unwrap();
        if intent == "queued" {
            engine.set_queue(1).await.unwrap();
            engine
                .add(
                    adapter::create_fixture("blocker", b"not downloaded"),
                    cfg.download_directory.clone(),
                    vec![0],
                    false,
                    false,
                )
                .await
                .unwrap();
            wait(&engine, |s| s.torrents[0].state == "waiting").await;
        }
        let mut content = vec![7; 64 * 1024 * 1024];
        let metainfo = adapter::create_fixture("original", &content);
        // Keep the torrent incomplete, so queue suspension is meaningful.
        content[0] = 8;
        std::fs::write(files.path().join("original"), &content).unwrap();
        let id = engine
            .add(
                metainfo,
                cfg.download_directory.clone(),
                vec![0],
                intent == "paused",
                true,
            )
            .await
            .unwrap();
        wait(&engine, |s| {
            s.torrents.iter().any(|t| t.id == id && t.state == intent)
        })
        .await;
        engine
            .set_file_priorities(id.clone(), vec![2])
            .await
            .unwrap();
        let before = wait(&engine, |s| {
            s.torrents
                .iter()
                .any(|t| t.id == id && t.state == intent && t.queue_position > 0)
        })
        .await
        .torrents
        .into_iter()
        .find(|t| t.id == id)
        .unwrap();
        let moving = tokio::spawn({
            let engine = engine.clone();
            let id = id.clone();
            let destination = target.path().to_string_lossy().into_owned();
            async move {
                if intent == "paused" {
                    engine.rename_file(id, 0, "renamed".into()).await
                } else {
                    engine.relocate(id, destination).await
                }
            }
        });
        let operation_id = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let snapshot = engine.snapshot().unwrap();
                if let Some(operation) = snapshot.file_operation
                    && operation.phase == "copying"
                    && operation.completed_bytes > 0
                {
                    assert_eq!(operation.torrent_id, id);
                    assert_eq!(operation.total_bytes, content.len() as u64);
                    assert!(operation.completed_bytes <= operation.total_bytes);
                    if engine.cancel_file_operation(operation.id.clone()).unwrap() {
                        break operation.id;
                    }
                }
                assert!(
                    !moving.is_finished(),
                    "Copy completed before cancellation was observed"
                );
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(matches!(moving.await.unwrap(), Err(CoreError::Cancelled)));
        assert!(engine.snapshot().unwrap().file_operation.is_none());
        assert!(!engine.cancel_file_operation(operation_id.clone()).unwrap());
        assert_eq!(
            std::fs::read(files.path().join("original")).unwrap(),
            content
        );
        assert!(!files.path().join("renamed").exists());
        assert!(!target.path().join("original").exists());
        let after = engine
            .snapshot()
            .unwrap()
            .torrents
            .into_iter()
            .find(|t| t.id == id)
            .unwrap();
        assert_eq!(after, before);
        // An old cancellation ID cannot poison a successful retry.
        let retry = tokio::spawn({
            let engine = engine.clone();
            let id = id.clone();
            let destination = target.path().to_string_lossy().into_owned();
            async move { engine.relocate(id, destination).await }
        });
        assert!(!engine.cancel_file_operation(operation_id).unwrap());
        retry.await.unwrap().unwrap();
        wait(&engine, |s| {
            s.torrents.iter().any(|t| t.id == id && t.state == intent)
        })
        .await;
        assert_eq!(
            std::fs::read(target.path().join("original")).unwrap(),
            content
        );
        engine.shutdown().await.unwrap();
        drop(engine);
        let engine = Engine::open(cfg).await.unwrap();
        let restored = wait(&engine, |s| {
            s.torrents.iter().any(|t| t.id == id && t.state == intent)
        })
        .await
        .torrents
        .into_iter()
        .find(|t| t.id == id)
        .unwrap();
        assert_eq!(restored.files[0].priority, 2);
        assert!(restored.files[0].selected);
        assert_eq!(restored.paused_intent, before.paused_intent);
        assert_eq!(restored.queue_position, before.queue_position);
        assert_eq!(
            restored.destination,
            target.path().canonicalize().unwrap().to_string_lossy()
        );
        engine.shutdown().await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn queue_caps_reordering_pause_and_completion_survive_restart() {
    let state = tempfile::tempdir().unwrap();
    let files = tempfile::tempdir().unwrap();
    let cfg = config(state.path(), files.path());
    let engine = Engine::open(cfg.clone()).await.unwrap();
    engine.set_queue(1).await.unwrap();
    let mut ids = Vec::new();
    let content = vec![3; 65536];
    for (name, paused) in [("a", false), ("b", false), ("c", true)] {
        ids.push(
            engine
                .add(
                    adapter::create_fixture(name, &content),
                    cfg.download_directory.clone(),
                    vec![0],
                    paused,
                    false,
                )
                .await
                .unwrap(),
        );
    }
    wait(&engine, |s| {
        s.torrents.iter().filter(|t| t.state == "queued").count() == 1
            && s.torrents.iter().filter(|t| t.state == "paused").count() == 1
    })
    .await;
    engine.set_paused(ids[1].clone(), true).await.unwrap(); // user pause while queued
    assert_eq!(
        engine
            .snapshot()
            .unwrap()
            .torrents
            .iter()
            .find(|t| t.id == ids[1])
            .unwrap()
            .state,
        "paused"
    );
    engine.set_paused(ids[1].clone(), false).await.unwrap();
    engine.move_in_queue(ids[1].clone(), 1).await.unwrap();
    let snapshot = wait(&engine, |s| {
        s.torrents.iter().find(|t| t.id == ids[0]).unwrap().state == "queued"
    })
    .await;
    assert_eq!(
        snapshot
            .torrents
            .iter()
            .find(|t| t.id == ids[1])
            .unwrap()
            .queue_position,
        1
    );
    engine.shutdown().await.unwrap();
    drop(engine);
    let engine = Engine::open(cfg).await.unwrap();
    wait(&engine, |s| {
        s.torrents.iter().find(|t| t.id == ids[0]).unwrap().state == "queued"
            && s.torrents.iter().find(|t| t.id == ids[2]).unwrap().state == "paused"
    })
    .await;
    assert_eq!(engine.snapshot().unwrap().max_active_downloads, 1);
    // Completing a queued item starts its seeding without occupying a slot.
    std::fs::write(files.path().join("a"), &content).unwrap();
    engine.force_recheck(ids[0].clone()).await.unwrap();
    wait(&engine, |s| {
        s.torrents.iter().find(|t| t.id == ids[0]).unwrap().state == "seeding"
    })
    .await;
    engine.set_paused(ids[1].clone(), true).await.unwrap();
    engine.set_paused(ids[2].clone(), false).await.unwrap();
    wait(&engine, |s| {
        s.torrents.iter().find(|t| t.id == ids[2]).unwrap().state == "waiting"
    })
    .await;
    engine.set_queue(0).await.unwrap();
    assert_eq!(
        engine
            .snapshot()
            .unwrap()
            .torrents
            .iter()
            .filter(|t| t.state == "queued")
            .count(),
        0
    );
    engine.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn v1_layout_priorities_failures_and_delete_use_current_names() {
    use std::os::unix::fs::PermissionsExt;
    let state = tempfile::tempdir().unwrap();
    let files = tempfile::tempdir().unwrap();
    let target = tempfile::tempdir().unwrap();
    let cfg = config(state.path(), files.path());
    let engine = Engine::open(cfg.clone()).await.unwrap();
    let content = vec![8; 70001];
    std::fs::write(files.path().join("original"), &content).unwrap();
    let id = engine
        .add(
            adapter::create_fixture("original", &content),
            cfg.download_directory.clone(),
            vec![0],
            true,
            true,
        )
        .await
        .unwrap();
    wait(&engine, |s| s.torrents[0].state == "paused").await;
    assert!(
        engine
            .set_file_priorities(id.clone(), vec![3])
            .await
            .is_err()
    );
    engine
        .set_file_priorities(id.clone(), vec![2])
        .await
        .unwrap();
    std::fs::write(files.path().join("occupied"), b"keep").unwrap();
    assert!(
        engine
            .rename_file(id.clone(), 0, "occupied".into())
            .await
            .is_err()
    );
    assert!(
        engine
            .rename_file(id.clone(), 0, "../escape".into())
            .await
            .is_err()
    );
    std::os::unix::fs::symlink(target.path(), files.path().join("link")).unwrap();
    assert!(
        engine
            .rename_file(id.clone(), 0, "link/escape".into())
            .await
            .is_err()
    );
    // A failed layout commit rolls back before source removal.
    let session = state.path().join("session");
    std::fs::set_permissions(&session, std::fs::Permissions::from_mode(0o500)).unwrap();
    let priority_result = engine.set_file_priorities(id.clone(), vec![0]).await;
    let result = engine.rename_file(id.clone(), 0, "failed".into()).await;
    std::fs::set_permissions(&session, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert!(result.is_err());
    assert!(priority_result.is_err());
    assert_eq!(engine.snapshot().unwrap().torrents[0].files[0].priority, 2);
    assert_eq!(
        std::fs::read(files.path().join("original")).unwrap(),
        content
    );
    assert!(!files.path().join("failed").exists());
    engine.force_recheck(id.clone()).await.unwrap();
    wait(&engine, |s| s.torrents[0].state == "paused").await;
    engine
        .rename_file(id.clone(), 0, "nested/renamed".into())
        .await
        .unwrap();
    wait(&engine, |s| s.torrents[0].state == "paused").await;
    assert!(!files.path().join("original").exists());
    engine
        .relocate(id.clone(), target.path().to_string_lossy().into())
        .await
        .unwrap();
    wait(&engine, |s| s.torrents[0].state == "paused").await;
    assert!(!files.path().join("nested/renamed").exists());
    engine.shutdown().await.unwrap();
    drop(engine);
    let engine = Engine::open(cfg).await.unwrap();
    let snapshot = wait(&engine, |s| s.torrents[0].state == "paused").await;
    assert_eq!(snapshot.torrents[0].id, id);
    assert_eq!(snapshot.torrents[0].files[0].priority, 2);
    assert_eq!(snapshot.torrents[0].files[0].name, "nested/renamed");
    assert_eq!(
        std::fs::read(target.path().join("nested/renamed")).unwrap(),
        content
    );
    std::fs::write(target.path().join("unrelated"), b"keep").unwrap();
    engine.remove(id, true).await.unwrap();
    assert!(!target.path().join("nested/renamed").exists());
    assert_eq!(
        std::fs::read(target.path().join("unrelated")).unwrap(),
        b"keep"
    );
    assert_eq!(
        std::fs::read(files.path().join("occupied")).unwrap(),
        b"keep"
    );
    engine.shutdown().await.unwrap();
}

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

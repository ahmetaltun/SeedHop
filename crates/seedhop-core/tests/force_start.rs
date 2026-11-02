use seedhop_core::{Engine, EngineConfig, ScheduleConfig, ScheduleRule, Snapshot, adapter};
use std::{path::Path, time::Duration};

fn config(state: &Path, files: &Path) -> EngineConfig {
    EngineConfig {
        state_directory: state.display().to_string(),
        download_directory: files.display().to_string(),
        allow_network_discovery: false,
        listen_port: 0,
    }
}

async fn settled(engine: &Engine) -> Snapshot {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let snapshot = engine.snapshot().unwrap();
            if snapshot.torrents.iter().all(|t| {
                t.state != "checking"
                    && t.queue_position > 0
                    && !(t.force_started
                        && !t.paused_intent
                        && matches!(t.state.as_str(), "queued" | "paused"))
            }) {
                return snapshot;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out: {:?}", engine.snapshot()))
}

async fn add(engine: &Engine, cfg: &EngineConfig, name: &str, paused: bool) -> String {
    let id = engine
        .add(
            adapter::create_fixture(name, b"content"),
            cfg.download_directory.clone(),
            vec![0],
            paused,
            false,
        )
        .await
        .unwrap();
    settled(engine).await;
    id
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn forced_downloads_do_not_consume_slots_and_pause_resume_restore_normal_admission() {
    let state = tempfile::tempdir().unwrap();
    let files = tempfile::tempdir().unwrap();
    let cfg = config(state.path(), files.path());
    let engine = Engine::open(cfg.clone()).await.unwrap();
    engine.set_queue(1).await.unwrap();
    let a = add(&engine, &cfg, "a", false).await;
    let b = add(&engine, &cfg, "b", false).await;
    let c = add(&engine, &cfg, "c", true).await;
    let d = add(&engine, &cfg, "d", false).await;
    engine.set_force_started(b.clone(), true).await.unwrap();
    engine.set_force_started(c.clone(), true).await.unwrap();
    engine.set_force_started(b.clone(), true).await.unwrap(); // idempotent
    let snapshot = settled(&engine).await;
    for id in [&a, &b, &c] {
        assert_eq!(
            snapshot
                .torrents
                .iter()
                .find(|t| &t.id == id)
                .unwrap()
                .state,
            "waiting"
        );
    }
    assert_eq!(
        snapshot.torrents.iter().find(|t| t.id == d).unwrap().state,
        "queued"
    );
    assert_eq!(
        snapshot
            .torrents
            .iter()
            .find(|t| t.id == c)
            .unwrap()
            .queue_position,
        3
    );
    engine.set_force_started(a.clone(), true).await.unwrap();
    assert_eq!(
        settled(&engine)
            .await
            .torrents
            .iter()
            .find(|t| t.id == d)
            .unwrap()
            .state,
        "waiting"
    );
    engine.set_force_started(a.clone(), false).await.unwrap();
    assert_eq!(
        settled(&engine)
            .await
            .torrents
            .iter()
            .find(|t| t.id == d)
            .unwrap()
            .state,
        "queued"
    );
    engine.move_in_queue(b.clone(), 4).await.unwrap();
    engine.shutdown().await.unwrap();
    drop(engine);
    let engine = Engine::open(cfg).await.unwrap();
    let snapshot = settled(&engine).await;
    for id in [&b, &c] {
        let torrent = snapshot.torrents.iter().find(|t| &t.id == id).unwrap();
        assert!(torrent.force_started && !torrent.paused_intent);
        assert_eq!(torrent.state, "waiting");
    }
    assert_eq!(
        snapshot
            .torrents
            .iter()
            .find(|t| t.id == b)
            .unwrap()
            .queue_position,
        4
    );
    engine.set_paused(c.clone(), true).await.unwrap();
    engine.set_paused(b.clone(), false).await.unwrap();
    let snapshot = settled(&engine).await;
    let paused = snapshot.torrents.iter().find(|t| t.id == c).unwrap();
    assert!(paused.paused_intent && !paused.force_started);
    let resumed = snapshot.torrents.iter().find(|t| t.id == b).unwrap();
    assert!(!resumed.force_started && !resumed.paused_intent);
    assert_eq!(resumed.state, "queued");
    engine.set_queue(0).await.unwrap();
    assert!(
        settled(&engine)
            .await
            .torrents
            .iter()
            .all(|t| t.state != "queued")
    );
    engine.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn schedules_and_bandwidth_still_apply_to_forced_downloads_and_seeds() {
    let state = tempfile::tempdir().unwrap();
    let files = tempfile::tempdir().unwrap();
    let cfg = config(state.path(), files.path());
    let engine = Engine::open(cfg.clone()).await.unwrap();
    let id = add(&engine, &cfg, "scheduled", false).await;
    engine.set_force_started(id.clone(), true).await.unwrap();
    let mut schedule = ScheduleConfig {
        enabled: true,
        rules: vec![ScheduleRule {
            days: 127,
            start_minute: 0,
            end_minute: 0,
            download_limit: 23000,
            upload_limit: 4500,
            suspend: true,
        }],
    };
    engine.set_schedule(schedule.clone()).await.unwrap();
    let snapshot = settled(&engine).await;
    assert_eq!(snapshot.torrents[0].state, "scheduled");
    assert!(snapshot.torrents[0].force_started && !snapshot.torrents[0].paused_intent);
    assert_eq!(
        (snapshot.download_limit, snapshot.upload_limit),
        (23000, 4500)
    );
    engine
        .set_alternative_limits(12000, 3000, true)
        .await
        .unwrap();
    let snapshot = engine.snapshot().unwrap();
    assert_eq!(
        (snapshot.download_limit, snapshot.upload_limit),
        (12000, 3000)
    );
    assert_eq!(snapshot.torrents[0].state, "scheduled");
    engine.shutdown().await.unwrap();
    drop(engine);
    let engine = Engine::open(cfg).await.unwrap();
    assert_eq!(settled(&engine).await.torrents[0].state, "scheduled");
    schedule.rules[0].suspend = false;
    engine.set_schedule(schedule.clone()).await.unwrap();
    assert_eq!(settled(&engine).await.torrents[0].state, "waiting");
    std::fs::write(files.path().join("scheduled"), b"content").unwrap();
    engine.force_recheck(id.clone()).await.unwrap();
    let snapshot = settled(&engine).await;
    assert_eq!(snapshot.torrents[0].state, "seeding");
    assert!(snapshot.torrents[0].force_started);
    let moved = tempfile::tempdir().unwrap();
    engine
        .relocate(id.clone(), moved.path().display().to_string())
        .await
        .unwrap();
    assert!(settled(&engine).await.torrents[0].force_started);
    schedule.rules[0].suspend = true;
    engine.set_schedule(schedule).await.unwrap();
    assert_eq!(settled(&engine).await.torrents[0].state, "scheduled");
    engine.set_force_started(id, false).await.unwrap();
    assert_eq!(settled(&engine).await.torrents[0].state, "scheduled");
    assert!(!engine.snapshot().unwrap().torrents[0].force_started);
    engine.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failed_save_and_missing_destination_do_not_enable_force_start() {
    let state = tempfile::tempdir().unwrap();
    let files = tempfile::tempdir().unwrap();
    let cfg = config(state.path(), files.path());
    let engine = Engine::open(cfg.clone()).await.unwrap();
    let id = add(&engine, &cfg, "safe", true).await;
    let queue = state.path().join("queue.json");
    let backup = state.path().join("queue.backup");
    std::fs::rename(&queue, &backup).unwrap();
    std::fs::create_dir(&queue).unwrap();
    assert!(engine.set_force_started(id.clone(), true).await.is_err());
    let torrent = &engine.snapshot().unwrap().torrents[0];
    assert!(torrent.paused_intent && !torrent.force_started);
    std::fs::remove_dir(&queue).unwrap();
    std::fs::rename(&backup, &queue).unwrap();
    std::fs::remove_dir_all(files.path()).unwrap();
    assert!(engine.set_force_started(id, true).await.is_err());
    assert!(!engine.snapshot().unwrap().torrents[0].force_started);
    assert!(
        engine
            .set_force_started("missing".into(), true)
            .await
            .is_err()
    );
    engine.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn old_queue_files_and_removed_torrents_do_not_inherit_overrides() {
    let state = tempfile::tempdir().unwrap();
    let files = tempfile::tempdir().unwrap();
    std::fs::write(
        state.path().join("queue.json"),
        br#"{"max_active":1,"order":[]}"#,
    )
    .unwrap();
    let cfg = config(state.path(), files.path());
    let engine = Engine::open(cfg.clone()).await.unwrap();
    let id = add(&engine, &cfg, "again", false).await;
    assert!(!engine.snapshot().unwrap().torrents[0].force_started);
    engine.set_force_started(id.clone(), true).await.unwrap();
    engine.remove(id.clone(), false).await.unwrap();
    let queue: serde_json::Value =
        serde_json::from_slice(&std::fs::read(state.path().join("queue.json")).unwrap()).unwrap();
    assert_eq!(queue["force_started"], serde_json::json!([]));
    let again = engine
        .add(
            adapter::create_fixture("again", b"content"),
            cfg.download_directory,
            vec![0],
            true,
            true,
        )
        .await
        .unwrap();
    assert_eq!(id, again);
    assert!(!settled(&engine).await.torrents[0].force_started);
    engine.shutdown().await.unwrap();
}

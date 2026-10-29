use seedhop_core::{Engine, EngineConfig, ScheduleConfig, ScheduleRule, adapter};
use std::time::Duration;

fn config(state: &std::path::Path, files: &std::path::Path) -> EngineConfig {
    EngineConfig {
        state_directory: state.display().to_string(),
        download_directory: files.display().to_string(),
        allow_network_discovery: false,
        listen_port: 0,
    }
}
fn schedule(suspend: bool) -> ScheduleConfig {
    ScheduleConfig {
        enabled: true,
        rules: vec![ScheduleRule {
            days: 127,
            start_minute: 0,
            end_minute: 0,
            download_limit: 23000,
            upload_limit: 4500,
            suspend,
        }],
    }
}
async fn settled(engine: &Engine) {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let snapshot = engine.snapshot().unwrap();
            if snapshot
                .torrents
                .iter()
                .all(|t| t.state != "checking" && t.queue_position > 0)
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn schedule_caps_suspend_seeding_and_downloads_preserve_intent_and_restart() {
    let state = tempfile::tempdir().unwrap();
    let files = tempfile::tempdir().unwrap();
    let cfg = config(state.path(), files.path());
    let mut engine = Engine::open(cfg.clone()).await.unwrap();
    engine.set_limits(80000, 16000).await.unwrap();
    for (name, paused, complete) in [
        ("running", false, false),
        ("paused", true, false),
        ("queued", false, false),
        ("seed", false, true),
    ] {
        if complete {
            std::fs::write(files.path().join(name), b"content").unwrap();
        }
        let id = engine
            .add(
                adapter::create_fixture(name, b"content"),
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
    let before = engine.snapshot().unwrap();
    engine.set_schedule(schedule(false)).await.unwrap();
    let capped = engine.snapshot().unwrap();
    assert!(capped.schedule_active);
    assert_eq!((capped.download_limit, capped.upload_limit), (23000, 4500));
    engine.set_limits(90000, 17000).await.unwrap();
    assert_eq!(engine.snapshot().unwrap().download_limit, 23000);
    engine.set_schedule(schedule(true)).await.unwrap();
    let suspended = engine.snapshot().unwrap();
    for t in &suspended.torrents {
        assert_eq!(
            t.state,
            if t.paused_intent {
                "paused"
            } else {
                "scheduled"
            }
        );
    }
    engine.shutdown().await.unwrap();
    drop(engine);
    engine = Engine::open(cfg.clone()).await.unwrap();
    settled(&engine).await;
    let restored = engine.snapshot().unwrap();
    assert_eq!(restored.schedule, schedule(true));
    assert_eq!(
        (restored.normal_download_limit, restored.normal_upload_limit),
        (90000, 17000)
    );
    assert!(
        restored
            .torrents
            .iter()
            .filter(|t| !t.paused_intent)
            .all(|t| t.state == "scheduled")
    );
    // A newly added transfer must enter the current window, not briefly run first.
    engine
        .add(
            adapter::create_fixture("new", b"content"),
            cfg.download_directory.clone(),
            vec![0],
            false,
            true,
        )
        .await
        .unwrap();
    settled(&engine).await;
    assert_eq!(
        engine
            .snapshot()
            .unwrap()
            .torrents
            .iter()
            .find(|t| t.name == "new")
            .unwrap()
            .state,
        "scheduled"
    );
    engine
        .set_schedule(ScheduleConfig {
            enabled: false,
            ..schedule(true)
        })
        .await
        .unwrap();
    let normal = engine.snapshot().unwrap();
    assert!(!normal.schedule_active);
    assert_eq!((normal.download_limit, normal.upload_limit), (90000, 17000));
    for old in &before.torrents {
        let current = normal.torrents.iter().find(|t| t.id == old.id).unwrap();
        assert_eq!(current.paused_intent, old.paused_intent);
        assert_eq!(current.files, old.files);
        assert_eq!(current.queue_position, old.queue_position);
        assert_eq!(current.state, old.state);
    }
    engine.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rejected_schedules_and_failed_writes_preserve_saved_settings() {
    let state = tempfile::tempdir().unwrap();
    let files = tempfile::tempdir().unwrap();
    let engine = Engine::open(config(state.path(), files.path()))
        .await
        .unwrap();
    engine.set_schedule(schedule(false)).await.unwrap();
    let path = state.path().join("schedule.json");
    let bytes = std::fs::read(&path).unwrap();
    let mut invalid = schedule(false);
    invalid.rules.push(invalid.rules[0].clone());
    assert!(engine.set_schedule(invalid).await.is_err());
    std::fs::create_dir(state.path().join("schedule.pending")).unwrap();
    assert!(engine.set_schedule(schedule(true)).await.is_err());
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    assert_eq!(engine.snapshot().unwrap().schedule, schedule(false));
    std::fs::remove_dir(state.path().join("schedule.pending")).unwrap();
    engine.set_schedule(schedule(true)).await.unwrap();
    assert_eq!(engine.snapshot().unwrap().schedule, schedule(true));
    engine.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn malformed_future_and_unreadable_schedules_are_preserved() {
    for bytes in [b"{".as_slice(),br#"{"version":2,"config":{"enabled":false,"rules":[]}}"#,br#"{"version":1,"config":{"enabled":true,"rules":[{"days":0,"startMinute":0,"endMinute":0,"downloadLimit":0,"uploadLimit":0,"suspend":false}]}}"#] {
        let state = tempfile::tempdir().unwrap(); let files = tempfile::tempdir().unwrap();
        let path = state.path().join("schedule.json"); std::fs::write(&path,bytes).unwrap();
        assert!(Engine::open(config(state.path(),files.path())).await.is_err());
        assert_eq!(std::fs::read(path).unwrap(),bytes);
    }
    let state = tempfile::tempdir().unwrap();
    let files = tempfile::tempdir().unwrap();
    std::fs::create_dir(state.path().join("schedule.json")).unwrap();
    assert!(
        Engine::open(config(state.path(), files.path()))
            .await
            .is_err()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failed_application_is_visible_and_timer_retries_without_losing_the_saved_window() {
    let state = tempfile::tempdir().unwrap();
    let files = tempfile::tempdir().unwrap();
    let cfg = config(state.path(), files.path());
    let engine = Engine::open(cfg.clone()).await.unwrap();
    std::fs::create_dir(state.path().join("queue.pending")).unwrap();
    engine
        .add(
            adapter::create_fixture("pending", b"content"),
            cfg.download_directory,
            vec![0],
            false,
            true,
        )
        .await
        .unwrap();
    assert!(engine.set_schedule(schedule(true)).await.is_err());
    let failed = engine.snapshot().unwrap();
    assert_eq!(failed.schedule, schedule(true));
    assert!(failed.scheduling_warning.is_some());
    std::fs::remove_dir(state.path().join("queue.pending")).unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let current = engine.snapshot().unwrap();
            if current.scheduling_warning.is_none() && current.torrents[0].state == "scheduled" {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    engine.shutdown().await.unwrap();
}

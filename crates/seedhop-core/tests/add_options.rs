use seedhop_core::{Engine, EngineConfig, Snapshot, TorrentAddOptions, adapter};
use std::{path::Path, time::Duration};

fn config(state: &Path, files: &Path) -> EngineConfig {
    EngineConfig {
        state_directory: state.display().to_string(),
        download_directory: files.display().to_string(),
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
async fn top_admission_preserves_active_slot_and_survives_restart() {
    let state = tempfile::tempdir().unwrap();
    let files = tempfile::tempdir().unwrap();
    let cfg = config(state.path(), files.path());
    let engine = Engine::open(cfg.clone()).await.unwrap();
    engine.set_queue(1).await.unwrap();
    let mut ids = vec![];
    for name in ["active", "waiting"] {
        ids.push(
            engine
                .add(
                    adapter::create_fixture(name, &[3; 65536]),
                    cfg.download_directory.clone(),
                    vec![0],
                    false,
                    false,
                )
                .await
                .unwrap(),
        );
        wait(&engine, |s| {
            s.torrents
                .iter()
                .all(|t| t.state != "checking" && t.queue_position > 0)
        })
        .await;
    }
    wait(&engine, |s| {
        s.torrents
            .iter()
            .any(|t| t.id == ids[0] && t.state == "waiting")
            && s.torrents
                .iter()
                .any(|t| t.id == ids[1] && t.state == "queued")
    })
    .await;
    let top = engine
        .add_with_options(
            adapter::create_fixture("top", &[4; 65536]),
            cfg.download_directory.clone(),
            vec![0],
            TorrentAddOptions {
                add_to_top: true,
                sequential: true,
                first_last_pieces: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let settled = wait(&engine, |s| {
        s.torrents
            .iter()
            .all(|t| t.state != "checking" && t.queue_position > 0)
    })
    .await;
    let item = |id: &str| settled.torrents.iter().find(|t| t.id == id).unwrap();
    assert_eq!(item(&ids[0]).state, "waiting");
    assert_eq!(item(&top).state, "queued");
    assert!(item(&ids[0]).queue_position < item(&top).queue_position);
    assert!(item(&top).queue_position < item(&ids[1]).queue_position);
    engine.shutdown().await.unwrap();
    drop(engine);
    let engine = Engine::open(cfg).await.unwrap();
    let restored = wait(&engine, |s| {
        s.torrents
            .iter()
            .all(|t| t.state != "checking" && t.queue_position > 0)
    })
    .await;
    for old in &settled.torrents {
        assert_eq!(
            restored
                .torrents
                .iter()
                .find(|t| t.id == old.id)
                .unwrap()
                .queue_position,
            old.queue_position
        );
    }
    engine.set_paused(ids[0].clone(), true).await.unwrap();
    wait(&engine, |s| {
        s.torrents
            .iter()
            .any(|t| t.id == top && t.state == "waiting")
    })
    .await;
    engine.shutdown().await.unwrap();
    let saved: serde_json::Value =
        serde_json::from_slice(&std::fs::read(state.path().join("session/session.json")).unwrap())
            .unwrap();
    let choices: Vec<_> = saved["torrents"]
        .as_object()
        .unwrap()
        .values()
        .map(|t| {
            (
                t["sequential"].as_bool().unwrap(),
                t["first_last_pieces"].as_bool().unwrap(),
            )
        })
        .collect();
    assert_eq!(choices.iter().filter(|p| **p == (true, true)).count(), 1);
    assert_eq!(choices.iter().filter(|p| **p == (false, false)).count(), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn queue_write_failure_does_not_add_and_retry_preserves_paused_options() {
    let state = tempfile::tempdir().unwrap();
    let files = tempfile::tempdir().unwrap();
    let cfg = config(state.path(), files.path());
    let engine = Engine::open(cfg.clone()).await.unwrap();
    let bytes = adapter::create_fixture("paused", b"content");
    let options = TorrentAddOptions {
        paused: true,
        add_to_top: true,
        sequential: true,
        ..Default::default()
    };
    std::fs::create_dir(state.path().join("queue.pending")).unwrap();
    assert!(
        engine
            .add_with_options(
                bytes.clone(),
                cfg.download_directory.clone(),
                vec![0],
                options
            )
            .await
            .is_err()
    );
    assert!(engine.snapshot().unwrap().torrents.is_empty());
    std::fs::remove_dir(state.path().join("queue.pending")).unwrap();
    let id = engine
        .add_with_options(bytes, cfg.download_directory, vec![0], options)
        .await
        .unwrap();
    wait(&engine, |s| {
        s.torrents.iter().any(|t| t.id == id && t.state == "paused")
    })
    .await;
    engine.shutdown().await.unwrap();
}

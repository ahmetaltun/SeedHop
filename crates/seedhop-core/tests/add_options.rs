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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn runtime_preferences_preserve_intent_and_survive_restart() {
    for intent in ["paused", "waiting", "queued", "seeding"] {
        let state = tempfile::tempdir().unwrap();
        let files = tempfile::tempdir().unwrap();
        let cfg = config(state.path(), files.path());
        let engine = Engine::open(cfg.clone()).await.unwrap();
        if intent == "queued" {
            engine.set_queue(1).await.unwrap();
            engine
                .add(
                    adapter::create_fixture("blocker", b"missing"),
                    cfg.download_directory.clone(),
                    vec![0],
                    false,
                    false,
                )
                .await
                .unwrap();
            wait(&engine, |s| s.torrents[0].state == "waiting").await;
        }
        let payload = vec![7; 65536];
        if intent == "seeding" {
            std::fs::write(files.path().join("target"), &payload).unwrap();
        }
        let id = engine
            .add(
                adapter::create_fixture("target", &payload),
                cfg.download_directory.clone(),
                vec![0],
                intent == "paused",
                intent == "seeding",
            )
            .await
            .unwrap();
        // Updates are valid immediately, including while initialization may still be running.
        engine
            .set_piece_preferences(id.clone(), Some(true), None)
            .await
            .unwrap();
        let before = wait(&engine, |s| {
            s.torrents
                .iter()
                .any(|t| t.id == id && t.state == intent && t.queue_position > 0)
        })
        .await;
        let before = before.torrents.iter().find(|t| t.id == id).unwrap();
        for (sequential, boundary) in [
            (false, false),
            (true, false),
            (true, true),
            (false, true),
            (false, false),
        ] {
            engine
                .set_piece_preferences(id.clone(), Some(sequential), None)
                .await
                .unwrap();
            engine
                .set_piece_preferences(id.clone(), None, Some(boundary))
                .await
                .unwrap();
            let snapshot = engine.snapshot().unwrap();
            let item = snapshot.torrents.iter().find(|t| t.id == id).unwrap();
            assert_eq!(
                (item.sequential, item.first_last_pieces),
                (sequential, boundary)
            );
            assert_eq!(item.state, before.state);
            assert_eq!(item.paused_intent, before.paused_intent);
            assert_eq!(item.queue_position, before.queue_position);
            assert_eq!(item.files, before.files);
            assert_eq!(item.downloaded_bytes, before.downloaded_bytes);
        }
        engine
            .set_piece_preferences(id.clone(), Some(true), Some(true))
            .await
            .unwrap();
        assert!(
            engine
                .set_piece_preferences(id.clone(), None, None)
                .await
                .is_err()
        );
        assert!(
            engine
                .set_piece_preferences("missing".into(), Some(true), None)
                .await
                .is_err()
        );
        engine.shutdown().await.unwrap();
        drop(engine);
        let engine = Engine::open(cfg).await.unwrap();
        let restored = wait(&engine, |s| {
            s.torrents
                .iter()
                .any(|t| t.id == id && t.state != "checking")
        })
        .await;
        let item = restored.torrents.iter().find(|t| t.id == id).unwrap();
        assert!(item.sequential && item.first_last_pieces);
        assert_eq!(item.paused_intent, before.paused_intent);
        engine.shutdown().await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn runtime_preference_failure_rolls_back_and_concurrent_updates_preserve_both_fields() {
    let state = tempfile::tempdir().unwrap();
    let files = tempfile::tempdir().unwrap();
    let cfg = config(state.path(), files.path());
    let engine = Engine::open(cfg.clone()).await.unwrap();
    let id = engine
        .add(
            adapter::create_fixture("target", b"data"),
            cfg.download_directory.clone(),
            vec![0],
            true,
            false,
        )
        .await
        .unwrap();
    wait(&engine, |s| s.torrents[0].state == "paused").await;
    let pending = state.path().join("session/session.json.tmp");
    std::fs::create_dir(&pending).unwrap();
    assert!(
        engine
            .set_piece_preferences(id.clone(), Some(true), Some(true))
            .await
            .is_err()
    );
    let item = engine.snapshot().unwrap().torrents.remove(0);
    assert!(!item.sequential && !item.first_last_pieces);
    assert!(item.paused_intent);
    std::fs::remove_dir(pending).unwrap();
    let (a, b) = tokio::join!(
        engine.set_piece_preferences(id.clone(), Some(true), None),
        engine.set_piece_preferences(id.clone(), None, Some(true))
    );
    a.unwrap();
    b.unwrap();
    assert!(engine.snapshot().unwrap().torrents[0].sequential);
    assert!(engine.snapshot().unwrap().torrents[0].first_last_pieces);
    engine.shutdown().await.unwrap();
    drop(engine);
    let engine = Engine::open(cfg).await.unwrap();
    let snapshot = wait(&engine, |s| s.torrents[0].state == "paused").await;
    assert!(snapshot.torrents[0].sequential && snapshot.torrents[0].first_last_pieces);
    engine.remove(id.clone(), false).await.unwrap();
    assert!(
        engine
            .set_piece_preferences(id, Some(false), None)
            .await
            .is_err()
    );
    engine.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn live_preference_changes_keep_the_same_peer_and_verify_exact_payload() {
    use librqbit::{AddTorrent, AddTorrentOptions, Session, SessionOptions};
    use librqbit_peer_protocol::Message;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let data: Vec<u8> = (0..32 * 16384).map(|i| (i % 251) as u8).collect();
    let files = tempfile::tempdir().unwrap();
    let session = Session::new_with_opts(
        files.path().into(),
        SessionOptions {
            dht: None,
            disable_trackers: true,
            disable_local_service_discovery: true,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (request_tx, mut request_rx) = tokio::sync::mpsc::unbounded_channel();
    let peer_task = tokio::spawn({
        let data = data.clone();
        async move {
            let (mut peer, _) = listener.accept().await.unwrap();
            let mut handshake = [0; 68];
            peer.read_exact(&mut handshake).await.unwrap();
            handshake[20..28].fill(0);
            handshake[25] = 0x10; // BEP 10 request-window negotiation.
            handshake[48..].copy_from_slice(b"-SHtest-123456789012");
            peer.write_all(&handshake).await.unwrap();
            // One outstanding request makes the next acquisition observable after each change.
            let extension = b"d1:mde4:reqqi1ee";
            peer.write_u32(extension.len() as u32 + 2).await.unwrap();
            peer.write_all(&[20, 0]).await.unwrap();
            peer.write_all(extension).await.unwrap();
            // All 32 pieces available; leave the engine's existing pipeline intact.
            peer.write_all(&[0, 0, 0, 5, 5, 255, 255, 255, 255, 0, 0, 0, 1, 1])
                .await
                .unwrap();
            let mut served = std::collections::HashSet::new();
            while served.len() < 32 {
                let len = peer.read_u32().await.unwrap() as usize;
                assert!(len < 1024 * 1024);
                let mut buf = vec![0; len + 4];
                buf[..4].copy_from_slice(&(len as u32).to_be_bytes());
                peer.read_exact(&mut buf[4..]).await.unwrap();
                match Message::deserialize(&buf, &[]).unwrap().0 {
                    Message::Request(request) => {
                        if served.len() < 4 {
                            let (tx, rx) = tokio::sync::oneshot::channel();
                            request_tx.send((request.index, tx)).unwrap();
                            rx.await.unwrap();
                        }
                        let start = request.index as usize * 16384 + request.begin as usize;
                        peer.write_u32(9 + request.length).await.unwrap();
                        peer.write_u8(7).await.unwrap();
                        peer.write_u32(request.index).await.unwrap();
                        peer.write_u32(request.begin).await.unwrap();
                        peer.write_all(&data[start..start + request.length as usize])
                            .await
                            .unwrap();
                        served.insert(request.index);
                    }
                    Message::Cancel(_) => {
                        panic!("Preference change cancelled an in-flight request")
                    }
                    _ => {}
                }
            }
        }
    });
    let handle = session
        .add_torrent(
            AddTorrent::from_bytes(adapter::create_fixture("live", &data)),
            Some(AddTorrentOptions {
                sequential: true,
                initial_peers: Some(vec![addr]),
                disable_trackers: true,
                ..Default::default()
            }),
        )
        .await
        .unwrap()
        .into_handle()
        .unwrap();
    for (expected, sequential, boundary) in [
        (0, false, true),
        (31, true, false),
        (1, true, true),
        (2, false, false),
    ] {
        let (index, ack) = tokio::time::timeout(Duration::from_secs(10), request_rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            index, expected,
            "The next piece did not reflect the live preference change"
        );
        session
            .update_piece_preferences(&handle, Some(sequential), Some(boundary))
            .await
            .unwrap();
        assert_eq!(handle.piece_preferences(), (sequential, boundary));
        assert!(!handle.is_paused());
        ack.send(()).unwrap();
    }
    tokio::time::timeout(Duration::from_secs(15), handle.wait_until_completed())
        .await
        .unwrap()
        .unwrap();
    peer_task.await.unwrap();
    assert_eq!(std::fs::read(files.path().join("live")).unwrap(), data);
    session.stop_and_wait().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn legacy_session_without_piece_preferences_restores_disabled_defaults() {
    let state = tempfile::tempdir().unwrap();
    let files = tempfile::tempdir().unwrap();
    let cfg = config(state.path(), files.path());
    let engine = Engine::open(cfg.clone()).await.unwrap();
    let id = engine
        .add_with_options(
            adapter::create_fixture("legacy", b"data"),
            cfg.download_directory.clone(),
            vec![0],
            TorrentAddOptions {
                paused: true,
                sequential: true,
                first_last_pieces: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    wait(&engine, |s| s.torrents[0].state == "paused").await;
    engine.shutdown().await.unwrap();
    drop(engine);
    let path = state.path().join("session/session.json");
    let mut saved: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    for item in saved["torrents"].as_object_mut().unwrap().values_mut() {
        item.as_object_mut().unwrap().remove("sequential");
        item.as_object_mut().unwrap().remove("first_last_pieces");
    }
    std::fs::write(path, serde_json::to_vec(&saved).unwrap()).unwrap();
    let engine = Engine::open(cfg).await.unwrap();
    let snapshot = wait(&engine, |s| s.torrents[0].state == "paused").await;
    assert_eq!(snapshot.torrents[0].id, id);
    assert!(!snapshot.torrents[0].sequential && !snapshot.torrents[0].first_last_pieces);
    engine.shutdown().await.unwrap();
}

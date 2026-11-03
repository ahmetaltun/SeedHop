use seedhop_core::{CoreError, Engine, EngineConfig, adapter};
use std::time::Duration;

fn config(state: &std::path::Path, downloads: &std::path::Path) -> EngineConfig {
    EngineConfig {
        state_directory: state.to_string_lossy().into(),
        download_directory: downloads.to_string_lossy().into(),
        allow_network_discovery: false,
        allow_local_network: true,
        listen_port: 0,
    }
}

#[allow(dead_code)]
mod support;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn force_recheck_detects_v1_corruption_and_preserves_intent() {
    let state = tempfile::tempdir().unwrap();
    let downloads = tempfile::tempdir().unwrap();
    let cfg = config(state.path(), downloads.path());
    let engine = Engine::open(cfg.clone()).await.unwrap();
    let content = vec![42; 65536];
    let path = downloads.path().join("data.bin");
    std::fs::write(&path, &content).unwrap();
    let id = engine
        .add(
            adapter::create_fixture("data.bin", &content),
            cfg.download_directory.clone(),
            vec![0],
            true,
            true,
        )
        .await
        .unwrap();
    support::wait_for(&engine, |t| t.state == "paused").await;
    std::fs::write(&path, vec![43; content.len()]).unwrap();
    engine.force_recheck(id.clone()).await.unwrap();
    assert!(engine.snapshot().unwrap().torrents[0].paused_intent);
    let checked = support::wait_for(&engine, |t| t.state == "paused").await;
    assert_eq!(checked.id, id);
    assert_eq!(checked.downloaded_bytes, 0);
    assert!(checked.files[0].selected);
    std::fs::write(&path, &content).unwrap();
    engine.force_recheck(id.clone()).await.unwrap();
    assert_eq!(
        support::wait_for(&engine, |t| t.state == "paused")
            .await
            .downloaded_bytes,
        content.len() as u64
    );
    engine.set_paused(id.clone(), false).await.unwrap();
    engine.force_recheck(id.clone()).await.unwrap();
    assert!(!engine.snapshot().unwrap().torrents[0].paused_intent);
    support::wait_for(&engine, |t| t.state == "seeding").await;
    engine.shutdown().await.unwrap();
    drop(engine);
    let engine = Engine::open(cfg).await.unwrap();
    assert_eq!(
        support::wait_for(&engine, |t| t.state == "seeding")
            .await
            .id,
        id
    );
    engine.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pause_selection_limits_and_removal_survive_restart() {
    let state = tempfile::tempdir().unwrap();
    let downloads = tempfile::tempdir().unwrap();
    let cfg = config(state.path(), downloads.path());
    let engine = Engine::open(cfg.clone()).await.unwrap();
    let bytes = adapter::create_fixture("hello.txt", b"hello from SeedHop");
    let preview = engine
        .inspect("file".into(), None, bytes.clone())
        .await
        .unwrap();
    assert_eq!(preview.files[0].size, 18);
    let id = engine
        .add(
            bytes.clone(),
            cfg.download_directory.clone(),
            vec![0],
            true,
            false,
        )
        .await
        .unwrap();
    assert!(
        engine
            .add(bytes, cfg.download_directory.clone(), vec![0], true, false)
            .await
            .is_err()
    );
    engine.set_limits(500_000, 100_000).await.unwrap();
    engine.select_files(id.clone(), vec![0]).await.unwrap();
    engine.shutdown().await.unwrap();
    drop(engine);
    let engine = Engine::open(cfg.clone()).await.unwrap();
    let snapshot = engine.snapshot().unwrap();
    assert_eq!(snapshot.torrents.len(), 1);
    assert_eq!(snapshot.download_limit, 500_000);
    assert_eq!(snapshot.torrents[0].id, id);
    // Initial verification can still be running, but the saved paused intent must be true.
    let session: serde_json::Value =
        serde_json::from_slice(&std::fs::read(state.path().join("session/session.json")).unwrap())
            .unwrap();
    assert!(
        session["torrents"]
            .as_object()
            .unwrap()
            .values()
            .all(|t| t["is_paused"] == true)
    );
    engine.remove(id, false).await.unwrap();
    engine.shutdown().await.unwrap();
    drop(engine);
    let engine = Engine::open(cfg).await.unwrap();
    assert!(engine.snapshot().unwrap().torrents.is_empty());
    engine.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn metadata_resolution_is_explicitly_cancellable() {
    let state = tempfile::tempdir().unwrap();
    let downloads = tempfile::tempdir().unwrap();
    let engine = Engine::open(config(state.path(), downloads.path()))
        .await
        .unwrap();
    let tracker = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = tracker.local_addr().unwrap();
    let tracker_task = tokio::spawn(async move {
        let (socket, _) = tracker.accept().await.unwrap();
        std::future::pending::<()>().await;
        drop(socket);
    });
    let e = engine.clone();
    let task = tokio::spawn(async move {
        e.inspect(
            "cancel-me".into(),
            Some(format!("magnet:?xt=urn:btih:1111111111111111111111111111111111111111&tr=http://{address}/announce")),
            vec![],
        )
        .await
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    engine.cancel("cancel-me");
    let result = tokio::time::timeout(Duration::from_secs(3), task)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(result, Err(CoreError::Cancelled)));
    engine.shutdown().await.unwrap();
    tracker_task.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn corrupted_state_is_preserved_and_never_replaced() {
    let state = tempfile::tempdir().unwrap();
    let downloads = tempfile::tempdir().unwrap();
    std::fs::create_dir(state.path().join("session")).unwrap();
    let path = state.path().join("session/session.json");
    std::fs::write(&path, b"{broken").unwrap();
    assert!(matches!(
        Engine::open(config(state.path(), downloads.path())).await,
        Err(CoreError::CorruptState(_))
    ));
    assert_eq!(std::fs::read(path).unwrap(), b"{broken");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn existing_files_require_explicit_reuse_and_are_not_overwritten() {
    let state = tempfile::tempdir().unwrap();
    let downloads = tempfile::tempdir().unwrap();
    std::fs::write(downloads.path().join("existing.txt"), b"do not overwrite").unwrap();
    let cfg = config(state.path(), downloads.path());
    let engine = Engine::open(cfg.clone()).await.unwrap();
    assert!(
        engine
            .add(
                adapter::create_fixture("existing.txt", b"new"),
                cfg.download_directory,
                vec![0],
                false,
                false
            )
            .await
            .is_err()
    );
    assert_eq!(
        std::fs::read(downloads.path().join("existing.txt")).unwrap(),
        b"do not overwrite"
    );
    engine.shutdown().await.unwrap();
}

#[test]
fn unsupported_and_unsafe_inputs_fail_without_disk_changes() {
    assert!(adapter::preview(b"not a torrent".to_vec()).is_err());
    assert!(adapter::preview(adapter::create_fixture("../outside", b"unsafe")).is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn abrupt_process_death_releases_lock_and_preserves_acknowledged_state() {
    use std::{
        io::{BufRead, BufReader},
        process::{Command, Stdio},
    };
    let state = tempfile::tempdir().unwrap();
    let downloads = tempfile::tempdir().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_seedhop-lab"))
        .arg("hold")
        .arg(state.path())
        .arg(downloads.path())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let stdout = child.stdout.take().unwrap();
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut line = String::new();
        let _ = BufReader::new(stdout).read_line(&mut line);
        let _ = sender.send(line);
    });
    let ready = receiver.recv_timeout(Duration::from_secs(10));
    if !ready.as_ref().is_ok_and(|line| line.trim() == "READY") {
        let _ = child.kill();
        let _ = child.wait();
        panic!("child engine did not start: {ready:?}");
    }
    assert!(matches!(
        Engine::open(config(state.path(), downloads.path())).await,
        Err(CoreError::Busy)
    ));
    child.kill().unwrap();
    child.wait().unwrap();
    let restored = Engine::open(config(state.path(), downloads.path()))
        .await
        .unwrap();
    let snapshot = restored.snapshot().unwrap();
    assert_eq!(snapshot.torrents.len(), 1);
    assert_eq!(snapshot.download_limit, 123_000);
    let session: serde_json::Value =
        serde_json::from_slice(&std::fs::read(state.path().join("session/session.json")).unwrap())
            .unwrap();
    assert!(
        session["torrents"]
            .as_object()
            .unwrap()
            .values()
            .all(|t| t["is_paused"] == true)
    );
    restored.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn persistence_failure_is_reported_and_unrelated_files_are_retained() {
    use std::os::unix::fs::PermissionsExt;
    let state = tempfile::tempdir().unwrap();
    let downloads = tempfile::tempdir().unwrap();
    let cfg = config(state.path(), downloads.path());
    let engine = Engine::open(cfg.clone()).await.unwrap();
    let id = engine
        .add(
            adapter::create_fixture("data.txt", b"data"),
            cfg.download_directory,
            vec![0],
            true,
            false,
        )
        .await
        .unwrap();
    std::fs::write(downloads.path().join("unrelated.txt"), b"keep this").unwrap();
    support::wait_for(&engine, |t| t.state == "paused").await;
    let directory = state.path().join("session");
    std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o500)).unwrap();
    let result = engine.select_files(id.clone(), vec![0]).await;
    let paused_retry = engine.set_paused(id.clone(), true).await;
    let recheck_retry = engine.force_recheck(id.clone()).await;
    std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert!(
        result.is_err(),
        "a failed durable write must reach the caller"
    );
    assert!(
        paused_retry.is_err(),
        "an already-paused retry must persist before success"
    );
    assert!(
        recheck_retry.is_err(),
        "recheck must report a failed persistence write"
    );
    assert_eq!(engine.snapshot().unwrap().torrents[0].state, "paused");
    engine.remove(id, true).await.unwrap();
    assert_eq!(
        std::fs::read(downloads.path().join("unrelated.txt")).unwrap(),
        b"keep this"
    );
    engine.shutdown().await.unwrap();
}

fn selective_fixture(name: &str, content: &[u8]) -> Vec<u8> {
    use sha1::{Digest, Sha1};
    let payload = content.len() - 127;
    let mut bytes = format!("d4:infod5:filesld6:lengthi{payload}e4:pathl11:payload.bineed6:lengthi127e4:pathl10:shared.txteee4:name{}:{}12:piece lengthi16384e6:pieces{}:", name.len(), name, content.chunks(16384).count() * 20).into_bytes();
    for piece in content.chunks(16384) {
        bytes.extend_from_slice(&Sha1::digest(piece));
    }
    bytes.extend_from_slice(b"ee");
    bytes
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unchecked_collision_and_shared_piece_bytes_stay_out_of_payload_files() {
    let swarm = support::Swarm::with_seed_files(
        32768 + 211,
        |content| selective_fixture("First torrent", content),
        |path, content| {
            let split = content.len() - 127;
            std::fs::write(path.join("payload.bin"), &content[..split]).unwrap();
            std::fs::write(path.join("shared.txt"), &content[split..]).unwrap();
        },
    )
    .await;
    for existing in [false, true] {
        let state = tempfile::tempdir().unwrap();
        let downloads = tempfile::tempdir().unwrap();
        let cfg = config(state.path(), downloads.path());
        if existing {
            std::fs::write(downloads.path().join("shared.txt"), b"keep this").unwrap();
        }
        let engine = Engine::open(cfg.clone()).await.unwrap();
        let id = engine
            .add(
                swarm.metadata.clone(),
                cfg.download_directory,
                vec![0],
                false,
                false,
            )
            .await
            .unwrap();
        support::wait_for(&engine, |item| item.state == "seeding").await;
        assert_eq!(
            std::fs::read(downloads.path().join("payload.bin")).unwrap(),
            &swarm.content[..swarm.content.len() - 127]
        );
        if existing {
            assert_eq!(
                std::fs::read(downloads.path().join("shared.txt")).unwrap(),
                b"keep this"
            );
            assert!(engine.select_files(id.clone(), vec![0, 1]).await.is_err());
            engine.remove(id, true).await.unwrap();
            assert_eq!(
                std::fs::read(downloads.path().join("shared.txt")).unwrap(),
                b"keep this"
            );
        } else {
            assert!(!downloads.path().join("shared.txt").exists());
            engine.shutdown().await.unwrap();
            drop(engine);
            let engine = Engine::open(config(state.path(), downloads.path()))
                .await
                .unwrap();
            support::wait_for(&engine, |item| item.state == "seeding").await;
            engine.select_files(id, vec![0, 1]).await.unwrap();
            support::wait_for(&engine, |item| item.state == "seeding").await;
            assert_eq!(
                std::fs::read(downloads.path().join("shared.txt")).unwrap(),
                &swarm.content[swarm.content.len() - 127..]
            );
            engine.shutdown().await.unwrap();
            continue;
        }
        engine.shutdown().await.unwrap();
    }
    swarm.seed.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_imports_use_torrent_folders_and_preserve_existing_library_layout() {
    let state = tempfile::tempdir().unwrap();
    let downloads = tempfile::tempdir().unwrap();
    let cfg = config(state.path(), downloads.path());
    let engine = Engine::open(cfg.clone()).await.unwrap();
    for name in ["First torrent", "Second torrent"] {
        engine
            .import_with_options(
                selective_fixture(name, &[42; 32768]),
                cfg.download_directory.clone(),
                vec![0],
                seedhop_core::TorrentAddOptions {
                    paused: true,
                    ..Default::default()
                },
            )
            .await
            .unwrap();
    }
    for item in engine.snapshot().unwrap().torrents {
        assert_eq!(
            std::path::Path::new(&item.destination),
            downloads.path().canonicalize().unwrap().join(&item.name)
        );
        assert!(
            std::path::Path::new(&item.destination)
                .join("payload.bin")
                .exists()
        );
        assert!(
            !std::path::Path::new(&item.destination)
                .join("shared.txt")
                .exists()
        );
    }
    assert!(!downloads.path().join("payload.bin").exists());
    engine.shutdown().await.unwrap();
    drop(engine);
    let engine = Engine::open(cfg).await.unwrap();
    assert_eq!(engine.snapshot().unwrap().torrents.len(), 2);
    engine.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn remove_native_import_cleans_empty_folders_and_preserves_other_files() {
    for (delete_files, unrelated) in [(true, false), (true, true), (false, false)] {
        let state = tempfile::tempdir().unwrap();
        let downloads = tempfile::tempdir().unwrap();
        let cfg = config(state.path(), downloads.path());
        let engine = Engine::open(cfg.clone()).await.unwrap();
        let mut metadata = selective_fixture("Folder cleanup", &[42; 32768]);
        let needle = b"4:pathl11:payload.bin";
        let offset = metadata
            .windows(needle.len())
            .position(|v| v == needle)
            .unwrap();
        metadata.splice(
            offset..offset + needle.len(),
            b"4:pathl6:nested11:payload.bin".iter().copied(),
        );
        let id = engine
            .import_with_options(
                metadata,
                cfg.download_directory.clone(),
                vec![0],
                seedhop_core::TorrentAddOptions {
                    paused: true,
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let root = downloads.path().join("Folder cleanup");
        assert!(root.join("nested/payload.bin").exists());
        if unrelated {
            std::fs::write(root.join("keep.txt"), b"unrelated").unwrap();
        }
        engine.shutdown().await.unwrap();
        drop(engine);
        let engine = Engine::open(cfg).await.unwrap();
        support::wait_for(&engine, |item| item.state == "paused").await;
        engine.remove(id, delete_files).await.unwrap();
        assert!(engine.snapshot().unwrap().torrents.is_empty());
        assert!(downloads.path().is_dir(), "download parent was deleted");
        if !delete_files {
            assert!(root.join("nested/payload.bin").exists());
        } else if unrelated {
            assert_eq!(std::fs::read(root.join("keep.txt")).unwrap(), b"unrelated");
            assert!(!root.join("nested").exists());
            assert!(!root.join(".seedhop-parts").exists());
        } else {
            assert!(
                !root.exists(),
                "empty torrent destination survived deletion: {:?}",
                std::fs::read_dir(&root).map(|entries| entries
                    .map(|entry| entry.unwrap().path())
                    .collect::<Vec<_>>())
            );
        }
        engine.shutdown().await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn remove_preserves_explicit_download_parent_even_when_named_after_torrent() {
    let state = tempfile::tempdir().unwrap();
    let downloads = tempfile::tempdir().unwrap();
    let cfg = config(state.path(), downloads.path());
    let engine = Engine::open(cfg.clone()).await.unwrap();
    let name = downloads.path().file_name().unwrap().to_str().unwrap();
    let id = engine
        .add(
            adapter::create_fixture(name, &[42; 32768]),
            cfg.download_directory,
            vec![0],
            true,
            false,
        )
        .await
        .unwrap();
    support::wait_for(&engine, |item| item.state == "paused").await;
    engine.remove(id, true).await.unwrap();
    assert!(downloads.path().is_dir());
    assert_eq!(std::fs::read_dir(downloads.path()).unwrap().count(), 0);
    engine.shutdown().await.unwrap();
}

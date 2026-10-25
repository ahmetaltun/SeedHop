use seedhop_core::{CoreError, Engine, EngineConfig, adapter};
use std::time::Duration;

fn config(state: &std::path::Path, downloads: &std::path::Path) -> EngineConfig {
    EngineConfig {
        state_directory: state.to_string_lossy().into(),
        download_directory: downloads.to_string_lossy().into(),
        allow_network_discovery: false,
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

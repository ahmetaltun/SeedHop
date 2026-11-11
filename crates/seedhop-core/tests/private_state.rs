//! Runtime acceptance is deferred. All-target compiler checks compile this suite.
use seedhop_core::{
    CoreError, Engine, NetworkInterface, NetworkPolicy, PrivateEngineConfig, ScheduleConfig,
    TorrentAddOptions, adapter,
};
use std::{collections::BTreeMap, path::Path, time::Duration};

fn config(downloads: &Path, network_policy: NetworkPolicy) -> PrivateEngineConfig {
    PrivateEngineConfig {
        download_directory: downloads.display().to_string(),
        network_policy,
        peer_encryption: librqbit::EncryptionPolicy::Require,
        allow_network_discovery: false,
    }
}

fn files(root: &Path) -> BTreeMap<String, Vec<u8>> {
    std::fs::read_dir(root)
        .unwrap()
        .map(|entry| {
            let path = entry.unwrap().path();
            (
                path.file_name().unwrap().to_string_lossy().into_owned(),
                std::fs::read(path).unwrap(),
            )
        })
        .collect()
}

fn private_rejection<T>(result: seedhop_core::Result<T>) {
    assert!(matches!(
        result,
        Err(CoreError::Validation {
            code: "session.private_unsupported",
            ..
        })
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn memory_engine_ignores_disk_state_and_rejects_persistent_mutations() {
    let downloads = tempfile::tempdir().unwrap();
    for name in [
        "engine.lock",
        "statistics.json",
        "connection-settings.json",
        "network-policy.json",
        "layout-transaction.json",
    ] {
        std::fs::write(downloads.path().join(name), b"not engine state").unwrap();
    }
    let before = files(downloads.path());
    let engine = Engine::open_private(config(downloads.path(), NetworkPolicy::default()))
        .await
        .unwrap();
    assert!(engine.is_private());
    assert!(engine.retain_lease().is_none());
    assert_eq!(engine.snapshot().unwrap().peer_encryption, "require");
    assert!(engine.snapshot().unwrap().torrents.is_empty());
    let preview = engine
        .inspect(
            "preview".into(),
            None,
            adapter::create_fixture("private", b"payload"),
        )
        .await
        .unwrap();
    assert_eq!(preview.name, "private");
    engine.set_limits(1024, 2048).await.unwrap();
    assert_eq!(engine.snapshot().unwrap().download_limit, 1024);
    private_rejection(engine.statistics_preferences());
    private_rejection(engine.set_statistics_retention(true).await);
    private_rejection(engine.set_peer_encryption("allow".into()).await);
    private_rejection(engine.set_network_policy("{}".into()).await);
    private_rejection(engine.set_schedule(ScheduleConfig::default()).await);
    private_rejection(engine.set_alternative_limits(10, 20, true).await);
    private_rejection(engine.relocate("missing".into(), "/missing".into()).await);
    private_rejection(engine.rename_file("missing".into(), 0, "new".into()).await);
    private_rejection(
        engine
            .resolve_storage_recovery("missing".into(), false)
            .await,
    );
    private_rejection(engine.remove("missing".into(), true).await);
    private_rejection(engine.select_files("missing".into(), vec![0]).await);
    private_rejection(engine.set_file_priorities("missing".into(), vec![1]).await);
    assert!(
        engine
            .import_with_options(
                Vec::new(),
                downloads.path().display().to_string(),
                vec![0],
                TorrentAddOptions::default()
            )
            .await
            .is_err()
    );
    assert!(
        engine
            .add_with_options(
                Vec::new(),
                downloads.path().display().to_string(),
                vec![0],
                TorrentAddOptions {
                    reuse: true,
                    ..Default::default()
                }
            )
            .await
            .is_err()
    );
    // Exercise maintenance after the immediate mutation checks; no state save
    // or rejected network-policy mutation may have stopped this private engine.
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert!(!engine.snapshot().unwrap().network_blocked);
    assert!(engine.snapshot().unwrap().torrents.is_empty());
    engine.shutdown().await.unwrap();
    drop(engine);
    assert_eq!(files(downloads.path()), before);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn private_start_never_falls_back_from_invalid_or_missing_protection() {
    let downloads = tempfile::tempdir().unwrap();
    let invalid = NetworkPolicy {
        version: 999,
        ..NetworkPolicy::default()
    };
    assert!(matches!(
        Engine::open_private(config(downloads.path(), invalid)).await,
        Err(CoreError::InvalidInput(_))
    ));
    let unavailable = NetworkPolicy {
        interface: Some(NetworkInterface {
            name: "seedhop-no-such-vpn".into(),
            index: u32::MAX,
            addresses: vec!["10.8.0.2".parse().unwrap()],
        }),
        dns_server: Some("1.1.1.1".parse().unwrap()),
        ..NetworkPolicy::default()
    };
    assert!(matches!(
        Engine::open_private(config(downloads.path(), unavailable)).await,
        Err(CoreError::NetworkBlocked)
    ));
    assert!(files(downloads.path()).is_empty());
}

async fn settled(engine: &Engine, id: &str) {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let snapshot = engine.snapshot().unwrap();
            let item = snapshot.torrents.iter().find(|item| item.id == id).unwrap();
            assert_ne!(item.state, "error", "private storage initialization failed");
            if item.state != "checking" {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn private_payloads_use_fresh_roots_and_survive_end_without_restoration() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let parent = tempfile::tempdir().unwrap();
    let foreign = parent.path().join("private-file");
    std::fs::write(&foreign, b"ordinary existing file").unwrap();
    let bytes = adapter::create_fixture("private-file", b"private bytes");
    let engine = Engine::open_private(config(parent.path(), NetworkPolicy::default()))
        .await
        .unwrap();
    assert!(
        engine
            .add(
                bytes.clone(),
                parent.path().display().to_string(),
                vec![0],
                true,
                true
            )
            .await
            .is_err()
    );
    assert_eq!(std::fs::read_dir(parent.path()).unwrap().count(), 1);
    let outside = tempfile::tempdir().unwrap();
    assert!(
        engine
            .add(
                bytes.clone(),
                outside.path().display().to_string(),
                vec![0],
                true,
                false
            )
            .await
            .is_err()
    );
    assert!(files(outside.path()).is_empty());
    let id = engine
        .import_with_options(
            bytes.clone(),
            parent.path().display().to_string(),
            vec![0],
            TorrentAddOptions {
                paused: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    settled(&engine, &id).await;
    let root = std::path::PathBuf::from(&engine.snapshot().unwrap().torrents[0].destination);
    assert_eq!(root.parent().unwrap().parent().unwrap(), parent.path());
    assert!(!root.display().to_string().contains(&id));
    assert_eq!(
        std::fs::metadata(&root).unwrap().permissions().mode() & 0o777,
        0o700
    );
    let payload = root.join("private-file");
    assert_eq!(
        std::fs::metadata(&payload).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(std::fs::metadata(&payload).unwrap().nlink(), 1);
    std::fs::write(&payload, b"private bytes").unwrap();
    engine.force_recheck(id.clone()).await.unwrap();
    settled(&engine, &id).await;
    private_rejection(engine.select_files(id.clone(), vec![0]).await);
    private_rejection(engine.set_file_priorities(id.clone(), vec![2]).await);
    engine.shutdown().await.unwrap();
    drop(engine);
    assert_eq!(std::fs::read(&payload).unwrap(), b"private bytes");
    assert_eq!(std::fs::read(&foreign).unwrap(), b"ordinary existing file");
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
    let fresh = Engine::open_private(config(parent.path(), NetworkPolicy::default()))
        .await
        .unwrap();
    assert!(fresh.snapshot().unwrap().torrents.is_empty());
    fresh
        .add(
            bytes,
            parent.path().display().to_string(),
            vec![0],
            true,
            false,
        )
        .await
        .unwrap();
    let new_root = std::path::PathBuf::from(&fresh.snapshot().unwrap().torrents[0].destination);
    assert_ne!(root, new_root);
    fresh.shutdown().await.unwrap();
    assert_eq!(std::fs::read(&payload).unwrap(), b"private bytes");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn private_recheck_refuses_replacement_directories() {
    let parent = tempfile::tempdir().unwrap();
    let engine = Engine::open_private(config(parent.path(), NetworkPolicy::default()))
        .await
        .unwrap();
    let id = engine
        .add(
            adapter::create_fixture("payload", b"data"),
            parent.path().display().to_string(),
            vec![0],
            true,
            false,
        )
        .await
        .unwrap();
    settled(&engine, &id).await;
    let root = std::path::PathBuf::from(&engine.snapshot().unwrap().torrents[0].destination);
    let moved = root.with_file_name("owned-original");
    std::fs::rename(&root, &moved).unwrap();
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("payload"), b"replacement must remain untouched").unwrap();
    assert!(engine.force_recheck(id.clone()).await.is_err());
    assert!(engine.set_paused(id, false).await.is_err());
    engine.shutdown().await.unwrap();
    assert_eq!(
        std::fs::read(root.join("payload")).unwrap(),
        b"replacement must remain untouched"
    );
    assert!(moved.join("payload").exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn private_initial_file_selection_keeps_boundary_bytes_without_hash_paths() {
    use sha1::{Digest, Sha1};
    let mut bytes = b"d4:infod5:filesld6:lengthi2e4:pathl3:oneeed6:lengthi2e4:pathl3:twoeee4:name4:root12:piece lengthi16384e6:pieces20:".to_vec();
    bytes.extend_from_slice(&Sha1::digest(b"abcd"));
    bytes.extend_from_slice(b"ee");
    let parent = tempfile::tempdir().unwrap();
    let engine = Engine::open_private(config(parent.path(), NetworkPolicy::default()))
        .await
        .unwrap();
    let id = engine
        .add(
            bytes,
            parent.path().display().to_string(),
            vec![0],
            true,
            false,
        )
        .await
        .unwrap();
    settled(&engine, &id).await;
    let root = std::path::PathBuf::from(&engine.snapshot().unwrap().torrents[0].destination);
    assert!(root.join("one").is_file());
    assert!(!root.join("two").exists());
    assert!(root.join(".seedhop-parts/1").is_file());
    assert_eq!(
        std::fs::read_dir(root.join(".seedhop-parts"))
            .unwrap()
            .count(),
        1
    );
    assert!(!root.join(".seedhop-parts").join(&id).exists());
    engine.remove(id, false).await.unwrap();
    engine.shutdown().await.unwrap();
    assert!(root.join("one").is_file());
    assert!(root.join(".seedhop-parts/1").is_file());
}

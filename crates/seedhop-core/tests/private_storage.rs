//! Direct storage-capability regressions; compiled now, runtime execution deferred.
use librqbit::{
    AddTorrent, AddTorrentOptions, ManagedTorrent, Session, SessionOptions,
    storage::{
        StorageFactory, StorageFactoryExt, TorrentStorage,
        filesystem::PinnedFilesystemStorageFactory,
    },
};
use seedhop_core::adapter;
use std::{fs::File, os::unix::fs::PermissionsExt, path::Path, sync::Arc};

async fn fixture(
    root: &Path,
    bytes: Vec<u8>,
) -> (
    Arc<Session>,
    Arc<ManagedTorrent>,
    PinnedFilesystemStorageFactory,
) {
    let factory =
        PinnedFilesystemStorageFactory::new(root.to_owned(), File::open(root).unwrap()).unwrap();
    let session = Session::new_with_opts(
        root.to_owned(),
        SessionOptions {
            dht: None,
            listen: None,
            persistence: None,
            disable_local_service_discovery: true,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let handle = session
        .add_torrent(
            AddTorrent::TorrentFileBytes(bytes.into()),
            Some(AddTorrentOptions {
                paused: true,
                output_folder: Some(root.display().to_string()),
                storage_factory: Some(factory.clone().boxed()),
                ..Default::default()
            }),
        )
        .await
        .unwrap()
        .into_handle()
        .unwrap();
    (session, handle, factory)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pinned_storage_reopens_owned_inodes_and_refuses_replacement_files() {
    let root = tempfile::Builder::new()
        .permissions(std::fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap();
    let (session, handle, factory) =
        fixture(root.path(), adapter::create_fixture("payload", b"abc")).await;
    let path = root.path().join("payload");
    std::fs::write(&path, b"abc").unwrap();
    let reopened = handle
        .with_metadata(|metadata| factory.create_and_init(handle.shared(), metadata))
        .unwrap()
        .unwrap();
    let mut data = [0; 3];
    reopened.pread_exact(0, 0, &mut data).unwrap();
    assert_eq!(&data, b"abc");
    drop(reopened);
    std::fs::rename(&path, root.path().join("owned-original")).unwrap();
    std::fs::write(&path, b"unrelated replacement").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert!(
        handle
            .with_metadata(|metadata| factory.create_and_init(handle.shared(), metadata))
            .unwrap()
            .is_err()
    );
    session.stop_and_wait().await.unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"unrelated replacement");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pinned_storage_refuses_replaced_nested_directories() {
    use sha1::{Digest, Sha1};
    let mut bytes = b"d4:infod5:filesld6:lengthi3e4:pathl6:nested7:payloadeee4:name4:root12:piece lengthi16384e6:pieces20:".to_vec();
    bytes.extend_from_slice(&Sha1::digest(b"abc"));
    bytes.extend_from_slice(b"ee");
    let root = tempfile::Builder::new()
        .permissions(std::fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap();
    let (session, handle, factory) = fixture(root.path(), bytes).await;
    let nested = root.path().join("nested");
    std::fs::rename(&nested, root.path().join("owned-nested")).unwrap();
    std::fs::create_dir(&nested).unwrap();
    std::fs::set_permissions(&nested, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::fs::write(nested.join("payload"), b"foreign nested data").unwrap();
    assert!(
        handle
            .with_metadata(|metadata| factory.create_and_init(handle.shared(), metadata))
            .unwrap()
            .is_err()
    );
    session.stop_and_wait().await.unwrap();
    assert_eq!(
        std::fs::read(nested.join("payload")).unwrap(),
        b"foreign nested data"
    );
}

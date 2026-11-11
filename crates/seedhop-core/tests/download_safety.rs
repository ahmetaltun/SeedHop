use seedhop_core::{Engine, EngineConfig, adapter, storage};
use std::{
    os::{fd::AsRawFd, unix::fs::PermissionsExt},
    path::Path,
};

#[cfg(target_os = "macos")]
fn quarantine(path: &Path) -> Vec<u8> {
    let file = std::fs::File::open(path).unwrap();
    let mut value = vec![0u8; 1024];
    let length = unsafe {
        libc::fgetxattr(
            file.as_raw_fd(),
            c"com.apple.quarantine".as_ptr(),
            value.as_mut_ptr().cast(),
            value.len(),
            0,
            0,
        )
    };
    assert!(length > 0, "missing quarantine: {}", path.display());
    value.truncate(length as usize);
    value
}

#[test]
fn state_directories_and_atomic_settings_are_owner_only() {
    let root = tempfile::tempdir().unwrap();
    let state = root.path().join("state");
    std::fs::create_dir(&state).unwrap();
    std::fs::set_permissions(&state, std::fs::Permissions::from_mode(0o755)).unwrap();
    let _lease = storage::LibraryLease::acquire(&state).unwrap();
    assert_eq!(
        std::fs::metadata(&state).unwrap().permissions().mode() & 0o777,
        0o700
    );
    storage::atomic_write(&state.join("settings.json"), b"{}").unwrap();
    assert_eq!(
        std::fs::metadata(state.join("settings.json"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    let outside = root.path().join("outside");
    std::fs::create_dir(&outside).unwrap();
    std::os::unix::fs::symlink(&outside, root.path().join("link")).unwrap();
    assert!(storage::LibraryLease::acquire(&root.path().join("link")).is_err());
}

#[cfg(target_os = "macos")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn payloads_receive_quarantine_and_copies_preserve_it() {
    let state = tempfile::tempdir().unwrap();
    let files = tempfile::tempdir().unwrap();
    let destination = tempfile::tempdir().unwrap();
    let engine = Engine::open(EngineConfig {
        state_directory: state.path().display().to_string(),
        download_directory: files.path().display().to_string(),
        allow_network_discovery: false,
        allow_local_network: true,
        listen_port: 0,
    })
    .await
    .unwrap();
    let id = engine
        .add(
            adapter::create_fixture("payload", b"data"),
            files.path().display().to_string(),
            vec![0],
            true,
            true,
        )
        .await
        .unwrap();
    for _ in 0..100 {
        if files.path().join("payload").exists() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let original = quarantine(&files.path().join("payload"));
    assert!(String::from_utf8_lossy(&original).starts_with("0081;"));
    engine
        .relocate(id, destination.path().display().to_string())
        .await
        .unwrap();
    let copied = quarantine(&destination.path().join("payload"));
    assert_eq!(original, copied);
    engine.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reused_hardlinks_and_special_files_are_refused_without_modifying_other_files() {
    for special in [false, true] {
        let state = tempfile::tempdir().unwrap();
        let files = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let sentinel = outside.path().join("original");
        std::fs::write(&sentinel, b"unrelated data").unwrap();
        let payload = files.path().join("payload");
        if special {
            use std::os::unix::ffi::OsStrExt;
            let path = std::ffi::CString::new(payload.as_os_str().as_bytes()).unwrap();
            assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
        } else {
            std::fs::hard_link(&sentinel, &payload).unwrap();
        }
        let engine = Engine::open(EngineConfig {
            state_directory: state.path().display().to_string(),
            download_directory: files.path().display().to_string(),
            allow_network_discovery: false,
            allow_local_network: true,
            listen_port: 0,
        })
        .await
        .unwrap();
        let result = engine
            .add(
                adapter::create_fixture("payload", b"torrent data"),
                files.path().display().to_string(),
                vec![0],
                false,
                true,
            )
            .await;
        if result.is_ok() {
            tokio::time::timeout(std::time::Duration::from_secs(5), async {
                loop {
                    if engine
                        .snapshot()
                        .unwrap()
                        .torrents
                        .iter()
                        .any(|t| t.state == "error")
                    {
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                }
            })
            .await
            .unwrap();
        }
        assert_eq!(std::fs::read(&sentinel).unwrap(), b"unrelated data");
        engine.shutdown().await.unwrap();
    }
}

#[test]
fn oversized_metadata_integers_return_errors_instead_of_panicking() {
    let malformed = b"d4:infod9:file treed5:emptyd0:d6:lengthi0eeee12:meta versi\xff\xffi22e4:name5:empty12:piece lengthi163800000004eee";
    assert!(adapter::preview(malformed.to_vec()).is_err());
    for length in [
        "18446744073709551616",
        "999999999999999999999999999999999999999999",
    ] {
        let input = format!("d{length}:x");
        assert!(adapter::preview(input.into_bytes()).is_err());
    }
}

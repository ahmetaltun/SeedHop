use seedhop_core::{CoreError, Engine, EngineConfig, TrackerDetails, adapter};
use sha1::{Digest, Sha1};
use sha2::Sha256;
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

fn string(value: &[u8]) -> Vec<u8> {
    [value.len().to_string().as_bytes(), b":", value].concat()
}
fn dict(mut entries: Vec<(&str, Vec<u8>)>) -> Vec<u8> {
    entries.sort_by_key(|(key, _)| *key);
    [
        b"d".to_vec(),
        entries
            .into_iter()
            .flat_map(|(k, v)| [string(k.as_bytes()), v].concat())
            .collect(),
        b"e".to_vec(),
    ]
    .concat()
}
fn fixture(urls: &[String], format: &str, private: bool) -> Vec<u8> {
    let payload = b"test payload";
    let name = b"payload.bin";
    let mut info = vec![
        ("name", string(name)),
        ("piece length", b"i16384e".to_vec()),
    ];
    if format != "v1" {
        info.push(("meta version", b"i2e".to_vec()));
        info.push((
            "file tree",
            dict(vec![(
                "payload.bin",
                dict(vec![(
                    "",
                    dict(vec![
                        ("length", format!("i{}e", payload.len()).into_bytes()),
                        ("pieces root", string(&Sha256::digest(payload))),
                    ]),
                )]),
            )]),
        ));
    }
    if format != "v2" {
        info.push(("length", format!("i{}e", payload.len()).into_bytes()));
        info.push(("pieces", string(&Sha1::digest(payload))));
    }
    if private {
        info.push(("private", b"i1e".to_vec()));
    }
    let mut root = vec![("info", dict(info))];
    if format != "v1" {
        root.push(("piece layers", dict(vec![])));
    }
    if let Some(url) = urls.first() {
        root.push(("announce", string(url.as_bytes())));
    }
    if urls.len() > 1 {
        root.push((
            "announce-list",
            [
                b"l".to_vec(),
                urls.iter()
                    .flat_map(|url| [b"l".to_vec(), string(url.as_bytes()), b"e".to_vec()].concat())
                    .collect(),
                b"e".to_vec(),
            ]
            .concat(),
        ));
    }
    dict(root)
}

struct Tracker {
    requests: Arc<Mutex<Vec<String>>>,
    url: String,
    task: tokio::task::JoinHandle<()>,
}
impl Tracker {
    async fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!(
            "http://{}/secret-passkey/announce?token=secret",
            listener.local_addr().unwrap()
        );
        let requests = Arc::new(Mutex::new(Vec::new()));
        let received = requests.clone();
        let task = tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                while !request.ends_with(b"\r\n\r\n") {
                    request.push(socket.read_u8().await.unwrap());
                    assert!(request.len() < 8192);
                }
                received
                    .lock()
                    .unwrap()
                    .push(String::from_utf8(request).unwrap());
                let body = b"d8:intervali1800e12:min intervali120e5:peers0:e";
                socket
                    .write_all(
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        )
                        .as_bytes(),
                    )
                    .await
                    .unwrap();
                socket.write_all(body).await.unwrap();
            }
        });
        Self {
            requests,
            url,
            task,
        }
    }
}
impl Drop for Tracker {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn details(engine: &Engine, id: &str, expected: usize) -> Vec<TrackerDetails> {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let rows = engine
                .torrent_details(id.into(), "trackers".into(), false)
                .unwrap()
                .trackers;
            if rows
                .iter()
                .filter(|r| r.reannounce_target_id.is_some())
                .count()
                == expected
            {
                return rows;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("tracker controls unavailable: {:?}", engine.snapshot()))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn live_controls_are_scoped_ephemeral_and_credential_safe_for_all_formats() {
    for format in ["v1", "v2", "hybrid"] {
        let tracker = Tracker::new().await;
        let state = tempfile::tempdir().unwrap();
        let files = tempfile::tempdir().unwrap();
        let engine = Engine::open(EngineConfig {
            state_directory: state.path().display().to_string(),
            download_directory: files.path().display().to_string(),
            allow_network_discovery: false,
            listen_port: 0,
        })
        .await
        .unwrap();
        let id = engine
            .add(
                fixture(std::slice::from_ref(&tracker.url), format, false),
                files.path().display().to_string(),
                vec![0],
                false,
                false,
            )
            .await
            .unwrap();
        let expected = if format == "hybrid" { 2 } else { 1 };
        let rows = details(&engine, &id, expected).await;
        let before = engine.snapshot().unwrap();
        let instance = before.instance_id.clone();
        assert!(before.torrents[0].can_reannounce);
        assert_eq!(tracker.requests.lock().unwrap().len(), expected);
        let json = serde_json::to_string(&rows).unwrap();
        assert!(!json.contains("secret") && !json.contains("passkey"));
        let target = rows[0].reannounce_target_id.clone().unwrap();
        let other = rows
            .iter()
            .find(|r| r.reannounce_target_id.as_ref() != Some(&target))
            .map(|r| r.hash.clone());
        let result = engine
            .force_reannounce(
                id.clone(),
                Some(vec![
                    target.clone(),
                    target.clone(),
                    "unknown-target".into(),
                ]),
                instance.clone(),
            )
            .await
            .unwrap();
        assert_eq!((result.scheduled, result.unavailable), (1, 1));
        let again = engine
            .force_reannounce(id.clone(), Some(vec![target.clone()]), instance.clone())
            .await
            .unwrap();
        assert_eq!(again.already_pending, 1);
        let pending = engine
            .torrent_details(id.clone(), "trackers".into(), false)
            .unwrap()
            .trackers;
        assert_eq!(pending.iter().filter(|r| r.reannounce_pending).count(), 1);
        if let Some(hash) = other {
            assert!(
                !pending
                    .iter()
                    .find(|r| r.hash == hash)
                    .unwrap()
                    .reannounce_pending
            );
        }
        let after = engine.snapshot().unwrap().torrents.remove(0);
        let original = &before.torrents[0];
        assert_eq!(
            (
                after.state,
                after.queue_position,
                after.paused_intent,
                after.downloaded_bytes,
                after.uploaded_bytes
            ),
            (
                original.state.clone(),
                original.queue_position,
                original.paused_intent,
                original.downloaded_bytes,
                original.uploaded_bytes
            )
        );
        assert!(matches!(
            engine
                .force_reannounce(id.clone(), None, "old-instance".into())
                .await,
            Err(CoreError::Validation {
                code: "input.reannounce_stale",
                ..
            })
        ));
        assert!(
            engine
                .force_reannounce(id.clone(), Some(vec![]), instance.clone())
                .await
                .is_err()
        );
        engine.set_paused(id.clone(), true).await.unwrap();
        assert!(
            engine
                .force_reannounce(id.clone(), None, instance.clone())
                .await
                .is_err()
        );
        assert!(
            engine
                .torrent_details(id.clone(), "trackers".into(), false)
                .unwrap()
                .trackers
                .iter()
                .all(|r| !r.reannounce_pending && r.reannounce_target_id.is_none())
        );
        engine.set_paused(id.clone(), false).await.unwrap();
        details(&engine, &id, expected).await;
        let retired = engine
            .force_reannounce(id.clone(), Some(vec![target]), instance.clone())
            .await
            .unwrap();
        assert_eq!(retired.unavailable, 1);
        engine.remove(id.clone(), false).await.unwrap();
        assert!(engine.force_reannounce(id, None, instance).await.is_err());
        engine.shutdown().await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn private_backups_and_trackerless_torrents_never_gain_controls() {
    let tracker = Tracker::new().await;
    let state = tempfile::tempdir().unwrap();
    let files = tempfile::tempdir().unwrap();
    let engine = Engine::open(EngineConfig {
        state_directory: state.path().display().to_string(),
        download_directory: files.path().display().to_string(),
        allow_network_discovery: false,
        listen_port: 0,
    })
    .await
    .unwrap();
    let backup = tracker.url.replace("secret-passkey", "backup-passkey");
    let id = engine
        .add(
            fixture(&[tracker.url.clone(), backup], "v2", true),
            files.path().display().to_string(),
            vec![0],
            false,
            false,
        )
        .await
        .unwrap();
    let rows = details(&engine, &id, 1).await;
    assert_eq!(tracker.requests.lock().unwrap().len(), 1);
    assert!(rows.iter().all(|r| r.reannounce_target_id.is_some()));
    // Existing private-torrent policy retains only the effective tracker.
    let instance = engine.snapshot().unwrap().instance_id;
    assert_eq!(
        engine
            .force_reannounce(id, None, instance.clone())
            .await
            .unwrap()
            .scheduled,
        1
    );
    let empty = engine
        .add(
            adapter::create_fixture("no-trackers", b"content"),
            files.path().display().to_string(),
            vec![0],
            false,
            false,
        )
        .await
        .unwrap();
    assert!(
        engine
            .force_reannounce(empty, None, instance)
            .await
            .is_err()
    );
    engine.shutdown().await.unwrap();
}

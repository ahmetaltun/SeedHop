use seedhop_core::{Engine, EngineConfig, adapter};
use serde_json::json;
use sha1::{Digest, Sha1};
use std::{
    hint::black_box,
    time::{Duration, Instant},
};

fn many_files(count: usize) -> Vec<u8> {
    let mut bytes = b"d4:infod5:filesl".to_vec();
    for i in 0..count {
        let name = format!("file-{i:05}.bin");
        bytes.extend_from_slice(format!("d6:lengthi1e4:pathl{}:{name}ee", name.len()).as_bytes());
    }
    bytes.extend_from_slice(b"e4:name12:Sample files12:piece lengthi16384e6:pieces20:");
    bytes.extend_from_slice(&Sha1::digest(vec![0; count]));
    bytes.extend_from_slice(b"ee");
    bytes
}
fn summary(mut values: Vec<f64>) -> serde_json::Value {
    values.sort_by(f64::total_cmp);
    json!({"samples":values,"medianMs":values[values.len()/2],"p95Ms":values[(values.len()*95).div_ceil(100)-1]})
}

/// Isolated paused libraries, real engine snapshots and serialization, no swarm traffic.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "controlled performance fixture; set SEEDHOP_PERF_OUTPUT and run explicitly"]
async fn library_snapshot_baseline() {
    let mut cases = Vec::new();
    for (transfers, files_per_transfer) in [(100, 1), (1000, 1), (1, 10000)] {
        let root = tempfile::tempdir().unwrap();
        let state = root.path().join("state");
        let files = root.path().join("files");
        std::fs::create_dir(&files).unwrap();
        let engine = Engine::open(EngineConfig {
            state_directory: state.display().to_string(),
            download_directory: files.display().to_string(),
            allow_network_discovery: false,
            listen_port: 0,
        })
        .await
        .unwrap();
        let setup = Instant::now();
        for i in 0..transfers {
            let metadata = if files_per_transfer == 1 {
                adapter::create_fixture(&format!("Sample-{i:04}.bin"), &[0])
            } else {
                many_files(files_per_transfer)
            };
            engine
                .add(
                    metadata,
                    files.display().to_string(),
                    (0..files_per_transfer as u32).collect(),
                    true,
                    true,
                )
                .await
                .unwrap();
            if i % 100 == 0 {
                eprintln!(
                    "Fixture {transfers} transfers / {files_per_transfer} files: added {}",
                    i + 1
                );
            }
        }
        tokio::time::timeout(Duration::from_secs(60), async {
            loop {
                let snapshot = engine.snapshot().unwrap();
                if snapshot.torrents.len() == transfers
                    && snapshot
                        .torrents
                        .iter()
                        .all(|t| t.state == "paused" && t.queue_position > 0)
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .unwrap();
        let setup_ms = setup.elapsed().as_secs_f64() * 1000.0;
        let mut snapshots = Vec::new();
        let mut encodes = Vec::new();
        let mut scans = Vec::new();
        let mut payload_bytes = 0;
        for sample in 0..17 {
            let start = Instant::now();
            let snapshot = engine.snapshot().unwrap();
            let elapsed = start.elapsed().as_secs_f64() * 1000.0;
            let start = Instant::now();
            let encoded = serde_json::to_vec(&snapshot).unwrap();
            let encode = start.elapsed().as_secs_f64() * 1000.0;
            payload_bytes = black_box(encoded.len());
            let mut order = snapshot.torrents.iter().collect::<Vec<_>>();
            order.sort_by_key(|t| t.queue_position);
            let start = Instant::now();
            let checksum: usize = snapshot
                .torrents
                .iter()
                .map(|torrent| {
                    order
                        .iter()
                        .position(|other| other.id == torrent.id)
                        .unwrap()
                })
                .sum();
            black_box(checksum);
            let scan = start.elapsed().as_secs_f64() * 1000.0;
            if sample >= 2 {
                snapshots.push(elapsed);
                encodes.push(encode);
                scans.push(scan);
            }
        }
        cases.push(json!({"transfers":transfers,"filesPerTransfer":files_per_transfer,"setupMs":setup_ms,"payloadBytes":payload_bytes,"snapshot":summary(snapshots),"encode":summary(encodes),"linearPositionScanDiagnostic":summary(scans)}));
        eprintln!("Measured {transfers} transfers / {files_per_transfer} files");
        engine.shutdown().await.unwrap();
    }
    let output = std::env::var("SEEDHOP_PERF_OUTPUT").expect("set SEEDHOP_PERF_OUTPUT");
    std::fs::write(
        output,
        serde_json::to_vec_pretty(
            &json!({"fixture":true,"networkDiscovery":false,"profile":"debug","cases":cases}),
        )
        .unwrap(),
    )
    .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn snapshot_membership_keeps_file_order_choices_and_restart() {
    let root = tempfile::tempdir().unwrap();
    let files = root.path().join("files");
    std::fs::create_dir(&files).unwrap();
    let config = EngineConfig {
        state_directory: root.path().join("state").display().to_string(),
        download_directory: files.display().to_string(),
        allow_network_discovery: false,
        listen_port: 0,
    };
    let engine = Engine::open(config.clone()).await.unwrap();
    let id = engine
        .add(
            many_files(8),
            config.download_directory.clone(),
            vec![7, 0, 3, 3],
            true,
            true,
        )
        .await
        .unwrap();
    async fn paused(engine: &Engine) -> seedhop_core::TorrentRecord {
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let item = engine.snapshot().unwrap().torrents.remove(0);
                if item.state == "paused" {
                    return item;
                }
                assert_ne!(item.state, "error");
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap()
    }
    let before = paused(&engine).await;
    assert_eq!(
        before.files.iter().map(|f| f.index).collect::<Vec<_>>(),
        (0..8).collect::<Vec<_>>()
    );
    assert_eq!(
        before
            .files
            .iter()
            .filter(|f| f.selected)
            .map(|f| f.index)
            .collect::<Vec<_>>(),
        vec![0, 3, 7]
    );
    engine
        .select_files(id.clone(), vec![5, 1, 1])
        .await
        .unwrap();
    let changed = paused(&engine).await;
    assert_eq!(
        changed
            .files
            .iter()
            .filter(|f| f.selected)
            .map(|f| f.index)
            .collect::<Vec<_>>(),
        vec![1, 5]
    );
    assert_eq!(
        changed
            .files
            .iter()
            .map(|f| (&f.name, f.priority))
            .collect::<Vec<_>>(),
        before
            .files
            .iter()
            .map(|f| (&f.name, f.priority))
            .collect::<Vec<_>>()
    );
    engine.shutdown().await.unwrap();
    drop(engine);
    let engine = Engine::open(config).await.unwrap();
    let restored = paused(&engine).await;
    assert_eq!(restored.id, id);
    assert!(restored.paused_intent);
    assert_eq!(
        restored
            .files
            .iter()
            .filter(|f| f.selected)
            .map(|f| f.index)
            .collect::<Vec<_>>(),
        vec![1, 5]
    );
    engine.shutdown().await.unwrap();
}

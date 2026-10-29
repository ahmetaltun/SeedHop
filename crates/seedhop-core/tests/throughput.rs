mod support;

use seedhop_core::Engine;
use std::time::Instant;
use support::{Swarm, config, wait_for};

#[tokio::test]
async fn changing_limits_wakes_pending_uploads_and_downloads() {
    use librqbit::limits::{Limits, LimitsConfig};
    use std::{num::NonZeroU32, sync::Arc, time::Duration};
    for upload in [false, true] {
        for replacement in [0, 1_000_000] {
            let limits = Arc::new(Limits::new(LimitsConfig {
                upload_bps: NonZeroU32::new(1),
                download_bps: NonZeroU32::new(1),
            }));
            let pending = tokio::spawn({
                let limits = limits.clone();
                async move {
                    let block = NonZeroU32::new(16384).unwrap();
                    if upload {
                        limits.prepare_for_upload(block).await
                    } else {
                        limits.prepare_for_download(block).await
                    }
                }
            });
            tokio::time::sleep(Duration::from_millis(20)).await;
            assert!(
                !pending.is_finished(),
                "fixture did not exhaust original limit"
            );
            if upload {
                limits.set_upload_bps(NonZeroU32::new(replacement));
            } else {
                limits.set_download_bps(NonZeroU32::new(replacement));
            }
            tokio::time::timeout(Duration::from_millis(500), pending)
                .await
                .expect("pending transfer kept waiting on obsolete speed limit")
                .unwrap()
                .unwrap();
        }
    }
}

/// Real disk writes, hash checks and both upload/download paths. Run alone for timing.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "controlled throughput benchmark; run explicitly with --nocapture"]
async fn loopback_throughput() {
    let swarm = Swarm::new(16 * 1024 * 1024).await;
    let state = tempfile::tempdir().unwrap();
    let files = tempfile::tempdir().unwrap();
    let down = Engine::open(config(state.path(), files.path()))
        .await
        .unwrap();
    let started = Instant::now();
    down.add(
        swarm.metadata.clone(),
        files.path().display().to_string(),
        vec![0],
        false,
        false,
    )
    .await
    .unwrap();
    let item = wait_for(&down, |item| item.state == "seeding").await;
    let elapsed = started.elapsed().as_secs_f64();
    assert_eq!(
        std::fs::read(files.path().join("payload.bin")).unwrap(),
        swarm.content
    );
    eprintln!(
        "THROUGHPUT {} bytes / {elapsed:.3} s = {:.3} MB/s; uploaded {}",
        item.total_bytes,
        item.total_bytes as f64 / elapsed / 1e6,
        swarm.seed.snapshot().unwrap().torrents[0]
            .network_uploaded_bytes
            .unwrap()
    );
    down.shutdown().await.unwrap();
    swarm.seed.shutdown().await.unwrap();
}

/// Isolates encryption CPU cost from discovery, disk and network variability.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "controlled throughput benchmark; compare debug and release profiles"]
async fn encrypted_transport_throughput() {
    use librqbit::mse::{self, EncryptionPolicy};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let (client, server) = tokio::io::duplex(256 * 1024);
    let (cr, cw) = tokio::io::split(client);
    let (mut sr, sw) = tokio::io::split(server);
    let payload = vec![73u8; 16 * 1024 * 1024];
    let client = async {
        let mut stream = mse::outgoing(cr, cw, [4; 20], &[], EncryptionPolicy::Require)
            .await
            .unwrap();
        let started = Instant::now();
        stream.write.write_all(&payload).await.unwrap();
        stream.write.flush().await.unwrap();
        let mut ack = [0];
        stream.read.read_exact(&mut ack).await.unwrap();
        let elapsed = started.elapsed().as_secs_f64();
        eprintln!(
            "ENCRYPTED_THROUGHPUT {} bytes / {elapsed:.3} s = {:.3} MB/s",
            payload.len(),
            payload.len() as f64 / elapsed / 1e6
        );
    };
    let server = async {
        let mut prefix = [0; 20];
        sr.read_exact(&mut prefix).await.unwrap();
        let mut stream = mse::incoming_after_prefix(
            sr,
            sw,
            prefix,
            |_| Some([4; 20]),
            EncryptionPolicy::Require,
        )
        .await
        .unwrap();
        let mut received = vec![0; payload.len()];
        stream.read.read_exact(&mut received).await.unwrap();
        assert_eq!(received, payload);
        stream.write.write_all(&[1]).await.unwrap();
        stream.write.flush().await.unwrap();
    };
    tokio::time::timeout(std::time::Duration::from_secs(45), async {
        tokio::join!(client, server);
    })
    .await
    .unwrap();
}

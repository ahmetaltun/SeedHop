//! Loopback-only transport tests exercise the production monitor futures.
use std::sync::{
    Arc, Weak,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

use futures::StreamExt;
use librqbit_core::hash_id::Id20;
use parking_lot::Mutex;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::{Semaphore, mpsc},
    time::Instant,
};

use crate::*;

struct Stats {
    control: Mutex<Option<Weak<TrackerControl>>>,
    complete: AtomicBool,
    gate: Arc<Semaphore>,
}
impl TorrentStatsProvider for Arc<Stats> {
    fn get(&self) -> TrackerCommsStats {
        TrackerCommsStats {
            total_bytes: 100,
            downloaded_bytes: if self.complete.load(Ordering::Relaxed) {
                100
            } else {
                0
            },
            torrent_state: TrackerCommsStatsState::Live,
            ..Default::default()
        }
    }
    fn register_control(&self, control: Weak<TrackerControl>) {
        *self.control.lock() = Some(control);
    }
    fn manual_gate(&self) -> Arc<Semaphore> {
        self.gate.clone()
    }
}

async fn settle(control: &TrackerControl) {
    for _ in 0..1000 {
        if !control.snapshot().in_flight {
            return;
        }
        tokio::task::yield_now().await;
    }
    panic!("monitor did not settle");
}

#[tokio::test]
async fn http_manual_refresh_uses_existing_stream_and_preserves_event_and_deadlines() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!(
        "http://{}/private-token/announce",
        listener.local_addr().unwrap()
    );
    let (requests_tx, mut requests_rx) = mpsc::unbounded_channel();
    let server = tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            loop {
                let byte = socket.read_u8().await.unwrap();
                request.push(byte);
                if request.ends_with(b"\r\n\r\n") {
                    break;
                }
                assert!(request.len() < 8192);
            }
            requests_tx
                .send((Instant::now(), String::from_utf8(request).unwrap()))
                .unwrap();
            let mut body = b"d8:intervali1800e12:min intervali120e5:peers6:".to_vec();
            body.extend_from_slice(&[127, 0, 0, 1, 0x1a, 0xe1]);
            body.push(b'e');
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
            socket.write_all(&body).await.unwrap();
        }
    });
    let stats = Arc::new(Stats {
        control: Mutex::new(None),
        complete: AtomicBool::new(false),
        gate: Arc::new(Semaphore::new(4)),
    });
    let udp = UdpTrackerClient::new(tokio_util::sync::CancellationToken::new(), None)
        .await
        .unwrap();
    let mut stream = TrackerComms::start(
        Id20::new([1; 20]),
        Id20::new([2; 20]),
        [url.parse().unwrap()].into_iter().collect(),
        Box::new(stats.clone()),
        None,
        6881,
        reqwest::Client::new(),
        udp,
    )
    .unwrap();
    let (peer_tx, mut peer_rx) = mpsc::unbounded_channel();
    let driver = tokio::spawn(async move {
        while let Some(peer) = stream.next().await {
            peer_tx.send(peer).unwrap();
        }
    });
    let (_, initial) = tokio::time::timeout(Duration::from_secs(5), requests_rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(initial.contains("event=started"));
    assert_eq!(
        peer_rx.recv().await.unwrap(),
        "127.0.0.1:6881".parse::<std::net::SocketAddr>().unwrap()
    );
    let control = stats.control.lock().as_ref().unwrap().upgrade().unwrap();
    settle(&control).await;
    tokio::time::pause();
    let requested = Instant::now();
    assert_eq!(control.request(), ReannounceDisposition::Scheduled);
    for _ in 0..20 {
        assert_eq!(control.request(), ReannounceDisposition::AlreadyPending);
    }
    tokio::time::advance(Duration::from_secs(119)).await;
    for _ in 0..50 {
        tokio::task::yield_now().await;
    }
    assert!(
        requests_rx.try_recv().is_err(),
        "manual announce bypassed minimum interval"
    );
    tokio::time::advance(Duration::from_secs(1)).await;
    tokio::time::resume();
    let (sent, manual) = requests_rx.recv().await.unwrap();
    assert!(sent.duration_since(requested) >= Duration::from_secs(120));
    assert!(!manual.contains("event="));
    assert_eq!(control.request(), ReannounceDisposition::AlreadyPending);
    peer_rx.recv().await.unwrap();
    settle(&control).await;
    assert!(!control.snapshot().pending);
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(1799)).await;
    for _ in 0..50 {
        tokio::task::yield_now().await;
    }
    assert!(
        requests_rx.try_recv().is_err(),
        "minimum interval replaced regular interval"
    );
    tokio::time::advance(Duration::from_secs(1)).await;
    tokio::time::resume();
    let (_, regular) = requests_rx.recv().await.unwrap();
    assert!(!regular.contains("event="));
    driver.abort();
    let _ = driver.await;
    assert_eq!(control.request(), ReannounceDisposition::Unavailable);
    server.abort();
}

#[tokio::test]
async fn udp_manual_refresh_waits_for_advertised_interval_and_does_not_replay_started() {
    use tokio::net::UdpSocket;
    let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let url = format!("udp://{}/announce", socket.local_addr().unwrap());
    let (events_tx, mut events_rx) = mpsc::unbounded_channel();
    let server = tokio::spawn(async move {
        let mut buffer = [0; 1024];
        loop {
            let (length, source) = socket.recv_from(&mut buffer).await.unwrap();
            assert!(length >= 16);
            let action = u32::from_be_bytes(buffer[8..12].try_into().unwrap());
            let mut response = Vec::new();
            response.extend_from_slice(&action.to_be_bytes());
            response.extend_from_slice(&buffer[12..16]);
            if action == 0 {
                response.extend_from_slice(&123_u64.to_be_bytes());
            } else {
                assert_eq!(action, 1);
                assert!(length >= 98);
                events_tx
                    .send(u32::from_be_bytes(buffer[80..84].try_into().unwrap()))
                    .unwrap();
                response.extend_from_slice(&1800_u32.to_be_bytes());
                response.extend_from_slice(&0_u32.to_be_bytes());
                response.extend_from_slice(&0_u32.to_be_bytes());
                response.extend_from_slice(&[127, 0, 0, 1, 0x1a, 0xe1]);
            }
            socket.send_to(&response, source).await.unwrap();
        }
    });
    let stats = Arc::new(Stats {
        control: Mutex::new(None),
        complete: AtomicBool::new(false),
        gate: Arc::new(Semaphore::new(4)),
    });
    let udp = UdpTrackerClient::new(tokio_util::sync::CancellationToken::new(), None)
        .await
        .unwrap();
    let mut stream = TrackerComms::start(
        Id20::new([1; 20]),
        Id20::new([2; 20]),
        [url.parse().unwrap()].into_iter().collect(),
        Box::new(stats.clone()),
        None,
        6881,
        reqwest::Client::new(),
        udp,
    )
    .unwrap();
    let (peer_tx, mut peer_rx) = mpsc::unbounded_channel();
    let driver = tokio::spawn(async move {
        while let Some(peer) = stream.next().await {
            peer_tx.send(peer).unwrap();
        }
    });
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(5), events_rx.recv())
            .await
            .unwrap(),
        Some(2)
    );
    peer_rx.recv().await.unwrap();
    let control = stats.control.lock().as_ref().unwrap().upgrade().unwrap();
    settle(&control).await;
    tokio::time::pause();
    assert_eq!(control.request(), ReannounceDisposition::Scheduled);
    tokio::time::advance(Duration::from_secs(1799)).await;
    for _ in 0..50 {
        tokio::task::yield_now().await;
    }
    assert!(
        events_rx.try_recv().is_err(),
        "UDP request bypassed advertised interval"
    );
    tokio::time::advance(Duration::from_secs(1)).await;
    tokio::time::resume();
    assert_eq!(events_rx.recv().await, Some(0));
    peer_rx.recv().await.unwrap();
    settle(&control).await;
    tokio::time::pause();
    stats.complete.store(true, Ordering::Relaxed);
    assert_eq!(control.request(), ReannounceDisposition::Scheduled);
    tokio::time::advance(Duration::from_secs(1800)).await;
    tokio::time::resume();
    assert_eq!(events_rx.recv().await, Some(1)); // Genuine completion wins over manual intent.
    peer_rx.recv().await.unwrap();
    settle(&control).await;
    tokio::time::pause();
    assert_eq!(control.request(), ReannounceDisposition::Scheduled);
    tokio::time::advance(Duration::from_secs(1800)).await;
    tokio::time::resume();
    assert_eq!(events_rx.recv().await, Some(0));
    driver.abort();
    let _ = driver.await;
    assert_eq!(control.request(), ReannounceDisposition::Unavailable);
    server.abort();
}

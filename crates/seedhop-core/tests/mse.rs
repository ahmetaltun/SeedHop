use librqbit::mse::{self, EncryptionPolicy as Policy, Error};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::test]
async fn encrypted_streams_preserve_initial_payload_and_partial_writes() {
    for initial_length in [0, 20, 68, 65535] {
        let (client, server) = tokio::io::duplex(2048);
        let (cr, cw) = tokio::io::split(client);
        let (mut sr, sw) = tokio::io::split(server);
        let initial = vec![3; initial_length];
        let payload = vec![7; 100_001];
        let response = vec![9; 90_003];
        let client = async {
            let mut stream = mse::outgoing(cr, cw, [4; 20], &initial, Policy::Require)
                .await
                .unwrap();
            assert!(stream.encrypted);
            stream.write.write_all(&payload).await.unwrap();
            stream.write.flush().await.unwrap();
            let mut received = vec![0; response.len()];
            stream.read.read_exact(&mut received).await.unwrap();
            assert_eq!(received, response);
        };
        let server = async {
            let mut first = [0; 20];
            sr.read_exact(&mut first).await.unwrap();
            let mut stream = mse::incoming_after_prefix(
                sr,
                sw,
                first,
                |hash| {
                    assert_eq!(hash, mse::stream_key_hash(&[4; 20]));
                    Some([4; 20])
                },
                Policy::Require,
            )
            .await
            .unwrap();
            assert!(stream.encrypted);
            let mut received = vec![0; initial.len() + payload.len()];
            stream.read.read_exact(&mut received).await.unwrap();
            assert_eq!(received, [initial.as_slice(), payload.as_slice()].concat());
            stream.write.write_all(&response).await.unwrap();
            stream.write.shutdown().await.unwrap();
        };
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            tokio::join!(client, server);
        })
        .await
        .unwrap();
    }
}

#[tokio::test]
async fn encryption_policies_and_invalid_public_keys_fail_closed() {
    let mut plain = mse::incoming_after_prefix(
        b"rest".as_slice(),
        tokio::io::sink(),
        *mse::PLAIN_PREFIX,
        |_| panic!("plaintext should not perform a key lookup"),
        Policy::Allow,
    )
    .await
    .unwrap();
    assert!(!plain.encrypted);
    let mut bytes = [0; 24];
    plain.read.read_exact(&mut bytes).await.unwrap();
    assert_eq!(
        &bytes,
        [mse::PLAIN_PREFIX.as_slice(), b"rest"].concat().as_slice()
    );
    let error = mse::incoming_after_prefix(
        tokio::io::empty(),
        tokio::io::sink(),
        *mse::PLAIN_PREFIX,
        |_| None,
        Policy::Require,
    )
    .await
    .err()
    .unwrap();
    assert!(matches!(error, Error::Policy));
    let error = mse::incoming_after_prefix(
        tokio::io::empty(),
        tokio::io::sink(),
        [0; 20],
        |_| None,
        Policy::Disabled,
    )
    .await
    .err()
    .unwrap();
    assert!(matches!(error, Error::Policy));
    for last in [0, 1] {
        let mut key = [0; 96];
        key[95] = last;
        let first = key[..20].try_into().unwrap();
        let error = mse::incoming_after_prefix(
            &key[20..],
            tokio::io::sink(),
            first,
            |_| panic!("invalid key reached lookup"),
            Policy::Allow,
        )
        .await
        .err()
        .unwrap();
        assert!(matches!(error, Error::PublicKey));
    }
    for key in forge::invalid_high_keys() {
        assert!(matches!(
            mse::incoming_after_prefix(
                &key[20..],
                tokio::io::sink(),
                key[..20].try_into().unwrap(),
                |_| panic!("invalid key reached lookup"),
                Policy::Allow
            )
            .await
            .err()
            .unwrap(),
            Error::PublicKey
        ));
    }
    assert!(matches!(
        mse::outgoing(
            tokio::io::empty(),
            tokio::io::sink(),
            [0; 20],
            &vec![0; 65536],
            Policy::Require
        )
        .await
        .err()
        .unwrap(),
        Error::Policy
    ));
}

mod forge {
    use crypto_bigint::{
        Encoding, U768,
        modular::runtime_mod::{DynResidue, DynResidueParams},
    };
    use rc4::{KeyInit, Rc4, StreamCipher, consts::U20};
    use sha1::{Digest, Sha1};
    use tokio::io::{AsyncRead, AsyncReadExt};
    pub type Cipher = Rc4<U20>;
    const P: U768 = U768::from_be_hex(
        "FFFFFFFFFFFFFFFFC90FDAA22168C234C4C6628B80DC1CD129024E088A67CC74020BBEA63B139B22514A08798E3404DDEF9519B3CD3A431B302B0A6DF25F14374FE1356D6D51C245E485B576625E7EC6F44C42E9A63A36210000000000090563",
    );
    pub fn invalid_high_keys() -> [[u8; 96]; 2] {
        [P.wrapping_sub(&U768::ONE).to_be_bytes(), P.to_be_bytes()]
    }
    pub fn public() -> [u8; 96] {
        let mut result = [0; 96];
        result[95] = 8;
        result
    }
    pub fn shared(remote: &[u8; 96]) -> [u8; 96] {
        DynResidue::new(&U768::from_be_slice(remote), DynResidueParams::new(&P))
            .pow(&U768::from(3u8))
            .retrieve()
            .to_be_bytes()
    }
    pub fn hash(parts: &[&[u8]]) -> [u8; 20] {
        let mut h = Sha1::new();
        for part in parts {
            h.update(part);
        }
        h.finalize().into()
    }
    pub fn cipher(label: &[u8], secret: &[u8], key: &[u8]) -> Cipher {
        let key = hash(&[label, secret, key]);
        let mut c = Cipher::new_from_slice(&key).unwrap();
        c.apply_keystream(&mut [0; 1024]);
        c
    }
    pub async fn scan<R: AsyncRead + Unpin>(read: &mut R, marker: &[u8]) {
        let mut window = vec![0; marker.len()];
        read.read_exact(&mut window).await.unwrap();
        for _ in 0..=512 {
            if window == marker {
                return;
            }
            window.rotate_left(1);
            read.read_exact(&mut window[marker.len() - 1..])
                .await
                .unwrap();
        }
        panic!("missing sync");
    }
}

#[tokio::test]
async fn malformed_mse_fields_are_rejected_without_allocating_claimed_padding() {
    use rc4::StreamCipher;
    for case in ["sync", "unknown", "verification", "cipher", "padding"] {
        let (client, server) = tokio::io::duplex(4096);
        let (mut cr, mut cw) = tokio::io::split(client);
        let (mut sr, sw) = tokio::io::split(server);
        let attacker = async {
            cw.write_all(&forge::public()).await.unwrap();
            let mut remote = [0; 96];
            cr.read_exact(&mut remote).await.unwrap();
            let secret = forge::shared(&remote);
            if case == "sync" {
                cw.write_all(&[1; 532]).await.unwrap();
                return;
            }
            let key = if case == "unknown" { [8; 20] } else { [4; 20] };
            let mut data = forge::hash(&[b"req1", &secret]).to_vec();
            let a = mse::stream_key_hash(&key);
            let b = forge::hash(&[b"req3", &secret]);
            data.extend(a.iter().zip(b).map(|(a, b)| a ^ b));
            let mut header = vec![0; 8];
            if case == "verification" {
                header[0] = 1;
            }
            header.extend((if case == "cipher" { 0u32 } else { 2u32 }).to_be_bytes());
            header.extend((if case == "padding" { 513u16 } else { 0u16 }).to_be_bytes());
            forge::cipher(b"keyA", &secret, &key).apply_keystream(&mut header);
            data.extend(header);
            cw.write_all(&data).await.unwrap();
        };
        let server = async {
            let mut first = [0; 20];
            sr.read_exact(&mut first).await.unwrap();
            mse::incoming_after_prefix(
                sr,
                sw,
                first,
                |hash| {
                    if hash == mse::stream_key_hash(&[4; 20]) {
                        Some([4; 20])
                    } else {
                        None
                    }
                },
                Policy::Require,
            )
            .await
            .err()
            .unwrap()
        };
        let (_, error) = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            tokio::join!(attacker, server)
        })
        .await
        .unwrap();
        assert!(matches!(
            (case, error),
            ("sync", Error::Sync)
                | ("unknown", Error::UnknownKey)
                | ("verification", Error::Verification)
                | ("cipher", Error::Policy)
                | ("padding", Error::Padding)
        ));
    }
}

#[tokio::test]
async fn header_only_mode_is_allowed_only_when_policy_permits_it() {
    use rc4::StreamCipher;
    for policy in [Policy::Require, Policy::Allow] {
        let (client, server) = tokio::io::duplex(4096);
        let (cr, cw) = tokio::io::split(client);
        let (mut sr, mut sw) = tokio::io::split(server);
        let client = async {
            let result = mse::outgoing(cr, cw, [4; 20], b"initial", policy).await;
            if policy == Policy::Require {
                assert!(matches!(result.err().unwrap(), Error::Policy));
            } else {
                let mut stream = result.unwrap();
                assert!(!stream.encrypted);
                let mut bytes = [0; 5];
                stream.read.read_exact(&mut bytes).await.unwrap();
                assert_eq!(&bytes, b"clear");
                stream.write.write_all(b"reply").await.unwrap();
                stream.write.flush().await.unwrap();
            }
        };
        let server = async {
            let mut remote = [0; 96];
            sr.read_exact(&mut remote).await.unwrap();
            let mut hello = forge::public().to_vec();
            hello.extend([6; 512]);
            sw.write_all(&hello).await.unwrap();
            let secret = forge::shared(&remote);
            forge::scan(&mut sr, &forge::hash(&[b"req1", &secret])).await;
            let mut ignored = [0; 20];
            sr.read_exact(&mut ignored).await.unwrap();
            let mut rx = forge::cipher(b"keyA", &secret, &[4; 20]);
            let mut header = [0; 14];
            sr.read_exact(&mut header).await.unwrap();
            rx.apply_keystream(&mut header);
            assert_eq!(
                u32::from_be_bytes(header[8..12].try_into().unwrap()),
                if policy == Policy::Require { 2 } else { 3 }
            );
            let pad = u16::from_be_bytes(header[12..14].try_into().unwrap()) as usize;
            let mut bytes = vec![0; pad + 2];
            sr.read_exact(&mut bytes).await.unwrap();
            rx.apply_keystream(&mut bytes);
            let length = u16::from_be_bytes(bytes[pad..].try_into().unwrap()) as usize;
            let mut bytes = vec![0; length];
            sr.read_exact(&mut bytes).await.unwrap();
            rx.apply_keystream(&mut bytes);
            assert_eq!(bytes, b"initial");
            let mut response = vec![0; 14];
            response[11] = 1;
            response[12..14].copy_from_slice(&512u16.to_be_bytes());
            response.extend([5; 512]);
            forge::cipher(b"keyB", &secret, &[4; 20]).apply_keystream(&mut response);
            sw.write_all(&response).await.unwrap();
            if policy == Policy::Allow {
                sw.write_all(b"clear").await.unwrap();
                let mut bytes = [0; 5];
                sr.read_exact(&mut bytes).await.unwrap();
                assert_eq!(&bytes, b"reply");
            }
        };
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            tokio::join!(client, server)
        })
        .await
        .unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn engine_binds_mse_to_the_torrent_and_handles_a_fragmented_initial_handshake() {
    use seedhop_core::{Engine, EngineConfig, adapter};
    let state = tempfile::tempdir().unwrap();
    let files = tempfile::tempdir().unwrap();
    let engine = Engine::open_with_peer_encryption(
        EngineConfig {
            state_directory: state.path().display().to_string(),
            download_directory: files.path().display().to_string(),
            allow_network_discovery: false,
            listen_port: 0,
        },
        Policy::Require,
    )
    .await
    .unwrap();
    let mut keys = Vec::new();
    for name in ["a", "b"] {
        let metadata = adapter::create_fixture(name, b"hello");
        let preview = adapter::preview(metadata.clone()).unwrap();
        let mut key = [0; 20];
        for (index, slot) in key.iter_mut().enumerate() {
            *slot = u8::from_str_radix(&preview.id[index * 2..index * 2 + 2], 16).unwrap();
        }
        keys.push(key);
        engine
            .add(
                metadata,
                files.path().display().to_string(),
                vec![0],
                false,
                false,
            )
            .await
            .unwrap();
    }
    let port = engine.snapshot().unwrap().listen_port;
    let handshake = |key: [u8; 20]| {
        let mut data = vec![19];
        data.extend_from_slice(b"BitTorrent protocol");
        data.extend([0; 8]);
        data.extend(key);
        data.extend_from_slice(b"-SHtest-123456789012");
        data
    };
    // A bare handshake must not pass a required-encryption listener.
    let mut bare = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .unwrap();
    bare.write_all(&handshake(keys[0])).await.unwrap();
    let closed = tokio::time::timeout(std::time::Duration::from_secs(5), bare.read(&mut [0; 1]))
        .await
        .unwrap();
    assert!(matches!(closed, Ok(0)) || closed.is_err());
    // Valid MSE for torrent A cannot smuggle a BitTorrent handshake for B.
    let peer = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .unwrap();
    let (read, write) = peer.into_split();
    let mut encrypted = mse::outgoing(read, write, keys[0], &handshake(keys[1]), Policy::Require)
        .await
        .unwrap();
    let closed = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        encrypted.read.read(&mut [0; 1]),
    )
    .await
    .unwrap();
    assert!(matches!(closed, Ok(0)) || closed.is_err());
    assert!(
        engine
            .snapshot()
            .unwrap()
            .torrents
            .iter()
            .all(|t| t.peers == 0 && t.downloaded_bytes == 0)
    );
    // Only the prefix of the BT handshake is IA; the rest follows negotiation.
    let peer = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .unwrap();
    let (read, write) = peer.into_split();
    let data = handshake(keys[0]);
    let mut encrypted = mse::outgoing(read, write, keys[0], &data[..17], Policy::Require)
        .await
        .unwrap();
    encrypted.write.write_all(&data[17..]).await.unwrap();
    encrypted.write.flush().await.unwrap();
    let mut response = [0; 68];
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        encrypted.read.read_exact(&mut response),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(&response[28..48], &keys[0]);
    drop(encrypted);
    engine.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn magnet_metadata_keeps_encrypted_utp_fallback_after_silent_tcp_handshakes() {
    use librqbit::{
        AddTorrent, AddTorrentOptions, AddTorrentResponse, ListenerMode, ListenerOptions, Session,
        SessionOptions,
    };
    use seedhop_core::adapter;
    use std::time::Duration;

    let seed_files = tempfile::tempdir().unwrap();
    let files = tempfile::tempdir().unwrap();
    std::fs::write(seed_files.path().join("payload.bin"), b"hello").unwrap();
    let metadata = adapter::create_fixture("payload.bin", b"hello");
    let id = adapter::preview(metadata.clone()).unwrap().id;
    let seed = Session::new_with_opts(
        seed_files.path().into(),
        SessionOptions {
            peer_encryption: Policy::Require,
            dht: None,
            disable_local_service_discovery: true,
            listen: Some(ListenerOptions {
                mode: ListenerMode::UtpOnly,
                listen_addr: "127.0.0.1:0".parse().unwrap(),
                ..Default::default()
            }),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let handle = seed
        .add_torrent(
            AddTorrent::from_bytes(metadata),
            Some(AddTorrentOptions {
                overwrite: true,
                ..Default::default()
            }),
        )
        .await
        .unwrap()
        .into_handle()
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), handle.wait_until_completed())
        .await
        .unwrap()
        .unwrap();
    let peer = seed.listen_addr().unwrap();
    let listener = tokio::net::TcpListener::bind(peer).await.unwrap();
    let silent_tcp = tokio::spawn(async move {
        let mut connections = tokio::task::JoinSet::new();
        loop {
            tokio::select! {
                accepted = listener.accept() => {
                    let (socket, _) = accepted.unwrap();
                    connections.spawn(async move {
                        tokio::time::sleep(Duration::from_secs(30)).await;
                        drop(socket);
                    });
                }
                _ = connections.join_next(), if !connections.is_empty() => {}
            }
        }
    });
    let session = Session::new_with_opts(
        files.path().into(),
        SessionOptions {
            peer_encryption: Policy::Prefer,
            dht: None,
            disable_local_service_discovery: true,
            listen: Some(ListenerOptions {
                mode: ListenerMode::TcpAndUtp,
                listen_addr: "127.0.0.1:0".parse().unwrap(),
                ..Default::default()
            }),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let result = tokio::time::timeout(
        Duration::from_secs(14),
        session.add_torrent(
            AddTorrent::from_url(format!("magnet:?xt=urn:btih:{id}")),
            Some(AddTorrentOptions {
                list_only: true,
                initial_peers: Some(vec![peer]),
                ..Default::default()
            }),
        ),
    )
    .await;
    session.stop_and_wait().await.unwrap();
    seed.stop_and_wait().await.unwrap();
    silent_tcp.abort();
    let _ = silent_tcp.await;
    assert!(matches!(
        result
            .expect("silent TCP prevented encrypted uTP metadata fallback")
            .unwrap(),
        AddTorrentResponse::ListOnly(_)
    ));
    assert_eq!(std::fs::read_dir(files.path()).unwrap().count(), 0);
}

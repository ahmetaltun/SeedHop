use seedhop_core::{
    CoreError, adapter,
    v2::{Metainfo, V2Error},
};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

// Exercise the production peer ledger without enabling rqbit's unrelated
// standalone workspace and its CLI/UI development dependencies.
#[path = "../../../vendor/rqbit/crates/librqbit/src/torrent_state/live/peer/cancelled_requests.rs"]
mod cancelled_requests;

type Hash = [u8; 32];
#[derive(Clone)]
enum B {
    Bytes(Vec<u8>),
    Number(u64),
    Dict(BTreeMap<Vec<u8>, B>),
    List(Vec<B>),
}
fn bytes(v: impl AsRef<[u8]>) -> B {
    B::Bytes(v.as_ref().to_vec())
}
fn dict(entries: &[(&[u8], B)]) -> B {
    B::Dict(
        entries
            .iter()
            .map(|(k, v)| (k.to_vec(), v.clone()))
            .collect(),
    )
}
fn encode(value: &B) -> Vec<u8> {
    match value {
        B::Bytes(v) => [v.len().to_string().as_bytes(), b":", v].concat(),
        B::Number(v) => format!("i{v}e").into_bytes(),
        B::Dict(v) => [
            b"d".to_vec(),
            v.iter()
                .flat_map(|(k, v)| [encode(&bytes(k)), encode(v)].concat())
                .collect(),
            b"e".to_vec(),
        ]
        .concat(),
        B::List(v) => [
            b"l".to_vec(),
            v.iter().flat_map(encode).collect(),
            b"e".to_vec(),
        ]
        .concat(),
    }
}
fn put(value: &mut B, key: &[u8], replacement: B) {
    match value {
        B::Dict(v) => {
            v.insert(key.into(), replacement);
        }
        _ => panic!(),
    }
}
fn get<'a>(value: &'a mut B, key: &[u8]) -> &'a mut B {
    match value {
        B::Dict(v) => v.get_mut(key).unwrap(),
        _ => panic!(),
    }
}
fn hash(data: &[u8]) -> Hash {
    Sha256::digest(data).into()
}
fn parent(a: Hash, b: Hash) -> Hash {
    hash(&[a.as_slice(), b.as_slice()].concat())
}
fn root(mut hashes: Vec<Hash>) -> Hash {
    while hashes.len() > 1 {
        hashes = hashes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|p| parent(p[0], p[1]))
            .collect();
    }
    hashes[0]
}
// Independent fixture builder constructs the full block tree, then extracts its piece layer.
fn fixture(files: &[(&str, Vec<u8>)], piece_length: usize) -> B {
    let mut tree = dict(&[]);
    let mut layers = BTreeMap::new();
    for (path, data) in files {
        let mut props = dict(&[(b"length", B::Number(data.len() as u64))]);
        if !data.is_empty() {
            let mut blocks: Vec<_> = data.chunks(16384).map(hash).collect();
            blocks.resize(blocks.len().next_power_of_two(), [0; 32]);
            let file_root = root(blocks.clone());
            put(&mut props, b"pieces root", bytes(file_root));
            if data.len() > piece_length {
                let layer: Vec<Hash> = blocks
                    .chunks_exact(piece_length / 16384)
                    .map(|part| root(part.to_vec()))
                    .collect();
                layers.insert(
                    file_root.to_vec(),
                    bytes(layer[..data.len().div_ceil(piece_length)].concat()),
                );
            }
        }
        let parts: Vec<_> = path.split('/').collect();
        let mut node = &mut tree;
        for part in &parts[..parts.len() - 1] {
            match node {
                B::Dict(v) => {
                    node = v
                        .entry(part.as_bytes().to_vec())
                        .or_insert_with(|| dict(&[]))
                }
                _ => panic!(),
            }
        }
        put(
            node,
            parts.last().unwrap().as_bytes(),
            dict(&[(b"", props)]),
        );
    }
    dict(&[
        (
            b"info",
            dict(&[
                (b"file tree", tree),
                (b"meta version", B::Number(2)),
                (b"name", bytes(b"payload")),
                (b"piece length", B::Number(piece_length as u64)),
            ]),
        ),
        (b"piece layers", B::Dict(layers)),
    ])
}
fn parse(value: &B) -> Metainfo {
    Metainfo::parse(&encode(value)).unwrap().unwrap()
}

#[test]
fn validates_block_and_piece_layers_at_different_heights() {
    for piece_length in [16384, 32768, 131072] {
        for length in [1, 16383, 16384, 16385, 32769, 131073, 400_001] {
            let data: Vec<_> = (0..length).map(|i| (i % 251) as u8).collect();
            let fixture = fixture(&[("file.bin", data.clone())], piece_length);
            let meta = parse(&fixture);
            assert_eq!(meta.total_bytes(), length as u64);
            for (index, piece) in data.chunks(piece_length).enumerate() {
                meta.verify_piece(index as u64, piece).unwrap();
                let mut damaged = piece.to_vec();
                damaged[0] ^= 1;
                assert_eq!(
                    meta.verify_piece(index as u64, &damaged),
                    Err(V2Error::HashMismatch)
                );
                assert!(
                    meta.verify_piece(index as u64, &piece[..piece.len() - 1])
                        .is_err()
                );
            }
            assert!(meta.verify_piece(meta.piece_count(), b"").is_err());
        }
    }
}

#[test]
fn maps_sorted_files_without_counting_alignment_gaps_or_empty_files() {
    let fixture = fixture(
        &[("z/file", vec![2; 17000]), ("a", vec![]), ("b", vec![1; 3])],
        16384,
    );
    let meta = parse(&fixture);
    assert_eq!(meta.total_bytes(), 17003);
    assert_eq!(meta.piece_count(), 3);
    assert_eq!(meta.files()[0].path(), ["a"]);
    assert_eq!(meta.files()[0].piece_count(), 0);
    assert_eq!(meta.files()[1].first_piece(), 0);
    assert_eq!(meta.files()[2].first_piece(), 1);
    meta.verify_piece(0, &[1; 3]).unwrap();
    meta.verify_piece(1, &[2; 16384]).unwrap();
    meta.verify_piece(2, &[2; 616]).unwrap();
}

#[test]
fn hashes_exact_info_bytes_and_ignores_outer_comments() {
    let mut fixture = fixture(&[("file", vec![7; 32])], 16384);
    let expected = hash(&encode(get(&mut fixture, b"info")));
    put(&mut fixture, b"comment", bytes(b"12:meta versioni2e"));
    assert_eq!(parse(&fixture).info_hash(), expected);
    let v1 = adapter::create_fixture("12:meta versioni2e", b"content");
    assert!(Metainfo::parse(&v1).unwrap().is_none());
    adapter::preview(v1).unwrap();
}

#[test]
fn rejects_corrupt_missing_extra_and_wrong_size_layers() {
    let original = fixture(&[("file", vec![3; 100000])], 32768);
    for mode in 0..4 {
        let mut fixture = original.clone();
        let B::Dict(layers) = get(&mut fixture, b"piece layers") else {
            panic!()
        };
        match mode {
            0 => layers.clear(),
            1 => {
                let B::Bytes(layer) = layers.values_mut().next().unwrap() else {
                    panic!()
                };
                layer[0] ^= 1;
            }
            2 => {
                let B::Bytes(layer) = layers.values_mut().next().unwrap() else {
                    panic!()
                };
                layer.pop();
            }
            _ => {
                layers.insert(vec![0; 32], bytes([1; 32]));
            }
        }
        assert!(Metainfo::parse(&encode(&fixture)).is_err());
    }
}

#[test]
fn validates_empty_file_roots_versions_and_piece_sizes() {
    let original = fixture(&[("file", vec![])], 16384);
    parse(&original);
    let mut invalid = original.clone();
    put(
        get(
            get(get(get(&mut invalid, b"info"), b"file tree"), b"file"),
            b"",
        ),
        b"pieces root",
        bytes([0; 32]),
    );
    assert!(Metainfo::parse(&encode(&invalid)).is_err());
    for size in [0, 8192, 17000, 1u64 << 32] {
        let mut invalid = original.clone();
        put(get(&mut invalid, b"info"), b"piece length", B::Number(size));
        assert!(Metainfo::parse(&encode(&invalid)).is_err());
    }
    let future = b"d4:infod12:meta versioni3eee";
    assert_eq!(Metainfo::parse(future).unwrap_err(), V2Error::Version(3));
}

#[test]
fn rejects_invalid_tree_shapes_and_excessively_nested_metadata() {
    let mut invalid = fixture(&[("file", vec![1])], 16384);
    put(
        get(get(&mut invalid, b"info"), b"file tree"),
        b"",
        dict(&[(b"length", B::Number(0))]),
    );
    assert!(Metainfo::parse(&encode(&invalid)).is_err());
    let mut nested = vec![b'l'; 10000];
    nested.extend(vec![b'e'; 10000]);
    assert!(Metainfo::parse(&nested).is_err());
    for malformed in [
        b"d4:infod12:meta versioni02eee".as_slice(),
        b"d4:infod12:meta versioni2e12:meta versioni2eee",
        b"d4:infod12:meta versioni2eeeextra",
        b"d4:infod12:meta versioni-0eee",
        b"d04:infodee",
    ] {
        assert!(Metainfo::parse(malformed).is_err());
    }
}

fn hybrid() -> B {
    let data = b"hello";
    let mut fixture = fixture(&[("file", data.to_vec())], 16384);
    let info = get(&mut fixture, b"info");
    put(info, b"name", bytes(b"file"));
    put(info, b"length", B::Number(data.len() as u64));
    put(info, b"pieces", bytes(sha1::Sha1::digest(data)));
    fixture
}

fn hybrid_files(files: &[(&str, Vec<u8>)], piece_length: usize, pad_last: bool) -> B {
    let mut metadata = fixture(files, piece_length);
    let mut entries = Vec::new();
    let mut content = Vec::new();
    for (index, (path, data)) in files.iter().enumerate() {
        entries.push(dict(&[
            (b"length", B::Number(data.len() as u64)),
            (b"path", B::List(path.split('/').map(bytes).collect())),
        ]));
        content.extend_from_slice(data);
        let padding = (piece_length - content.len() % piece_length) % piece_length;
        if padding != 0 && (index + 1 < files.len() || pad_last) {
            entries.push(dict(&[
                (b"length", B::Number(padding as u64)),
                (
                    b"path",
                    B::List(vec![bytes(".pad"), bytes(format!("{index}_{padding}"))]),
                ),
                (b"attr", bytes("p")),
            ]));
            content.resize(content.len() + padding, 0);
        }
    }
    let info = get(&mut metadata, b"info");
    put(info, b"files", B::List(entries));
    put(
        info,
        b"pieces",
        bytes(
            content
                .chunks(piece_length)
                .flat_map(|part| sha1::Sha1::digest(part).to_vec())
                .collect::<Vec<_>>(),
        ),
    );
    metadata
}

#[test]
fn app_accepts_pure_v2_and_hybrid_with_full_sha256_identity() {
    let pure = encode(&fixture(&[("file", vec![1])], 16384));
    assert_eq!(adapter::preview(pure).unwrap().id.len(), 64);
    let hybrid = hybrid();
    assert!(parse(&hybrid).is_hybrid());
    let preview = adapter::preview(encode(&hybrid)).unwrap();
    assert_eq!(preview.files[0].name, "file");
    assert_eq!(preview.total_bytes, 5);
    let mut invalid = hybrid;
    put(get(&mut invalid, b"info"), b"length", B::Number(6));
    assert!(matches!(
        adapter::preview(encode(&invalid)),
        Err(CoreError::InvalidInput(_))
    ));
}

#[test]
fn hybrid_multifile_requires_exact_order_and_padding() {
    let mut fixture = fixture(&[("a", vec![1; 3]), ("dir/b", vec![2; 2])], 16384);
    let entries = vec![
        dict(&[
            (b"length", B::Number(3)),
            (b"path", B::List(vec![bytes(b"a")])),
        ]),
        dict(&[
            (b"attr", bytes(b"p")),
            (b"length", B::Number(16381)),
            (b"path", B::List(vec![bytes(b".pad"), bytes(b"16381")])),
        ]),
        dict(&[
            (b"length", B::Number(2)),
            (b"path", B::List(vec![bytes(b"dir"), bytes(b"b")])),
        ]),
    ];
    let info = get(&mut fixture, b"info");
    put(info, b"files", B::List(entries.clone()));
    put(info, b"pieces", bytes([0; 40]));
    parse(&fixture);
    for mode in 0..4 {
        let mut invalid = fixture.clone();
        let mut entries = entries.clone();
        match mode {
            0 => {
                entries.remove(1);
            }
            1 => entries.swap(0, 2),
            2 => put(&mut entries[1], b"length", B::Number(16380)),
            _ => put(&mut entries[2], b"path", B::List(vec![bytes(b"other")])),
        }
        put(get(&mut invalid, b"info"), b"files", B::List(entries));
        assert!(Metainfo::parse(&encode(&invalid)).is_err());
    }
}

#[test]
fn duplicate_content_can_share_a_piece_layer() {
    let fixture = fixture(&[("a", vec![3; 50000]), ("b", vec![3; 50000])], 32768);
    let meta = parse(&fixture);
    assert_eq!(meta.piece_count(), 4);
    meta.verify_piece(2, &[3; 32768]).unwrap();
    meta.verify_piece(3, &[3; 17232]).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn malformed_v2_import_does_not_create_transfer_files_or_session_membership() {
    use seedhop_core::{Engine, EngineConfig};
    let state = tempfile::tempdir().unwrap();
    let downloads = tempfile::tempdir().unwrap();
    let engine = Engine::open(EngineConfig {
        state_directory: state.path().to_string_lossy().into(),
        download_directory: downloads.path().to_string_lossy().into(),
        allow_network_discovery: false,
        listen_port: 0,
    })
    .await
    .unwrap();
    let mut malformed = fixture(&[("file", vec![1])], 16384);
    let B::Dict(properties) = get(
        get(get(get(&mut malformed, b"info"), b"file tree"), b"file"),
        b"",
    ) else {
        panic!()
    };
    properties.remove(b"pieces root".as_slice());
    assert!(matches!(
        engine
            .add(
                encode(&malformed),
                downloads.path().to_string_lossy().into(),
                vec![0],
                false,
                false
            )
            .await,
        Err(CoreError::InvalidInput(_))
    ));
    assert!(engine.snapshot().unwrap().torrents.is_empty());
    assert_eq!(std::fs::read_dir(downloads.path()).unwrap().count(), 0);
    engine.shutdown().await.unwrap();
}

#[test]
fn rejects_missing_roots_symlinks_and_file_directory_collisions() {
    let original = fixture(&[("file", vec![1])], 16384);
    for mode in 0..5 {
        let mut invalid = original.clone();
        let node = get(get(get(&mut invalid, b"info"), b"file tree"), b"file");
        match mode {
            0 => {
                let B::Dict(properties) = get(node, b"") else {
                    panic!()
                };
                properties.remove(b"pieces root".as_slice());
            }
            1 => put(get(node, b""), b"pieces root", bytes([0; 31])),
            2 => put(get(node, b""), b"attr", bytes(b"l")),
            3 => put(
                get(node, b""),
                b"symlink path",
                B::List(vec![bytes(b"target")]),
            ),
            _ => put(
                node,
                b"child",
                dict(&[(b"", dict(&[(b"length", B::Number(0))]))]),
            ),
        }
        assert!(Metainfo::parse(&encode(&invalid)).is_err());
    }
    let mut invalid = hybrid();
    put(get(&mut invalid, b"info"), b"attr", bytes(b"l"));
    assert!(adapter::preview(encode(&invalid)).is_err());
}

#[test]
fn rejects_unsorted_v2_and_truncated_metadata_without_panicking() {
    let fixture = encode(&fixture(&[("file", vec![1; 50000])], 32768));
    for end in 0..fixture.len() {
        assert!(Metainfo::parse(&fixture[..end]).is_err());
    }
    // Valid syntax with unsorted info keys must not be reencoded to compute an identity.
    let unsorted = b"d4:infod12:piece lengthi16384e12:meta versioni2eee";
    assert_eq!(
        Metainfo::parse(unsorted).unwrap_err(),
        V2Error::Invalid("v2 dictionary keys are not sorted")
    );
    // Names in bytes or extensions are not version markers.
    let v1 = b"d4:infod6:lengthi1e4:name1:x12:piece lengthi16384e6:pieces20:12345678901234567890ee";
    assert!(Metainfo::parse(v1).unwrap().is_none());
}

#[test]
fn rejects_oversized_paths_before_expanding_shared_prefixes() {
    for path in [
        vec!["x".repeat(240); 18].join("/"),
        vec!["x".repeat(255); 18].join("/"),
    ] {
        assert!(Metainfo::parse(&encode(&fixture(&[(&path, vec![1])], 16384))).is_err());
    }
}

#[test]
fn authenticates_magnet_piece_layers_and_rejects_uncorrelated_or_corrupt_proofs() {
    use seedhop_core::v2::{HashRequest, PendingMetainfo};
    let data: Vec<u8> = (0..17_000_001).map(|i| (i % 251) as u8).collect();
    let mut fixture = fixture(&[("a", data.clone()), ("b", data.clone())], 32768);
    let meta = parse(&fixture);
    let info = encode(get(&mut fixture, b"info"));
    let mut pending = PendingMetainfo::from_info(&info).unwrap().unwrap();
    let mut count = 0;
    while let Some(request) = pending.next_request() {
        let hashes = meta
            .serve_hashes(&request, |_, offset, output| {
                output.copy_from_slice(&data[offset as usize..offset as usize + output.len()]);
                Ok(())
            })
            .unwrap();
        let mut corrupt = hashes.clone();
        corrupt[0][0] ^= 1;
        assert_eq!(
            pending.accept(request, &corrupt),
            Err(V2Error::HashMismatch)
        );
        let unrelated = HashRequest {
            index: request.index + request.length,
            ..request
        };
        assert!(pending.accept(unrelated, &hashes).is_err());
        assert!(
            pending
                .accept(request, &hashes[..hashes.len() - 1])
                .is_err()
        );
        pending.accept(request, &hashes).unwrap();
        count += 1;
    }
    assert_eq!(count, 2, "identical files share two requested proof ranges");
    let verified = pending.finish().unwrap();
    assert_eq!(verified.info_hash(), meta.info_hash());
    verified.verify_piece(0, &data[..32768]).unwrap();
    assert!(
        PendingMetainfo::from_info(&info)
            .unwrap()
            .unwrap()
            .finish()
            .is_err()
    );
}

#[test]
fn serves_block_and_piece_proofs_at_each_tree_height() {
    use seedhop_core::v2::HashRequest;
    let data: Vec<u8> = (0..400_001).map(|i| (i % 197) as u8).collect();
    let fixture = fixture(&[("file", data.clone())], 131072);
    let meta = parse(&fixture);
    let height = (data.len().div_ceil(16384).next_power_of_two()).ilog2();
    for base in [0, 3] {
        for log_count in 1..=height - base {
            let length = 1u32 << log_count;
            let width = 1u32 << (height - base);
            for index in (0..width).step_by(length as usize) {
                let request = HashRequest {
                    pieces_root: meta.files()[0].pieces_root().unwrap(),
                    base_layer: base,
                    index,
                    length,
                    proof_layers: height - base - 1,
                };
                let hashes = meta
                    .serve_hashes(&request, |_, offset, output| {
                        output.copy_from_slice(
                            &data[offset as usize..offset as usize + output.len()],
                        );
                        Ok(())
                    })
                    .unwrap();
                meta.verify_hashes(&request, &hashes).unwrap();
            }
        }
    }
}

fn engine_config(
    state: &std::path::Path,
    downloads: &std::path::Path,
) -> seedhop_core::EngineConfig {
    seedhop_core::EngineConfig {
        state_directory: state.to_string_lossy().into(),
        download_directory: downloads.to_string_lossy().into(),
        allow_network_discovery: false,
        listen_port: 0,
    }
}
async fn settled(engine: &seedhop_core::Engine) -> seedhop_core::TorrentRecord {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let item = engine
            .snapshot()
            .unwrap()
            .torrents
            .into_iter()
            .next()
            .unwrap();
        assert_ne!(item.state, "error", "{item:?}");
        if item.state != "checking" {
            return item;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "initial verification timed out"
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn full_v2_identity_selection_and_sha256_recheck_survive_restart() {
    use seedhop_core::Engine;
    let state = tempfile::tempdir().unwrap();
    let downloads = tempfile::tempdir().unwrap();
    let data = vec![37; 70_001];
    let bytes = encode(&fixture(&[("a", data.clone()), ("b", vec![4; 17])], 32768));
    std::fs::write(downloads.path().join("a"), &data).unwrap();
    std::fs::write(downloads.path().join("b"), [4; 17]).unwrap();
    let cfg = engine_config(state.path(), downloads.path());
    let engine = Engine::open(cfg.clone()).await.unwrap();
    let id = engine
        .add(
            bytes.clone(),
            cfg.download_directory.clone(),
            vec![0],
            true,
            true,
        )
        .await
        .unwrap();
    assert_eq!(id.len(), 64);
    let item = settled(&engine).await;
    assert_eq!(item.state, "paused");
    assert_eq!(item.files.len(), 2);
    assert!(item.files[0].selected);
    assert!(!item.files[1].selected);
    assert_eq!(item.downloaded_bytes, 70_001); // selected payload, without alignment gaps
    assert!(
        engine
            .add(bytes, cfg.download_directory.clone(), vec![0], true, true)
            .await
            .is_err()
    );
    engine.shutdown().await.unwrap();
    drop(engine);
    let session_path = state.path().join("session/session.json");
    let saved: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&session_path).unwrap()).unwrap();
    let saved = saved["torrents"]
        .as_object()
        .unwrap()
        .values()
        .next()
        .unwrap();
    assert_eq!(saved["info_hash_v2"].as_str(), Some(id.as_str()));
    assert_eq!(saved["info_hash"].as_str().unwrap().len(), 40);
    std::fs::write(downloads.path().join("a"), vec![38; 70_001]).unwrap();
    let engine = Engine::open(cfg.clone()).await.unwrap();
    let item = settled(&engine).await;
    assert_eq!(item.id, id);
    assert_eq!(item.downloaded_bytes, 0);
    assert_eq!(item.state, "paused");
    engine.remove(id, false).await.unwrap();
    engine.shutdown().await.unwrap();
    assert!(downloads.path().join("a").exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn v2_empty_files_have_no_pieces_and_complete_without_network_data() {
    use seedhop_core::Engine;
    let state = tempfile::tempdir().unwrap();
    let downloads = tempfile::tempdir().unwrap();
    let cfg = engine_config(state.path(), downloads.path());
    let engine = Engine::open(cfg.clone()).await.unwrap();
    engine
        .add(
            encode(&fixture(&[("a", vec![]), ("dir/b", vec![])], 16384)),
            cfg.download_directory,
            vec![0, 1],
            false,
            false,
        )
        .await
        .unwrap();
    let item = settled(&engine).await;
    assert_eq!(item.total_bytes, 0);
    assert_eq!(item.state, "seeding");
    assert_eq!(
        std::fs::metadata(downloads.path().join("dir/b"))
            .unwrap()
            .len(),
        0
    );
    engine.shutdown().await.unwrap();
}

#[test]
fn hash_wire_messages_roundtrip_across_every_ring_buffer_split() {
    use librqbit_peer_protocol::{HashRequest, Message};
    let request = HashRequest {
        pieces_root: [7; 32],
        base_layer: 1,
        index: 0,
        length: 4,
        proof_layers: 3,
    };
    for message in [
        Message::HashRequest(request),
        Message::HashReject(request),
        Message::Hashes {
            request,
            hashes: vec![[1; 32], [2; 32], [3; 32], [4; 32], [5; 32], [6; 32]],
        },
    ] {
        let mut bytes = [0u8; 1024];
        let length = message.serialize(&mut bytes, &Default::default).unwrap();
        for split in 0..=length {
            let (parsed, consumed) =
                Message::deserialize(&bytes[..split], &bytes[split..length]).unwrap();
            assert_eq!(consumed, length);
            assert_eq!(format!("{parsed:?}"), format!("{message:?}"));
        }
        assert!(
            message
                .serialize(&mut bytes[..length - 1], &Default::default)
                .is_err()
        );
    }
    for invalid in [
        HashRequest {
            length: 0,
            ..request
        },
        HashRequest {
            length: 3,
            ..request
        },
        HashRequest {
            length: 1024,
            ..request
        },
        HashRequest {
            index: 1,
            ..request
        },
        HashRequest {
            base_layer: 32,
            ..request
        },
    ] {
        // Framed requests/rejects preserve parameters for semantic validation
        // and correlation. Hash data remains subject to strict response bounds.
        for message in [Message::HashRequest(invalid), Message::HashReject(invalid)] {
            let mut bytes = [0; 1024];
            let length = message.serialize(&mut bytes, &Default::default).unwrap();
            for split in 0..=length {
                let (parsed, consumed) =
                    Message::deserialize(&bytes[..split], &bytes[split..length]).unwrap();
                assert_eq!(consumed, length);
                assert_eq!(format!("{parsed:?}"), format!("{message:?}"));
            }
        }
        let message = Message::Hashes {
            request: invalid,
            hashes: vec![],
        };
        let mut bytes = [0; 1024];
        let length = message.serialize(&mut bytes, &Default::default).unwrap();
        assert!(Message::deserialize(&bytes[..length], &[]).is_err());
    }
}

mod wire {
    use librqbit_peer_protocol::Message;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    pub async fn send<W: tokio::io::AsyncWrite + Unpin>(peer: &mut W, message: Message<'_>) {
        let mut buf = [0; 1024];
        let len = message.serialize(&mut buf, &Default::default).unwrap();
        peer.write_all(&buf[..len]).await.unwrap();
        peer.flush().await.unwrap();
    }
    pub async fn frame<R: tokio::io::AsyncRead + Unpin>(peer: &mut R) -> Vec<u8> {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            let len = peer.read_u32().await.unwrap();
            assert!(len <= 1024 * 1024);
            let mut buf = vec![0; len as usize + 4];
            buf[..4].copy_from_slice(&len.to_be_bytes());
            peer.read_exact(&mut buf[4..]).await.unwrap();
            buf
        })
        .await
        .expect("peer did not answer request")
    }

    pub async fn connect(port: u16, hash: [u8; 32]) -> tokio::net::TcpStream {
        connect_advertised(port, hash).await.0
    }

    pub async fn connect_advertised(
        port: u16,
        hash: [u8; 32],
    ) -> (
        tokio::net::TcpStream,
        librqbit_peer_protocol::extended::PeerExtendedMessageIds,
    ) {
        let mut peer = match tokio::net::TcpStream::connect(("::1", port)).await {
            Ok(peer) => peer,
            Err(_) => tokio::net::TcpStream::connect(("127.0.0.1", port))
                .await
                .unwrap(),
        };
        let mut handshake = [0; 68];
        handshake[0] = 19;
        handshake[1..20].copy_from_slice(b"BitTorrent protocol");
        handshake[25] = 0x10; // BEP 10 extensions.
        handshake[28..48].copy_from_slice(&hash[..20]);
        handshake[48..].copy_from_slice(b"-SHtest-123456789012");
        peer.write_all(&handshake).await.unwrap();
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            peer.read_exact(&mut handshake),
        )
        .await
        .expect("BitTorrent handshake timed out; check loopback listener ownership")
        .unwrap();
        assert_eq!(&handshake[28..48], &hash[..20]);
        let mut advertised = None;
        loop {
            let buf = frame(&mut peer).await;
            match Message::deserialize(&buf, &[]).unwrap().0 {
                Message::Extended(
                    librqbit_peer_protocol::extended::ExtendedMessage::Handshake(handshake),
                ) => advertised = Some(handshake.m),
                Message::Unchoke => break,
                _ => {}
            }
        }
        (peer, advertised.expect("missing extension handshake"))
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn v2_upload_requests_receive_piece_or_reject_without_losing_connection() {
    use librqbit_peer_protocol::{Message, Request};
    use wire::{frame, send};

    for hybrid in [false, true] {
        for complete in [false, true] {
            let state = tempfile::tempdir().unwrap();
            let files = tempfile::tempdir().unwrap();
            if complete {
                std::fs::write(files.path().join("file"), b"hello").unwrap();
            }
            let metadata = if hybrid {
                self::hybrid()
            } else {
                fixture(&[("file", b"hello".to_vec())], 16384)
            };
            let hash = parse(&metadata).info_hash();
            let engine = seedhop_core::Engine::open(engine_config(state.path(), files.path()))
                .await
                .unwrap();
            engine
                .add(
                    encode(&metadata),
                    files.path().display().to_string(),
                    vec![0],
                    false,
                    true,
                )
                .await
                .unwrap();
            settled(&engine).await;
            let mut peer = wire::connect(engine.snapshot().unwrap().listen_port, hash).await;
            send(&mut peer, Message::Interested).await;
            // Invalid index, invalid range, then a valid request on the same
            // connection. The last request is available only in the seed case.
            for request in [
                Request {
                    index: 42,
                    begin: 0,
                    length: 5,
                },
                Request {
                    index: 0,
                    begin: 16384,
                    length: 5,
                },
                Request {
                    index: 0,
                    begin: 0,
                    length: 5,
                },
            ] {
                send(&mut peer, Message::Request(request)).await;
                let buf = frame(&mut peer).await;
                match Message::deserialize(&buf, &[]).unwrap().0 {
                    Message::Reject(rejected) => {
                        assert_eq!(
                            (rejected.index, rejected.begin, rejected.length),
                            (request.index, request.begin, request.length)
                        );
                        assert!(!complete || request.index != 0 || request.begin != 0);
                    }
                    Message::Piece(piece) => {
                        assert!(complete);
                        assert_eq!(piece.index, request.index);
                        assert_eq!(piece.begin, request.begin);
                        assert_eq!([piece.data().0, piece.data().1].concat(), b"hello");
                    }
                    message => panic!("unexpected response {message:?}"),
                }
            }
            drop(peer);
            engine.shutdown().await.unwrap();
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn v2_bitfield_order_and_spare_bits_are_enforced_on_the_wire() {
    use librqbit_peer_protocol::{Message, Request};
    use tokio::io::AsyncReadExt;
    for hybrid in [false, true] {
        let state = tempfile::tempdir().unwrap();
        let files = tempfile::tempdir().unwrap();
        std::fs::write(files.path().join("file"), b"hello").unwrap();
        let metadata = if hybrid {
            self::hybrid()
        } else {
            fixture(&[("file", b"hello".to_vec())], 16384)
        };
        let hash = parse(&metadata).info_hash();
        let engine = seedhop_core::Engine::open(engine_config(state.path(), files.path()))
            .await
            .unwrap();
        engine
            .add(
                encode(&metadata),
                files.path().display().to_string(),
                vec![0],
                false,
                true,
            )
            .await
            .unwrap();
        settled(&engine).await;
        let port = engine.snapshot().unwrap().listen_port;
        for case in ["spare", "late", "duplicate"] {
            let mut peer = wire::connect(port, hash).await;
            match case {
                "spare" => wire::send(&mut peer, Message::Bitfield((&[1u8][..]).into())).await,
                "late" => {
                    wire::send(&mut peer, Message::KeepAlive).await;
                    wire::send(&mut peer, Message::Bitfield((&[0u8][..]).into())).await;
                }
                "duplicate" => {
                    wire::send(
                        &mut peer,
                        Message::Extended(
                            librqbit_peer_protocol::extended::ExtendedMessage::Handshake(
                                librqbit_peer_protocol::extended::handshake::ExtendedHandshake::new(
                                ),
                            ),
                        ),
                    )
                    .await;
                    wire::send(&mut peer, Message::Bitfield((&[0u8][..]).into())).await;
                    // A valid initial bitfield permits normal payload exchange.
                    wire::send(
                        &mut peer,
                        Message::Request(Request {
                            index: 0,
                            begin: 0,
                            length: 5,
                        }),
                    )
                    .await;
                    let response = wire::frame(&mut peer).await;
                    assert!(matches!(
                        Message::deserialize(&response, &[]).unwrap().0,
                        Message::Piece(_)
                    ));
                    wire::send(&mut peer, Message::Bitfield((&[0u8][..]).into())).await;
                }
                _ => unreachable!(),
            }
            let result =
                tokio::time::timeout(std::time::Duration::from_secs(5), peer.read(&mut [0; 1]))
                    .await
                    .expect("invalid bitfield did not close the peer");
            assert!(
                matches!(result, Ok(0)) || result.is_err(),
                "unexpected bytes after {case} bitfield: {result:?}"
            );
        }
        engine.shutdown().await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn v2_metadata_connection_rejects_requests_and_unsolicited_payload() {
    use librqbit::{AddTorrent, AddTorrentOptions, Session, SessionOptions};
    use librqbit_peer_protocol::{HashRequest, Message, Request};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let files = tempfile::tempdir().unwrap();
    let session = Session::new_with_opts(
        files.path().into(),
        SessionOptions {
            dht: None,
            disable_trackers: true,
            disable_local_service_discovery: true,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let resolving = tokio::spawn({
        let session = session.clone();
        async move {
            session
                .add_torrent(
                    AddTorrent::from_url(format!("magnet:?xt=urn:btmh:1220{}", "ab".repeat(32))),
                    Some(AddTorrentOptions {
                        list_only: true,
                        initial_peers: Some(vec![addr]),
                        disable_trackers: true,
                        ..Default::default()
                    }),
                )
                .await
        }
    });
    let (mut peer, _) = tokio::time::timeout(std::time::Duration::from_secs(5), listener.accept())
        .await
        .unwrap()
        .unwrap();
    let mut handshake = [0; 68];
    peer.read_exact(&mut handshake).await.unwrap();
    handshake[48..].copy_from_slice(b"-SHtest-123456789012");
    peer.write_all(&handshake).await.unwrap();
    let request = Request {
        index: 0,
        begin: 0,
        length: 16384,
    };
    let hash_request = HashRequest {
        pieces_root: [1; 32],
        base_layer: 0,
        index: 0,
        length: 2,
        proof_layers: 0,
    };
    wire::send(&mut peer, Message::Request(request)).await;
    wire::send(&mut peer, Message::HashRequest(hash_request)).await;
    let mut rejected = [false; 2];
    while !rejected.iter().all(|v| *v) {
        let buf = wire::frame(&mut peer).await;
        match Message::deserialize(&buf, &[]).unwrap().0 {
            Message::Reject(r) => {
                assert_eq!(
                    (r.index, r.begin, r.length),
                    (request.index, request.begin, request.length)
                );
                rejected[0] = true;
            }
            Message::HashReject(r) => {
                assert_eq!(r, hash_request);
                rejected[1] = true;
            }
            Message::Extended(_) | Message::Unchoke => {}
            msg => panic!("unexpected metadata peer response {msg:?}"),
        }
    }
    // A data block without any request from the metadata consumer must close
    // the connection, rather than being silently ignored.
    peer.write_all(&[0, 0, 0, 10, 7, 0, 0, 0, 0, 0, 0, 0, 0, 1])
        .await
        .unwrap();
    let result = tokio::time::timeout(std::time::Duration::from_secs(5), peer.read(&mut [0; 1]))
        .await
        .expect("unsolicited metadata payload did not close peer");
    assert!(matches!(result, Ok(0)) || result.is_err());
    resolving.abort();
    let _ = resolving.await;
    session.stop_and_wait().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn v2_download_continues_on_the_same_peer_after_a_correlated_reject() {
    use librqbit::{AddTorrent, AddTorrentOptions, Session, SessionOptions};
    use librqbit_peer_protocol::Message;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let data: Vec<_> = (0..32768).map(|i| (i % 251) as u8).collect();
    let metadata = encode(&fixture(&[("file", data.clone())], 16384));
    let files = tempfile::tempdir().unwrap();
    let session = Session::new_with_opts(
        files.path().into(),
        SessionOptions {
            dht: None,
            disable_trackers: true,
            disable_local_service_discovery: true,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let peer_task = tokio::spawn({
        let data = data.clone();
        async move {
            let (mut peer, _) = listener.accept().await.unwrap();
            let mut handshake = [0; 68];
            peer.read_exact(&mut handshake).await.unwrap();
            handshake[20..28].fill(0);
            handshake[48..].copy_from_slice(b"-SHtest-123456789012");
            peer.write_all(&handshake).await.unwrap();
            wire::send(&mut peer, Message::Bitfield((&[0xc0u8][..]).into())).await;
            wire::send(&mut peer, Message::Unchoke).await;
            let mut rejected = None;
            let mut served = 0;
            loop {
                let buf = wire::frame(&mut peer).await;
                if let Message::Request(request) = Message::deserialize(&buf, &[]).unwrap().0 {
                    if rejected.is_none() {
                        rejected = Some(request.index);
                        wire::send(&mut peer, Message::Reject(request)).await;
                        continue;
                    }
                    if served == 0 {
                        assert_ne!(Some(request.index), rejected);
                    }
                    let start = request.index as usize * 16384 + request.begin as usize;
                    let payload = &data[start..start + request.length as usize];
                    peer.write_u32(9 + request.length).await.unwrap();
                    peer.write_u8(7).await.unwrap();
                    peer.write_u32(request.index).await.unwrap();
                    peer.write_u32(request.begin).await.unwrap();
                    peer.write_all(payload).await.unwrap();
                    served += 1;
                    if served == 1 {
                        wire::send(&mut peer, Message::Have(rejected.unwrap())).await;
                    } else {
                        break;
                    }
                }
            }
        }
    });
    let handle = session
        .add_torrent(
            AddTorrent::from_bytes(metadata),
            Some(AddTorrentOptions {
                initial_peers: Some(vec![addr]),
                disable_trackers: true,
                output_folder: Some(files.path().display().to_string()),
                ..Default::default()
            }),
        )
        .await
        .unwrap()
        .into_handle()
        .unwrap();
    tokio::time::timeout(
        std::time::Duration::from_secs(15),
        handle.wait_until_completed(),
    )
    .await
    .unwrap()
    .unwrap();
    peer_task.await.unwrap();
    assert_eq!(std::fs::read(files.path().join("file")).unwrap(), data);
    session.stop_and_wait().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn v2_magnet_retries_a_transient_handshake_failure_from_a_known_peer() {
    if std::env::var_os("SEEDHOP_DIAGNOSTICS").is_some() {
        let _ = tracing_subscriber::fmt()
            .with_env_filter("librqbit=debug")
            .with_test_writer()
            .try_init();
    }
    use librqbit::{AddTorrent, AddTorrentOptions, AddTorrentResponse, Session, SessionOptions};
    use librqbit_peer_protocol::{
        Message,
        extended::{ExtendedMessage, handshake::ExtendedHandshake},
    };
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut metadata = fixture(&[("file", b"hello".to_vec())], 16384);
    let hash = parse(&metadata).info_hash();
    let info = encode(get(&mut metadata, b"info"));
    let files = tempfile::tempdir().unwrap();
    let session = Session::new_with_opts(
        files.path().into(),
        SessionOptions {
            // This fixture tests plain handshake/identity retries.
            peer_encryption: librqbit::EncryptionPolicy::Disabled,
            dht: None,
            disable_trackers: true,
            disable_local_service_discovery: true,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let peer_task = tokio::spawn(async move {
        let (mut peer, _) = listener.accept().await.unwrap();
        let mut handshake = [0; 68];
        peer.read_exact(&mut handshake).await.unwrap();
        drop(peer); // A socket was reachable before its torrent became ready.
        let (mut peer, _) = listener.accept().await.unwrap();
        peer.read_exact(&mut handshake).await.unwrap();
        handshake[48..].copy_from_slice(b"-SHtest-123456789012");
        peer.write_all(&handshake).await.unwrap();
        let mut extension = ExtendedHandshake::new();
        extension.metadata_size = Some(info.len() as u32);
        wire::send(
            &mut peer,
            Message::Extended(ExtendedMessage::Handshake(extension)),
        )
        .await;
        loop {
            let buf = wire::frame(&mut peer).await;
            if let Message::Extended(ExtendedMessage::Handshake(h)) =
                Message::deserialize(&buf, &[]).unwrap().0
            {
                let id = h.m.ut_metadata.unwrap();
                let header = format!("d8:msg_typei1e5:piecei0e10:total_sizei{}ee", info.len());
                peer.write_u32((2 + header.len() + info.len()) as u32)
                    .await
                    .unwrap();
                peer.write_all(&[20, id]).await.unwrap();
                peer.write_all(header.as_bytes()).await.unwrap();
                peer.write_all(&info).await.unwrap();
                // Keep the connection alive until the resolver consumes info.
                let _ = tokio::io::copy(&mut peer, &mut tokio::io::sink()).await;
                return;
            }
        }
    });
    let hash: String = hash.iter().map(|b| format!("{b:02x}")).collect();
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        session.add_torrent(
            AddTorrent::from_url(format!("magnet:?xt=urn:btmh:1220{hash}")),
            Some(AddTorrentOptions {
                list_only: true,
                initial_peers: Some(vec![addr]),
                disable_trackers: true,
                ..Default::default()
            }),
        ),
    )
    .await
    .unwrap()
    .unwrap();
    let AddTorrentResponse::ListOnly(list) = response else {
        panic!("unexpected membership")
    };
    assert_eq!(
        adapter::preview(list.torrent_bytes.to_vec()).unwrap().id,
        hash
    );
    assert_eq!(std::fs::read_dir(files.path()).unwrap().count(), 0);
    peer_task.await.unwrap();
    session.stop_and_wait().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn btih_hybrid_small_files_resolve_from_a_strictly_legacy_metadata_peer() {
    use librqbit::{AddTorrent, AddTorrentOptions, AddTorrentResponse, Session, SessionOptions};
    use librqbit_peer_protocol::{
        Message,
        extended::{ExtendedMessage, handshake::ExtendedHandshake},
    };
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    for private in [false, true] {
        let mut metadata = hybrid_files(
            &[("first", b"hello".to_vec()), ("second", b"world".to_vec())],
            16384,
            false,
        );
        if private {
            put(get(&mut metadata, b"info"), b"private", B::Number(1));
        }
        let expected = parse(&metadata).info_hash();
        let info = encode(get(&mut metadata, b"info"));
        let v1_hash = sha1::Sha1::digest(&info);
        let magnet = format!("magnet:?xt=urn:btih:{v1_hash:x}");
        let files = tempfile::tempdir().unwrap();
        let session = Session::new_with_opts(
            files.path().into(),
            SessionOptions {
                peer_encryption: librqbit::EncryptionPolicy::Disabled,
                dht: None,
                disable_trackers: true,
                disable_local_service_discovery: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let peer_task = tokio::spawn(async move {
            let (mut peer, _) = listener.accept().await.unwrap();
            let mut handshake = [0; 68];
            peer.read_exact(&mut handshake).await.unwrap();
            assert_eq!(&handshake[28..48], v1_hash.as_slice());
            handshake[20..28].fill(0);
            handshake[25] = 0x10; // BEP 10 only: no v2 upgrade support.
            handshake[48..].copy_from_slice(b"-V1test-123456789012");
            peer.write_all(&handshake).await.unwrap();
            let mut extension = ExtendedHandshake::new();
            extension.metadata_size = Some(info.len() as u32);
            wire::send(
                &mut peer,
                Message::Extended(ExtendedMessage::Handshake(extension)),
            )
            .await;
            loop {
                let buf = wire::frame(&mut peer).await;
                if let Message::Extended(ExtendedMessage::Handshake(h)) =
                    Message::deserialize(&buf, &[]).unwrap().0
                {
                    let id = h.m.ut_metadata.unwrap();
                    let header = format!("d8:msg_typei1e5:piecei0e10:total_sizei{}ee", info.len());
                    peer.write_u32((2 + header.len() + info.len()) as u32)
                        .await
                        .unwrap();
                    peer.write_all(&[20, id]).await.unwrap();
                    peer.write_all(header.as_bytes()).await.unwrap();
                    peer.write_all(&info).await.unwrap();
                    break;
                }
            }
            // No second connection using the derived v2 identity is needed.
            assert!(
                tokio::time::timeout(std::time::Duration::from_millis(400), listener.accept())
                    .await
                    .is_err()
            );
        });
        let response = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            session.add_torrent(
                AddTorrent::from_url(magnet),
                Some(AddTorrentOptions {
                    list_only: true,
                    initial_peers: Some(vec![addr]),
                    disable_trackers: true,
                    ..Default::default()
                }),
            ),
        )
        .await
        .unwrap()
        .unwrap();
        let AddTorrentResponse::ListOnly(list) = response else {
            panic!("unexpected membership")
        };
        let preview = adapter::preview(list.torrent_bytes.to_vec()).unwrap();
        assert_eq!(
            preview.id,
            expected
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        );
        assert_eq!(preview.total_bytes, 10);
        assert_eq!(std::fs::read_dir(files.path()).unwrap().count(), 0);
        session.with_torrents(|items| assert_eq!(items.count(), 0));
        peer_task.await.unwrap();
        session.stop_and_wait().await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn private_hybrid_discovery_never_appends_session_extra_trackers() {
    use librqbit::{AddTorrent, AddTorrentOptions, Session, SessionOptions};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    async fn tracker() -> (String, Arc<AtomicUsize>, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/announce", listener.local_addr().unwrap());
        let calls = Arc::new(AtomicUsize::new(0));
        let observed = calls.clone();
        let task = tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                observed.fetch_add(1, Ordering::SeqCst);
                let _ = socket.read(&mut [0; 8192]).await;
                let body = b"d8:intervali1e5:peers0:e";
                let header = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = socket.write_all(header.as_bytes()).await;
                let _ = socket.write_all(body).await;
            }
        });
        (url, calls, task)
    }
    // The public control proves the configured extra tracker and listener work.
    for (private, explicit_count) in [(false, 0), (true, 0), (true, 1), (true, 2)] {
        let (extra, extra_calls, extra_task) = tracker().await;
        let (first, first_calls, first_task) = tracker().await;
        let (second, second_calls, second_task) = tracker().await;
        let mut metadata = hybrid();
        if private {
            put(get(&mut metadata, b"info"), b"private", B::Number(1));
        }
        if explicit_count > 0 {
            put(&mut metadata, b"announce", bytes(&first));
        }
        if explicit_count > 1 {
            put(
                &mut metadata,
                b"announce-list",
                B::List(vec![
                    B::List(vec![bytes(first)]),
                    B::List(vec![bytes(second)]),
                ]),
            );
        }
        let files = tempfile::tempdir().unwrap();
        let session = Session::new_with_opts(
            files.path().into(),
            SessionOptions {
                dht: None,
                disable_local_service_discovery: true,
                trackers: [extra.parse().unwrap()].into_iter().collect(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let _handle = session
            .add_torrent(
                AddTorrent::from_bytes(encode(&metadata)),
                Some(AddTorrentOptions {
                    output_folder: Some(files.path().to_string_lossy().into()),
                    ..Default::default()
                }),
            )
            .await
            .unwrap()
            .into_handle()
            .unwrap();
        if !private || explicit_count > 0 {
            let calls = if private { &first_calls } else { &extra_calls };
            tokio::time::timeout(std::time::Duration::from_secs(5), async {
                while calls.load(Ordering::SeqCst) == 0 {
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                }
            })
            .await
            .expect("expected tracker was never contacted");
        }
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
        if private {
            assert_eq!(
                extra_calls.load(Ordering::SeqCst),
                0,
                "private torrent contacted a global tracker"
            );
        }
        assert_eq!(
            second_calls.load(Ordering::SeqCst),
            0,
            "private tracker policy did not keep its first tracker"
        );
        session.stop_and_wait().await.unwrap();
        for task in [extra_task, first_task, second_task] {
            task.abort();
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn private_metadata_stops_extra_trackers_and_unrelated_peers_before_proofs_finish() {
    use librqbit::{AddTorrent, AddTorrentOptions, AddTorrentResponse, Session, SessionOptions};
    use librqbit_peer_protocol::{
        Message,
        extended::{ExtendedMessage, handshake::ExtendedHandshake},
    };
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    for dual in [false, true] {
        let mut metadata = if dual {
            hybrid_files(&[("file", vec![7; 70001])], 32768, false)
        } else {
            fixture(&[("file", vec![7; 70001])], 32768)
        };
        put(get(&mut metadata, b"info"), b"private", B::Number(1));
        let metainfo = Arc::new(parse(&metadata));
        let expected = metainfo.info_hash();
        let info = Arc::new(encode(get(&mut metadata, b"info")));
        let v1 = sha1::Sha1::digest(&*info);
        let sink = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let sink_addr = sink.local_addr().unwrap();
        let accepted = Arc::new(AtomicUsize::new(0));
        let closed = Arc::new(AtomicUsize::new(0));
        let sink_task = tokio::spawn({
            let accepted = accepted.clone();
            let closed = closed.clone();
            async move {
                let mut connections = tokio::task::JoinSet::new();
                loop {
                    tokio::select! {
                        result = sink.accept() => {
                            let (mut peer, _) = result.unwrap(); let accepted = accepted.clone(); let closed = closed.clone();
                            connections.spawn(async move {
                                let mut handshake = [0; 68];
                                if peer.read_exact(&mut handshake).await.is_err() { return; }
                                handshake[48..].copy_from_slice(b"-SHtest-123456789012");
                                if peer.write_all(&handshake).await.is_err() { return; }
                                accepted.fetch_add(1, Ordering::SeqCst);
                                let mut bytes = [0; 1024];
                                while let Ok(count) = peer.read(&mut bytes).await { if count == 0 { break; } }
                                closed.fetch_add(1, Ordering::SeqCst);
                            });
                        },
                        _ = connections.join_next(), if !connections.is_empty() => {},
                    }
                }
            }
        });
        let tracker = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let tracker_url = format!("http://{}/announce", tracker.local_addr().unwrap());
        let announces = Arc::new(AtomicUsize::new(0));
        let tracker_task = tokio::spawn({
            let announces = announces.clone();
            async move {
                loop {
                    let (mut socket, _) = tracker.accept().await.unwrap();
                    let _ = socket.read(&mut [0; 8192]).await;
                    announces.fetch_add(1, Ordering::SeqCst);
                    let mut body = b"d8:intervali1e5:peers6:".to_vec();
                    body.extend([127, 0, 0, 1]);
                    body.extend(sink_addr.port().to_be_bytes());
                    body.push(b'e');
                    let header = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = socket.write_all(header.as_bytes()).await;
                    let _ = socket.write_all(&body).await;
                }
            }
        });
        let provider = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let provider_addr = provider.local_addr().unwrap();
        let proofs_waiting = Arc::new(AtomicUsize::new(0));
        let (release, gate) = tokio::sync::watch::channel(false);
        let provider_task = tokio::spawn({
            let accepted = accepted.clone();
            let proofs_waiting = proofs_waiting.clone();
            async move {
                let mut connections = tokio::task::JoinSet::new();
                loop {
                    tokio::select! {
                        result = provider.accept() => {
                            let (mut peer, _) = result.unwrap(); let info = info.clone(); let metainfo = metainfo.clone();
                            let accepted = accepted.clone(); let proofs_waiting = proofs_waiting.clone(); let mut gate = gate.clone();
                            connections.spawn(async move {
                                let mut handshake = [0; 68];
                                if peer.read_exact(&mut handshake).await.is_err() { return; }
                                handshake[48..].copy_from_slice(b"-SHtest-123456789012");
                                if peer.write_all(&handshake).await.is_err() { return; }
                                let mut extended = ExtendedHandshake::new(); extended.metadata_size = Some(info.len() as u32);
                                wire::send(&mut peer, Message::Extended(ExtendedMessage::Handshake(extended))).await;
                                let mut metadata_id = None;
                                while let Ok(length) = peer.read_u32().await {
                                    assert!(length <= 1024 * 1024);
                                    let mut frame = vec![0; length as usize + 4]; frame[..4].copy_from_slice(&length.to_be_bytes());
                                    if peer.read_exact(&mut frame[4..]).await.is_err() { return; }
                                    match Message::deserialize(&frame, &[]).unwrap().0 {
                                        Message::Extended(ExtendedMessage::Handshake(h)) => metadata_id = h.m.ut_metadata,
                                        Message::Extended(ExtendedMessage::UtMetadata(librqbit_peer_protocol::extended::ut_metadata::UtMetadata::Request(0))) => {
                                            tokio::time::timeout(std::time::Duration::from_secs(5), async {
                                                while accepted.load(Ordering::SeqCst) == 0 { tokio::time::sleep(std::time::Duration::from_millis(5)).await; }
                                            }).await.expect("extra tracker did not supply its public peer");
                                            let header = format!("d8:msg_typei1e5:piecei0e10:total_sizei{}ee", info.len());
                                            let Some(id) = metadata_id else { panic!("client did not advertise metadata") };
                                            if peer.write_u32((2 + header.len() + info.len()) as u32).await.is_err() { return; }
                                            if peer.write_all(&[20,id]).await.is_err() || peer.write_all(header.as_bytes()).await.is_err() || peer.write_all(&info).await.is_err() { return; }
                                        },
                                        Message::HashRequest(request) => {
                                            proofs_waiting.fetch_add(1, Ordering::SeqCst);
                                            while !*gate.borrow_and_update() { if gate.changed().await.is_err() { return; } }
                                            let hashes = metainfo.serve_hashes(&request, |_,_,_| panic!("piece-layer fixture should not read payload")).unwrap();
                                            wire::send(&mut peer, Message::Hashes { request, hashes }).await;
                                        },
                                        _ => {},
                                    }
                                }
                            });
                        },
                        _ = connections.join_next(), if !connections.is_empty() => {},
                    }
                }
            }
        });
        let files = tempfile::tempdir().unwrap();
        let session = Session::new_with_opts(
            files.path().into(),
            SessionOptions {
                peer_encryption: librqbit::EncryptionPolicy::Disabled,
                dht: None,
                disable_local_service_discovery: true,
                trackers: [tracker_url.parse().unwrap()].into_iter().collect(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let v2 = expected
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let mut magnet = format!("magnet:?xt=urn:btmh:1220{v2}");
        if dual {
            magnet.push_str(&format!("&xt=urn:btih:{v1:x}"));
        }
        let resolving = tokio::spawn({
            let session = session.clone();
            async move {
                session
                    .add_torrent(
                        AddTorrent::from_url(magnet),
                        Some(AddTorrentOptions {
                            list_only: true,
                            initial_peers: Some(vec![provider_addr]),
                            force_tracker_interval: Some(std::time::Duration::from_millis(100)),
                            ..Default::default()
                        }),
                    )
                    .await
            }
        });
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while proofs_waiting.load(Ordering::SeqCst) == 0 {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
            while closed.load(Ordering::SeqCst) != accepted.load(Ordering::SeqCst) {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("public peer attempts remained alive after private dictionary authentication");
        assert!(accepted.load(Ordering::SeqCst) > 0);
        assert!(
            !resolving.is_finished(),
            "proofs were deliberately withheld"
        );
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        let before = announces.load(Ordering::SeqCst);
        tokio::time::sleep(std::time::Duration::from_millis(600)).await;
        assert_eq!(
            announces.load(Ordering::SeqCst),
            before,
            "global tracker kept announcing while private proofs were pending"
        );
        assert_eq!(
            closed.load(Ordering::SeqCst),
            accepted.load(Ordering::SeqCst)
        );
        release.send(true).unwrap();
        let result = tokio::time::timeout(std::time::Duration::from_secs(5), resolving)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let AddTorrentResponse::ListOnly(list) = result else {
            panic!("unexpected membership")
        };
        assert_eq!(
            adapter::preview(list.torrent_bytes.to_vec()).unwrap().id,
            v2
        );
        session.with_torrents(|items| assert_eq!(items.count(), 0));
        assert_eq!(std::fs::read_dir(files.path()).unwrap().count(), 0);
        session.stop_and_wait().await.unwrap();
        for task in [provider_task, sink_task, tracker_task] {
            task.abort();
            let _ = task.await;
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn unauthenticated_private_dictionary_cannot_disable_public_metadata_discovery() {
    use librqbit::{AddTorrent, AddTorrentOptions, AddTorrentResponse, Session, SessionOptions};
    use librqbit_peer_protocol::{
        Message,
        extended::{ExtendedMessage, handshake::ExtendedHandshake, ut_metadata::UtMetadata},
    };
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    for dual in [false, true] {
        let swarm = support::Swarm::with_metadata(70001, |data| {
            encode(&if dual {
                hybrid_files(&[("payload.bin", data.to_vec())], 32768, false)
            } else {
                fixture(&[("payload.bin", data.to_vec())], 32768)
            })
        })
        .await;
        let mut forged = if dual {
            hybrid_files(&[("payload.bin", swarm.content.clone())], 32768, false)
        } else {
            fixture(&[("payload.bin", swarm.content.clone())], 32768)
        };
        let expected = parse(&forged).info_hash();
        let original = encode(get(&mut forged, b"info"));
        let v1 = sha1::Sha1::digest(&original);
        put(get(&mut forged, b"info"), b"private", B::Number(1));
        let bad_info = Arc::new(encode(get(&mut forged, b"info")));
        assert_ne!(hash(&bad_info), expected);
        let rejected = Arc::new(AtomicBool::new(false));
        let supplied = Arc::new(AtomicUsize::new(0));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let bad_addr = listener.local_addr().unwrap();
        let attacker = tokio::spawn({
            let rejected = rejected.clone();
            let supplied = supplied.clone();
            async move {
                let mut connections = tokio::task::JoinSet::new();
                loop {
                    tokio::select! {
                        result = listener.accept() => {
                            let (mut peer, _) = result.unwrap(); let info = bad_info.clone(); let rejected = rejected.clone(); let supplied = supplied.clone();
                            connections.spawn(async move {
                                let mut handshake = [0; 68];
                                if peer.read_exact(&mut handshake).await.is_err() { return; }
                                handshake[48..].copy_from_slice(b"-SHtest-123456789012");
                                if peer.write_all(&handshake).await.is_err() { return; }
                                let mut extended = ExtendedHandshake::new(); extended.metadata_size = Some(info.len() as u32);
                                wire::send(&mut peer, Message::Extended(ExtendedMessage::Handshake(extended))).await;
                                let mut metadata_id = None; let mut sent = false;
                                while let Ok(length) = peer.read_u32().await {
                                    assert!(length <= 1024 * 1024);
                                    let mut frame = vec![0; length as usize + 4]; frame[..4].copy_from_slice(&length.to_be_bytes());
                                    if peer.read_exact(&mut frame[4..]).await.is_err() { break; }
                                    match Message::deserialize(&frame, &[]).unwrap().0 {
                                        Message::Extended(ExtendedMessage::Handshake(h)) => metadata_id = h.m.ut_metadata,
                                        Message::Extended(ExtendedMessage::UtMetadata(UtMetadata::Request(0))) => {
                                            let header = format!("d8:msg_typei1e5:piecei0e10:total_sizei{}ee", info.len());
                                            if peer.write_u32((2 + header.len() + info.len()) as u32).await.is_err() { return; }
                                            if peer.write_all(&[20,metadata_id.unwrap()]).await.is_err() || peer.write_all(header.as_bytes()).await.is_err() || peer.write_all(&info).await.is_err() { return; }
                                            supplied.fetch_add(1, Ordering::SeqCst); sent = true;
                                        },
                                        _ => {},
                                    }
                                }
                                if sent { rejected.store(true, Ordering::SeqCst); }
                            });
                        },
                        _ = connections.join_next(), if !connections.is_empty() => {},
                    }
                }
            }
        });
        let tracker = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/announce", tracker.local_addr().unwrap());
        let good_port = swarm.seed.snapshot().unwrap().listen_port;
        let tracker_task = tokio::spawn({
            let rejected = rejected.clone();
            async move {
                loop {
                    let (mut socket, _) = tracker.accept().await.unwrap();
                    let _ = socket.read(&mut [0; 8192]).await;
                    let advertise = rejected.load(Ordering::SeqCst);
                    let mut body = if advertise {
                        b"d8:intervali1e5:peers6:".to_vec()
                    } else {
                        b"d8:intervali1e5:peers0:".to_vec()
                    };
                    if advertise {
                        body.extend([127, 0, 0, 1]);
                        body.extend(good_port.to_be_bytes());
                    }
                    body.push(b'e');
                    let header = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = socket.write_all(header.as_bytes()).await;
                    let _ = socket.write_all(&body).await;
                }
            }
        });
        let files = tempfile::tempdir().unwrap();
        let session = Session::new_with_opts(
            files.path().into(),
            SessionOptions {
                peer_encryption: librqbit::EncryptionPolicy::Disabled,
                dht: None,
                disable_local_service_discovery: true,
                trackers: [url.parse().unwrap()].into_iter().collect(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let expected_hex = expected
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let mut magnet = format!("magnet:?xt=urn:btmh:1220{expected_hex}");
        if dual {
            magnet.push_str(&format!("&xt=urn:btih:{v1:x}"));
        }
        let response = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            session.add_torrent(
                AddTorrent::from_url(magnet),
                Some(AddTorrentOptions {
                    list_only: true,
                    initial_peers: Some(vec![bad_addr]),
                    force_tracker_interval: Some(std::time::Duration::from_millis(100)),
                    ..Default::default()
                }),
            ),
        )
        .await
        .expect("forged private flag stopped discovery of a healthy public metadata provider")
        .unwrap();
        let AddTorrentResponse::ListOnly(list) = response else {
            panic!("unexpected membership")
        };
        assert_eq!(
            adapter::preview(list.torrent_bytes.to_vec()).unwrap().id,
            expected_hex
        );
        assert!(rejected.load(Ordering::SeqCst) && supplied.load(Ordering::SeqCst) > 0);
        assert_eq!(std::fs::read_dir(files.path()).unwrap().count(), 0);
        session.with_torrents(|items| assert_eq!(items.count(), 0));
        session.stop_and_wait().await.unwrap();
        for task in [attacker, tracker_task] {
            task.abort();
            let _ = task.await;
        }
        swarm.seed.shutdown().await.unwrap();
    }
}

#[test]
fn dual_magnet_parser_rejects_conflicting_duplicates_and_keeps_both_algorithms() {
    for (algorithm, length) in [("btih:", 20), ("btmh:1220", 32)] {
        let first = format!("urn:{algorithm}{}", "ab".repeat(length));
        let second = format!("urn:{algorithm}{}", "cd".repeat(length));
        let error = librqbit::Magnet::parse(&format!("magnet:?xt={first}&xt={second}"))
            .err()
            .unwrap();
        assert!(error.to_string().contains("conflicting"));
        assert!(librqbit::Magnet::parse(&format!("magnet:?xt={first}&xt={first}")).is_ok());
    }
    let parsed = librqbit::Magnet::parse(&format!(
        "magnet:?xt=urn:btih:{}&xt=urn:btmh:1220{}",
        "ab".repeat(20),
        "cd".repeat(32)
    ))
    .unwrap();
    assert!(parsed.as_id20().is_some() && parsed.as_id32().is_some());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dual_magnet_checks_both_full_hashes_before_payload_or_proof_requests() {
    use librqbit::{AddTorrent, AddTorrentOptions, Session, SessionOptions};
    use librqbit_peer_protocol::{
        Message,
        extended::{ExtendedMessage, handshake::ExtendedHandshake},
    };
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let hex = |bytes: &[u8]| {
        bytes
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    };
    for mismatch in ["sha1", "sha256", "v1-only-format", "v2-only-format"] {
        let mut metadata = match mismatch {
            "v1-only-format" => dict(&[(
                b"info",
                dict(&[
                    (b"name", bytes(b"file")),
                    (b"length", B::Number(5)),
                    (b"piece length", B::Number(16384)),
                    (b"pieces", bytes(sha1::Sha1::digest(b"hello"))),
                ]),
            )]),
            "v2-only-format" => fixture(&[("file", vec![7; 70001])], 32768),
            _ => hybrid_files(&[("file", vec![7; 70001])], 32768, false),
        };
        let info = Arc::new(encode(get(&mut metadata, b"info")));
        let actual_v2 = hash(&info);
        let actual_v1: [u8; 20] = sha1::Sha1::digest(&*info).into();
        let (mut expected_v1, mut expected_v2) = (actual_v1, actual_v2);
        if mismatch == "sha1" {
            expected_v1[19] ^= 1;
        } else if mismatch == "sha256" {
            expected_v2[31] ^= 1;
        }
        // The v2 mismatch is outside the 20-byte wire identifier.
        assert_eq!(&expected_v2[..20], &actual_v2[..20]);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let metadata_sent = Arc::new(AtomicUsize::new(0));
        let forbidden_requests = Arc::new(AtomicUsize::new(0));
        let server = tokio::spawn({
            let metadata_sent = metadata_sent.clone();
            let forbidden_requests = forbidden_requests.clone();
            async move {
                let mut connections = tokio::task::JoinSet::new();
                loop {
                    tokio::select! {
                        accepted = listener.accept() => {
                            let (mut peer, _) = accepted.unwrap();
                            let info = info.clone(); let metadata_sent = metadata_sent.clone(); let forbidden_requests = forbidden_requests.clone();
                            connections.spawn(async move {
                                let mut handshake = [0; 68];
                                if peer.read_exact(&mut handshake).await.is_err() { return; }
                                if handshake[28..48] != actual_v1 && handshake[28..48] != actual_v2[..20] { return; }
                                handshake[48..].copy_from_slice(b"-SHtest-123456789012");
                                peer.write_all(&handshake).await.unwrap();
                                let mut extended = ExtendedHandshake::new(); extended.metadata_size = Some(info.len() as u32);
                                wire::send(&mut peer, Message::Extended(ExtendedMessage::Handshake(extended))).await;
                                let mut sent = false;
                                while let Ok(length) = peer.read_u32().await {
                                    assert!(length <= 1024 * 1024);
                                    let mut body = vec![0; length as usize];
                                    if peer.read_exact(&mut body).await.is_err() { break; }
                                    if matches!(body.first(), Some(6 | 21)) { forbidden_requests.fetch_add(1, Ordering::SeqCst); }
                                    if !sent && body.starts_with(&[20, 0]) {
                                        let frame = [length.to_be_bytes().as_slice(), body.as_slice()].concat();
                                        if let Message::Extended(ExtendedMessage::Handshake(h)) = Message::deserialize(&frame, &[]).unwrap().0 {
                                            let id = h.m.ut_metadata.unwrap();
                                            let header = format!("d8:msg_typei1e5:piecei0e10:total_sizei{}ee", info.len());
                                            peer.write_u32((2 + header.len() + info.len()) as u32).await.unwrap();
                                            peer.write_all(&[20,id]).await.unwrap(); peer.write_all(header.as_bytes()).await.unwrap(); peer.write_all(&info).await.unwrap();
                                            metadata_sent.fetch_add(1, Ordering::SeqCst); sent = true;
                                        }
                                    }
                                }
                            });
                        },
                        _ = connections.join_next(), if !connections.is_empty() => {},
                    }
                }
            }
        });
        let files = tempfile::tempdir().unwrap();
        let session = Session::new_with_opts(
            files.path().into(),
            SessionOptions {
                peer_encryption: librqbit::EncryptionPolicy::Disabled,
                dht: None,
                disable_trackers: true,
                disable_local_service_discovery: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            session.add_torrent(
                AddTorrent::from_url(format!(
                    "magnet:?xt=urn:btih:{}&xt=urn:btmh:1220{}",
                    hex(&expected_v1),
                    hex(&expected_v2)
                )),
                Some(AddTorrentOptions {
                    list_only: true,
                    initial_peers: Some(vec![addr]),
                    ..Default::default()
                }),
            ),
        )
        .await
        .expect("invalid dual magnet did not exhaust its bounded peer candidates");
        assert!(result.is_err());
        assert!(metadata_sent.load(Ordering::SeqCst) > 0);
        assert_eq!(
            forbidden_requests.load(Ordering::SeqCst),
            0,
            "a mismatched dictionary was used before checking both full commitments"
        );
        session.with_torrents(|items| assert_eq!(items.count(), 0));
        assert_eq!(std::fs::read_dir(files.path()).unwrap().count(), 0);
        session.stop_and_wait().await.unwrap();
        server.abort();
        let _ = server.await;
    }
}

#[test]
fn legacy_payload_layers_authenticate_roots_and_share_identical_files() {
    use seedhop_core::v2::LegacyPieceLayers;
    for piece_length in [16384, 32768, 65536] {
        for pad_last in [false, true] {
            let large: Vec<_> = (0..piece_length * 2 + 173)
                .map(|i| (i % 251) as u8)
                .collect();
            let files = [
                ("a", large.clone()),
                ("b", b"short".to_vec()),
                ("c", large),
                ("empty", vec![]),
            ];
            let mut metadata = hybrid_files(&files, piece_length, pad_last);
            let expected = parse(&metadata);
            let info = encode(get(&mut metadata, b"info"));
            let make = || LegacyPieceLayers::from_info(&info).unwrap().unwrap();
            assert!(matches!(make().finish(), Err(V2Error::Invalid(_))));
            let mut layers = make();
            let needed = layers.needed_pieces();
            assert_eq!(needed.len(), 3, "identical files must share one layer");
            for &canonical in needed.iter().rev() {
                let candidates = layers.candidate_pieces(canonical).unwrap();
                assert_eq!(candidates.len(), 2);
                let index = *candidates.last().unwrap();
                let file = expected
                    .files()
                    .iter()
                    .find(|f| {
                        u64::from(index) >= f.first_piece()
                            && u64::from(index) - f.first_piece() < f.piece_count()
                    })
                    .unwrap();
                let data = &files
                    .iter()
                    .find(|(name, _)| *name == file.path()[0])
                    .unwrap()
                    .1;
                let offset = (u64::from(index) - file.first_piece()) as usize * piece_length;
                let end = (offset + piece_length).min(data.len());
                let blocks: Vec<_> = data[offset..end].chunks(16384).map(hash).collect();
                assert!(matches!(
                    layers.complete_piece(index, vec![]),
                    Err(V2Error::Invalid(_))
                ));
                layers.complete_piece(index, blocks.clone()).unwrap();
                assert!(matches!(
                    layers.complete_piece(canonical, blocks),
                    Err(V2Error::Invalid(_))
                ));
            }
            assert!(layers.is_complete());
            assert_eq!(layers.finish().unwrap(), expected);
            let mut corrupt = make();
            let mut failure = None;
            for index in corrupt.needed_pieces() {
                let offset = index as usize * piece_length;
                let data = &files[0].1[offset..(offset + piece_length).min(files[0].1.len())];
                let mut blocks: Vec<_> = data.chunks(16384).map(hash).collect();
                if index == 0 {
                    blocks[0][0] ^= 1;
                }
                if let Err(error) = corrupt.complete_piece(index, blocks) {
                    failure = Some(error);
                }
            }
            assert_eq!(failure, Some(V2Error::HybridHashMismatch));
            assert!(!corrupt.is_complete());
            assert_eq!(corrupt.finish(), Err(V2Error::HybridHashMismatch));
        }
    }
    let mut pure = fixture(&[("file", vec![1; 70001])], 32768);
    assert!(matches!(
        LegacyPieceLayers::from_info(&encode(get(&mut pure, b"info"))),
        Err(V2Error::Invalid(_))
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mismatched_saved_full_identity_is_rejected_without_overwriting_state() {
    use seedhop_core::Engine;
    let state = tempfile::tempdir().unwrap();
    let downloads = tempfile::tempdir().unwrap();
    let cfg = engine_config(state.path(), downloads.path());
    let engine = Engine::open(cfg.clone()).await.unwrap();
    engine
        .add(
            encode(&fixture(&[("file", vec![1; 20])], 16384)),
            cfg.download_directory.clone(),
            vec![0],
            true,
            false,
        )
        .await
        .unwrap();
    settled(&engine).await;
    engine.shutdown().await.unwrap();
    drop(engine);
    let path = state.path().join("session/session.json");
    let mut saved: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    saved["torrents"]
        .as_object_mut()
        .unwrap()
        .values_mut()
        .next()
        .unwrap()["info_hash_v2"] = "00".repeat(32).into();
    let bytes = serde_json::to_vec(&saved).unwrap();
    std::fs::write(&path, &bytes).unwrap();
    assert!(matches!(
        Engine::open(cfg).await,
        Err(CoreError::CorruptState(_))
    ));
    assert_eq!(std::fs::read(path).unwrap(), bytes);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hybrid_v1_handshake_serves_real_and_virtual_padding_blocks() {
    use librqbit_peer_protocol::{Message, Request};
    use tokio::io::AsyncReadExt;
    for (pad_last, encrypted) in [false, true]
        .into_iter()
        .flat_map(|pad| [false, true].map(|encrypted| (pad, encrypted)))
    {
        let files = vec![("a", vec![7; 3]), ("b", vec![9; 70001])];
        let mut metadata = hybrid_files(&files, 65536, pad_last);
        let meta = parse(&metadata);
        let mut wire_hash = [0; 32];
        wire_hash[..20].copy_from_slice(&sha1::Sha1::digest(encode(get(&mut metadata, b"info"))));
        let state = tempfile::tempdir().unwrap();
        let downloads = tempfile::tempdir().unwrap();
        for (path, data) in &files {
            std::fs::write(downloads.path().join(path), data).unwrap();
        }
        let mut config = engine_config(state.path(), downloads.path());
        // macOS permits an IPv4 loopback listener to overlap a reused IPv6
        // wildcard port. Keep that decoy alive to prove our fixture reaches
        // the intended IPv6 listener instead of waiting on another service.
        let decoy = if cfg!(target_os = "macos") {
            let socket = tokio::net::TcpSocket::new_v4().unwrap();
            socket.set_reuseaddr(true).unwrap();
            socket.bind("127.0.0.1:0".parse().unwrap()).unwrap();
            let decoy = socket.listen(1).unwrap();
            config.listen_port = decoy.local_addr().unwrap().port();
            Some(decoy)
        } else {
            None
        };
        let engine = seedhop_core::Engine::open_with_peer_encryption(
            config,
            if encrypted {
                librqbit::EncryptionPolicy::Require
            } else {
                librqbit::EncryptionPolicy::Allow
            },
        )
        .await
        .unwrap();
        engine
            .add(
                encode(&metadata),
                downloads.path().display().to_string(),
                vec![0, 1],
                false,
                true,
            )
            .await
            .unwrap();
        assert_eq!(settled(&engine).await.state, "seeding");
        let port = engine.snapshot().unwrap().listen_port;
        let (mut peer_read, mut peer_write): (
            Box<dyn tokio::io::AsyncRead + Unpin + Send>,
            Box<dyn tokio::io::AsyncWrite + Unpin + Send>,
        ) = if encrypted {
            let peer = tokio::net::TcpStream::connect(("::1", port)).await.unwrap();
            let (read, write) = peer.into_split();
            let mut handshake = vec![19];
            handshake.extend_from_slice(b"BitTorrent protocol");
            handshake.extend([0; 8]);
            handshake.extend_from_slice(&wire_hash[..20]);
            handshake.extend_from_slice(b"-SHtest-123456789012");
            let mut stream = tokio::time::timeout(
                std::time::Duration::from_secs(5),
                librqbit::mse::outgoing(
                    read,
                    write,
                    wire_hash[..20].try_into().unwrap(),
                    &handshake,
                    librqbit::EncryptionPolicy::Require,
                ),
            )
            .await
            .expect("MSE handshake timed out")
            .unwrap();
            let mut response = [0; 68];
            tokio::time::timeout(
                std::time::Duration::from_secs(5),
                stream.read.read_exact(&mut response),
            )
            .await
            .expect("encrypted BitTorrent handshake timed out")
            .unwrap();
            assert_eq!(&response[28..48], &wire_hash[..20]);
            loop {
                let frame = wire::frame(&mut stream.read).await;
                if matches!(
                    Message::deserialize(&frame, &[]).unwrap().0,
                    Message::Unchoke
                ) {
                    break;
                }
            }
            (Box::new(stream.read), Box::new(stream.write))
        } else {
            let (read, write) = wire::connect(port, wire_hash).await.into_split();
            (Box::new(read), Box::new(write))
        };
        wire::send(&mut peer_write, Message::Interested).await;
        let mut expected = vec![7; 3];
        expected.resize(65536, 0);
        expected.extend_from_slice(&vec![9; 70001]);
        if pad_last {
            expected.resize(196608, 0);
        }
        assert_eq!(meta.hybrid_total_bytes(), Some(expected.len() as u64));
        for (index, piece) in expected.chunks(65536).enumerate() {
            for (block, data) in piece.chunks(16384).enumerate() {
                wire::send(
                    &mut peer_write,
                    Message::Request(Request::new(
                        index as u32,
                        block as u32 * 16384,
                        data.len() as u32,
                    )),
                )
                .await;
                let frame = wire::frame(&mut peer_read).await;
                let Message::Piece(received) = Message::deserialize(&frame, &[]).unwrap().0 else {
                    panic!("expected v1 payload")
                };
                assert_eq!([received.data().0, received.data().1].concat(), data);
            }
        }
        drop(peer_read);
        drop(peer_write);
        if let Some(decoy) = decoy {
            assert!(
                tokio::time::timeout(std::time::Duration::from_millis(50), decoy.accept())
                    .await
                    .is_err(),
                "fixture connected to the IPv4 decoy"
            );
        }
        engine.shutdown().await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn inconsistent_hybrid_payload_commitments_never_become_seedable() {
    let mut metadata = hybrid();
    put(get(&mut metadata, b"info"), b"pieces", bytes([0; 20]));
    let state = tempfile::tempdir().unwrap();
    let downloads = tempfile::tempdir().unwrap();
    std::fs::write(downloads.path().join("file"), b"hello").unwrap();
    let engine = seedhop_core::Engine::open(engine_config(state.path(), downloads.path()))
        .await
        .unwrap();
    engine
        .add(
            encode(&metadata),
            downloads.path().display().to_string(),
            vec![0],
            false,
            true,
        )
        .await
        .unwrap();
    let item = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let item = engine.snapshot().unwrap().torrents.remove(0);
            if item.state == "error" {
                break item;
            }
            assert_ne!(item.state, "seeding");
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(item.downloaded_bytes, 0);
    assert_eq!(item.peers, 0);
    assert!(item.error.unwrap().contains("inconsistent"));
    assert_eq!(
        std::fs::read(downloads.path().join("file")).unwrap(),
        b"hello"
    );
    engine.shutdown().await.unwrap();
}

#[test]
fn padded_piece_prefix_preserves_data_across_every_ring_buffer_split() {
    use librqbit_peer_protocol::{Message, Piece};
    let message = Message::Piece(Piece::from_data(3, 0, b"hello\0\0\0"));
    let mut buffer = [0; 64];
    let length = message.serialize(&mut buffer, &Default::default).unwrap();
    for split in 0..=length {
        let Message::Piece(piece) = Message::deserialize(&buffer[..split], &buffer[split..length])
            .unwrap()
            .0
        else {
            panic!()
        };
        let prefix = piece.prefix(5).unwrap();
        assert_eq!(prefix.index, 3);
        assert_eq!([prefix.data().0, prefix.data().1].concat(), b"hello");
        assert!(piece.prefix(9).is_none());
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hybrid_initial_peer_can_require_the_v2_identity_without_v1_upgrade() {
    use librqbit::{AddTorrent, AddTorrentOptions, Session, SessionOptions};
    use librqbit_peer_protocol::{Message, Piece};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut metadata = hybrid();
    let v2_hash = parse(&metadata).info_hash();
    let v1_hash = sha1::Sha1::digest(encode(get(&mut metadata, b"info")));
    let files = tempfile::tempdir().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let peer_task = tokio::spawn(async move {
        let (mut peer, _) = listener.accept().await.unwrap();
        let mut handshake = [0; 68];
        peer.read_exact(&mut handshake).await.unwrap();
        assert_eq!(&handshake[28..48], v1_hash.as_slice());
        drop(peer);
        let (mut peer, _) = listener.accept().await.unwrap();
        peer.read_exact(&mut handshake).await.unwrap();
        assert_eq!(&handshake[28..48], &v2_hash[..20]);
        handshake[20..28].fill(0);
        handshake[48..].copy_from_slice(b"-SHtest-123456789012");
        peer.write_all(&handshake).await.unwrap();
        wire::send(&mut peer, Message::Bitfield((&[0x80u8][..]).into())).await;
        wire::send(&mut peer, Message::Unchoke).await;
        loop {
            let frame = wire::frame(&mut peer).await;
            if let Message::Request(request) = Message::deserialize(&frame, &[]).unwrap().0 {
                assert_eq!(request.length, 5);
                wire::send(&mut peer, Message::Piece(Piece::from_data(0, 0, b"hello"))).await;
                return;
            }
        }
    });
    let session = Session::new_with_opts(
        files.path().into(),
        SessionOptions {
            // This fixture tests plain handshake/identity retries.
            peer_encryption: librqbit::EncryptionPolicy::Disabled,
            dht: None,
            disable_trackers: true,
            disable_local_service_discovery: true,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let handle = session
        .add_torrent(
            AddTorrent::from_bytes(encode(&metadata)),
            Some(AddTorrentOptions {
                initial_peers: Some(vec![address]),
                output_folder: Some(files.path().display().to_string()),
                disable_trackers: true,
                ..Default::default()
            }),
        )
        .await
        .unwrap()
        .into_handle()
        .unwrap();
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        handle.wait_until_completed(),
    )
    .await
    .unwrap()
    .unwrap();
    peer_task.await.unwrap();
    assert_eq!(std::fs::read(files.path().join("file")).unwrap(), b"hello");
    session.stop_and_wait().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hybrid_v1_nonzero_padding_is_rejected_before_payload_is_written() {
    use librqbit::{AddTorrent, AddTorrentOptions, Session, SessionOptions};
    use librqbit_peer_protocol::{Message, Piece};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut metadata = hybrid_files(&[("a", vec![7; 3]), ("b", vec![9; 2])], 32768, false);
    let v1_hash = sha1::Sha1::digest(encode(get(&mut metadata, b"info")));
    let downloads = tempfile::tempdir().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let peer_task = tokio::spawn(async move {
        let (mut peer, _) = listener.accept().await.unwrap();
        let mut handshake = [0; 68];
        peer.read_exact(&mut handshake).await.unwrap();
        assert_eq!(&handshake[28..48], v1_hash.as_slice());
        handshake[20..28].fill(0);
        handshake[48..].copy_from_slice(b"-SHtest-123456789012");
        peer.write_all(&handshake).await.unwrap();
        wire::send(&mut peer, Message::Bitfield((&[0x80u8][..]).into())).await;
        wire::send(&mut peer, Message::Unchoke).await;
        loop {
            let frame = wire::frame(&mut peer).await;
            if let Message::Request(request) = Message::deserialize(&frame, &[]).unwrap().0 {
                assert_eq!(
                    (request.index, request.begin, request.length),
                    (0, 0, 16384)
                );
                let mut data = vec![0; 16384];
                data[..3].fill(7);
                data[3] = 1; // A byte in the virtual BEP 47 padding region.
                let mut frame = vec![0; data.len() + 13];
                let length = Message::Piece(Piece::from_data(0, 0, &data))
                    .serialize(&mut frame, &Default::default)
                    .unwrap();
                peer.write_all(&frame[..length]).await.unwrap();
                let result =
                    tokio::time::timeout(std::time::Duration::from_secs(5), peer.read(&mut [0; 1]))
                        .await
                        .unwrap();
                assert!(matches!(result, Ok(0)) || result.is_err());
                return;
            }
        }
    });
    let session = Session::new_with_opts(
        downloads.path().into(),
        SessionOptions {
            dht: None,
            disable_trackers: true,
            disable_local_service_discovery: true,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let handle = session
        .add_torrent(
            AddTorrent::from_bytes(encode(&metadata)),
            Some(AddTorrentOptions {
                initial_peers: Some(vec![address]),
                output_folder: Some(downloads.path().display().to_string()),
                disable_trackers: true,
                ..Default::default()
            }),
        )
        .await
        .unwrap()
        .into_handle()
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), peer_task)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(handle.stats().progress_bytes, 0);
    assert_eq!(
        std::fs::read(downloads.path().join("a")).unwrap(),
        vec![0; 3]
    );
    session.stop_and_wait().await.unwrap();
}

#[allow(dead_code)]
mod support;

async fn recovery_swarm(hybrid: bool) -> support::Swarm {
    support::Swarm::with_metadata(4 * 1024 * 1024, |data| {
        let mut meta = fixture(&[("payload.bin", data.to_vec())], 32768);
        if hybrid {
            let info = get(&mut meta, b"info");
            put(info, b"name", bytes(b"payload.bin"));
            put(info, b"length", B::Number(data.len() as u64));
            put(
                info,
                b"pieces",
                bytes(
                    data.chunks(32768)
                        .flat_map(sha1::Sha1::digest)
                        .collect::<Vec<_>>(),
                ),
            );
        }
        encode(&meta)
    })
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cancelling_v2_and_hybrid_moves_cleans_completed_and_partial_unselected_copies() {
    for hybrid in [false, true] {
        let state = tempfile::tempdir().unwrap();
        let downloads = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        let cfg = engine_config(state.path(), downloads.path());
        let data = vec![
            ("a-unselected", vec![2; 32768]),
            ("b-selected", vec![3; 64 * 1024 * 1024]),
        ];
        let metadata = encode(&if hybrid {
            hybrid_files(&data, 32768, false)
        } else {
            fixture(&data, 32768)
        });
        let preview = seedhop_core::adapter::preview(metadata.clone()).unwrap();
        for (file, (_, bytes)) in preview.files.iter().zip(&data) {
            let path = downloads.path().join(&file.name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, bytes).unwrap();
        }
        let engine = seedhop_core::Engine::open(cfg.clone()).await.unwrap();
        let id = engine
            .add(
                metadata,
                cfg.download_directory.clone(),
                vec![1],
                true,
                true,
            )
            .await
            .unwrap();
        settled(&engine).await;
        engine
            .set_file_priorities(id.clone(), vec![0, 2])
            .await
            .unwrap();
        let before = settled(&engine).await;
        let moving = tokio::spawn({
            let engine = engine.clone();
            let id = id.clone();
            let destination = target.path().to_string_lossy().into_owned();
            async move { engine.relocate(id, destination).await }
        });
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                if let Some(operation) = engine.snapshot().unwrap().file_operation
                    && operation.phase == "copying"
                    && operation.completed_bytes > 32768
                    && engine.cancel_file_operation(operation.id).unwrap()
                {
                    break;
                }
                assert!(
                    !moving.is_finished(),
                    "Copy completed before cancellation was observed"
                );
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(matches!(
            moving.await.unwrap(),
            Err(seedhop_core::CoreError::Cancelled)
        ));
        assert_eq!(settled(&engine).await, before);
        for (file, (_, bytes)) in preview.files.iter().zip(&data) {
            assert_eq!(
                std::fs::read(downloads.path().join(&file.name)).unwrap(),
                *bytes
            );
            assert!(!target.path().join(&file.name).exists());
        }
        engine.shutdown().await.unwrap();
        drop(engine);
        let engine = seedhop_core::Engine::open(cfg).await.unwrap();
        let restored = settled(&engine).await;
        assert_eq!(restored.id, id);
        assert_eq!(restored.destination, before.destination);
        assert_eq!(restored.state, "paused");
        assert!(restored.paused_intent);
        assert!(!restored.files[0].selected && restored.files[1].selected);
        assert_eq!(restored.files[0].priority, 0);
        assert_eq!(restored.files[1].priority, 2);
        engine.shutdown().await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn v2_and_hybrid_layout_changes_keep_identity_bytes_and_running_intent() {
    for hybrid in [false, true] {
        let swarm = recovery_swarm(hybrid).await;
        for paused in [false, true] {
            let state = tempfile::tempdir().unwrap();
            let downloads = tempfile::tempdir().unwrap();
            let target = tempfile::tempdir().unwrap();
            let cfg = engine_config(state.path(), downloads.path());
            std::fs::write(downloads.path().join("payload.bin"), &swarm.content).unwrap();
            let engine = seedhop_core::Engine::open(cfg.clone()).await.unwrap();
            let id = engine
                .add(
                    swarm.metadata.clone(),
                    cfg.download_directory.clone(),
                    vec![0],
                    paused,
                    true,
                )
                .await
                .unwrap();
            settled(&engine).await;
            engine
                .set_file_priorities(id.clone(), vec![2])
                .await
                .unwrap();
            engine
                .rename_file(id.clone(), 0, "nested/renamed.bin".into())
                .await
                .unwrap();
            settled(&engine).await;
            engine
                .relocate(id.clone(), target.path().to_string_lossy().into())
                .await
                .unwrap();
            let item = settled(&engine).await;
            assert_eq!(item.id, id);
            assert_eq!(item.state, if paused { "paused" } else { "seeding" });
            assert_eq!(item.files[0].priority, 2);
            assert!(item.files[0].selected);
            assert_eq!(
                std::fs::read(target.path().join("nested/renamed.bin")).unwrap(),
                swarm.content
            );
            assert!(!downloads.path().join("nested/renamed.bin").exists());
            engine.shutdown().await.unwrap();
            drop(engine);
            let engine = seedhop_core::Engine::open(cfg).await.unwrap();
            let item = settled(&engine).await;
            assert_eq!(item.id, id);
            assert_eq!(item.state, if paused { "paused" } else { "seeding" });
            assert_eq!(item.files[0].name, "nested/renamed.bin");
            assert_eq!(item.files[0].priority, 2);
            engine.remove(id, true).await.unwrap();
            assert!(!target.path().join("nested/renamed.bin").exists());
            engine.shutdown().await.unwrap();
        }
        swarm.seed.shutdown().await.unwrap();
    }
    let state = tempfile::tempdir().unwrap();
    let downloads = tempfile::tempdir().unwrap();
    let target = tempfile::tempdir().unwrap();
    let cfg = engine_config(state.path(), downloads.path());
    let engine = seedhop_core::Engine::open(cfg.clone()).await.unwrap();
    let a = vec![2; 65536];
    let b = vec![3; 65536];
    std::fs::write(downloads.path().join("a"), &a).unwrap();
    std::fs::write(downloads.path().join("b"), &b).unwrap();
    let id = engine
        .add(
            encode(&fixture(&[("a", a), ("b", b.clone())], 32768)),
            cfg.download_directory.clone(),
            vec![1],
            true,
            true,
        )
        .await
        .unwrap();
    settled(&engine).await;
    engine
        .set_file_priorities(id.clone(), vec![0, 2])
        .await
        .unwrap();
    engine
        .rename_file(id.clone(), 1, "new-b".into())
        .await
        .unwrap();
    settled(&engine).await;
    engine
        .relocate(id.clone(), target.path().to_string_lossy().into())
        .await
        .unwrap();
    settled(&engine).await;
    engine.shutdown().await.unwrap();
    drop(engine);
    let engine = seedhop_core::Engine::open(cfg).await.unwrap();
    let item = settled(&engine).await;
    assert!(!item.files[0].selected && item.files[1].selected);
    assert_eq!(item.files[0].priority, 0);
    assert_eq!(item.files[1].priority, 2);
    assert_eq!(item.state, "paused");
    assert_eq!(std::fs::read(target.path().join("new-b")).unwrap(), b);
    engine.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn partial_v2_and_hybrid_downloads_continue_after_relocation_and_rename() {
    for hybrid in [false, true] {
        let swarm = recovery_swarm(hybrid).await;
        let state = tempfile::tempdir().unwrap();
        let downloads = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        let cfg = engine_config(state.path(), downloads.path());
        let engine = seedhop_core::Engine::open(cfg.clone()).await.unwrap();
        engine.set_limits(256 * 1024, 0).await.unwrap();
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
        let partial = support::wait_for(&engine, |t| {
            t.downloaded_bytes >= 65536 && t.state != "checking"
        })
        .await;
        assert!(partial.downloaded_bytes < partial.total_bytes);
        engine
            .relocate(id.clone(), target.path().to_string_lossy().into())
            .await
            .unwrap();
        settled(&engine).await;
        engine
            .rename_file(id.clone(), 0, "moved.bin".into())
            .await
            .unwrap();
        settled(&engine).await;
        engine.set_limits(0, 0).await.unwrap();
        let finished = support::wait_for(&engine, |t| t.state == "seeding").await;
        assert_eq!(finished.id, id);
        assert!(finished.files[0].selected);
        assert_eq!(
            std::fs::read(target.path().join("moved.bin")).unwrap(),
            swarm.content
        );
        assert!(!downloads.path().join("payload.bin").exists());
        engine.shutdown().await.unwrap();
        swarm.seed.shutdown().await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn force_recheck_v2_and_hybrid_repair_live_corruption_and_keep_paused_selection() {
    for hybrid in [false, true] {
        let swarm = recovery_swarm(hybrid).await;
        let state = tempfile::tempdir().unwrap();
        let downloads = tempfile::tempdir().unwrap();
        let cfg = engine_config(state.path(), downloads.path());
        let path = downloads.path().join("payload.bin");
        std::fs::write(&path, &swarm.content).unwrap();
        let engine = seedhop_core::Engine::open(cfg.clone()).await.unwrap();
        let id = engine
            .add(
                swarm.metadata.clone(),
                cfg.download_directory,
                vec![0],
                true,
                true,
            )
            .await
            .unwrap();
        support::wait_for(&engine, |t| t.state == "paused").await;
        std::fs::write(&path, vec![0; swarm.content.len()]).unwrap();
        engine.force_recheck(id.clone()).await.unwrap();
        let checked = support::wait_for(&engine, |t| t.state == "paused").await;
        assert_eq!(checked.downloaded_bytes, 0);
        assert_eq!(checked.id, id);
        engine.set_paused(id.clone(), false).await.unwrap();
        support::wait_for(&engine, |t| t.state == "seeding").await;
        assert_eq!(std::fs::read(&path).unwrap(), swarm.content);
        // A live recheck must stop peers, discard the old verified map, and
        // automatically repair bytes without an extra Resume command.
        std::fs::write(&path, vec![1; swarm.content.len()]).unwrap();
        engine.force_recheck(id.clone()).await.unwrap();
        support::wait_for(&engine, |t| t.state == "seeding").await;
        assert_eq!(std::fs::read(&path).unwrap(), swarm.content);
        engine.shutdown().await.unwrap();
        swarm.seed.shutdown().await.unwrap();
    }
    // A multi-file selection remains unchanged after hashing and restart.
    let state = tempfile::tempdir().unwrap();
    let downloads = tempfile::tempdir().unwrap();
    let cfg = engine_config(state.path(), downloads.path());
    let engine = seedhop_core::Engine::open(cfg.clone()).await.unwrap();
    let id = engine
        .add(
            encode(&fixture(
                &[("a", vec![2; 65536]), ("b", vec![3; 65536])],
                32768,
            )),
            cfg.download_directory.clone(),
            vec![1],
            true,
            false,
        )
        .await
        .unwrap();
    settled(&engine).await;
    engine.force_recheck(id.clone()).await.unwrap();
    let item = settled(&engine).await;
    assert!(!item.files[0].selected && item.files[1].selected);
    assert_eq!(item.state, "paused");
    engine.shutdown().await.unwrap();
    drop(engine);
    let engine = seedhop_core::Engine::open(cfg).await.unwrap();
    let item = settled(&engine).await;
    assert_eq!(item.id, id);
    assert_eq!(item.state, "paused");
    assert!(!item.files[0].selected && item.files[1].selected);
    engine.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn v2_and_hybrid_interruption_and_corrupt_wire_data_recover() {
    use std::{sync::atomic::Ordering, time::Duration};
    if std::env::var_os("SEEDHOP_TEST_LOG").is_some() {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .with_test_writer()
            .try_init();
    }
    for hybrid in [false, true] {
        let swarm = recovery_swarm(hybrid).await;
        let state = tempfile::tempdir().unwrap();
        let downloads = tempfile::tempdir().unwrap();
        let engine = seedhop_core::Engine::open(engine_config(state.path(), downloads.path()))
            .await
            .unwrap();
        engine.set_limits(512 * 1024, 0).await.unwrap();
        swarm.corrupt_batch_blocks.store(4, Ordering::SeqCst);
        swarm.corrupt.store(true, Ordering::SeqCst);
        engine
            .add(
                swarm.metadata.clone(),
                downloads.path().to_string_lossy().into(),
                vec![0],
                false,
                false,
            )
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(15), async {
            // Two complete pieces must pass through the corrupting relay.
            while swarm.corrupt_blocks.load(Ordering::SeqCst) < 4 {
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .unwrap_or_else(|_| {
            panic!(
                "corruption gate failed: hybrid={hybrid} blocks={} snapshot={:?}",
                swarm.corrupt_blocks.load(Ordering::SeqCst),
                engine.snapshot()
            )
        });
        swarm.online.send(false).unwrap();
        let stalled = support::wait_for(&engine, |t| t.peers == 0).await;
        assert_eq!(
            stalled.downloaded_bytes, 0,
            "corrupt data must never advance verified progress"
        );
        assert_ne!(stalled.state, "seeding");
        tokio::time::timeout(Duration::from_secs(15), async {
            while swarm.offline_tracker_requests.load(Ordering::SeqCst) == 0 {
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .unwrap();
        swarm.corrupt.store(false, Ordering::SeqCst);
        swarm.online.send(true).unwrap();
        let partial = support::wait_for(&engine, |t| t.downloaded_bytes >= 65536).await;
        assert!(partial.downloaded_bytes < partial.total_bytes);
        swarm.online.send(false).unwrap();
        support::wait_for(&engine, |t| t.peers == 0).await;
        // Shutdown and restart from a genuinely partial, SHA-256 verified transfer.
        engine.shutdown().await.unwrap();
        drop(engine);
        let engine = seedhop_core::Engine::open(engine_config(state.path(), downloads.path()))
            .await
            .unwrap();
        let restored = support::wait_for(&engine, |t| t.state != "checking").await;
        assert!(restored.downloaded_bytes > 0 && restored.downloaded_bytes < restored.total_bytes);
        swarm.online.send(true).unwrap();
        support::wait_for(&engine, |t| t.state == "seeding").await;
        assert_eq!(
            std::fs::read(downloads.path().join("payload.bin")).unwrap(),
            swarm.content
        );
        engine.shutdown().await.unwrap();
        swarm.seed.shutdown().await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn v2_and_hybrid_missing_destination_preserve_state_and_recover() {
    for hybrid in [false, true] {
        let swarm = recovery_swarm(hybrid).await;
        let state = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        let drive = root.path().join("drive");
        let disconnected = root.path().join("disconnected");
        std::fs::create_dir(&drive).unwrap();
        let cfg = engine_config(state.path(), &drive);
        let engine = seedhop_core::Engine::open(cfg.clone()).await.unwrap();
        let id = engine
            .add(
                swarm.metadata.clone(),
                cfg.download_directory.clone(),
                vec![0],
                true,
                false,
            )
            .await
            .unwrap();
        support::wait_for(&engine, |t| t.state == "paused").await;
        let session_path = state.path().join("session/session.json");
        let saved = std::fs::read(&session_path).unwrap();
        std::fs::rename(&drive, &disconnected).unwrap();
        assert!(engine.force_recheck(id.clone()).await.is_err());
        assert!(engine.set_paused(id.clone(), false).await.is_err());
        assert_eq!(std::fs::read(&session_path).unwrap(), saved);
        assert!(!drive.exists());
        engine.shutdown().await.unwrap();
        drop(engine);
        let mut cfg = cfg;
        cfg.download_directory = root.path().to_string_lossy().into();
        assert!(matches!(
            seedhop_core::Engine::open(cfg.clone()).await,
            Err(CoreError::Storage(_))
        ));
        assert_eq!(std::fs::read(&session_path).unwrap(), saved);
        assert!(!drive.exists());
        std::fs::rename(&disconnected, &drive).unwrap();
        let engine = seedhop_core::Engine::open(cfg).await.unwrap();
        support::wait_for(&engine, |t| t.state == "paused").await;
        engine.set_paused(id, false).await.unwrap();
        support::wait_for(&engine, |t| t.state == "seeding").await;
        assert_eq!(
            std::fs::read(drive.join("payload.bin")).unwrap(),
            swarm.content
        );
        engine.shutdown().await.unwrap();
        swarm.seed.shutdown().await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn malicious_magnet_hash_proof_never_creates_membership_and_retry_recovers() {
    use std::{sync::atomic::Ordering, time::Duration};
    for hybrid in [false, true] {
        let swarm = recovery_swarm(hybrid).await;
        let state = tempfile::tempdir().unwrap();
        let downloads = tempfile::tempdir().unwrap();
        let engine = seedhop_core::Engine::open(engine_config(state.path(), downloads.path()))
            .await
            .unwrap();
        let meta = Metainfo::parse(&swarm.metadata).unwrap().unwrap();
        let identity: String = meta
            .info_hash()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        let tracker = swarm.announce.clone();
        let mut magnet = url::Url::parse("magnet:?").unwrap();
        magnet
            .query_pairs_mut()
            .append_pair("xt", &format!("urn:btmh:1220{identity}"))
            .append_pair("tr", &tracker);
        swarm.corrupt_proofs.store(true, Ordering::SeqCst);
        let task = {
            let engine = engine.clone();
            let magnet = magnet.to_string();
            tokio::spawn(async move {
                engine
                    .inspect("hostile-proof".into(), Some(magnet), vec![])
                    .await
            })
        };
        tokio::time::timeout(Duration::from_secs(15), async {
            while swarm.corrupted_proofs.load(Ordering::SeqCst) == 0 {
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .unwrap();
        // Give the receiver time to reject the authenticated-root mismatch.
        tokio::time::sleep(Duration::from_millis(100)).await;
        engine.cancel("hostile-proof");
        assert!(
            tokio::time::timeout(Duration::from_secs(2), task)
                .await
                .unwrap()
                .unwrap()
                .is_err()
        );
        assert!(engine.snapshot().unwrap().torrents.is_empty());
        assert_eq!(std::fs::read_dir(downloads.path()).unwrap().count(), 0);
        swarm.corrupt_proofs.store(false, Ordering::SeqCst);
        let preview = tokio::time::timeout(
            Duration::from_secs(15),
            engine.inspect("retry-proof".into(), Some(magnet.to_string()), vec![]),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(preview.id, identity);
        engine
            .add(
                preview.torrent_bytes,
                downloads.path().to_string_lossy().into(),
                vec![0],
                false,
                false,
            )
            .await
            .unwrap();
        support::wait_for(&engine, |t| t.state == "seeding").await;
        assert_eq!(
            std::fs::read(downloads.path().join("payload.bin")).unwrap(),
            swarm.content
        );
        engine.shutdown().await.unwrap();
        swarm.seed.shutdown().await.unwrap();
    }
}

#[test]
fn metadata_unknown_types_and_future_header_fields_preserve_frame_boundaries() {
    use librqbit_peer_protocol::{
        MY_EXTENDED_UT_METADATA, Message, MessageDeserializeError,
        extended::{ExtendedMessage, ut_metadata::UtMetadata},
    };
    for kind in [3, 4294967296, -1] {
        let header = format!("d8:msg_typei{kind}eeopaque future payload");
        let mut frame = ((header.len() + 2) as u32).to_be_bytes().to_vec();
        frame.extend([20, MY_EXTENDED_UT_METADATA]);
        frame.extend(header.as_bytes());
        // Include a following message; ignoring the unknown extension must
        // consume exactly its declared frame, never the next wire message.
        frame.extend([0, 0, 0, 1, 1]);
        for split in 0..=frame.len() {
            let parsed = Message::deserialize(&frame[..split], &frame[split..]);
            let (message, consumed) = match parsed {
                Err(MessageDeserializeError::NeedContiguous) => {
                    Message::deserialize(&frame, &[]).unwrap()
                }
                result => result.unwrap(),
            };
            assert!(
                matches!(message, Message::Extended(ExtendedMessage::UtMetadata(UtMetadata::Unknown(value))) if value == kind)
            );
            assert_eq!(consumed, frame.len() - 5);
            assert!(matches!(
                Message::deserialize(&frame[consumed..], &[]).unwrap().0,
                Message::Unchoke
            ));
        }
    }
    let header = format!("d6:future200:{}8:msg_typei0e5:piecei7ee", "x".repeat(200));
    let mut frame = ((header.len() + 2) as u32).to_be_bytes().to_vec();
    frame.extend([20, MY_EXTENDED_UT_METADATA]);
    frame.extend(header.as_bytes());
    assert!(matches!(
        Message::deserialize(&frame, &[]).unwrap().0,
        Message::Extended(ExtendedMessage::UtMetadata(UtMetadata::Request(7)))
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn metadata_limits_are_shared_and_keepalives_cannot_block_a_healthy_provider() {
    use librqbit::{AddTorrent, AddTorrentOptions, AddTorrentResponse, Session, SessionOptions};
    use librqbit_peer_protocol::{
        Message,
        extended::{ExtendedMessage, handshake::ExtendedHandshake, ut_metadata::UtMetadata},
    };
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    struct Active(Arc<AtomicUsize>);
    impl Drop for Active {
        fn drop(&mut self) {
            self.0.fetch_sub(1, Ordering::SeqCst);
        }
    }
    for release_early in [true, false] {
        let mut metadata = fixture(&[("file", vec![7; 123])], 32768);
        let id = parse(&metadata)
            .info_hash()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        let info = Arc::new(encode(get(&mut metadata, b"info")));
        let active = Arc::new(AtomicUsize::new(0));
        let maximum = Arc::new(AtomicUsize::new(0));
        let accepted = Arc::new(AtomicUsize::new(0));
        let healthy_requests = Arc::new(AtomicUsize::new(0));
        let (release, gate) = tokio::sync::watch::channel(false);
        let mut peers = Vec::new();
        let mut tasks = Vec::new();
        for healthy in [false, false, false, false, true] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            peers.push(listener.local_addr().unwrap());
            let info = info.clone();
            let active = active.clone();
            let maximum = maximum.clone();
            let accepted = accepted.clone();
            let healthy_requests = healthy_requests.clone();
            let gate = gate.clone();
            tasks.push(tokio::spawn(async move {
                let mut connections = tokio::task::JoinSet::new();
                loop {
                    tokio::select! {
                        connection = listener.accept() => {
                            let (mut socket, _) = connection.unwrap(); let info = info.clone(); let active = active.clone(); let maximum = maximum.clone();
                            let accepted = accepted.clone(); let healthy_requests = healthy_requests.clone(); let mut gate = gate.clone();
                            connections.spawn(async move {
                                let mut handshake = [0;68]; if socket.read_exact(&mut handshake).await.is_err() { return; }
                                accepted.fetch_add(1, Ordering::SeqCst);
                                if healthy {
                                    // Fill the dictionary budget before this provider advertises metadata.
                                    while maximum.load(Ordering::SeqCst) < 4 {
                                        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                                    }
                                }
                                handshake[48..].copy_from_slice(b"-SHtest-123456789012"); if socket.write_all(&handshake).await.is_err() { return; }
                                let mut extension = ExtendedHandshake::new(); extension.metadata_size = Some(if healthy { info.len() as u32 } else { 32*1024*1024 });
                                wire::send(&mut socket, Message::Extended(ExtendedMessage::Handshake(extension))).await;
                                if !healthy {
                                    let (mut reader, mut writer) = socket.into_split();
                                    let read_requests = async {
                                        let mut allocation = None;
                                        while let Ok(length) = reader.read_u32().await {
                                            assert!(length < 32768);
                                            let mut frame = vec![0; length as usize + 4];
                                            frame[..4].copy_from_slice(&length.to_be_bytes());
                                            if reader.read_exact(&mut frame[4..]).await.is_err() { return; }
                                            if allocation.is_none() && matches!(Message::deserialize(&frame, &[]).unwrap().0,
                                                Message::Extended(ExtendedMessage::UtMetadata(UtMetadata::Request(_)))) {
                                                allocation = Some(Active(active.clone()));
                                                let count = active.fetch_add(1, Ordering::SeqCst) + 1;
                                                maximum.fetch_max(count, Ordering::SeqCst);
                                            }
                                        }
                                    };
                                    tokio::pin!(read_requests);
                                    let mut tick = tokio::time::interval(std::time::Duration::from_millis(100));
                                    loop { tokio::select! {
                                        _ = gate.changed() => return,
                                        _ = &mut read_requests => return,
                                        _ = tick.tick() => { if writer.write_all(&[0;4]).await.is_err() { return; } },
                                    } }
                                }
                                let mut metadata_id = None;
                                let mut allocation = None;
                                while let Ok(length) = socket.read_u32().await {
                                    assert!(length < 32768); let mut frame = vec![0; length as usize + 4]; frame[..4].copy_from_slice(&length.to_be_bytes());
                                    if socket.read_exact(&mut frame[4..]).await.is_err() { return; }
                                    match Message::deserialize(&frame, &[]).unwrap().0 {
                                        Message::Extended(ExtendedMessage::Handshake(h)) => metadata_id = h.m.ut_metadata,
                                        Message::Extended(ExtendedMessage::UtMetadata(UtMetadata::Request(0))) => {
                                            if allocation.is_none() {
                                                allocation = Some(Active(active.clone()));
                                                let count = active.fetch_add(1, Ordering::SeqCst) + 1;
                                                maximum.fetch_max(count, Ordering::SeqCst);
                                            }
                                            healthy_requests.fetch_add(1, Ordering::SeqCst);
                                            let header = format!("d8:msg_typei1e5:piecei0e10:total_sizei{}ee", info.len());
                                            if socket.write_u32((2+header.len()+info.len()) as u32).await.is_err() { return; }
                                            if socket.write_all(&[20,metadata_id.unwrap()]).await.is_err() || socket.write_all(header.as_bytes()).await.is_err() || socket.write_all(&info).await.is_err() { return; }
                                        }, _ => {},
                                    }
                                }
                            });
                        },
                        _ = connections.join_next(), if !connections.is_empty() => {},
                    }
                }
            }));
        }
        let files = tempfile::tempdir().unwrap();
        let session = Session::new_with_opts(
            files.path().into(),
            SessionOptions {
                peer_encryption: librqbit::EncryptionPolicy::Disabled,
                dht: None,
                disable_local_service_discovery: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let mut resolving = tokio::task::JoinSet::new();
        for _ in 0..if release_early { 2 } else { 1 } {
            let session = session.clone();
            let id = id.clone();
            let mut initial = peers.clone();
            // Repeated caller endpoints must not acquire duplicate slots.
            initial.splice(0..0, [peers[0]; 16]);
            resolving.spawn(async move {
                session
                    .add_torrent(
                        AddTorrent::from_url(format!("magnet:?xt=urn:btmh:1220{id}")),
                        Some(AddTorrentOptions {
                            list_only: true,
                            initial_peers: Some(initial),
                            ..Default::default()
                        }),
                    )
                    .await
            });
        }
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while accepted.load(Ordering::SeqCst) < 4 {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
        assert_eq!(maximum.load(Ordering::SeqCst), 4);
        assert_eq!(healthy_requests.load(Ordering::SeqCst), 0);
        if release_early {
            release.send(true).unwrap();
        }
        tokio::time::timeout(std::time::Duration::from_secs(25), async {
            while let Some(result) = resolving.join_next().await {
                assert!(matches!(
                    result.unwrap().unwrap(),
                    AddTorrentResponse::ListOnly(_)
                ));
            }
        })
        .await
        .expect("keepalive-only providers starved the healthy candidate");
        assert_eq!(maximum.load(Ordering::SeqCst), 4);
        assert!(healthy_requests.load(Ordering::SeqCst) > 0);
        assert_eq!(std::fs::read_dir(files.path()).unwrap().count(), 0);
        session.with_torrents(|items| assert_eq!(items.count(), 0));
        session.stop_and_wait().await.unwrap();
        for task in tasks {
            task.abort();
            let _ = task.await;
        }
    }
}

// Keep raw path bytes in the fixture; generated info and payload commitments
// remain independent of the production filename mapper.
fn raw_name_fixture(names: &[Vec<u8>], hybrid: bool) -> (B, Vec<Vec<u8>>) {
    let mut names = names.to_vec();
    names.sort();
    let labels = (0..names.len())
        .map(|i| format!("temp{i:04}"))
        .collect::<Vec<_>>();
    let data = (0..names.len())
        .map(|i| vec![(i + 1) as u8; 123 + i])
        .collect::<Vec<_>>();
    let files = labels
        .iter()
        .zip(&data)
        .map(|(name, data)| (name.as_str(), data.clone()))
        .collect::<Vec<_>>();
    let mut metadata = if hybrid {
        hybrid_files(&files, 32768, true)
    } else {
        fixture(&files, 32768)
    };
    let info = get(&mut metadata, b"info");
    let B::Dict(tree) = get(info, b"file tree") else {
        panic!()
    };
    let old = std::mem::take(tree);
    *tree = names.iter().cloned().zip(old.into_values()).collect();
    if hybrid {
        let B::List(entries) = get(info, b"files") else {
            panic!()
        };
        for (entry, name) in entries.iter_mut().step_by(2).zip(names) {
            put(entry, b"path", B::List(vec![bytes(name)]));
        }
    }
    (metadata, data)
}

fn nonportable_names() -> Vec<Vec<u8>> {
    [
        b"..".to_vec(),
        b".".to_vec(),
        b"bad/name".to_vec(),
        b"bad\\name".to_vec(),
        b"bad\0name".to_vec(),
        vec![0xff],
        vec![0xc0, 0xae, 0xc0, 0xae],
        b"%FF".to_vec(),
        b"CON.txt".to_vec(),
        b"COM1".to_vec(),
        b"tail. ".to_vec(),
        b"%hfake".to_vec(),
        b"A".to_vec(),
        b"a".to_vec(),
        "é".as_bytes().to_vec(),
        "e\u{301}".as_bytes().to_vec(),
        "ß".as_bytes().to_vec(),
        b"ss".to_vec(),
        "a\u{200d}".as_bytes().to_vec(),
        "\u{200d}".as_bytes().to_vec(),
        ".\u{200d}".as_bytes().to_vec(),
        "🏠.txt".as_bytes().to_vec(),
        "\u{feff}hello".as_bytes().to_vec(),
        b"hello".to_vec(),
        vec![b'x'; 256],
        vec![b'x'; 257],
    ]
    .into_iter()
    .collect()
}

#[test]
fn sanitizes_nonportable_names_without_changing_hashes_or_piece_order() {
    for hybrid in [false, true] {
        let (mut metadata, data) = raw_name_fixture(&nonportable_names(), hybrid);
        let expected = hash(&encode(get(&mut metadata, b"info")));
        let parsed = parse(&metadata);
        assert_eq!(parsed.info_hash(), expected);
        let preview = adapter::preview(encode(&metadata)).unwrap();
        let mut names = std::collections::BTreeSet::new();
        for (index, (file, data)) in parsed.files().iter().zip(data).enumerate() {
            assert_eq!(file.path().len(), 1);
            let name = &file.path()[0];
            assert!(name.len() <= 248 && !name.is_empty() && name != "." && name != "..");
            assert!(
                !name
                    .chars()
                    .any(|c| c.is_control() || matches!(c, '/' | '\\' | ':'))
            );
            assert!(names.insert(name.clone()));
            assert_eq!(preview.files[index].name, *name);
            assert!(parsed.verify_piece(index as u64, &data).is_ok());
        }
        assert!(
            names.contains("🏠.txt"),
            "ordinary Unicode names stay readable"
        );
        assert!(
            names.contains("%FF") && names.contains("%25FF"),
            "literal escapes must remain distinct from invalid bytes"
        );
    }
}

#[test]
fn sanitized_directory_aliases_preserve_distinct_file_trees() {
    let parsed = parse(&fixture(
        &[
            ("Dir/file", vec![1]),
            ("dir/FILE", vec![2]),
            ("é/x", vec![3]),
            ("e\u{301}/y", vec![4]),
        ],
        32768,
    ));
    let roots = parsed
        .files()
        .iter()
        .map(|f| f.path()[0].clone())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(roots.len(), 4);
    assert!(roots.iter().all(|root| root.contains("%h")));
}

#[test]
fn hybrid_filename_validation_compares_raw_bytes_before_sanitizing() {
    let (mut metadata, _) = raw_name_fixture(&[b"..".to_vec()], true);
    let sanitized = parse(&metadata).files()[0].path()[0].clone();
    let B::List(entries) = get(get(&mut metadata, b"info"), b"files") else {
        panic!()
    };
    put(&mut entries[0], b"path", B::List(vec![bytes(sanitized)]));
    assert_eq!(
        Metainfo::parse(&encode(&metadata)).unwrap_err(),
        V2Error::Invalid("hybrid file order, length or alignment differs from v2")
    );
    let mut metadata = hybrid();
    let info = get(&mut metadata, b"info");
    let B::Dict(tree) = get(info, b"file tree") else {
        panic!()
    };
    let node = tree.remove(b"file".as_slice()).unwrap();
    tree.insert(vec![0xff], node);
    put(info, b"name", bytes([0xff]));
    assert!(adapter::preview(encode(&metadata)).is_ok());
    put(get(&mut metadata, b"info"), b"name", bytes(b"%FF"));
    assert_eq!(
        Metainfo::parse(&encode(&metadata)).unwrap_err(),
        V2Error::Invalid("hybrid single-file layout differs from v2")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sanitized_names_create_distinct_files_and_survive_recheck_and_restart() {
    use std::io::Write;
    for hybrid in [false, true] {
        let (metadata, data) = raw_name_fixture(&nonportable_names(), hybrid);
        let bytes = encode(&metadata);
        let preview = adapter::preview(bytes.clone()).unwrap();
        let state = tempfile::tempdir().unwrap();
        let downloads = tempfile::tempdir().unwrap();
        let config = engine_config(state.path(), downloads.path());
        let engine = seedhop_core::Engine::open(config.clone()).await.unwrap();
        let id = engine
            .add(
                bytes,
                config.download_directory.clone(),
                (0..data.len() as u32).collect(),
                true,
                false,
            )
            .await
            .unwrap();
        support::wait_for(&engine, |t| t.state == "paused").await;
        for (file, data) in preview.files.iter().zip(&data) {
            // Existing zero-length files are created by paused initialization;
            // distinct filesystem identity is checked before writing payload.
            assert_eq!(
                std::fs::metadata(downloads.path().join(&file.name))
                    .unwrap()
                    .len(),
                data.len() as u64
            );
        }
        use std::os::unix::fs::MetadataExt;
        let inodes = preview
            .files
            .iter()
            .map(|f| {
                std::fs::metadata(downloads.path().join(&f.name))
                    .unwrap()
                    .ino()
            })
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            inodes.len(),
            data.len(),
            "filesystem normalization aliased storage paths"
        );
        for (file, data) in preview.files.iter().zip(&data) {
            std::fs::OpenOptions::new()
                .write(true)
                .open(downloads.path().join(&file.name))
                .unwrap()
                .write_all(data)
                .unwrap();
        }
        engine.force_recheck(id.clone()).await.unwrap();
        let checked = support::wait_for(&engine, |t| t.state == "paused").await;
        assert_eq!(checked.downloaded_bytes, preview.total_bytes);
        engine.shutdown().await.unwrap();
        drop(engine);
        let engine = seedhop_core::Engine::open(config).await.unwrap();
        let restored = support::wait_for(&engine, |t| t.state == "paused").await;
        assert_eq!(restored.id, id);
        assert_eq!(restored.downloaded_bytes, preview.total_bytes);
        assert_eq!(
            restored
                .files
                .iter()
                .map(|f| f.name.clone())
                .collect::<Vec<_>>(),
            preview
                .files
                .iter()
                .map(|f| f.name.clone())
                .collect::<Vec<_>>()
        );
        for (file, data) in preview.files.iter().zip(&data) {
            assert_eq!(
                std::fs::read(downloads.path().join(&file.name)).unwrap(),
                *data
            );
        }
        engine.remove(id, false).await.unwrap();
        engine.shutdown().await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn legacy_v2_and_hybrid_sessions_keep_existing_names_and_migrate_once() {
    use std::io::Write;
    for hybrid in [false, true] {
        let mut raw = vec![
            b"100%.bin".to_vec(),
            b"CON.txt".to_vec(),
            "é.txt".as_bytes().to_vec(),
            b"plain".to_vec(),
            b"renamed".to_vec(),
        ];
        raw.sort();
        let (metadata, data) = raw_name_fixture(&raw, hybrid);
        let bytes = encode(&metadata);
        let mut historical = librqbit::torrent_from_bytes(&bytes).unwrap();
        historical.info.data.v2 = None;
        let encoding = historical.info.data.detect_encoding();
        let mut old = raw
            .iter()
            .map(|part| encoding.decode(part).0.into_owned())
            .collect::<Vec<_>>();
        let preview = adapter::preview(bytes.clone()).unwrap();
        let state = tempfile::tempdir().unwrap();
        let downloads = tempfile::tempdir().unwrap();
        let config = engine_config(state.path(), downloads.path());
        let engine = seedhop_core::Engine::open(config.clone()).await.unwrap();
        let id = engine
            .add(
                bytes,
                config.download_directory.clone(),
                (0..data.len() as u32).collect(),
                true,
                false,
            )
            .await
            .unwrap();
        support::wait_for(&engine, |t| t.state == "paused").await;
        for (file, data) in preview.files.iter().zip(&data) {
            std::fs::OpenOptions::new()
                .write(true)
                .open(downloads.path().join(&file.name))
                .unwrap()
                .write_all(data)
                .unwrap();
        }
        let renamed = raw.iter().position(|name| name == b"renamed").unwrap();
        engine
            .rename_file(id.clone(), renamed as u32, "kept.bin".into())
            .await
            .unwrap();
        old[renamed] = "kept.bin".into();
        engine.force_recheck(id.clone()).await.unwrap();
        support::wait_for(&engine, |t| t.state == "paused").await;
        engine.shutdown().await.unwrap();
        drop(engine);
        for (index, file) in preview.files.iter().enumerate() {
            if index != renamed && file.name != old[index] {
                std::fs::rename(
                    downloads.path().join(&file.name),
                    downloads.path().join(&old[index]),
                )
                .unwrap();
            }
        }
        let session_path = state.path().join("session/session.json");
        let mut saved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&session_path).unwrap()).unwrap();
        let item = saved["torrents"]
            .as_object_mut()
            .unwrap()
            .values_mut()
            .next()
            .unwrap()
            .as_object_mut()
            .unwrap();
        item.remove("file_name_mapping_version");
        item.insert(
            "file_names".into(),
            serde_json::json!({renamed.to_string():"kept.bin"}),
        );
        std::fs::write(&session_path, serde_json::to_vec(&saved).unwrap()).unwrap();
        let engine = seedhop_core::Engine::open(config.clone()).await.unwrap();
        let restored = support::wait_for(&engine, |t| t.state == "paused").await;
        assert_eq!(
            restored
                .files
                .iter()
                .map(|f| f.name.clone())
                .collect::<Vec<_>>(),
            old
        );
        // A journaled rename must first materialize the legacy map so either
        // side of an interrupted transaction has a recognizable saved layout.
        engine
            .rename_file(id.clone(), renamed as u32, "kept-again.bin".into())
            .await
            .unwrap();
        old[renamed] = "kept-again.bin".into();
        support::wait_for(&engine, |t| t.state == "paused").await;
        engine.force_recheck(id.clone()).await.unwrap();
        let checked = support::wait_for(&engine, |t| t.state == "paused").await;
        assert_eq!(checked.downloaded_bytes, preview.total_bytes);
        engine.shutdown().await.unwrap();
        drop(engine);
        let saved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&session_path).unwrap()).unwrap();
        let item = saved["torrents"]
            .as_object()
            .unwrap()
            .values()
            .next()
            .unwrap();
        assert_eq!(item["file_name_mapping_version"], 1);
        assert_eq!(item["file_names"].as_object().unwrap().len(), data.len());
        let engine = seedhop_core::Engine::open(config).await.unwrap();
        let restored = support::wait_for(&engine, |t| t.state == "paused").await;
        assert_eq!(restored.downloaded_bytes, preview.total_bytes);
        assert_eq!(
            restored
                .files
                .iter()
                .map(|f| f.name.clone())
                .collect::<Vec<_>>(),
            old
        );
        for (name, data) in old.iter().zip(data) {
            assert_eq!(std::fs::read(downloads.path().join(name)).unwrap(), data);
        }
        assert_eq!(
            std::fs::read_dir(downloads.path()).unwrap().count(),
            old.len(),
            "migration created replacement files"
        );
        engine.remove(id, false).await.unwrap();
        engine.shutdown().await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn versioned_filename_maps_reject_incomplete_and_future_versions_before_io() {
    let state = tempfile::tempdir().unwrap();
    let downloads = tempfile::tempdir().unwrap();
    let config = engine_config(state.path(), downloads.path());
    let engine = seedhop_core::Engine::open(config.clone()).await.unwrap();
    engine
        .add(
            encode(&fixture(&[("100%.bin", vec![7])], 32768)),
            config.download_directory.clone(),
            vec![0],
            true,
            false,
        )
        .await
        .unwrap();
    support::wait_for(&engine, |t| t.state == "paused").await;
    engine.shutdown().await.unwrap();
    drop(engine);
    let path = state.path().join("session/session.json");
    let saved: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    for mode in 0..3 {
        let mut invalid = saved.clone();
        let item = invalid["torrents"]
            .as_object_mut()
            .unwrap()
            .values_mut()
            .next()
            .unwrap();
        match mode {
            0 => item["file_names"] = serde_json::json!({}),
            1 => item["file_name_mapping_version"] = serde_json::json!(2),
            _ => item["file_name_mapping_version"] = serde_json::json!("1"),
        }
        let bytes = serde_json::to_vec(&invalid).unwrap();
        std::fs::write(&path, &bytes).unwrap();
        assert!(matches!(
            seedhop_core::Engine::open(config.clone()).await,
            Err(CoreError::CorruptState(_))
        ));
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        assert_eq!(std::fs::read_dir(downloads.path()).unwrap().count(), 1);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn v2_advisory_names_cannot_reject_valid_files_or_escape_default_output() {
    use librqbit::{AddTorrent, AddTorrentOptions, AddTorrentResponse, Session, SessionOptions};
    let files = tempfile::tempdir().unwrap();
    let session = Session::new_with_opts(
        files.path().into(),
        SessionOptions {
            dht: None,
            disable_local_service_discovery: true,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    for name in [
        b"..".to_vec(),
        b"/escape".to_vec(),
        b"a/b".to_vec(),
        b"CON.txt".to_vec(),
        vec![0xff],
        vec![b'x'; 1000],
    ] {
        let mut metadata = fixture(&[("a", vec![1]), ("b", vec![2])], 32768);
        put(get(&mut metadata, b"info"), b"name", bytes(name));
        let expected = parse(&metadata).info_hash();
        let response = session
            .add_torrent(
                AddTorrent::from_bytes(encode(&metadata)),
                Some(AddTorrentOptions {
                    list_only: true,
                    ..Default::default()
                }),
            )
            .await
            .unwrap();
        let AddTorrentResponse::ListOnly(list) = response else {
            panic!()
        };
        assert_eq!(
            adapter::preview(list.torrent_bytes.to_vec()).unwrap().id,
            expected
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        );
        assert_eq!(list.output_folder.parent(), Some(files.path()));
        assert!(list.output_folder.file_name().unwrap().len() <= 248);
    }
    assert_eq!(std::fs::read_dir(files.path()).unwrap().count(), 0);
    session.stop_and_wait().await.unwrap();
}

async fn peer_interest(
    peer: &mut tokio::net::TcpStream,
    expected: bool,
) -> Vec<librqbit_peer_protocol::Request> {
    use librqbit_peer_protocol::Message;
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        let mut cancelled = Vec::new();
        loop {
            let frame = wire::frame(peer).await;
            match Message::deserialize(&frame, &[]).unwrap().0 {
                Message::Interested => {
                    assert!(
                        expected,
                        "peer remained interested in unavailable/unselected pieces"
                    );
                    return cancelled;
                }
                Message::NotInterested => {
                    assert!(
                        !expected,
                        "peer lost interest despite useful advertised pieces"
                    );
                    return cancelled;
                }
                Message::Cancel(request) => cancelled.push(request),
                Message::Request(request) => {
                    panic!("payload request {request:?} arrived while peer was choking")
                }
                Message::Have(_) | Message::KeepAlive => {}
                message => panic!("unexpected state message {message:?}"),
            }
        }
    })
    .await
    .expect("interest did not follow useful pieces while choked")
}

async fn peer_request(
    peer: &mut tokio::net::TcpStream,
    piece: u32,
) -> librqbit_peer_protocol::Request {
    use librqbit_peer_protocol::Message;
    loop {
        let frame = wire::frame(peer).await;
        match Message::deserialize(&frame, &[]).unwrap().0 {
            Message::Request(request) => {
                assert_eq!(request.index, piece);
                return request;
            }
            Message::Have(_) | Message::KeepAlive => {}
            message => panic!("unexpected payload-phase message {message:?}"),
        }
    }
}

async fn peer_payload(
    peer: &mut tokio::net::TcpStream,
    request: librqbit_peer_protocol::Request,
    data: &[u8],
) {
    use tokio::io::AsyncWriteExt;
    let block = &data[request.begin as usize..(request.begin + request.length) as usize];
    peer.write_u32(9 + request.length).await.unwrap();
    peer.write_all(&[7]).await.unwrap();
    peer.write_u32(request.index).await.unwrap();
    peer.write_u32(request.begin).await.unwrap();
    peer.write_all(block).await.unwrap();
    peer.flush().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn choked_peer_interest_tracks_selection_and_cancelled_responses_remain_correlated() {
    use librqbit_peer_protocol::Message;
    use tokio::io::AsyncReadExt;
    for (hybrid, legacy) in [(false, false), (true, false), (true, true)] {
        let data = [vec![7; 32768], vec![8; 32768], vec![9; 32768]];
        let files = [
            ("a", data[0].clone()),
            ("b", data[1].clone()),
            ("c", data[2].clone()),
        ];
        let mut metadata = if hybrid {
            hybrid_files(&files, 32768, false)
        } else {
            fixture(&files, 32768)
        };
        let mut hash = parse(&metadata).info_hash();
        if legacy {
            hash[..20].copy_from_slice(&sha1::Sha1::digest(encode(get(&mut metadata, b"info"))));
        }
        let state = tempfile::tempdir().unwrap();
        let downloads = tempfile::tempdir().unwrap();
        let engine = seedhop_core::Engine::open_with_peer_encryption(
            engine_config(state.path(), downloads.path()),
            librqbit::EncryptionPolicy::Disabled,
        )
        .await
        .unwrap();
        let id = engine
            .add(
                encode(&metadata),
                downloads.path().display().to_string(),
                vec![0],
                false,
                false,
            )
            .await
            .unwrap();
        support::wait_for(&engine, |t| t.state != "checking").await;
        let mut peer = wire::connect(engine.snapshot().unwrap().listen_port, hash).await;
        wire::send(&mut peer, Message::Bitfield((&[0x40u8][..]).into())).await;
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(350), peer.read_u8())
                .await
                .is_err(),
            "unhelpful choked peer received interest or requests"
        );
        wire::send(&mut peer, Message::Have(0)).await;
        peer_interest(&mut peer, true).await;
        engine.select_files(id.clone(), vec![2]).await.unwrap();
        peer_interest(&mut peer, false).await;
        engine.select_files(id.clone(), vec![0]).await.unwrap();
        peer_interest(&mut peer, true).await;
        wire::send(&mut peer, Message::Unchoke).await;
        let first = peer_request(&mut peer, 0).await;
        let second = peer_request(&mut peer, 0).await;
        wire::send(&mut peer, Message::Choke).await;
        engine.select_files(id.clone(), vec![2]).await.unwrap();
        let cancelled = peer_interest(&mut peer, false).await;
        assert_eq!(cancelled.len(), 2);
        let cancelled = cancelled
            .iter()
            .map(|r| (r.index, r.begin, r.length))
            .collect::<std::collections::BTreeSet<_>>();
        assert!(
            cancelled.contains(&(first.index, first.begin, first.length))
                && cancelled.contains(&(second.index, second.begin, second.length))
        );
        peer_payload(&mut peer, first, &data[0]).await;
        if legacy {
            peer_payload(&mut peer, second, &data[0]).await;
        } else {
            wire::send(&mut peer, Message::Reject(second)).await;
        }
        wire::send(&mut peer, Message::Have(2)).await;
        peer_interest(&mut peer, true).await;
        wire::send(&mut peer, Message::Unchoke).await;
        for _ in 0..2 {
            let request = peer_request(&mut peer, 2).await;
            peer_payload(&mut peer, request, &data[2]).await;
        }
        support::wait_for(&engine, |t| t.state == "seeding").await;
        assert_eq!(std::fs::read(downloads.path().join("c")).unwrap(), data[2]);
        assert_ne!(
            std::fs::read(downloads.path().join("a")).unwrap(),
            data[0],
            "late cancelled payload was written"
        );
        drop(peer);
        engine.shutdown().await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn choked_peer_loses_interest_when_another_peer_completes_its_only_useful_piece() {
    use librqbit_peer_protocol::Message;
    for hybrid in [false, true] {
        let data = [vec![7; 32768], vec![8; 32768], vec![9; 32768]];
        let files = [
            ("a", data[0].clone()),
            ("b", data[1].clone()),
            ("c", data[2].clone()),
        ];
        let metadata = if hybrid {
            hybrid_files(&files, 32768, false)
        } else {
            fixture(&files, 32768)
        };
        let hash = parse(&metadata).info_hash();
        let state = tempfile::tempdir().unwrap();
        let downloads = tempfile::tempdir().unwrap();
        let engine = seedhop_core::Engine::open_with_peer_encryption(
            engine_config(state.path(), downloads.path()),
            librqbit::EncryptionPolicy::Disabled,
        )
        .await
        .unwrap();
        engine
            .add(
                encode(&metadata),
                downloads.path().display().to_string(),
                vec![0, 2],
                false,
                false,
            )
            .await
            .unwrap();
        support::wait_for(&engine, |t| t.state != "checking").await;
        let port = engine.snapshot().unwrap().listen_port;
        let mut choked = wire::connect(port, hash).await;
        wire::send(&mut choked, Message::Bitfield((&[0x80u8][..]).into())).await;
        peer_interest(&mut choked, true).await;
        let mut source = wire::connect(port, hash).await;
        wire::send(&mut source, Message::Bitfield((&[0x80u8][..]).into())).await;
        peer_interest(&mut source, true).await;
        wire::send(&mut source, Message::Unchoke).await;
        for _ in 0..2 {
            let request = peer_request(&mut source, 0).await;
            peer_payload(&mut source, request, &data[0]).await;
        }
        peer_interest(&mut choked, false).await;
        wire::send(&mut choked, Message::Have(2)).await;
        peer_interest(&mut choked, true).await;
        wire::send(&mut choked, Message::Unchoke).await;
        for _ in 0..2 {
            let request = peer_request(&mut choked, 2).await;
            peer_payload(&mut choked, request, &data[2]).await;
        }
        support::wait_for(&engine, |t| t.state == "seeding").await;
        assert_eq!(std::fs::read(downloads.path().join("a")).unwrap(), data[0]);
        assert_eq!(std::fs::read(downloads.path().join("c")).unwrap(), data[2]);
        drop(choked);
        drop(source);
        engine.shutdown().await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn choked_interest_follows_unselected_stream_start_seek_and_drop() {
    use librqbit::{
        AddTorrent, AddTorrentOptions, AddTorrentResponse, ListenerOptions, Session,
        SessionOptions, TorrentStatsState,
    };
    use librqbit_peer_protocol::Message;
    use tokio::io::{AsyncReadExt, AsyncSeekExt};
    let data = [vec![7; 32768], vec![8; 131072], vec![9; 32768]];
    let metadata = fixture(
        &[
            ("a", data[0].clone()),
            ("b", data[1].clone()),
            ("c", data[2].clone()),
        ],
        32768,
    );
    let hash = parse(&metadata).info_hash();
    let files = tempfile::tempdir().unwrap();
    for (name, data) in ["a", "b", "c"].iter().zip(&data) {
        std::fs::write(files.path().join(name), vec![0; data.len()]).unwrap();
    }
    let session = Session::new_with_opts(
        files.path().into(),
        SessionOptions {
            dht: None,
            disable_local_service_discovery: true,
            peer_encryption: librqbit::EncryptionPolicy::Disabled,
            listen: Some(ListenerOptions {
                listen_addr: "127.0.0.1:0".parse().unwrap(),
                ..Default::default()
            }),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let response = session
        .add_torrent(
            AddTorrent::from_bytes(encode(&metadata)),
            Some(AddTorrentOptions {
                only_files: Some(vec![0]),
                output_folder: Some(files.path().display().to_string()),
                overwrite: true,
                ..Default::default()
            }),
        )
        .await
        .unwrap();
    let AddTorrentResponse::Added(_, torrent) = response else {
        panic!()
    };
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while !matches!(torrent.stats().state, TorrentStatsState::Live) {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    let mut peer = wire::connect(session.listen_addr().unwrap().port(), hash).await;
    wire::send(&mut peer, Message::Bitfield((&[0x40u8][..]).into())).await;
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(350), peer.read_u8())
            .await
            .is_err()
    );
    let mut stream = torrent.clone().stream(1).await.unwrap();
    peer_interest(&mut peer, true).await;
    stream.seek(std::io::SeekFrom::Start(65536)).await.unwrap();
    peer_interest(&mut peer, false).await;
    stream.seek(std::io::SeekFrom::Start(0)).await.unwrap();
    peer_interest(&mut peer, true).await;
    drop(stream);
    peer_interest(&mut peer, false).await;
    let mut stream = torrent.clone().stream(1).await.unwrap();
    peer_interest(&mut peer, true).await;
    wire::send(&mut peer, Message::Unchoke).await;
    for _ in 0..2 {
        let request = peer_request(&mut peer, 1).await;
        peer_payload(&mut peer, request, &data[1][..32768]).await;
    }
    peer_interest(&mut peer, false).await;
    let mut received = vec![0; 32768];
    tokio::time::timeout(
        std::time::Duration::from_secs(3),
        stream.read_exact(&mut received),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(received, data[1][..32768]);
    assert_eq!(torrent.only_files(), Some(vec![0]));
    drop(stream);
    drop(peer);
    session.stop_and_wait().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn hybrid_choke_rules_preserve_v2_reject_duty_and_restart_legacy_requests() {
    use librqbit_peer_protocol::{
        Message,
        extended::{ExtendedMessage, handshake::ExtendedHandshake},
    };
    use tokio::io::AsyncReadExt;
    for legacy in [false, true] {
        let data = vec![7; 32768];
        let mut metadata =
            hybrid_files(&[("a", data.clone()), ("b", vec![8; 32768])], 32768, false);
        let mut hash = parse(&metadata).info_hash();
        if legacy {
            hash[..20].copy_from_slice(&sha1::Sha1::digest(encode(get(&mut metadata, b"info"))));
        }
        let state = tempfile::tempdir().unwrap();
        let files = tempfile::tempdir().unwrap();
        let engine = seedhop_core::Engine::open_with_peer_encryption(
            engine_config(state.path(), files.path()),
            librqbit::EncryptionPolicy::Disabled,
        )
        .await
        .unwrap();
        engine
            .add(
                encode(&metadata),
                files.path().display().to_string(),
                vec![0],
                false,
                false,
            )
            .await
            .unwrap();
        support::wait_for(&engine, |t| t.state != "checking").await;
        let mut peer = wire::connect(engine.snapshot().unwrap().listen_port, hash).await;
        let mut extension = ExtendedHandshake::new();
        extension.reqq = Some(1);
        wire::send(
            &mut peer,
            Message::Extended(ExtendedMessage::Handshake(extension)),
        )
        .await;
        wire::send(&mut peer, Message::Bitfield((&[0x80u8][..]).into())).await;
        peer_interest(&mut peer, true).await;
        wire::send(&mut peer, Message::Unchoke).await;
        let first = peer_request(&mut peer, 0).await;
        wire::send(&mut peer, Message::Choke).await;
        wire::send(&mut peer, Message::Unchoke).await;
        if !legacy {
            assert!(
                tokio::time::timeout(std::time::Duration::from_millis(350), peer.read_u8())
                    .await
                    .is_err(),
                "v2 choke implicitly retired a pending request"
            );
            wire::send(&mut peer, Message::Reject(first)).await;
            peer_interest(&mut peer, false).await;
            wire::send(&mut peer, Message::Have(0)).await;
            peer_interest(&mut peer, true).await;
        }
        let request = tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                let frame = wire::frame(&mut peer).await;
                match Message::deserialize(&frame, &[]).unwrap().0 {
                    Message::Request(request) => break request,
                    Message::Cancel(cancelled) => assert_eq!(
                        (cancelled.index, cancelled.begin, cancelled.length),
                        (first.index, first.begin, first.length)
                    ),
                    message => panic!("unexpected post-choke message {message:?}"),
                }
            }
        })
        .await
        .expect("legacy request window stayed full after an implicit choke cancellation");
        assert_eq!(
            (request.index, request.begin, request.length),
            (first.index, first.begin, first.length)
        );
        peer_payload(&mut peer, request, &data).await;
        let request = peer_request(&mut peer, 0).await;
        assert_eq!(request.begin, 16384);
        peer_payload(&mut peer, request, &data).await;
        support::wait_for(&engine, |t| t.state == "seeding").await;
        assert_eq!(std::fs::read(files.path().join("a")).unwrap(), data);
        drop(peer);
        engine.shutdown().await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn metadata_serving_rejects_out_of_range_indices_and_keeps_connection() {
    use librqbit_peer_protocol::{
        Message,
        extended::{
            ExtendedMessage, PeerExtendedMessageIds, handshake::ExtendedHandshake,
            ut_metadata::UtMetadata,
        },
    };
    use tokio::io::AsyncWriteExt;
    for hybrid in [false, true] {
        let state = tempfile::tempdir().unwrap();
        let files = tempfile::tempdir().unwrap();
        let mut metadata = if hybrid {
            self::hybrid()
        } else {
            fixture(&[("file", b"hello".to_vec())], 16384)
        };
        put(
            get(&mut metadata, b"info"),
            b"extension",
            bytes(vec![42; 20000]),
        );
        let hash = parse(&metadata).info_hash();
        let info = encode(get(&mut metadata, b"info"));
        let count = info.len().div_ceil(16384) as u32;
        assert_eq!(count, 2);
        let engine = seedhop_core::Engine::open(engine_config(state.path(), files.path()))
            .await
            .unwrap();
        engine
            .add(
                encode(&metadata),
                files.path().display().to_string(),
                vec![0],
                false,
                true,
            )
            .await
            .unwrap();
        settled(&engine).await;
        let mut peer = wire::connect(engine.snapshot().unwrap().listen_port, hash).await;
        wire::send(
            &mut peer,
            Message::Extended(ExtendedMessage::Handshake(ExtendedHandshake::new())),
        )
        .await;
        for index in [count, count + 1, u32::MAX, 0, 1] {
            let mut buffer = [0; 1024];
            let len = Message::Extended(ExtendedMessage::UtMetadata(UtMetadata::Request(index)))
                .serialize(&mut buffer, &PeerExtendedMessageIds::my)
                .unwrap();
            peer.write_all(&buffer[..len]).await.unwrap();
            loop {
                let response = wire::frame(&mut peer).await;
                match Message::deserialize(&response, &[]).unwrap().0 {
                    Message::Extended(ExtendedMessage::UtMetadata(UtMetadata::Reject(actual))) => {
                        assert!(index >= count);
                        assert_eq!(actual, index);
                        break;
                    }
                    Message::Extended(ExtendedMessage::UtMetadata(UtMetadata::Data(data))) => {
                        assert!(index < count);
                        assert_eq!(data.piece(), index);
                        assert_eq!(data.total_size() as usize, info.len());
                        let start = index as usize * 16384;
                        let expected = &info[start..(start + 16384).min(info.len())];
                        let mut actual = vec![0; data.len()];
                        data.copy_to_slice(&mut actual);
                        assert_eq!(actual, expected);
                        break;
                    }
                    Message::Extended(_) | Message::Interested | Message::NotInterested => {}
                    response => panic!("unexpected metadata response {response:?}"),
                }
            }
        }
        engine.shutdown().await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hash_serving_uses_only_verified_payload_and_accepts_another_identical_copy() {
    use librqbit_peer_protocol::{HashRequest, Message};
    let data: Vec<_> = (0..65536).map(|i| (i % 251) as u8).collect();
    for hybrid in [false, true] {
        for available in [false, true] {
            let state = tempfile::tempdir().unwrap();
            let files = tempfile::tempdir().unwrap();
            // First copy contains unverified zeros. Second is corrupt or verified.
            std::fs::write(files.path().join("a"), vec![0; data.len()]).unwrap();
            std::fs::write(
                files.path().join("b"),
                if available {
                    data.clone()
                } else {
                    vec![0; data.len()]
                },
            )
            .unwrap();
            let entries = [("a", data.clone()), ("b", data.clone())];
            let metadata = if hybrid {
                hybrid_files(&entries, 32768, false)
            } else {
                fixture(&entries, 32768)
            };
            let meta = parse(&metadata);
            let root = meta.files()[0].pieces_root().unwrap();
            let engine = seedhop_core::Engine::open(engine_config(state.path(), files.path()))
                .await
                .unwrap();
            engine
                .add(
                    encode(&metadata),
                    files.path().display().to_string(),
                    vec![1],
                    false,
                    true,
                )
                .await
                .unwrap();
            settled(&engine).await;
            let mut peer =
                wire::connect(engine.snapshot().unwrap().listen_port, meta.info_hash()).await;
            for base_layer in [0, 1] {
                let request = HashRequest {
                    pieces_root: root,
                    base_layer,
                    index: 0,
                    length: 2,
                    proof_layers: 1 - base_layer,
                };
                wire::send(&mut peer, Message::HashRequest(request)).await;
                loop {
                    let response = wire::frame(&mut peer).await;
                    match Message::deserialize(&response, &[]).unwrap().0 {
                        Message::Hashes {
                            request: actual,
                            hashes,
                        } => {
                            assert!(
                                available || base_layer == 1,
                                "served unverified leaf hashes"
                            );
                            assert_eq!(actual, request);
                            meta.verify_hashes(&request, &hashes).unwrap();
                            break;
                        }
                        Message::HashReject(actual) => {
                            assert!(!available && base_layer == 0);
                            assert_eq!(actual, request);
                            break;
                        }
                        Message::Interested | Message::NotInterested => {}
                        other => panic!("unexpected hash reply: {other:?}"),
                    }
                }
            }
            if available {
                // Serving payload never consumes the availability needed for
                // the mandatory subsequent leaf proof.
                wire::send(
                    &mut peer,
                    Message::Request(librqbit_peer_protocol::Request {
                        index: 2,
                        begin: 0,
                        length: 16384,
                    }),
                )
                .await;
                loop {
                    let response = wire::frame(&mut peer).await;
                    if let Message::Piece(piece) = Message::deserialize(&response, &[]).unwrap().0 {
                        assert_eq!(piece.data().0, &data[..16384]);
                        break;
                    }
                }
                let request = HashRequest {
                    pieces_root: root,
                    base_layer: 0,
                    index: 0,
                    length: 2,
                    proof_layers: 1,
                };
                wire::send(&mut peer, Message::HashRequest(request)).await;
                let response = wire::frame(&mut peer).await;
                match Message::deserialize(&response, &[]).unwrap().0 {
                    Message::Hashes {
                        request: actual,
                        hashes,
                    } => {
                        assert_eq!(actual, request);
                        meta.verify_hashes(&request, &hashes).unwrap();
                    }
                    other => panic!("missing post-upload proof: {other:?}"),
                }
            }
            engine.shutdown().await.unwrap();
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn metadata_reply_backpressure_preserves_pipelined_requests() {
    use librqbit_peer_protocol::{
        Message,
        extended::{
            ExtendedMessage, PeerExtendedMessageIds, handshake::ExtendedHandshake,
            ut_metadata::UtMetadata,
        },
    };
    use tokio::io::AsyncWriteExt;
    let state = tempfile::tempdir().unwrap();
    let files = tempfile::tempdir().unwrap();
    let mut metadata = fixture(&[("file", b"hello".to_vec())], 16384);
    put(
        get(&mut metadata, b"info"),
        b"extension",
        bytes(vec![23; 20000]),
    );
    let meta = parse(&metadata);
    let info = encode(get(&mut metadata, b"info"));
    let engine = seedhop_core::Engine::open(engine_config(state.path(), files.path()))
        .await
        .unwrap();
    engine
        .add(
            encode(&metadata),
            files.path().display().to_string(),
            vec![0],
            false,
            true,
        )
        .await
        .unwrap();
    settled(&engine).await;
    let mut peer = wire::connect(engine.snapshot().unwrap().listen_port, meta.info_hash()).await;
    wire::send(
        &mut peer,
        Message::Extended(ExtendedMessage::Handshake(ExtendedHandshake::new())),
    )
    .await;
    // Four MiB of replies exceed the socket buffer and the 32-response budget.
    // Delay reading until every tiny request has been sent.
    let mut requests = Vec::new();
    for _ in 0..256 {
        let mut buffer = [0; 128];
        let len = Message::Extended(ExtendedMessage::UtMetadata(UtMetadata::Request(0)))
            .serialize(&mut buffer, &PeerExtendedMessageIds::my)
            .unwrap();
        requests.extend_from_slice(&buffer[..len]);
    }
    peer.write_all(&requests).await.unwrap();
    // Another peer must remain usable while this peer is under backpressure.
    let mut healthy = wire::connect(engine.snapshot().unwrap().listen_port, meta.info_hash()).await;
    wire::send(
        &mut healthy,
        Message::Request(librqbit_peer_protocol::Request {
            index: 99,
            begin: 0,
            length: 5,
        }),
    )
    .await;
    loop {
        let response = wire::frame(&mut healthy).await;
        if let Message::Reject(request) = Message::deserialize(&response, &[]).unwrap().0 {
            assert_eq!(request.index, 99);
            break;
        }
    }
    let mut received = 0;
    while received < 256 {
        let response = wire::frame(&mut peer).await;
        if let Message::Extended(ExtendedMessage::UtMetadata(UtMetadata::Data(data))) =
            Message::deserialize(&response, &[]).unwrap().0
        {
            assert_eq!(data.piece(), 0);
            assert_eq!(data.total_size() as usize, info.len());
            let mut actual = vec![0; data.len()];
            data.copy_to_slice(&mut actual);
            assert_eq!(actual, info[..16384]);
            received += 1;
        }
    }
    engine.shutdown().await.unwrap();
}

#[path = "support/gated_storage.rs"]
mod gated_storage;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stream_priority_cannot_reserve_a_piece_during_its_hash_verification() {
    use librqbit::{
        AddTorrent, AddTorrentOptions, AddTorrentResponse, ListenerOptions, Session,
        SessionOptions, TorrentStatsState, storage::StorageFactoryExt,
    };
    use librqbit_peer_protocol::Message;
    use std::sync::{Arc, atomic::Ordering};
    use tokio::io::AsyncReadExt;
    for hybrid in [false, true] {
        let files = tempfile::tempdir().unwrap();
        let data = vec![9; 32768];
        let entries = [("a", vec![7; 32768]), ("b", data.clone())];
        std::fs::write(files.path().join("a"), &entries[0].1).unwrap();
        std::fs::write(files.path().join("b"), vec![0; data.len()]).unwrap();
        let metadata = if hybrid {
            hybrid_files(&entries, 32768, false)
        } else {
            fixture(&entries, 32768)
        };
        let hash = parse(&metadata).info_hash();
        let gate = Arc::new(gated_storage::Gate::default());
        let session = Session::new_with_opts(
            files.path().into(),
            SessionOptions {
                dht: None,
                disable_trackers: true,
                disable_local_service_discovery: true,
                peer_encryption: librqbit::EncryptionPolicy::Disabled,
                listen: Some(ListenerOptions {
                    mode: librqbit::ListenerMode::TcpOnly,
                    listen_addr: "127.0.0.1:0".parse().unwrap(),
                    ..Default::default()
                }),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let response = session
            .add_torrent(
                AddTorrent::from_bytes(encode(&metadata)),
                Some(AddTorrentOptions {
                    only_files: Some(vec![0]),
                    disable_trackers: true,
                    storage_factory: Some(gated_storage::Factory(gate.clone()).boxed()),
                    output_folder: Some(files.path().display().to_string()),
                    overwrite: true,
                    ..Default::default()
                }),
            )
            .await
            .unwrap();
        let AddTorrentResponse::Added(_, torrent) = response else {
            panic!()
        };
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while !matches!(torrent.stats().state, TorrentStatsState::Live) {
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        let mut stream = torrent.clone().stream(1).await.unwrap();
        let port = session.listen_addr().unwrap().port();
        let mut source = wire::connect(port, hash).await;
        wire::send(&mut source, Message::Bitfield((&[0x40u8][..]).into())).await;
        peer_interest(&mut source, true).await;
        wire::send(&mut source, Message::Unchoke).await;
        for _ in 0..2 {
            let request = peer_request(&mut source, 1).await;
            peer_payload(&mut source, request, &data).await;
        }
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            while !gate.verifying.load(Ordering::Acquire) {
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        let mut other = wire::connect(port, hash).await;
        wire::send(&mut other, Message::Bitfield((&[0x40u8][..]).into())).await;
        wire::send(&mut other, Message::Unchoke).await;
        let duplicate = tokio::time::timeout(std::time::Duration::from_millis(350), async {
            loop {
                let frame = wire::frame(&mut other).await;
                if let Message::Request(request) = Message::deserialize(&frame, &[]).unwrap().0 {
                    break request;
                }
            }
        })
        .await;
        gate.release();
        assert!(
            duplicate.is_err(),
            "priority reserved a verifying piece: {duplicate:?}"
        );
        let mut actual = vec![0; data.len()];
        tokio::time::timeout(
            std::time::Duration::from_secs(3),
            stream.read_exact(&mut actual),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(actual, data);
        assert_eq!(std::fs::read(files.path().join("b")).unwrap(), data);
        drop(stream);
        drop(source);
        drop(other);
        session.stop_and_wait().await.unwrap();
    }
}

#[tokio::test]
async fn transfer_rate_limits_accept_blocks_larger_than_the_burst() {
    use librqbit::limits::{Limits, LimitsConfig};
    use std::num::NonZeroU32;
    let limit = Limits::new(LimitsConfig {
        upload_bps: NonZeroU32::new(128),
        download_bps: NonZeroU32::new(128),
    });
    for upload in [true, false] {
        let started = std::time::Instant::now();
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            if upload {
                limit
                    .prepare_for_upload(NonZeroU32::new(256).unwrap())
                    .await
            } else {
                limit
                    .prepare_for_download(NonZeroU32::new(256).unwrap())
                    .await
            }
        })
        .await
        .expect("rate-limited block stalled")
        .expect("block larger than burst must wait, not fail");
        assert!(started.elapsed() >= std::time::Duration::from_millis(900));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hash_replies_share_torrent_and_session_upload_rate_limits_across_peers() {
    use librqbit::{
        AddTorrent, AddTorrentOptions, AddTorrentResponse, ListenerOptions, Session,
        SessionOptions, TorrentStatsState, limits::LimitsConfig,
    };
    use librqbit_peer_protocol::{HashRequest, Message};
    use std::num::NonZeroU32;
    for session_limit in [false, true] {
        let files = tempfile::tempdir().unwrap();
        let data = vec![7; 65536];
        std::fs::write(files.path().join("file"), &data).unwrap();
        let metadata = fixture(&[("file", data)], 32768);
        let meta = parse(&metadata);
        let limited = LimitsConfig {
            upload_bps: NonZeroU32::new(256),
            download_bps: None,
        };
        let session = Session::new_with_opts(
            files.path().into(),
            SessionOptions {
                dht: None,
                disable_trackers: true,
                disable_local_service_discovery: true,
                peer_encryption: librqbit::EncryptionPolicy::Disabled,
                listen: Some(ListenerOptions {
                    mode: librqbit::ListenerMode::TcpOnly,
                    listen_addr: "127.0.0.1:0".parse().unwrap(),
                    ..Default::default()
                }),
                ratelimits: if session_limit {
                    limited
                } else {
                    Default::default()
                },
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let response = session
            .add_torrent(
                AddTorrent::from_bytes(encode(&metadata)),
                Some(AddTorrentOptions {
                    output_folder: Some(files.path().display().to_string()),
                    overwrite: true,
                    disable_trackers: true,
                    ratelimits: if session_limit {
                        Default::default()
                    } else {
                        limited
                    },
                    ..Default::default()
                }),
            )
            .await
            .unwrap();
        let AddTorrentResponse::Added(_, torrent) = response else {
            panic!()
        };
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while !matches!(torrent.stats().state, TorrentStatsState::Live) {
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        let port = session.listen_addr().unwrap().port();
        let mut peers = [
            wire::connect(port, meta.info_hash()).await,
            wire::connect(port, meta.info_hash()).await,
        ];
        let request = HashRequest {
            pieces_root: meta.files()[0].pieces_root().unwrap(),
            base_layer: 0,
            index: 0,
            length: 2,
            proof_layers: 1,
        };
        let started = std::time::Instant::now();
        for index in 0..8 {
            let peer = &mut peers[index % 2];
            wire::send(peer, Message::HashRequest(request)).await;
            loop {
                let response = wire::frame(peer).await;
                match Message::deserialize(&response, &[]).unwrap().0 {
                    Message::Hashes {
                        request: actual,
                        hashes,
                    } => {
                        assert_eq!(actual, request);
                        assert_eq!(hashes.len(), 3);
                        meta.verify_hashes(&request, &hashes).unwrap();
                        break;
                    }
                    Message::Interested | Message::NotInterested => {}
                    message => panic!("unexpected rate-limited hash response {message:?}"),
                }
            }
        }
        // Eight 96-byte hash payloads exhaust a shared 256-byte burst.
        assert!(
            started.elapsed() >= std::time::Duration::from_millis(1500),
            "hash replies bypassed shared upload policy: {:?}",
            started.elapsed()
        );
        drop(peers);
        session.stop_and_wait().await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hash_serving_rejects_optional_layers_without_losing_required_proofs() {
    use librqbit_peer_protocol::{HashRequest, Message};
    let state = tempfile::tempdir().unwrap();
    let files = tempfile::tempdir().unwrap();
    let data = vec![7; 131072];
    std::fs::write(files.path().join("file"), &data).unwrap();
    let metadata = fixture(&[("file", data)], 65536);
    let meta = parse(&metadata);
    let engine = seedhop_core::Engine::open(engine_config(state.path(), files.path()))
        .await
        .unwrap();
    engine
        .add(
            encode(&metadata),
            files.path().display().to_string(),
            vec![0],
            false,
            true,
        )
        .await
        .unwrap();
    settled(&engine).await;
    let mut peer = wire::connect(engine.snapshot().unwrap().listen_port, meta.info_hash()).await;
    for base_layer in [1, 0, 2] {
        let request = HashRequest {
            pieces_root: meta.files()[0].pieces_root().unwrap(),
            base_layer,
            index: 0,
            length: 2,
            proof_layers: 2 - base_layer,
        };
        wire::send(&mut peer, Message::HashRequest(request)).await;
        loop {
            let response = wire::frame(&mut peer).await;
            match Message::deserialize(&response, &[]).unwrap().0 {
                Message::HashReject(actual) => {
                    assert_eq!(base_layer, 1);
                    assert_eq!(actual, request);
                    break;
                }
                Message::Hashes {
                    request: actual,
                    hashes,
                } => {
                    assert_ne!(base_layer, 1);
                    assert_eq!(actual, request);
                    meta.verify_hashes(&request, &hashes).unwrap();
                    break;
                }
                Message::Interested | Message::NotInterested => {}
                other => panic!("unexpected hash-layer response {other:?}"),
            }
        }
    }
    engine.shutdown().await.unwrap();
}

#[test]
fn extension_handshakes_support_omitted_fields_and_zero_disables_serialization() {
    use librqbit_peer_protocol::{
        Message,
        extended::{
            ExtendedMessage, PeerExtendedMessageIds, handshake::ExtendedHandshake,
            ut_metadata::UtMetadata,
        },
    };
    let mut buffer = [0; 1024];
    Message::Extended(ExtendedMessage::Handshake(ExtendedHandshake::default()))
        .serialize(&mut buffer, &Default::default)
        .expect("optional extension fields must serialize");
    let payload = b"d4:reqqi1ee";
    let mut frame = ((payload.len() + 2) as u32).to_be_bytes().to_vec();
    frame.extend_from_slice(&[20, 0]);
    frame.extend_from_slice(payload);
    assert!(
        matches!(Message::deserialize(&frame, &[]).unwrap().0, Message::Extended(ExtendedMessage::Handshake(handshake)) if handshake.m == PeerExtendedMessageIds::default())
    );
    let disabled = PeerExtendedMessageIds {
        ut_metadata: Some(0),
        ut_pex: Some(0),
    };
    assert!(
        Message::Extended(ExtendedMessage::UtMetadata(UtMetadata::Request(0)))
            .serialize(&mut buffer, &|| disabled)
            .is_err(),
        "ID zero is reserved for extension handshakes"
    );
}

async fn extension_update(peer: &mut tokio::net::TcpStream, update: B) {
    use tokio::io::AsyncWriteExt;
    let mut payload = vec![20, 0];
    payload.extend_from_slice(&encode(&update));
    peer.write_u32(payload.len() as u32).await.unwrap();
    peer.write_all(&payload).await.unwrap();
}

async fn metadata_request_with_reply_id(peer: &mut tokio::net::TcpStream, expected_id: u8) {
    use librqbit_peer_protocol::{
        Message,
        extended::{ExtendedMessage, PeerExtendedMessageIds, ut_metadata::UtMetadata},
    };
    use tokio::io::AsyncWriteExt;
    let mut buffer = [0; 1024];
    let len = Message::Extended(ExtendedMessage::UtMetadata(UtMetadata::Request(0)))
        .serialize(&mut buffer, &PeerExtendedMessageIds::my)
        .unwrap();
    peer.write_all(&buffer[..len]).await.unwrap();
    loop {
        let mut response = wire::frame(peer).await;
        if response[4] == 20 {
            assert_eq!(response[5], expected_id);
            response[5] = librqbit_peer_protocol::MY_EXTENDED_UT_METADATA;
            assert!(
                matches!(Message::deserialize(&response, &[]).unwrap().0, Message::Extended(ExtendedMessage::UtMetadata(UtMetadata::Data(data))) if data.piece() == 0)
            );
            return;
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn extension_id_updates_are_additive_and_zero_does_not_become_a_handshake() {
    use librqbit_peer_protocol::{Message, Request};
    use tokio::io::AsyncWriteExt;
    let state = tempfile::tempdir().unwrap();
    let files = tempfile::tempdir().unwrap();
    let metadata = fixture(&[("file", b"hello".to_vec())], 16384);
    let engine = seedhop_core::Engine::open(engine_config(state.path(), files.path()))
        .await
        .unwrap();
    engine
        .add(
            encode(&metadata),
            files.path().display().to_string(),
            vec![0],
            false,
            true,
        )
        .await
        .unwrap();
    settled(&engine).await;
    let mut peer = wire::connect(
        engine.snapshot().unwrap().listen_port,
        parse(&metadata).info_hash(),
    )
    .await;
    extension_update(
        &mut peer,
        dict(&[(
            b"m",
            dict(&[(b"ut_metadata", B::Number(11)), (b"ut_pex", B::Number(0))]),
        )]),
    )
    .await;
    metadata_request_with_reply_id(&mut peer, 11).await;
    extension_update(
        &mut peer,
        dict(&[(b"m", dict(&[(b"ut_pex", B::Number(0))]))]),
    )
    .await;
    metadata_request_with_reply_id(&mut peer, 11).await;
    extension_update(&mut peer, dict(&[(b"reqq", B::Number(1))])).await;
    metadata_request_with_reply_id(&mut peer, 11).await;
    extension_update(
        &mut peer,
        dict(&[(b"m", dict(&[(b"ut_metadata", B::Number(0))]))]),
    )
    .await;
    // A request after disabling the reply extension must not produce ID zero.
    let payload = b"d8:msg_typei0e5:piecei0ee";
    peer.write_u32((payload.len() + 2) as u32).await.unwrap();
    peer.write_all(&[20, librqbit_peer_protocol::MY_EXTENDED_UT_METADATA])
        .await
        .unwrap();
    peer.write_all(payload).await.unwrap();
    wire::send(
        &mut peer,
        Message::Request(Request {
            index: 99,
            begin: 0,
            length: 5,
        }),
    )
    .await;
    loop {
        let response = wire::frame(&mut peer).await;
        match Message::deserialize(&response, &[]).unwrap().0 {
            Message::Reject(request) => {
                assert_eq!(request.index, 99);
                break;
            }
            Message::Interested | Message::NotInterested => {}
            other => panic!("disabled metadata emitted a message: {other:?}"),
        }
    }
    extension_update(
        &mut peer,
        dict(&[(b"m", dict(&[(b"ut_metadata", B::Number(13))]))]),
    )
    .await;
    metadata_request_with_reply_id(&mut peer, 13).await;
    engine.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn private_torrents_omit_discovery_extensions_and_ignore_hostile_pex() {
    use librqbit_peer_protocol::{
        Message,
        extended::{ExtendedMessage, PeerExtendedMessageIds, ut_pex::UtPex},
    };
    use tokio::io::AsyncWriteExt;
    for hybrid in [false, true] {
        for private in [false, true] {
            let state = tempfile::tempdir().unwrap();
            let files = tempfile::tempdir().unwrap();
            let mut metadata = if hybrid {
                self::hybrid()
            } else {
                fixture(&[("file", b"hello".to_vec())], 16384)
            };
            if private {
                put(get(&mut metadata, b"info"), b"private", B::Number(1));
            }
            let engine = seedhop_core::Engine::open_with_peer_encryption(
                engine_config(state.path(), files.path()),
                librqbit::EncryptionPolicy::Disabled,
            )
            .await
            .unwrap();
            engine
                .add(
                    encode(&metadata),
                    files.path().display().to_string(),
                    vec![0],
                    false,
                    true,
                )
                .await
                .unwrap();
            settled(&engine).await;
            let (mut peer, advertised) = wire::connect_advertised(
                engine.snapshot().unwrap().listen_port,
                parse(&metadata).info_hash(),
            )
            .await;
            assert_eq!(advertised.ut_pex.is_some(), !private);
            assert_eq!(advertised.ut_metadata.is_some(), !private);
            let target = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let pex = UtPex::from_addrs(
                std::iter::once(target.local_addr().unwrap()),
                std::iter::empty(),
            );
            let mut buffer = [0; 1024];
            let len = Message::Extended(ExtendedMessage::UtPex(pex.as_borrowed()))
                .serialize(&mut buffer, &PeerExtendedMessageIds::my)
                .unwrap();
            peer.write_all(&buffer[..len]).await.unwrap();
            let connected =
                tokio::time::timeout(std::time::Duration::from_secs(1), target.accept()).await;
            assert_eq!(
                connected.is_ok(),
                !private,
                "private={private}, hybrid={hybrid}"
            );
            drop(connected);
            engine.shutdown().await.unwrap();
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn repeated_extension_handshakes_do_not_duplicate_pex_or_leak_private_peers() {
    use librqbit::{
        AddTorrent, AddTorrentOptions, AddTorrentResponse, ListenerOptions, Session,
        SessionOptions, TorrentStatsState,
    };
    use librqbit_peer_protocol::{Message, extended::ExtendedMessage};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    for private in [false, true] {
        let files = tempfile::tempdir().unwrap();
        let mut metadata = fixture(&[("file", b"hello".to_vec())], 16384);
        if private {
            put(get(&mut metadata, b"info"), b"private", B::Number(1));
        }
        let hash = parse(&metadata).info_hash();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let source_addr = listener.local_addr().unwrap();
        let (ready, ready_rx) = tokio::sync::oneshot::channel();
        let source = tokio::spawn(async move {
            let (mut peer, _) = listener.accept().await.unwrap();
            let mut handshake = [0; 68];
            peer.read_exact(&mut handshake).await.unwrap();
            handshake[48..].copy_from_slice(b"-PXsrc-1234567890123");
            peer.write_all(&handshake).await.unwrap();
            wire::send(&mut peer, Message::Bitfield((&[0x80u8][..]).into())).await;
            let _ = ready.send(());
            let mut buffer = [0; 1024];
            loop {
                tokio::select! {
                    result = peer.read(&mut buffer) => if !matches!(result, Ok(n) if n > 0) { return; },
                    _ = tokio::time::sleep(std::time::Duration::from_secs(1)) => wire::send(&mut peer, Message::KeepAlive).await,
                }
            }
        });
        let session = Session::new_with_opts(
            files.path().into(),
            SessionOptions {
                dht: None,
                disable_trackers: true,
                disable_local_service_discovery: true,
                peer_encryption: librqbit::EncryptionPolicy::Disabled,
                listen: Some(ListenerOptions {
                    mode: librqbit::ListenerMode::TcpOnly,
                    listen_addr: "127.0.0.1:0".parse().unwrap(),
                    ..Default::default()
                }),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let response = session
            .add_torrent(
                AddTorrent::from_bytes(encode(&metadata)),
                Some(AddTorrentOptions {
                    output_folder: Some(files.path().display().to_string()),
                    overwrite: true,
                    disable_trackers: true,
                    initial_peers: Some(vec![source_addr]),
                    ..Default::default()
                }),
            )
            .await
            .unwrap();
        let AddTorrentResponse::Added(_, torrent) = response else {
            panic!()
        };
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while !matches!(torrent.stats().state, TorrentStatsState::Live) {
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
            ready_rx.await.unwrap();
        })
        .await
        .unwrap();
        let mut observer = wire::connect(session.listen_addr().unwrap().port(), hash).await;
        for _ in 0..3 {
            extension_update(
                &mut observer,
                dict(&[(b"m", dict(&[(b"ut_pex", B::Number(9))]))]),
            )
            .await;
        }
        extension_update(
            &mut observer,
            dict(&[(b"m", dict(&[(b"ut_pex", B::Number(0))]))]),
        )
        .await;
        for _ in 0..3 {
            extension_update(
                &mut observer,
                dict(&[(b"m", dict(&[(b"ut_pex", B::Number(9))]))]),
            )
            .await;
        }
        let (mut observer_read, mut observer_write) = observer.into_split();
        let keepalive = tokio::spawn(async move {
            loop {
                wire::send(&mut observer_write, Message::KeepAlive).await;
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            }
        });
        let mut exchanges = 0;
        let _ = tokio::time::timeout(std::time::Duration::from_secs(12), async {
            loop {
                let length = observer_read.read_u32().await.unwrap() as usize;
                assert!(length < 65536);
                let mut response = vec![0; length + 4];
                response[..4].copy_from_slice(&(length as u32).to_be_bytes());
                observer_read.read_exact(&mut response[4..]).await.unwrap();
                if response.get(4) == Some(&20) {
                    assert_eq!(response[5], 9);
                    response[5] = librqbit_peer_protocol::MY_EXTENDED_UT_PEX;
                    match Message::deserialize(&response, &[]).unwrap().0 {
                        Message::Extended(ExtendedMessage::UtPex(pex)) => {
                            assert!(pex.added_peers().any(|peer| peer.addr == source_addr));
                            exchanges += 1;
                        }
                        other => panic!("unexpected discovery message {other:?}"),
                    }
                }
            }
        })
        .await;
        assert_eq!(
            exchanges,
            usize::from(!private),
            "duplicate or private PEX; private={private}"
        );
        drop(observer_read);
        keepalive.abort();
        let _ = keepalive.await;
        session.stop_and_wait().await.unwrap();
        source.abort();
        let _ = source.await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn lsd_offers_reach_public_swarms_and_never_private_swarms() {
    use librqbit::{
        AddTorrent, AddTorrentOptions, AddTorrentResponse, ListenerOptions, Session,
        SessionOptions, TorrentStatsState,
    };
    use tokio::io::AsyncReadExt;
    for hybrid in [false, true] {
        for private in [false, true] {
            let files = tempfile::tempdir().unwrap();
            let mut metadata = if hybrid {
                self::hybrid()
            } else {
                fixture(&[("file", b"hello".to_vec())], 16384)
            };
            if private {
                put(get(&mut metadata, b"info"), b"private", B::Number(1));
            }
            let mut identities = vec![parse(&metadata).info_hash()[..20].to_vec()];
            if hybrid {
                identities.push(sha1::Sha1::digest(encode(get(&mut metadata, b"info"))).to_vec());
            }
            let session = Session::new_with_opts(
                files.path().into(),
                SessionOptions {
                    dht: None,
                    disable_trackers: true,
                    disable_local_service_discovery: false,
                    peer_encryption: librqbit::EncryptionPolicy::Disabled,
                    listen: Some(ListenerOptions {
                        mode: librqbit::ListenerMode::TcpOnly,
                        listen_addr: "127.0.0.1:0".parse().unwrap(),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
            let response = session
                .add_torrent(
                    AddTorrent::from_bytes(encode(&metadata)),
                    Some(AddTorrentOptions {
                        output_folder: Some(files.path().display().to_string()),
                        overwrite: true,
                        disable_trackers: true,
                        ..Default::default()
                    }),
                )
                .await
                .unwrap();
            let AddTorrentResponse::Added(_, torrent) = response else {
                panic!()
            };
            tokio::time::timeout(std::time::Duration::from_secs(5), async {
                while !matches!(torrent.stats().state, TorrentStatsState::Live) {
                    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                }
            })
            .await
            .unwrap();
            for identity in identities {
                let target = tokio::net::TcpListener::bind("[::1]:0").await.unwrap();
                let port = target.local_addr().unwrap().port();
                let identity_hex: String =
                    identity.iter().map(|byte| format!("{byte:02x}")).collect();
                let message = format!(
                    "BT-SEARCH * HTTP/1.1\r\nHost: [ff15::efc0:988f]:6771\r\nPort: {port}\r\nInfohash: {identity_hex}\r\ncookie: 54321\r\n\r\n\r\n"
                );
                let offering = tokio::spawn(async move {
                    let sender = tokio::net::UdpSocket::bind("[::1]:0").await.unwrap();
                    loop {
                        sender
                            .send_to(message.as_bytes(), "[::1]:6771")
                            .await
                            .unwrap();
                        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                    }
                });
                let accepted =
                    tokio::time::timeout(std::time::Duration::from_secs(1), target.accept()).await;
                offering.abort();
                let _ = offering.await;
                assert_eq!(
                    accepted.is_ok(),
                    !private,
                    "hybrid={hybrid}, private={private}"
                );
                if let Ok(Ok((mut peer, _))) = accepted {
                    let mut handshake = [0; 68];
                    tokio::time::timeout(
                        std::time::Duration::from_secs(1),
                        peer.read_exact(&mut handshake),
                    )
                    .await
                    .unwrap()
                    .unwrap();
                    assert_eq!(&handshake[28..48], identity);
                }
            }
            session.stop_and_wait().await.unwrap();
        }
    }
}

#[test]
fn future_metainfo_version_precedes_current_layout_validation() {
    for info in [
        dict(&[(b"meta version", B::Number(3))]),
        dict(&[
            (b"file tree", B::Number(0)),
            (b"meta version", B::Number(3)),
            (b"piece length", B::Number(1)),
        ]),
    ] {
        assert!(matches!(
            Metainfo::check_info_version(&encode(&info)),
            Err(V2Error::Version(3))
        ));
        let metainfo = encode(&dict(&[(b"info", info)]));
        assert!(matches!(
            Metainfo::parse(&metainfo),
            Err(V2Error::Version(3))
        ));
        let error = adapter::preview(metainfo).unwrap_err().to_string();
        assert!(error.contains("version 3 is not supported"), "{error}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unsupported_hash_parameters_receive_correlated_rejects_without_closing() {
    use librqbit_peer_protocol::{HashRequest, Message};
    for hybrid in [false, true] {
        let state = tempfile::tempdir().unwrap();
        let files = tempfile::tempdir().unwrap();
        let data = vec![7; 65536];
        std::fs::write(files.path().join("file"), &data).unwrap();
        let metadata = if hybrid {
            hybrid_files(&[("file", data)], 32768, false)
        } else {
            fixture(&[("file", data)], 32768)
        };
        let meta = parse(&metadata);
        let engine = seedhop_core::Engine::open(engine_config(state.path(), files.path()))
            .await
            .unwrap();
        engine
            .add(
                encode(&metadata),
                files.path().display().to_string(),
                vec![0],
                false,
                true,
            )
            .await
            .unwrap();
        settled(&engine).await;
        let mut peer =
            wire::connect(engine.snapshot().unwrap().listen_port, meta.info_hash()).await;
        let valid = HashRequest {
            pieces_root: meta.files()[0].pieces_root().unwrap(),
            base_layer: 0,
            index: 0,
            length: 2,
            proof_layers: 1,
        };
        for request in [
            HashRequest { length: 0, ..valid },
            HashRequest { length: 3, ..valid },
            HashRequest {
                length: 1024,
                ..valid
            },
            HashRequest {
                length: u32::MAX,
                ..valid
            },
            HashRequest {
                index: u32::MAX,
                ..valid
            },
            HashRequest {
                base_layer: u32::MAX,
                proof_layers: u32::MAX,
                ..valid
            },
            HashRequest {
                pieces_root: [0; 32],
                ..valid
            },
            valid,
        ] {
            wire::send(&mut peer, Message::HashRequest(request)).await;
            loop {
                let response = wire::frame(&mut peer).await;
                match Message::deserialize(&response, &[]).unwrap().0 {
                    Message::HashReject(actual) => {
                        assert_ne!(request, valid);
                        assert_eq!(actual, request);
                        break;
                    }
                    Message::Hashes {
                        request: actual,
                        hashes,
                    } => {
                        assert_eq!(request, valid);
                        assert_eq!(actual, request);
                        meta.verify_hashes(&request, &hashes).unwrap();
                        break;
                    }
                    Message::Interested | Message::NotInterested => {}
                    other => panic!("unexpected hash response {other:?}"),
                }
            }
        }
        engine.shutdown().await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn magnet_metadata_tries_fresh_peers_before_retrying_failures() {
    use librqbit::{AddTorrent, AddTorrentOptions, AddTorrentResponse, Session, SessionOptions};
    use librqbit_peer_protocol::{
        Message,
        extended::{ExtendedMessage, handshake::ExtendedHandshake, ut_metadata::UtMetadata},
    };
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    // Also retain recovery when the only provider fails its first connection.
    for (fresh_provider, stalled, peer_count) in
        [(true, false, 9), (false, false, 1), (true, true, 65)]
    {
        let mut metadata = fixture(&[("file", vec![7; 123])], 32768);
        let id = parse(&metadata)
            .info_hash()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        let info = Arc::new(encode(get(&mut metadata, b"info")));
        let mut peers = Vec::new();
        let mut tasks = Vec::new();
        for index in 0..peer_count {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            peers.push(listener.local_addr().unwrap());
            let info = info.clone();
            tasks.push(tokio::spawn(async move {
                let mut connections = tokio::task::JoinSet::new();
                let mut attempts = 0;
                loop {
                    tokio::select! {
                        accepted = listener.accept() => {
                            let (mut socket, _) = accepted.unwrap();
                            attempts += 1;
                            let attempt = attempts;
                            let info = info.clone();
                            connections.spawn(async move {
                                let mut handshake = [0; 68];
                                if socket.read_exact(&mut handshake).await.is_err() { return; }
                                let healthy = if fresh_provider { index == peer_count - 1 } else { attempt > 1 };
                                if !healthy {
                                    if stalled || attempt > 1 {
                                        // A retry must not occupy the lookup ahead of a fresh provider.
                                        tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                                    }
                                    return;
                                }
                                handshake[48..].copy_from_slice(b"-SHtest-123456789012");
                                if socket.write_all(&handshake).await.is_err() { return; }
                                let mut extension = ExtendedHandshake::new();
                                extension.metadata_size = Some(info.len() as u32);
                                wire::send(&mut socket, Message::Extended(ExtendedMessage::Handshake(extension))).await;
                                let mut metadata_id = None;
                                while let Ok(length) = socket.read_u32().await {
                                    assert!(length < 32768);
                                    let mut frame = vec![0; length as usize + 4];
                                    frame[..4].copy_from_slice(&length.to_be_bytes());
                                    if socket.read_exact(&mut frame[4..]).await.is_err() { return; }
                                    match Message::deserialize(&frame, &[]).unwrap().0 {
                                        Message::Extended(ExtendedMessage::Handshake(h)) => metadata_id = h.m.ut_metadata,
                                        Message::Extended(ExtendedMessage::UtMetadata(UtMetadata::Request(0))) => {
                                            let header = format!("d8:msg_typei1e5:piecei0e10:total_sizei{}ee", info.len());
                                            if socket.write_u32((2 + header.len() + info.len()) as u32).await.is_err() { return; }
                                            if socket.write_all(&[20, metadata_id.unwrap()]).await.is_err()
                                                || socket.write_all(header.as_bytes()).await.is_err()
                                                || socket.write_all(&info).await.is_err() { return; }
                                        }
                                        _ => {}
                                    }
                                }
                            });
                        }
                        _ = connections.join_next(), if !connections.is_empty() => {}
                    }
                }
            }));
        }
        let files = tempfile::tempdir().unwrap();
        let session = Session::new_with_opts(
            files.path().into(),
            SessionOptions {
                peer_encryption: librqbit::EncryptionPolicy::Disabled,
                dht: None,
                disable_local_service_discovery: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(if stalled { 14 } else { 3 }),
            session.add_torrent(
                AddTorrent::from_url(format!("magnet:?xt=urn:btmh:1220{id}")),
                Some(AddTorrentOptions {
                    list_only: true,
                    initial_peers: Some(peers),
                    ..Default::default()
                }),
            ),
        )
        .await;
        session.stop_and_wait().await.unwrap();
        for task in tasks {
            task.abort();
            let _ = task.await;
        }
        assert!(matches!(
            result
                .expect("failed-peer retries starved fresh metadata provider")
                .unwrap(),
            AddTorrentResponse::ListOnly(_)
        ));
        assert_eq!(std::fs::read_dir(files.path()).unwrap().count(), 0);
    }
}

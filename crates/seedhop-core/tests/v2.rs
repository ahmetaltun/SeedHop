use seedhop_core::{
    CoreError, adapter,
    v2::{Metainfo, V2Error},
};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

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
fn rejects_unsafe_ambiguous_and_excessively_nested_trees() {
    for name in ["..", ".", "bad\\name", "bad\0name"] {
        assert!(Metainfo::parse(&encode(&fixture(&[(name, vec![1])], 16384))).is_err());
    }
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
    assert!(matches!(
        engine
            .add(
                encode(&fixture(&[("..", vec![1])], 16384)),
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
    for path in ["x".repeat(256), vec!["x".repeat(255); 17].join("/")] {
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
        let message = Message::HashRequest(invalid);
        let mut bytes = [0; 1024];
        let length = message.serialize(&mut bytes, &Default::default).unwrap();
        assert!(Message::deserialize(&bytes[..length], &[]).is_err());
    }
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

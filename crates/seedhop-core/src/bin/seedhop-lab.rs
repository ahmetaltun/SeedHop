use seedhop_core::{Engine, EngineConfig, adapter};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

fn peer_encryption() -> Result<librqbit::EncryptionPolicy, Box<dyn std::error::Error>> {
    Ok(match std::env::var("SEEDHOP_PEER_ENCRYPTION").as_deref() {
        Err(std::env::VarError::NotPresent) | Ok("allow") => librqbit::EncryptionPolicy::Allow,
        Ok("disabled") => librqbit::EncryptionPolicy::Disabled,
        Ok("prefer") => librqbit::EncryptionPolicy::Prefer,
        Ok("require") => librqbit::EncryptionPolicy::Require,
        _ => return Err("invalid SEEDHOP_PEER_ENCRYPTION policy".into()),
    })
}
async fn open_engine(
    config: EngineConfig,
) -> Result<std::sync::Arc<Engine>, Box<dyn std::error::Error>> {
    Ok(Engine::open_with_peer_encryption(config, peer_encryption()?).await?)
}

#[cfg(feature = "layout-fault-injection")]
async fn layout_lab(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    use std::os::unix::fs::FileExt;
    let command = args[1].as_str();
    if command == "layout-inspect" {
        println!(
            "{}",
            serde_json::to_string(&adapter::preview(std::fs::read(&args[2])?)?)?
        );
        return Ok(());
    }
    let state = args.get(2).ok_or("missing state directory")?.clone();
    let files = args.get(3).ok_or("missing files directory")?.clone();
    let engine = open_engine(EngineConfig {
        state_directory: state,
        download_directory: files.clone(),
        allow_network_discovery: false,
        listen_port: 0,
    })
    .await?;
    if command == "layout-prepare" {
        let bytes = std::fs::read(args.get(4).ok_or("missing torrent")?)?;
        let preview = adapter::preview(bytes.clone())?;
        let intent = args.get(5).map(String::as_str).unwrap_or("paused");
        let selected = preview.files.last().ok_or("missing files")?.clone();
        if intent != "paused" {
            let file = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(PathBuf::from(&files).join(&selected.name))?;
            let mut byte = [0];
            file.read_exact_at(&mut byte, 0)?;
            byte[0] ^= 1;
            file.write_all_at(&byte, 0)?;
            file.sync_all()?;
        }
        if intent == "queued" {
            engine.set_queue(1).await?;
            engine
                .add(
                    adapter::create_fixture("queue-blocker", b"not downloaded"),
                    files.clone(),
                    vec![0],
                    false,
                    false,
                )
                .await?;
        }
        let id = engine
            .add(bytes, files, vec![selected.index], intent == "paused", true)
            .await?;
        loop {
            if engine
                .snapshot()?
                .torrents
                .iter()
                .find(|torrent| torrent.id == id)
                .is_some_and(|torrent| torrent.state != "checking")
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let mut priorities = vec![0; preview.files.len()];
        priorities[selected.index as usize] = 2;
        engine.set_file_priorities(id, priorities).await?;
        engine
            .set_queue(if intent == "queued" { 1 } else { 0 })
            .await?;
    } else {
        let started = Instant::now();
        while engine
            .snapshot()?
            .torrents
            .iter()
            .any(|torrent| torrent.state == "checking")
        {
            if started.elapsed() > Duration::from_secs(20) {
                return Err("layout verification timed out".into());
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        if command == "layout-change" {
            let torrent = engine
                .snapshot()?
                .torrents
                .into_iter()
                .find(|torrent| torrent.name != "queue-blocker")
                .ok_or("missing transfer")?;
            if args.get(5).map(String::as_str) == Some("rename") {
                engine
                    .rename_file(
                        torrent.id,
                        torrent.files.last().ok_or("missing file")?.index,
                        "renamed.bin".into(),
                    )
                    .await?;
            } else {
                engine
                    .relocate(
                        torrent.id,
                        args.get(4).ok_or("missing destination")?.clone(),
                    )
                    .await?;
            }
        } else if command == "layout-resolve" {
            let notice = engine
                .snapshot()?
                .storage_recovery
                .ok_or("no recovery notice")?;
            engine
                .resolve_storage_recovery(
                    notice.id,
                    args.get(4).map(String::as_str) == Some("keep"),
                )
                .await?;
        }
    }
    let started = Instant::now();
    while engine
        .snapshot()?
        .torrents
        .iter()
        .any(|torrent| torrent.state == "checking")
    {
        if started.elapsed() > Duration::from_secs(20) {
            return Err("layout verification timed out".into());
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    println!("{}", serde_json::to_string(&engine.snapshot()?)?);
    engine.shutdown().await?;
    Ok(())
}

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::var_os("SEEDHOP_DIAGNOSTICS").is_some() {
        tracing_subscriber::fmt()
            .with_env_filter("librqbit=debug,librqbit_tracker_comms=debug")
            .with_writer(std::io::stderr)
            .init();
    }
    let args: Vec<_> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        #[cfg(feature = "layout-fault-injection")]
        Some(
            "layout-inspect" | "layout-prepare" | "layout-change" | "layout-status"
            | "layout-resolve",
        ) => layout_lab(&args).await?,
        Some(command @ ("inspect-v2" | "verify-v2-piece")) => {
            let path = args.get(2).ok_or("missing torrent")?;
            let file = std::fs::File::open(path)?;
            use std::io::Read;
            let mut bytes = Vec::new();
            file.take(seedhop_core::v2::MAX_METAINFO_LENGTH as u64 + 1)
                .read_to_end(&mut bytes)?;
            let meta = seedhop_core::v2::Metainfo::parse(&bytes)?.ok_or("This is a v1 torrent.")?;
            if command == "verify-v2-piece" {
                let index: u64 = args.get(3).ok_or("missing piece index")?.parse()?;
                let piece = std::fs::File::open(args.get(4).ok_or("missing piece data file")?)?;
                let mut data = Vec::new();
                piece.take(meta.piece_length() + 1).read_to_end(&mut data)?;
                meta.verify_piece(index, &data)?;
                println!("Verified SHA-256 piece {index}");
            } else {
                println!("{}", serde_json::to_string_pretty(&meta)?);
            }
        }
        // Controlled DHT bootstrap for the discovery harness. Uses the same vendored
        // Session discovery path as Engine without changing the app's public config.
        Some(command @ ("probe-dht" | "probe-magnet-dht")) => {
            let source = args.get(2).ok_or("missing torrent or magnet")?.clone();
            let magnet = command == "probe-magnet-dht";
            let torrent = if magnet {
                None
            } else {
                Some(std::fs::read(&source)?)
            };
            let downloads = PathBuf::from(args.get(3).ok_or("missing downloads")?);
            let bootstrap = args.get(4).ok_or("missing local bootstrap")?.clone();
            let private = !magnet && args.get(5).is_some_and(|s| s == "private");
            let session = librqbit::Session::new_with_opts(
                downloads.clone(),
                librqbit::SessionOptions {
                    peer_encryption: peer_encryption()?,
                    client_name_and_version: if magnet {
                        Some(format!("SeedHop/{}", env!("CARGO_PKG_VERSION")))
                    } else {
                        None
                    },
                    dht: Some(librqbit::DhtSessionConfig {
                        bootstrap_addrs: Some(vec![bootstrap]),
                        persistence: None,
                        port: Some(0),
                    }),
                    disable_local_service_discovery: true,
                    fastresume: false,
                    trackers: if magnet {
                        [args.get(5).ok_or("missing extra tracker")?.parse()?]
                            .into_iter()
                            .collect()
                    } else {
                        Default::default()
                    },
                    listen: Some(Default::default()),
                    ..Default::default()
                },
            )
            .await?;
            let handle = session
                .add_torrent(
                    if magnet {
                        librqbit::AddTorrent::from_url(source)
                    } else {
                        librqbit::AddTorrent::from_bytes(torrent.unwrap())
                    },
                    Some(librqbit::AddTorrentOptions {
                        output_folder: Some(downloads.to_string_lossy().into()),
                        force_tracker_interval: if magnet {
                            Some(Duration::from_millis(100))
                        } else {
                            None
                        },
                        ..Default::default()
                    }),
                )
                .await?
                .into_handle()
                .ok_or("missing handle")?;
            let result: Result<(), Box<dyn std::error::Error>> = if private {
                tokio::time::sleep(Duration::from_secs(4)).await;
                let stats = handle.stats();
                if stats.error.is_some() || serde_json::to_value(&stats)?["state"] != "live" {
                    Err(format!("private torrent did not start: {stats:?}").into())
                } else if stats.progress_bytes != 0 {
                    Err("private torrent unexpectedly downloaded without a tracker".into())
                } else {
                    Ok(())
                }
            } else {
                match tokio::time::timeout(Duration::from_secs(30), handle.wait_until_completed())
                    .await
                {
                    Ok(Ok(())) => Ok(()),
                    other => Err(format!("DHT transfer failed: {other:?}").into()),
                }
            };
            println!("{}", serde_json::to_string(&handle.stats())?);
            session.stop_and_wait().await?;
            result?;
        }
        Some("seed") => {
            use std::io::Write;
            let torrent = std::fs::read(args.get(2).ok_or("missing torrent")?)?;
            let state = args.get(3).ok_or("missing state")?.clone();
            let downloads = args.get(4).ok_or("missing downloads")?.clone();
            let engine = open_engine(EngineConfig {
                state_directory: state,
                download_directory: downloads.clone(),
                allow_network_discovery: false,
                listen_port: 0,
            })
            .await?;
            let selected = adapter::preview(torrent.clone())?
                .files
                .iter()
                .map(|file| file.index)
                .collect();
            engine
                .add(torrent, downloads, selected, false, true)
                .await?;
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                let snapshot = engine.snapshot()?;
                if snapshot.torrents.iter().any(|t| t.state == "seeding") {
                    println!("{}", serde_json::to_string(&snapshot)?);
                    std::io::stdout().flush()?;
                    break;
                }
                if Instant::now() >= deadline {
                    return Err("seed did not verify fixture".into());
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            std::future::pending::<()>().await;
        }
        Some("hold") => {
            use std::io::Write;
            let state = args.get(2).ok_or("missing state")?.clone();
            let downloads = args.get(3).ok_or("missing downloads")?.clone();
            let engine = open_engine(EngineConfig {
                state_directory: state,
                download_directory: downloads.clone(),
                allow_network_discovery: false,
                listen_port: 0,
            })
            .await?;
            engine
                .add(
                    adapter::create_fixture("crash.txt", b"recover safely"),
                    downloads,
                    vec![0],
                    true,
                    false,
                )
                .await?;
            engine.set_limits(123_000, 45_000).await?;
            println!("READY");
            std::io::stdout().flush()?;
            std::future::pending::<()>().await;
        }
        Some("fixture") => {
            let root = PathBuf::from(args.get(2).ok_or("fixture requires an output directory")?);
            std::fs::create_dir_all(&root)?;
            let content: Vec<u8> = (0..1024 * 1024).map(|i| (i % 251) as u8).collect();
            std::fs::write(root.join("seedhop-fixture.bin"), &content)?;
            std::fs::write(
                root.join("fixture.torrent"),
                adapter::create_fixture("seedhop-fixture.bin", &content),
            )?;
            println!("Created 1 MiB deterministic fixture in {}", root.display());
        }
        Some(command @ ("download" | "download-magnet" | "inspect-magnet")) => {
            let source = args.get(2).ok_or("missing torrent or magnet")?;
            let state = args.get(3).ok_or("missing state folder")?.clone();
            let downloads = args.get(4).ok_or("missing downloads folder")?.clone();
            let engine = open_engine(EngineConfig {
                state_directory: state,
                download_directory: downloads.clone(),
                allow_network_discovery: false,
                listen_port: 0,
            })
            .await?;
            let metadata = if command != "download" {
                tokio::time::timeout(
                    Duration::from_secs(30),
                    engine.inspect("interop-metadata".into(), Some(source.clone()), vec![]),
                )
                .await??
            } else {
                adapter::preview(std::fs::read(source)?)?
            };
            if command == "inspect-magnet" {
                let output =
                    serde_json::json!({ "metadata": metadata, "snapshot": engine.snapshot()? });
                engine.shutdown().await?;
                println!("{}", serde_json::to_string(&output)?);
                return Ok(());
            }
            let selected = if let Some(indices) = args.get(5) {
                indices
                    .split(',')
                    .map(str::parse::<u32>)
                    .collect::<Result<Vec<_>, _>>()?
            } else {
                metadata.files.iter().map(|file| file.index).collect()
            };
            let id = engine
                .add(metadata.torrent_bytes, downloads, selected, false, false)
                .await?;
            let transfer_timeout = std::env::var("SEEDHOP_TRANSFER_TIMEOUT_SECS")
                .ok()
                .map(|value| value.parse::<u64>())
                .transpose()?
                .unwrap_or(45);
            if !(5..=300).contains(&transfer_timeout) {
                return Err("SEEDHOP_TRANSFER_TIMEOUT_SECS must be between 5 and 300".into());
            }
            let deadline = Instant::now() + Duration::from_secs(transfer_timeout);
            loop {
                let snapshot = engine.snapshot()?;
                let item = snapshot
                    .torrents
                    .iter()
                    .find(|t| t.id == id)
                    .ok_or("torrent missing")?;
                if item.state == "seeding" {
                    println!("{}", serde_json::to_string_pretty(&snapshot)?);
                    break;
                }
                if item.state == "error" || Instant::now() >= deadline {
                    return Err(format!("Transfer failed: {item:?}").into());
                }
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
            engine.shutdown().await?;
        }
        _ => {
            return Err(
                "Usage: seedhop-lab probe-dht TORRENT DOWNLOADS BOOTSTRAP [private] | probe-magnet-dht MAGNET DOWNLOADS BOOTSTRAP EXTRA_TRACKER | inspect-v2 TORRENT | inspect-magnet MAGNET STATE DOWNLOADS | verify-v2-piece TORRENT INDEX FILE | fixture DIRECTORY | seed TORRENT STATE FILES | download TORRENT STATE DOWNLOADS [INDICES] | download-magnet MAGNET STATE DOWNLOADS [INDICES] | hold STATE DOWNLOADS".into(),
            );
        }
    }
    Ok(())
}

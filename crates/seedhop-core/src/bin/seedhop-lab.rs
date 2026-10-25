use seedhop_core::{Engine, EngineConfig, adapter};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

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
        Some("probe-dht") => {
            let torrent = std::fs::read(args.get(2).ok_or("missing torrent")?)?;
            let downloads = PathBuf::from(args.get(3).ok_or("missing downloads")?);
            let bootstrap = args.get(4).ok_or("missing local bootstrap")?.clone();
            let private = args.get(5).is_some_and(|s| s == "private");
            let session = librqbit::Session::new_with_opts(
                downloads.clone(),
                librqbit::SessionOptions {
                    dht: Some(librqbit::DhtSessionConfig {
                        bootstrap_addrs: Some(vec![bootstrap]),
                        persistence: None,
                        port: Some(0),
                    }),
                    disable_local_service_discovery: true,
                    fastresume: false,
                    listen: Some(Default::default()),
                    ..Default::default()
                },
            )
            .await?;
            let handle = session
                .add_torrent(
                    librqbit::AddTorrent::from_bytes(torrent),
                    Some(librqbit::AddTorrentOptions {
                        output_folder: Some(downloads.to_string_lossy().into()),
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
            let engine = Engine::open(EngineConfig {
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
            let engine = Engine::open(EngineConfig {
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
        Some(command @ ("download" | "download-magnet")) => {
            let source = args.get(2).ok_or("missing torrent or magnet")?;
            let state = args.get(3).ok_or("missing state folder")?.clone();
            let downloads = args.get(4).ok_or("missing downloads folder")?.clone();
            let engine = Engine::open(EngineConfig {
                state_directory: state,
                download_directory: downloads.clone(),
                allow_network_discovery: false,
                listen_port: 0,
            })
            .await?;
            let metadata = if command == "download-magnet" {
                tokio::time::timeout(
                    Duration::from_secs(30),
                    engine.inspect("interop-metadata".into(), Some(source.clone()), vec![]),
                )
                .await??
            } else {
                adapter::preview(std::fs::read(source)?)?
            };
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
            let deadline = Instant::now() + Duration::from_secs(45);
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
                "Usage: seedhop-lab probe-dht TORRENT DOWNLOADS BOOTSTRAP [private] | inspect-v2 TORRENT | verify-v2-piece TORRENT INDEX FILE | fixture DIRECTORY | seed TORRENT STATE FILES | download TORRENT STATE DOWNLOADS [INDICES] | download-magnet MAGNET STATE DOWNLOADS [INDICES] | hold STATE DOWNLOADS".into(),
            );
        }
    }
    Ok(())
}

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
            engine.add(torrent, downloads, vec![0], false, true).await?;
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
        Some("download") => {
            let torrent = std::fs::read(args.get(2).ok_or("missing torrent")?)?;
            let state = args.get(3).ok_or("missing state folder")?.clone();
            let downloads = args.get(4).ok_or("missing downloads folder")?.clone();
            let engine = Engine::open(EngineConfig {
                state_directory: state,
                download_directory: downloads.clone(),
                allow_network_discovery: false,
                listen_port: 0,
            })
            .await?;
            let id = engine
                .add(torrent, downloads, vec![0], false, false)
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
                "Usage: seedhop-lab fixture DIRECTORY | download TORRENT STATE DOWNLOADS".into(),
            );
        }
    }
    Ok(())
}

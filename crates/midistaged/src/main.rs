use clap::{Parser, Subcommand};
use midistage_protocol::Broker;
use midistaged::{
    inventory::PROFILES,
    runtime::Runtime,
    service::Service,
    settings::{RuntimeLock, SettingsStore, atomic_write},
};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

#[derive(Parser)]
#[command(
    name = "midistaged",
    about = "MIDI 機材のアプリ別使用権と接続を管理する共通サービス"
)]
struct Args {
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    /// 同一ユーザーの loopback サービスを起動する。
    Serve {
        #[arg(long)]
        state_dir: Option<PathBuf>,
    },
    /// CoreMIDI の物理ポートを列挙する。接続や MIDI 送信は行わない。
    ListPorts,
}
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .init();
    match args.command {
        Command::ListPorts =>
        {
            #[cfg(target_os = "macos")]
            for (id, device) in midistaged::native::discover() {
                if let Some(error) = device.error {
                    println!("{id}: {error}");
                }
                for port in device.ports {
                    println!(
                        "{id}\t{}\t{}\t{}",
                        if port.input { "IN" } else { "OUT" },
                        port.uid as i32,
                        port.name
                    );
                }
            }
        }
        Command::Serve { state_dir } => {
            let directory = match state_dir {
                Some(path) => path,
                None => PathBuf::from(
                    std::env::var_os("HOME")
                        .ok_or_else(|| anyhow::anyhow!("HOME is unavailable; pass --state-dir"))?,
                )
                .join("Library/Application Support/Midistage"),
            };
            let _lock = RuntimeLock::acquire(&directory)?;
            let mut broker = Broker::new(uuid::Uuid::new_v4().to_string());
            for (id, name) in PROFILES {
                broker.add_device(id, id, false);
                broker.devices.get_mut(*id).expect("known device").name = (*name).into();
            }
            let runtime = Runtime::new(broker, SettingsStore::new(&directory))?;
            let service = Service::start(runtime).await?;
            atomic_write(
                &directory,
                "endpoint.json",
                &serde_json::to_vec(&service.endpoint)?,
            )?;
            let running = Arc::new(AtomicBool::new(true));
            let worker_running = running.clone();
            let shared = service.runtime.clone();
            let worker = std::thread::spawn(move || {
                #[cfg(target_os = "macos")]
                {
                    let mut native = midistaged::native::NativeRuntime::new(shared);
                    while worker_running.load(Ordering::Acquire) {
                        native.poll();
                        std::thread::sleep(std::time::Duration::from_millis(50));
                    }
                }
            });
            tracing::info!(
                port = service.endpoint.port,
                "midistaged listening on loopback"
            );
            #[cfg(unix)]
            {
                let mut terminate =
                    tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
                tokio::select! { result = tokio::signal::ctrl_c() => { result?; }, _ = terminate.recv() => {} }
            }
            running.store(false, Ordering::Release);
            // native Drop が lease を無効にして送信完了を待つ。timeout で引き渡さない。
            tokio::task::spawn_blocking(move || worker.join())
                .await?
                .map_err(|_| anyhow::anyhow!("MIDI worker panicked"))?;
            service.shutdown().await?;
            std::fs::remove_file(directory.join("endpoint.json"))?;
        }
    }
    Ok(())
}

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
    version,
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
        /// ログの置き場（既定 ~/Library/Logs/midistaged、日次ローテーション）。stderr にも出す
        #[arg(long)]
        log_dir: Option<PathBuf>,
    },
    /// CoreMIDI の物理ポートを列挙する。接続や MIDI 送信は行わない。
    ListPorts,
}
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    // serve だけファイルにも書く。guard は main の終わりまで持つ（drop で flush）
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let _log_guard = match &args.command {
        Command::Serve { log_dir, .. } => {
            let dir = midistaged::paths::log_dir(home.as_deref(), log_dir.clone())?;
            std::fs::create_dir_all(&dir)?;
            let (file, guard) = tracing_appender::non_blocking(tracing_appender::rolling::daily(
                &dir,
                "midistaged.log",
            ));
            use tracing_subscriber::layer::SubscriberExt;
            use tracing_subscriber::util::SubscriberInitExt;
            tracing_subscriber::registry()
                .with(
                    tracing_subscriber::EnvFilter::try_from_default_env()
                        .unwrap_or_else(|_| "info".into()),
                )
                .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
                .with(
                    tracing_subscriber::fmt::layer()
                        .with_ansi(false)
                        .with_writer(file),
                )
                .init();
            Some(guard)
        }
        _ => {
            tracing_subscriber::fmt()
                .with_writer(std::io::stderr)
                .init();
            None
        }
    };
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
        Command::Serve { state_dir, .. } => {
            let directory = midistaged::paths::state_dir(home.as_deref(), state_dir)?;
            // 別の midistaged（開発用の cargo run など）が動いていれば静かに退く。exit 0 なので
            // launchd の KeepAlive {SuccessfulExit=false} は起こし直さない（design 05）
            let _lock = match RuntimeLock::acquire(&directory) {
                Ok(lock) => lock,
                Err(e) => {
                    tracing::info!(
                        "another midistaged holds {}: {e}; exiting",
                        directory.display()
                    );
                    return Ok(());
                }
            };
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

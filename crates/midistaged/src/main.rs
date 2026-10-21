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
        /// Windows: アプリ用の仮想ポート（bridge）を作る。今の Windows MIDI Services は仮想ポートを閉じると
        /// MIDI サービスが固まることがある（microsoft/MIDI#1047。修正は Windows の更新）。試すときだけ
        #[arg(long)]
        experimental_windows_bridge: bool,
    },
    /// CoreMIDI の物理ポートを列挙する。接続や MIDI 送信は行わない。
    ListPorts,
    /// ROTO-CONTROL の設定を Serial（admin port）で全部読む。読み取りだけ。
    /// 読んでいる間は Roto の MIDI が止まる（数十秒）。結果は JSON で保存する
    RotoDump {
        /// 保存先（既定 ./roto-dump.json）
        #[arg(long, default_value = "roto-dump.json")]
        out: PathBuf,
        /// admin port を明示する（既定は VID/PID で探す）
        #[arg(long)]
        port: Option<String>,
    },
}
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    // serve だけファイルにも書く。guard は main の終わりまで持つ（drop で flush）
    let home = std::env::var_os(midistaged::paths::ROOT_VAR).map(PathBuf::from);
    let _log_guard = match &args.command {
        Command::Serve { log_dir, .. } => {
            let dir = midistaged::paths::log_dir(home.as_deref(), log_dir.clone())?;
            std::fs::create_dir_all(&dir)?;
            let (file, guard) = tracing_appender::non_blocking(tracing_appender::rolling::daily(
                &dir,
                "midistaged.log",
            ));
            // MIDI トレースは別ファイル（trace.log）。通常のログと stderr には混ぜない
            let (trace_file, trace_guard) =
                tracing_appender::non_blocking(tracing_appender::rolling::daily(&dir, "trace.log"));
            use tracing_subscriber::layer::SubscriberExt;
            use tracing_subscriber::util::SubscriberInitExt;
            let directives = std::env::var("RUST_LOG").unwrap_or_else(|_| "info".into());
            tracing_subscriber::registry()
                .with(midistaged::logging::layers(
                    &directives,
                    std::io::stderr,
                    file,
                    trace_file,
                    tracing_subscriber::fmt::time::SystemTime,
                ))
                .init();
            Some((guard, trace_guard))
        }
        _ => {
            tracing_subscriber::fmt()
                .with_writer(std::io::stderr)
                .init();
            None
        }
    };
    match args.command {
        Command::RotoDump { out, port } => {
            #[cfg(not(target_os = "macos"))]
            {
                let _ = (out, port);
                anyhow::bail!("roto-dump is not available on this OS yet (design 07)");
            }
            #[cfg(target_os = "macos")]
            {
                // 同期 I/O（最大 2 分弱）。async の runtime を塞がない
                let dump = tokio::task::spawn_blocking(move || {
                    midistaged::roto_admin::read_all(port.as_deref())
                })
                .await??;
                std::fs::write(&out, serde_json::to_vec_pretty(&dump)?)?;
                let free: Vec<_> = dump
                    .setups
                    .iter()
                    .filter(|s| s.is_free())
                    .map(|s| s.index)
                    .collect();
                println!(
                    "port {} / FW {}.{}.{} ({}) / mode {:?} / current {:?}",
                    dump.port,
                    dump.firmware.major,
                    dump.firmware.minor,
                    dump.firmware.patch,
                    dump.firmware.commit,
                    dump.mode,
                    dump.current_setup
                );
                for s in dump.setups.iter().filter(|s| !s.is_free()) {
                    println!(
                        "  setup {:02}: {:<13} knobs {:2} switches {:2}",
                        s.index,
                        s.name.as_deref().unwrap_or("?"),
                        s.knobs.len(),
                        s.switches.len()
                    );
                }
                println!("free setups ({}): {:?}", free.len(), free);
                println!(
                    "port open {} ms for {} requests (MIDI was paused while open) -> {}",
                    dump.port_open_ms,
                    dump.requests,
                    out.display()
                );
            }
        }
        Command::ListPorts => {
            #[cfg(target_os = "macos")]
            let devices = midistaged::native::discover();
            // Windows MIDI Services（design 07）。API が使えなければ理由を返して失敗する
            #[cfg(windows)]
            let devices = midistaged::winmidi::discover()?;
            // 空の一覧を成功として返さない
            #[cfg(not(any(target_os = "macos", windows)))]
            let devices: std::collections::BTreeMap<
                String,
                midistaged::inventory::DevicePorts,
            > = anyhow::bail!("list-ports has no MIDI backend on this OS yet (design 07)");
            for (id, device) in devices {
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
        Command::Serve {
            state_dir,
            experimental_windows_bridge,
            ..
        } => {
            #[cfg(not(windows))]
            let _ = experimental_windows_bridge;
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
            // CoreMIDI に触る前に。後から挿された機材を拾うための run loop（native.rs）
            #[cfg(target_os = "macos")]
            midistaged::native::start_notify_thread();
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
            let trace_dir = directory.clone();
            midistaged::trace::refresh(&trace_dir);
            // design 06: 設定ポート（Roto の Serial）の用事を 1 つずつ。MIDI の poll を止めないよう別 thread
            // Serial の無い OS では用事ごとに理由を言って失敗する（受け付けたまま終わらせない。design 07）
            {
                let setups = service.runtime.clone();
                let setups_running = running.clone();
                let setups_backup_dir = directory.join("roto-backup");
                std::thread::Builder::new()
                    .name("device-setups".into())
                    .spawn(move || {
                        while setups_running.load(Ordering::Acquire) {
                            let job = {
                                let mut r = setups.lock().expect("runtime mutex poisoned");
                                r.plan_setup_switches();
                                r.next_setup_job()
                            };
                            if let Some(job) = job {
                                let result = match &job.action {
                                    midistaged::runtime::SetupAction::Select => {
                                        midistaged::roto_admin::select_setup(job.setup)
                                    }
                                    midistaged::runtime::SetupAction::Apply(desired) => {
                                        midistaged::roto_admin::apply_setup(
                                            job.setup,
                                            desired,
                                            &setups_backup_dir,
                                        )
                                        .map(|a| {
                                            // Serial を開いている間 MIDI が止まる。所要を残す（間引きの設計に使う）
                                            tracing::info!(
                                                "{}: setup {} written: {} op(s), port open {} ms, {} requests, backup {}",
                                                job.device_id, job.setup, a.ops, a.port_open_ms, a.requests, a.backup.display()
                                            );
                                        })
                                    }
                                }
                                .map_err(|e| e.to_string());
                                setups
                                    .lock()
                                    .expect("runtime mutex poisoned")
                                    .finish_setup_job(&job, result);
                            } else {
                                std::thread::sleep(std::time::Duration::from_millis(50));
                            }
                        }
                    })?;
            }
            let worker = std::thread::spawn(move || {
                #[cfg(target_os = "macos")]
                {
                    let mut native = midistaged::native::NativeRuntime::new(shared);
                    let mut ticks: u32 = 0;
                    while worker_running.load(Ordering::Acquire) {
                        native.poll();
                        // MIDI トレースのフラグは 1 秒ごと（20 × 50 ms）に見る。再起動なしで切り替わる
                        ticks = ticks.wrapping_add(1);
                        if ticks.is_multiple_of(20) {
                            midistaged::trace::refresh(&trace_dir);
                        }
                        std::thread::sleep(std::time::Duration::from_millis(50));
                    }
                }
                // Windows MIDI Services（design 07）。段取りは native_runtime、OS の呼び出しは winmidi
                #[cfg(windows)]
                {
                    let mut native = midistaged::native_runtime::NativeRuntime::new(
                        shared,
                        midistaged::winmidi::Midi2Backend::new(experimental_windows_bridge),
                    );
                    let mut ticks: u32 = 0;
                    while worker_running.load(Ordering::Acquire) {
                        native.poll();
                        ticks = ticks.wrapping_add(1);
                        if ticks.is_multiple_of(20) {
                            midistaged::trace::refresh(&trace_dir);
                        }
                        std::thread::sleep(std::time::Duration::from_millis(50));
                    }
                }
                // MIDI の backend が無い OS。lease と状態だけで動き、機材は waiting のまま
                #[cfg(not(any(target_os = "macos", windows)))]
                {
                    let _ = (worker_running, shared, trace_dir);
                    tracing::warn!(
                        "no MIDI backend on this OS yet (design 07); devices stay waiting"
                    );
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
            // Windows: Ctrl-C / Ctrl-Break、コンソールを閉じる、ログオフ、シャットダウン
            #[cfg(windows)]
            {
                use tokio::signal::windows;
                let mut brk = windows::ctrl_break()?;
                let mut close = windows::ctrl_close()?;
                let mut logoff = windows::ctrl_logoff()?;
                let mut shutdown = windows::ctrl_shutdown()?;
                tokio::select! {
                    result = tokio::signal::ctrl_c() => { result?; },
                    _ = brk.recv() => {},
                    _ = close.recv() => {},
                    _ = logoff.recv() => {},
                    _ = shutdown.recv() => {},
                }
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

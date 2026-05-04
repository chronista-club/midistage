//! midistage CLI binary
//!
//! KORG MIDI 2.0 device configurator。 v0 計画は CLAUDE.md / README.md 参照。

use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "midistage")]
#[command(version)]
#[command(about = "KORG MIDI 2.0 device configurator", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// 接続中の MIDI device 列挙
    ListPorts,
    /// `~/.config/midistage/*.kdl` を列挙
    ListProfiles,
    /// blank KDL profile template を生成 (defaults 埋め込み、 device 接続不要)
    Init { profile: String },
    /// device → file (KORG SysEx Func=0x10/0x0E request → 0x40/0x51 receive)
    Pull {
        profile: Option<String>,
        /// `--scene N` で特定 scene のみ、 `--scene all` で全 scene
        #[arg(long)]
        scene: Option<String>,
        /// global data も pull する
        #[arg(long)]
        global: bool,
    },
    /// file → device (KORG SysEx Func=0x40/0x51)
    Push {
        profile: Option<String>,
        #[arg(long)]
        scene: Option<String>,
        #[arg(long)]
        global: bool,
        /// push 後 Func=0x11 で device の指定 memory slot に焼く (0~7)
        #[arg(long)]
        write_to_memory: Option<u8>,
    },
    /// device pull 結果と file の差分表示 (no write)
    Diff { profile: Option<String> },
    /// active scene 切替 (Func=0x14、 0~15)
    SceneChange { number: u8 },
    /// device mode 切替 (Func=0x00、 normal / native)
    Mode { mode: String },
    /// DAW preset 切替 (Func=0x49、 Logic / Cubase / Live / FL Studio / ...)
    ControllerMode { name: String },
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::ListPorts => {
            // TODO v0-alpha: midistage_core::coremidi_sys を使って source / destination 列挙
            println!("TODO v0-alpha: list-ports");
        }
        Commands::ListProfiles => {
            // TODO v0-alpha: ~/.config/midistage/*.kdl を walk
            println!("TODO v0-alpha: list-profiles");
        }
        Commands::Init { profile } => {
            // TODO v0-alpha: blank KDL template 生成 (config/examples/keystage-default.kdl 参照)
            println!("TODO v0-alpha: init {}", profile);
        }
        Commands::Pull { profile, scene, global } => {
            // TODO v0-alpha: Func=0x10 SysEx → response 受信 → 7-bit decode → Scene struct → KDL
            println!("TODO v0-alpha: pull profile={:?} scene={:?} global={}", profile, scene, global);
        }
        Commands::Push { profile, scene, global, write_to_memory } => {
            // TODO v0-beta: KDL → Scene struct → 500-byte → 7-bit encode → Func=0x40 SysEx
            println!(
                "TODO v0-beta: push profile={:?} scene={:?} global={} mem={:?}",
                profile, scene, global, write_to_memory
            );
        }
        Commands::Diff { profile } => {
            // TODO v0-beta: pull 結果と file 比較
            println!("TODO v0-beta: diff profile={:?}", profile);
        }
        Commands::SceneChange { number } => {
            // TODO v0-gamma: Func=0x14
            println!("TODO v0-gamma: scene-change {}", number);
        }
        Commands::Mode { mode } => {
            // TODO v0-gamma: Func=0x00
            println!("TODO v0-gamma: mode {}", mode);
        }
        Commands::ControllerMode { name } => {
            // TODO v0-gamma: Func=0x49
            println!("TODO v0-gamma: controller-mode {}", name);
        }
    }

    Ok(())
}

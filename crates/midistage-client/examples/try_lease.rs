//! 引き継ぎを手で試すための小さなアプリ（アプリ B の代役。design 07）。
//!
//! 機材の使用を ON にして（必要なら引き継いで）、lease を持っている間は bridge の入力ポートを
//! 開いて届いた MIDI を表示する。Quiesce が来たら入力を閉じて Quiesced を返す。
//! 終わるときは使用を OFF にして手放す。
//!
//! ```text
//! cargo run -p midistage-client --example try_lease -- nanokontrol --takeover --seconds 30
//! ```
use midistage_client::{Client, Endpoint, Event, Hello, Quiesced, SendMidi, SetEnabled, Snapshot};
use std::{path::PathBuf, time::Duration};

struct Args {
    device: String,
    client: String,
    takeover: bool,
    /// lease を取ったら SendMidi で Device Inquiry を送る（応答が入力に返るはず）
    inquiry: bool,
    seconds: u64,
    state_dir: Option<PathBuf>,
}

fn args() -> anyhow::Result<Args> {
    let mut it = std::env::args().skip(1);
    let mut args = Args {
        device: String::new(),
        client: "try-lease".into(),
        takeover: false,
        inquiry: false,
        seconds: 30,
        state_dir: None,
    };
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--client" => args.client = it.next().unwrap_or_default(),
            "--takeover" => args.takeover = true,
            "--inquiry" => args.inquiry = true,
            "--seconds" => args.seconds = it.next().unwrap_or_default().parse()?,
            "--state-dir" => args.state_dir = it.next().map(PathBuf::from),
            device => args.device = device.into(),
        }
    }
    anyhow::ensure!(
        !args.device.is_empty(),
        "usage: try_lease <device_id> [--client ID] [--takeover] [--inquiry] [--seconds N] [--state-dir DIR]"
    );
    Ok(args)
}

/// midistaged の endpoint.json（既定の置き場は midistaged の paths.rs と同じ）
fn endpoint(state_dir: Option<PathBuf>) -> anyhow::Result<Endpoint> {
    let dir = match state_dir {
        Some(dir) => dir,
        None if cfg!(windows) => {
            PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap_or_default()).join("Midistage")
        }
        None => PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
            .join("Library/Application Support/Midistage"),
    };
    let path = dir.join("endpoint.json");
    let bytes = std::fs::read(&path)
        .map_err(|e| anyhow::anyhow!("{}: {e} (is midistaged serve running?)", path.display()))?;
    Ok(serde_json::from_slice(&bytes)?)
}

/// 自分が lease を持っていて、bridge の入力ポートが出ていればその名前
fn my_input(snapshot: &Snapshot, device: &str) -> Option<(String, String)> {
    let view = snapshot
        .devices
        .iter()
        .find(|d| d.state.device_id == device)?;
    let lease = view.state.lease.as_ref()?;
    (lease.session_id == snapshot.session_id)
        .then(|| view.native_ports.inputs.first().cloned())
        .flatten()
        .map(|port| (lease.token.clone(), port))
}

fn open_input(port_name: &str) -> anyhow::Result<midir::MidiInputConnection<()>> {
    let mut input = midir::MidiInput::new("try-lease")?;
    input.ignore(midir::Ignore::None);
    let port = input
        .ports()
        .into_iter()
        .find(|p| input.port_name(p).ok().as_deref() == Some(port_name))
        .ok_or_else(|| anyhow::anyhow!("MIDI input {port_name:?} is not visible to this app"))?;
    input
        .connect(
            &port,
            "try-lease",
            |_, bytes, _| println!("  MIDI {bytes:02x?}"),
            (),
        )
        .map_err(|e| anyhow::anyhow!("{e}"))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = args()?;
    let hello = Hello {
        protocol_version: midistage_client::protocol::PROTOCOL_VERSION,
        client_id: args.client.clone(),
        display_name: args.client.clone(),
        auth_token: String::new(),
        native_midi: true,
        initial_enabled_profiles: vec![],
    };
    let (client, snapshot) = Client::connect(&endpoint(args.state_dir)?, hello).await?;
    let view = snapshot
        .devices
        .iter()
        .find(|d| d.state.device_id == args.device)
        .ok_or_else(|| anyhow::anyhow!("no device {:?}", args.device))?;
    println!(
        "{}: phase {:?}, owner {:?}",
        args.device, view.state.phase, view.state.assignment.client_id
    );
    client
        .set_enabled(&SetEnabled {
            device_id: args.device.clone(),
            enabled: true,
            expected_revision: view.state.assignment.revision,
            takeover: args.takeover,
        })
        .await?;
    let mut opened: Option<(String, midir::MidiInputConnection<()>)> = None;
    let mut last = String::new();
    let mut stopping = false;
    let deadline = tokio::time::sleep(Duration::from_secs(args.seconds));
    tokio::pin!(deadline);
    loop {
        let event = tokio::select! {
            event = client.recv_event() => event?,
            _ = &mut deadline, if !stopping => {
                // 手放す。Quiesce が来たら Quiesced を返して終わる
                stopping = true;
                let snapshot = client.snapshot().await?;
                let revision = snapshot.devices.iter().find(|d| d.state.device_id == args.device)
                    .map(|d| d.state.assignment.revision).unwrap_or_default();
                println!("releasing {}", args.device);
                client.set_enabled(&SetEnabled {
                    device_id: args.device.clone(),
                    enabled: false,
                    expected_revision: revision,
                    takeover: false,
                }).await?;
                continue;
            }
            _ = tokio::time::sleep(Duration::from_secs(5)), if stopping => break,
        };
        match event {
            Event::State { snapshot } => {
                if let Some(view) = snapshot
                    .devices
                    .iter()
                    .find(|d| d.state.device_id == args.device)
                {
                    let line = format!(
                        "{:?} owner {:?} lease {:?} in {:?} out {:?} error {:?}",
                        view.state.phase,
                        view.state.assignment.client_id,
                        view.state.lease.as_ref().map(|l| &l.token),
                        view.native_ports.inputs,
                        view.native_ports.outputs,
                        view.error
                    );
                    if line != last {
                        println!("{}: {line}", args.device);
                        last = line;
                    }
                }
                if opened.is_none()
                    && !stopping
                    && let Some((token, port)) = my_input(&snapshot, &args.device)
                {
                    match open_input(&port) {
                        Ok(connection) => {
                            println!("listening on {port:?}");
                            if args.inquiry
                                && let Some(output) = snapshot
                                    .devices
                                    .iter()
                                    .find(|d| d.state.device_id == args.device)
                                    .and_then(|d| d.native_ports.outputs.first())
                            {
                                let result = client
                                    .send_midi(&SendMidi {
                                        device_id: args.device.clone(),
                                        lease_token: token.clone(),
                                        port_name: output.clone(),
                                        bytes: vec![0xf0, 0x7e, 0x7f, 0x06, 0x01, 0xf7],
                                    })
                                    .await;
                                println!("SendMidi Device Inquiry -> {:?}", result.map(|_| "ok"));
                            }
                            opened = Some((token, connection));
                        }
                        Err(error) => println!("cannot listen yet: {error}"),
                    }
                }
            }
            Event::Quiesce {
                device_id,
                lease_token,
                ..
            } if device_id == args.device => {
                // 入力を止めてから Quiesced（design 03）
                if let Some((_, connection)) = opened.take() {
                    connection.close();
                }
                println!("quiesce {device_id} ({lease_token}) -> input closed, Quiesced");
                client
                    .quiesced(&Quiesced {
                        device_id,
                        lease_token,
                    })
                    .await?;
                if stopping {
                    break;
                }
            }
            _ => {}
        }
    }
    client.close().await?;
    Ok(())
}

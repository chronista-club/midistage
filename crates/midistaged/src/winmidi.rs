//! Windows の OS 境界（design 07）。Windows MIDI Services（Windows.Devices.Midi2）を使う。
//! 実機を開くのは [`WinBridge`] だけで、列挙は read-only。
//!
//! bridge は物理 endpoint 1 つにつき仮想デバイス 1 つ（双方向）。アプリは仮想デバイスの client 側を
//! 開き、サービスが device 側と物理 endpoint の間を中継する。物理側は
//! `WaitForEndpointReceiptOnSend` でつなぐので、送信はドライバが受け取るまで戻らない
//! （= CoreMIDI の SysEx completion 相当。spike 2026-10-19）。
use crate::{
    inventory::{self, DevicePorts, Endpoint, Port},
    io_fence::{IoFence, OutputRequest, OutputSink},
    native_runtime::{self, Backend, Bridge, Finalizers, Shared},
    trace,
    watchdog::Watchdog,
    winmidi_bindings::{
        MidiApi, MidiDeclaredEndpointInfo, MidiEndpointConnection, MidiEndpointConnectionSettings,
        MidiEndpointDeviceInformation, MidiFunctionBlock, MidiFunctionBlockDirection,
        MidiFunctionBlockRepresentsMidi10Connection, MidiFunctionBlockUIHint, MidiGroup,
        MidiMessageReceivedEventArgs, MidiSession, MidiVirtualDevice,
        MidiVirtualDeviceCreationConfig, MidiVirtualDeviceManager,
    },
};
use midistage_core::midi1;
use midistage_protocol::{Lease, wire::NativePorts};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, mpsc},
    thread,
    time::{Duration, Instant},
};
use windows::Foundation::TypedEventHandler;
use windows_core::{HSTRING, Ref};

/// 列挙（FindAll）は 1 回 約 60 ms（実測 2026-10-19）。poll（50 ms）ごとには呼ばない
const LIST_INTERVAL: Duration = Duration::from_secs(1);
/// 仮想デバイスの作成は普段 20〜110 ms（実測 2026-10-19）。戻らなければ midisrv が固まっている
const OPEN_LIMIT: Duration = Duration::from_secs(10);

/// 物理機材ごとのポート。API（dll）や midisrv が使えなければ理由を返す
pub fn discover() -> anyhow::Result<BTreeMap<String, DevicePorts>> {
    Ok(inventory::inventory(&inventory::endpoint_ports(
        &endpoints()?,
    )))
}

fn endpoints() -> anyhow::Result<Vec<Endpoint>> {
    // dll が無いと factory の取得で失敗する（exe の隣に Windows.Devices.Midi2.dll / .pri）
    let available = MidiApi::EnsureServiceAvailable().map_err(|e| {
        anyhow::anyhow!(
            "Windows.Devices.Midi2 is unavailable ({e}); put Windows.Devices.Midi2.dll / .pri next to midistaged.exe (scripts/winmidi.sh fetch, design 07)"
        )
    })?;
    anyhow::ensure!(
        available,
        "Windows MIDI Services (midisrv) is not available"
    );
    let list = MidiEndpointDeviceInformation::FindAll()?.ok_or_else(|| {
        anyhow::anyhow!("MidiEndpointDeviceInformation::FindAll returned nothing")
    })?;
    Ok(list
        .into_iter()
        .map(|e| Endpoint {
            id: e.EndpointDeviceId().to_string(),
            name: e.Name().to_string(),
            transport: e
                .GetTransportSuppliedInfo()
                .map(|t| t.TransportCode().to_string())
                .unwrap_or_default(),
            parent: e
                .GetParentDeviceInformation()
                .map(|p| p.Id().to_string())
                .unwrap_or_default(),
        })
        .collect())
}

/// 列挙の結果: 機材ごとのポートと、送受信で引く uid → endpoint ID
type Listed = (BTreeMap<String, DevicePorts>, BTreeMap<u32, String>);

struct Listing {
    at: Instant,
    result: Result<Listed, String>,
}

/// Windows MIDI Services の backend。session はサービスに 1 つ
#[derive(Default)]
pub struct Midi2Backend {
    /// bridge（仮想デバイス）を作るか。今の midisrv（transport 1.0.15）は仮想デバイスの接続を閉じると
    /// 固まる（microsoft/MIDI#1047、修正は Windows の 11D）ので、明示したときだけ（design 07）
    bridge: bool,
    session: Option<MidiSession>,
    listing: Option<Listing>,
    /// bridge を開く呼び出しは midisrv が固まると戻らない。worker 全体を止めない
    watchdog: Watchdog,
}
impl Midi2Backend {
    pub fn new(bridge: bool) -> Self {
        Self {
            bridge,
            ..Self::default()
        }
    }
}
impl Backend for Midi2Backend {
    type Bridge = WinBridge;
    fn discover(&mut self) -> anyhow::Result<BTreeMap<String, DevicePorts>> {
        if self
            .listing
            .as_ref()
            .is_none_or(|l| l.at.elapsed() >= LIST_INTERVAL)
        {
            let result = endpoints()
                .map(|endpoints| {
                    // 送受信で引く uid → endpoint ID
                    let ids = endpoints
                        .iter()
                        .flat_map(|e| {
                            [true, false]
                                .map(|input| (inventory::endpoint_uid(&e.id, input), e.id.clone()))
                        })
                        .collect();
                    let devices = inventory::inventory(&inventory::endpoint_ports(&endpoints));
                    (devices, ids)
                })
                .map_err(|e| e.to_string());
            self.listing = Some(Listing {
                at: Instant::now(),
                result,
            });
        }
        match &self.listing.as_ref().expect("listed").result {
            Ok((devices, _)) => Ok(devices.clone()),
            Err(error) => Err(anyhow::anyhow!("{error}")),
        }
    }
    fn open(
        &mut self,
        runtime: &Shared,
        device_id: &str,
        lease: Lease,
        ports: &[Port],
    ) -> anyhow::Result<WinBridge> {
        anyhow::ensure!(
            self.bridge,
            "the Windows MIDI bridge is off until the Windows MIDI Services fix (microsoft/MIDI#1047); serve --experimental-windows-bridge to try it (design 07)"
        );
        tracing::debug!("{device_id}: opening a bridge");
        let session = match &self.session {
            Some(session) => session.clone(),
            None => {
                let session = MidiSession::Create(&HSTRING::from("midistaged"))?
                    .ok_or_else(|| anyhow::anyhow!("MidiSession could not be created"))?;
                self.session = Some(session.clone());
                session
            }
        };
        let ids = match self.listing.as_ref().map(|l| &l.result) {
            Some(Ok((_, ids))) => ids.clone(),
            _ => anyhow::bail!("MIDI devices are not listed"),
        };
        let (runtime, device_id, ports) = (runtime.clone(), device_id.to_owned(), ports.to_vec());
        let result = self.watchdog.call("MIDI bridge open", OPEN_LIMIT, move || {
            WinBridge::open(&runtime, session, &ids, &device_id, lease, &ports)
        });
        if result.is_err() {
            // midisrv を再起動すると session は死ぬ。次は作り直す
            self.session = None;
        }
        result
    }
}

/// 物理 endpoint 1 つ分の中継
struct Link {
    physical: MidiEndpointConnection,
    physical_token: Option<i64>,
    virtual_: MidiEndpointConnection,
    virtual_token: Option<i64>,
    _device: MidiVirtualDevice,
}

pub struct WinBridge {
    runtime: Shared,
    session: MidiSession,
    links: Vec<Link>,
    /// 物理 OUT の uid → 物理側の connection（送信 worker と後始末が使う）
    outputs: Arc<BTreeMap<u32, MidiEndpointConnection>>,
    sender: Option<mpsc::SyncSender<OutputRequest>>,
    /// アプリ → 実機の中継が積む先。drop で先に取り上げる（handler が WinRT に残っても worker を終わらせる）
    relay_sender: Arc<Mutex<Option<mpsc::SyncSender<OutputRequest>>>>,
    worker: Option<thread::JoinHandle<()>>,
    fence: Arc<IoFence>,
    ports: NativePorts,
    output_sinks: BTreeMap<String, OutputSink>,
    finalizers: Finalizers,
}

/// UMP の utility（MT 0）と stream（MT F）は endpoint 間の話で、機材の MIDI ではない。中継しない
fn relayed(words: &[u32]) -> bool {
    words.first().is_some_and(|w| !matches!(w >> 28, 0x0 | 0xf))
}

fn words(args: &MidiMessageReceivedEventArgs) -> (u64, Vec<u32>) {
    let mut w = [0u32; 4];
    let n = args.FillWordArray(0, &mut w) as usize;
    (args.Timestamp(), w[..n.min(4)].to_vec())
}

/// MIDI 1.0 メッセージ 1 つを物理 endpoint へ。ドライバが受け取るまで戻らない
fn send_message(connection: &MidiEndpointConnection, bytes: &[u8]) -> anyhow::Result<()> {
    let packets = midi1::encode(bytes);
    anyhow::ensure!(!packets.is_empty(), "not a complete MIDI 1.0 message");
    for packet in packets {
        let result = connection.SendSingleMessageWordArray(0, 0, packet.len() as u8, &packet);
        anyhow::ensure!(
            MidiEndpointConnection::SendMessageSucceeded(result)?,
            "MIDI send failed ({:#x})",
            result.0
        );
    }
    Ok(())
}

fn create_virtual_device(name: &str, instance: &str) -> anyhow::Result<MidiVirtualDevice> {
    // MIDI 1.0 の機材の代わりなので、MIDI 1.0 protocol・group 1 つ・双方向の function block 1 つ
    let info = MidiDeclaredEndpointInfo::CreateInstance(
        &HSTRING::from(name),
        &HSTRING::from(instance),
        true,
        false,
        false,
        false,
        true,
        1,
        1,
        1,
    )?;
    let config = MidiVirtualDeviceCreationConfig::CreateInstance(
        &HSTRING::from(name),
        &HSTRING::from("midistage bridge"),
        &HSTRING::from("midistage"),
        &info,
    )?;
    let block = MidiFunctionBlock::new()?;
    block.SetNumber(0);
    block.SetName(&HSTRING::from(name));
    block.SetIsActive(true);
    block.SetDirection(MidiFunctionBlockDirection::Bidirectional);
    block.SetUIHint(MidiFunctionBlockUIHint::Bidirectional);
    block.SetRepresentsMidi10Connection(
        MidiFunctionBlockRepresentsMidi10Connection::YesBandwidthUnrestricted,
    );
    block.SetFirstGroup(&MidiGroup::CreateInstance(0)?);
    block.SetGroupCount(1);
    config
        .FunctionBlocks()
        .ok_or_else(|| anyhow::anyhow!("virtual device has no function block list"))?
        .Append(&block)?;
    MidiVirtualDeviceManager::CreateVirtualDevice(&config)?
        .ok_or_else(|| anyhow::anyhow!("virtual device could not be created"))
}

impl WinBridge {
    fn open(
        runtime: &Shared,
        session: MidiSession,
        ids: &BTreeMap<u32, String>,
        device_id: &str,
        lease: Lease,
        ports: &[Port],
    ) -> anyhow::Result<Self> {
        let client_id = runtime
            .lock()
            .expect("runtime mutex poisoned")
            .broker
            .client_for_session(&lease.session_id)
            .unwrap_or("unknown")
            .to_owned();
        // 物理 endpoint ごとに IN / OUT をまとめる
        let mut endpoints: BTreeMap<&str, (Option<&Port>, Option<&Port>)> = BTreeMap::new();
        for port in ports {
            let id = ids
                .get(&port.uid)
                .ok_or_else(|| anyhow::anyhow!("port {} disappeared", port.name))?;
            let entry = endpoints.entry(id.as_str()).or_default();
            if port.input {
                entry.0 = Some(port);
            } else {
                entry.1 = Some(port);
            }
        }
        let fence = Arc::new(IoFence::new(device_id.into(), lease.clone()));
        let (sender, receiver) = mpsc::sync_channel(256);
        let finalizers: Finalizers = Arc::default();
        let mut bridge = Self {
            runtime: runtime.clone(),
            session: session.clone(),
            links: vec![],
            outputs: Arc::default(),
            relay_sender: Arc::new(Mutex::new(Some(sender.clone()))),
            sender: Some(sender),
            worker: None,
            fence,
            ports: NativePorts::default(),
            output_sinks: BTreeMap::new(),
            finalizers,
        };
        let mut outputs = BTreeMap::new();
        for (endpoint_id, (input, output)) in endpoints {
            let port = input.or(output).expect("an endpoint has a port");
            let name =
                inventory::windows_bridge_name(&client_id, &lease.token, port.uid, &port.name);
            let instance = format!(
                "midistage-{:08x}",
                inventory::fnv1a32(format!("{client_id}/{}/{}", lease.token, port.uid).as_bytes())
            );
            tracing::debug!("{device_id}: connecting {endpoint_id}");
            let physical = session
                .CreateEndpointConnection2(
                    &HSTRING::from(endpoint_id),
                    &MidiEndpointConnectionSettings::CreateInstance(true)?,
                )
                .ok_or_else(|| anyhow::anyhow!("cannot connect to {}", port.name))?;
            let made = (|| {
                tracing::debug!("{device_id}: creating virtual device {name}");
                let device = create_virtual_device(&name, &instance)?;
                tracing::debug!("{device_id}: connecting the virtual device");
                let virtual_ = session
                    .CreateEndpointConnection(&device.DeviceEndpointDeviceId())
                    .ok_or_else(|| {
                        anyhow::anyhow!("cannot connect to the virtual device {name}")
                    })?;
                anyhow::Ok((device, virtual_))
            })();
            let (device, virtual_) = match made {
                Ok(made) => made,
                Err(error) => {
                    // まだ links に入っていない物理側の接続を session に残さない
                    session.DisconnectEndpointConnection(physical.ConnectionId());
                    return Err(error);
                }
            };
            virtual_.AddMessageProcessingPlugin(&device);
            let mut link = Link {
                physical: physical.clone(),
                physical_token: None,
                virtual_: virtual_.clone(),
                virtual_token: None,
                _device: device,
            };
            if let Some(port) = input {
                link.physical_token =
                    Some(bridge.relay_input(&virtual_, &physical, &client_id, &port.name)?);
                bridge.ports.inputs.push(name.clone());
            }
            if let Some(port) = output {
                link.virtual_token =
                    Some(bridge.relay_output(&virtual_, port.uid, &client_id, &port.name)?);
                outputs.insert(port.uid, physical.clone());
                bridge.output_sinks.insert(
                    name.clone(),
                    OutputSink {
                        destination: port.uid,
                        sender: bridge.sender.as_ref().expect("sender exists").clone(),
                    },
                );
                bridge.ports.outputs.push(name.clone());
            }
            // 先に links へ入れる（途中で失敗しても drop で閉じる）
            bridge.links.push(link);
            tracing::debug!("{device_id}: opening {name}");
            anyhow::ensure!(virtual_.Open(), "cannot open the virtual device {name}");
            tracing::debug!("{device_id}: opening {}", port.name);
            anyhow::ensure!(physical.Open(), "cannot open {}", port.name);
        }
        bridge.outputs = Arc::new(outputs);
        let worker_outputs = bridge.outputs.clone();
        let (worker_runtime, worker_fence) = (runtime.clone(), bridge.fence.clone());
        let worker_finalizers = bridge.finalizers.clone();
        let worker_client = client_id.clone();
        bridge.worker = Some(thread::spawn(move || {
            native_runtime::output_worker(
                worker_runtime,
                worker_fence,
                receiver,
                worker_finalizers,
                worker_client,
                |destination, bytes, permit| {
                    let connection = worker_outputs
                        .get(&destination)
                        .ok_or_else(|| anyhow::anyhow!("destination disappeared"))?;
                    let result = send_message(connection, &bytes);
                    drop(permit);
                    result
                },
            )
        }));
        // MIDI の通り道ができた。トレースの有無に関係なく info に（「どこまで接続が来ているか」の要）
        tracing::info!(
            "{device_id}: virtual ports opened for {client_id} ({}) | in {:?} | out {:?}",
            lease.token,
            bridge.ports.inputs,
            bridge.ports.outputs
        );
        if trace::enabled() {
            trace::bridge_open(
                device_id,
                &client_id,
                &lease.token,
                &bridge.ports.inputs,
                &bridge.ports.outputs,
            );
        }
        Ok(bridge)
    }

    /// 実機 → アプリ。lease が有効な間だけ、元の timestamp のまま UMP を仮想デバイスへ
    fn relay_input(
        &self,
        virtual_: &MidiEndpointConnection,
        physical: &MidiEndpointConnection,
        client_id: &str,
        port_name: &str,
    ) -> anyhow::Result<i64> {
        let (runtime, fence) = (self.runtime.clone(), self.fence.clone());
        let (forward, client, port) =
            (virtual_.clone(), client_id.to_owned(), port_name.to_owned());
        let decoder = Mutex::new(midi1::Decoder::default());
        let handler = TypedEventHandler::new(move |_, args: Ref<MidiMessageReceivedEventArgs>| {
            let Some(args) = args.as_ref() else {
                return Ok(());
            };
            let (timestamp, words) = words(args);
            if !relayed(&words) {
                return Ok(());
            }
            let permit = {
                let r = runtime.lock().expect("runtime mutex poisoned");
                fence.begin(&r.broker)
            };
            // 先に転送し、記録はその後（トレースで遅らせない）
            let verdict = match permit {
                Ok(_permit) => {
                    let result =
                        forward.SendSingleMessageWordArray(timestamp, 0, words.len() as u8, &words);
                    if MidiEndpointConnection::SendMessageSucceeded(result).unwrap_or(false) {
                        "FWD".to_owned()
                    } else {
                        let message = format!("virtual device send failed ({:#x})", result.0);
                        native_runtime::fault(&runtime, &fence, message.clone());
                        format!("FAIL {message}")
                    }
                }
                Err(_) => "DROP no lease".to_owned(),
            };
            let message = decoder.lock().expect("decoder mutex poisoned").push(&words);
            if trace::enabled()
                && let Some(bytes) = message.filter(|b| trace::keep(b))
            {
                trace::port_line(
                    "dev→app",
                    &fence.device_id,
                    &client,
                    &port,
                    &verdict,
                    &bytes,
                );
            }
            Ok(())
        });
        Ok(physical.MessageReceived(&handler)?)
    }

    /// アプリ → 実機。MIDI 1.0 メッセージに組み立て直して送信 worker に積む
    fn relay_output(
        &self,
        virtual_: &MidiEndpointConnection,
        destination: u32,
        client_id: &str,
        port_name: &str,
    ) -> anyhow::Result<i64> {
        let (runtime, fence) = (self.runtime.clone(), self.fence.clone());
        let relay = self.relay_sender.clone();
        let (client, port) = (client_id.to_owned(), port_name.to_owned());
        let decoder = Mutex::new(midi1::Decoder::default());
        let handler = TypedEventHandler::new(move |_, args: Ref<MidiMessageReceivedEventArgs>| {
            let Some(args) = args.as_ref() else {
                return Ok(());
            };
            let (_, words) = words(args);
            if !relayed(&words) {
                return Ok(());
            }
            let Some(bytes) = decoder.lock().expect("decoder mutex poisoned").push(&words) else {
                return Ok(());
            };
            // 記録は送信待ちに積んだ後。bytes は積むと手放すので、オンのときだけ写す
            let traced = trace::enabled().then(|| bytes.clone());
            let request = OutputRequest {
                destination,
                bytes,
                completion: None,
            };
            let queued = match relay.lock().expect("relay mutex poisoned").as_ref() {
                Some(sender) => sender.try_send(request).is_ok(),
                // 閉じている最中。もう積まない
                None => return Ok(()),
            };
            if !queued {
                if trace::enabled() {
                    trace::overflow(&fence.device_id, &client);
                }
                native_runtime::fault(
                    &runtime,
                    &fence,
                    "MIDI output queue overflow; lease stopped".into(),
                );
                return Ok(());
            }
            if let Some(bytes) = traced {
                trace::port_line("app→svc", &fence.device_id, &client, &port, "QUEUE", &bytes);
            }
            Ok(())
        });
        Ok(virtual_.MessageReceived(&handler)?)
    }
}

impl Bridge for WinBridge {
    fn fence(&self) -> &Arc<IoFence> {
        &self.fence
    }
    fn ports(&self) -> &NativePorts {
        &self.ports
    }
    fn output_sinks(&self) -> &BTreeMap<String, OutputSink> {
        &self.output_sinks
    }
    /// 後始末の frame を送る。物理側は受け取り確認付きなので、送信が戻れば吐き切っている
    /// （CoreMIDI の MIDIFlushOutput に当たる手順は要らない）
    fn finish_release(&self, runtime: &Shared) -> anyhow::Result<()> {
        let pending = self
            .finalizers
            .lock()
            .expect("finalizers mutex poisoned")
            .clone();
        for (uid, frame) in pending {
            let permit = {
                let r = runtime.lock().expect("runtime mutex poisoned");
                self.fence.begin_cleanup(&r.broker)?
            };
            // 抜線済みの endpoint には戻すべき接続モードが無い
            if let Some(connection) = self.outputs.get(&uid).filter(|c| c.IsOpen()) {
                send_message(connection, &frame)?;
            }
            drop(permit);
            self.finalizers
                .lock()
                .expect("finalizers mutex poisoned")
                .remove(&uid);
        }
        Ok(())
    }
}

impl Drop for WinBridge {
    fn drop(&mut self) {
        tracing::info!(
            "{}: virtual ports closed ({})",
            self.fence.device_id,
            self.fence.lease.token
        );
        if trace::enabled() {
            trace::bridge_close(&self.fence.device_id, &self.fence.lease.token);
        }
        let device = self.fence.device_id.clone();
        // 入力の中継を止めてから、送信 worker を終わらせる
        for link in &self.links {
            if let Some(token) = link.physical_token {
                let _ = link.physical.RemoveMessageReceived(token);
            }
            if let Some(token) = link.virtual_token {
                let _ = link.virtual_.RemoveMessageReceived(token);
            }
        }
        tracing::debug!("{device}: input handlers removed");
        self.output_sinks.clear();
        self.sender.take();
        self.relay_sender
            .lock()
            .expect("relay mutex poisoned")
            .take();
        // runtime mutex の外で drop すること。worker は認可時に同じ mutex を取る
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        tracing::debug!("{device}: output worker finished");
        while !self.fence.is_idle() {
            thread::sleep(Duration::from_millis(1));
        }
        let needs_finalization = !self
            .finalizers
            .lock()
            .expect("finalizers mutex poisoned")
            .is_empty();
        if needs_finalization && let Err(error) = self.finish_release(&self.runtime) {
            tracing::error!(device = %self.fence.device_id, %error, "device finalization failed during close");
        }
        // 仮想デバイスは device 側の connection を閉じると消える。今の midisrv（transport 1.0.15）は
        // ここで戻らないことがある（microsoft/MIDI#1047、実測 2026-10-19）。別 thread に逃がし、
        // native の worker を止めない。中継と送信はこの時点で止まっている
        let (session, links) = (self.session.clone(), std::mem::take(&mut self.links));
        let spawned = thread::Builder::new()
            .name(format!("close bridge {device}"))
            .spawn(move || {
                for link in links {
                    tracing::debug!("{device}: disconnecting the virtual device");
                    session.DisconnectEndpointConnection(link.virtual_.ConnectionId());
                    tracing::debug!("{device}: disconnecting the physical endpoint");
                    session.DisconnectEndpointConnection(link.physical.ConnectionId());
                }
                tracing::debug!("{device}: disconnected");
            });
        if let Err(error) = spawned {
            tracing::error!("cannot close the MIDI bridge: {error}");
        }
    }
}

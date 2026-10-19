//! OS に依らない native MIDI の段取り（design 07）。機材の出入り、lease ごとの bridge の開閉、
//! 引き継ぎの完了待ち、送信の直列化。OS の呼び出しは [`Backend`] / [`Bridge`] の向こう側。
//!
//! 今は Windows（`winmidi`）が使う。macOS の `native.rs` は同じ段取りを自前で持っており、
//! こちらへの載せ替えは macOS で検証できる別 PR で行う。
use crate::{
    inventory::{DevicePorts, Port},
    io_fence::{InFlight, IoFence, OutputRequest, OutputSink},
    runtime::{NativeBinding, Runtime},
    trace,
};
use midistage_protocol::{Lease, Phase, wire::NativePorts};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, mpsc},
};

pub type Shared = Arc<Mutex<Runtime>>;
/// 引き継ぎの前に送り直す後始末（Keystage の接続モード解除）。物理 destination の uid → frame
pub type Finalizers = Arc<Mutex<BTreeMap<u32, Vec<u8>>>>;

/// OS の MIDI。列挙と bridge の作成
pub trait Backend {
    type Bridge: Bridge;
    /// 物理機材ごとのポート（read-only）
    fn discover(&mut self) -> anyhow::Result<BTreeMap<String, DevicePorts>>;
    /// lease の持ち主専用の仮想ポートを作り、物理ポートとつなぐ。実機を開くのはここだけ
    fn open(
        &mut self,
        runtime: &Shared,
        device_id: &str,
        lease: Lease,
        ports: &[Port],
    ) -> anyhow::Result<Self::Bridge>;
}

/// lease 1 つ分の通り道。drop で閉じる（送信 worker の終わりと in-flight の完了を待つ）
pub trait Bridge {
    fn fence(&self) -> &Arc<IoFence>;
    fn ports(&self) -> &NativePorts;
    fn output_sinks(&self) -> &BTreeMap<String, OutputSink>;
    /// 引き継ぎの最後: 後始末の frame を送り、出力を吐き切る。失敗なら releasing のまま
    fn finish_release(&self, runtime: &Shared) -> anyhow::Result<()>;
}

pub struct NativeRuntime<B: Backend> {
    runtime: Shared,
    backend: B,
    bridges: BTreeMap<String, B::Bridge>,
    inventory: BTreeMap<String, DevicePorts>,
    /// 列挙の失敗（dll が無い等）。同じ理由を毎 poll 書かない
    discover_error: Option<String>,
}
impl<B: Backend> Drop for NativeRuntime<B> {
    fn drop(&mut self) {
        {
            let mut r = self.runtime.lock().expect("runtime mutex poisoned");
            let sessions: Vec<_> = r
                .broker
                .devices
                .values()
                .filter_map(|d| d.lease.as_ref().map(|l| l.session_id.clone()))
                .collect();
            for session in sessions {
                r.disconnect(&session);
            }
            r.output_sinks.clear();
            r.native_bindings.clear();
        }
        // runtime mutex の外で閉じる。worker は認可時に同じ mutex を取る
        self.bridges.clear();
    }
}
impl<B: Backend> NativeRuntime<B> {
    pub fn new(runtime: Shared, backend: B) -> Self {
        Self {
            runtime,
            backend,
            bridges: BTreeMap::new(),
            inventory: BTreeMap::new(),
            discover_error: None,
        }
    }
    /// service の control loop と別 thread で呼ぶ。OS の open / close は runtime mutex の外
    pub fn poll(&mut self) {
        let found = match self.backend.discover() {
            Ok(found) => {
                if self.discover_error.take().is_some() {
                    tracing::info!("MIDI devices can be listed again");
                }
                found
            }
            Err(error) => {
                let error = error.to_string();
                if self.discover_error.as_ref() != Some(&error) {
                    tracing::error!("MIDI devices cannot be listed: {error}");
                    self.discover_error = Some(error);
                }
                BTreeMap::new()
            }
        };
        self.apply_inventory(found);
        let states: Vec<_> = self
            .runtime
            .lock()
            .expect("runtime mutex poisoned")
            .broker
            .devices
            .values()
            .cloned()
            .collect();
        for state in states {
            if state.phase == Phase::Releasing {
                self.try_release(&state.device_id);
            } else if state.phase == Phase::Active
                && !self.bridges.contains_key(&state.device_id)
                && let Some(lease) = state.lease
            {
                self.try_open(&state.device_id, lease);
            }
        }
        // 挿抜・引き継ぎ完了など、この poll で起きた状態の変化を info に
        self.runtime
            .lock()
            .expect("runtime mutex poisoned")
            .log_flow();
    }

    fn apply_inventory(&mut self, found: BTreeMap<String, DevicePorts>) {
        let mut r = self.runtime.lock().expect("runtime mutex poisoned");
        let disappeared: Vec<_> = r
            .broker
            .devices
            .keys()
            .filter(|id| !found.contains_key(*id))
            .cloned()
            .collect();
        for id in disappeared {
            let _ = r.broker.set_present(&id, false);
        }
        for (id, device) in &found {
            if !r.broker.devices.contains_key(id) {
                r.broker.add_device(id, "generic", false);
                r.broker
                    .devices
                    .get_mut(id)
                    .expect("discovered device")
                    .name = device
                    .ports
                    .first()
                    .map(|p| p.name.clone())
                    .unwrap_or_else(|| id.clone());
            }
            if self.inventory.get(id) != Some(device) {
                // ポートが変わったら、開いている bridge は古い。いったん外す
                if self.bridges.contains_key(id) {
                    let _ = r.broker.set_present(id, false);
                }
                if let Some(error) = &device.error {
                    r.driver_errors.insert(id.clone(), error.clone());
                } else {
                    r.driver_errors.remove(id);
                }
            }
            let _ = r.broker.set_present(id, !device.ports.is_empty());
        }
        drop(r);
        self.inventory = found;
    }

    fn try_release(&mut self, device_id: &str) {
        let ready = {
            let r = self.runtime.lock().expect("runtime mutex poisoned");
            r.broker.is_quiescent(device_id)
                && self
                    .bridges
                    .get(device_id)
                    .is_none_or(|b| b.fence().is_idle())
        };
        if !ready {
            return;
        }
        // 後始末の送信は時間がかかりうる。runtime mutex の外で
        if let Some(bridge) = self.bridges.get(device_id)
            && let Err(error) = bridge.finish_release(&self.runtime)
        {
            self.runtime
                .lock()
                .expect("runtime mutex poisoned")
                .driver_errors
                .insert(device_id.into(), error.to_string());
            return;
        }
        let released = {
            let mut r = self.runtime.lock().expect("runtime mutex poisoned");
            let result = match self.bridges.get(device_id) {
                Some(bridge) => bridge.fence().drained(&mut r.broker),
                None => r.broker.drained(device_id),
            };
            if result.is_ok()
                && let Some(binding) = r.native_bindings.remove(device_id)
            {
                for name in binding.ports.outputs {
                    r.output_sinks.remove(&name);
                }
            }
            result.is_ok()
        };
        if released {
            self.bridges.remove(device_id);
        }
    }

    fn try_open(&mut self, device_id: &str, lease: Lease) {
        let native = self
            .runtime
            .lock()
            .expect("runtime mutex poisoned")
            .native_requested(&lease.session_id);
        let Some(device) = self.inventory.get(device_id).filter(|_| native) else {
            return;
        };
        let ports = device.ports.clone();
        match self
            .backend
            .open(&self.runtime, device_id, lease.clone(), &ports)
        {
            Ok(bridge) => {
                {
                    let mut r = self.runtime.lock().expect("runtime mutex poisoned");
                    r.driver_errors.remove(device_id);
                    r.output_sinks.extend(bridge.output_sinks().clone());
                    r.native_bindings.insert(
                        device_id.into(),
                        NativeBinding {
                            lease,
                            ports: bridge.ports().clone(),
                        },
                    );
                }
                self.bridges.insert(device_id.into(), bridge);
            }
            Err(error) => {
                let error = error.to_string();
                let mut r = self.runtime.lock().expect("runtime mutex poisoned");
                // 開けない理由が続く間（Windows の bridge が無効など）、poll ごとに同じ警告を書かない
                if r.driver_errors.get(device_id) != Some(&error) {
                    tracing::warn!(device = %device_id, "MIDI bridge open failed: {error}");
                }
                r.driver_errors.insert(device_id.into(), error);
            }
        }
    }
}

pub fn fault(runtime: &Shared, fence: &IoFence, message: String) {
    runtime
        .lock()
        .expect("runtime mutex poisoned")
        .record_fault(&fence.device_id, &fence.lease, message);
}

/// 送信の直列化（bridge ごとに 1 thread）。lease の世代で認可し、古い lease の送信は捨てる。
/// `send` は物理 destination（uid）へ 1 メッセージ送り、**ドライバが受け取るまで**戻らない。
/// permit は送信が終わるまで `send` が持つ（引き継ぎはそれを待つ）
pub fn output_worker(
    runtime: Shared,
    fence: Arc<IoFence>,
    receiver: mpsc::Receiver<OutputRequest>,
    finalizers: Finalizers,
    client_id: String,
    mut send: impl FnMut(u32, Vec<u8>, InFlight) -> anyhow::Result<()>,
) {
    while let Ok(message) = receiver.recv() {
        let permit = {
            let r = runtime.lock().expect("runtime mutex poisoned");
            fence.begin(&r.broker)
        };
        let via = if message.completion.is_some() {
            "SendMidi"
        } else {
            "port"
        };
        let Ok(permit) = permit else {
            // 古い仮想ポートに溜まった送信は破棄
            if trace::enabled() {
                trace::send_line(
                    &fence.device_id,
                    &client_id,
                    via,
                    "DROP stale lease",
                    &message.bytes,
                );
            }
            if let Some(completion) = message.completion {
                let _ = completion.send(Err("stale_lease".into()));
            }
            continue;
        };
        let traced = trace::enabled().then(|| message.bytes.clone());
        // 成功の記録まで fence を持つ（送信の完了の後）
        let _operation = permit.clone();
        let inverse = midistage_profiles::keystage::disconnect_after(&message.bytes);
        let disconnect = midistage_profiles::keystage::is_disconnect(&message.bytes);
        let result = send(message.destination, message.bytes, permit);
        if result.is_ok() {
            let mut pending = finalizers.lock().expect("finalizers mutex poisoned");
            if let Some(frame) = inverse {
                pending.insert(message.destination, frame);
            } else if disconnect {
                pending.remove(&message.destination);
            }
        }
        if let Some(bytes) = traced {
            match &result {
                Ok(()) => trace::send_line(&fence.device_id, &client_id, via, "SENT", &bytes),
                Err(error) => trace::send_line(
                    &fence.device_id,
                    &client_id,
                    via,
                    &format!("FAIL {error}"),
                    &bytes,
                ),
            }
        }
        if let Err(error) = &result {
            fault(&runtime, &fence, error.to_string());
        }
        if let Some(completion) = message.completion {
            let _ = completion.send(result.map_err(|e| e.to_string()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::SettingsStore;
    use midistage_protocol::{Broker, PROTOCOL_VERSION, wire::Hello};
    use std::sync::atomic::{AtomicBool, Ordering};

    type Log = Arc<Mutex<Vec<String>>>;

    struct Fake {
        devices: Arc<Mutex<anyhow::Result<BTreeMap<String, DevicePorts>>>>,
        log: Log,
        fail_release: Arc<AtomicBool>,
    }
    struct FakeBridge {
        fence: Arc<IoFence>,
        ports: NativePorts,
        sinks: BTreeMap<String, OutputSink>,
        log: Log,
        fail_release: Arc<AtomicBool>,
    }
    impl Backend for Fake {
        type Bridge = FakeBridge;
        fn discover(&mut self) -> anyhow::Result<BTreeMap<String, DevicePorts>> {
            match &*self.devices.lock().unwrap() {
                Ok(devices) => Ok(devices.clone()),
                Err(e) => Err(anyhow::anyhow!("{e}")),
            }
        }
        fn open(
            &mut self,
            _runtime: &Shared,
            device_id: &str,
            lease: Lease,
            ports: &[Port],
        ) -> anyhow::Result<FakeBridge> {
            self.log
                .lock()
                .unwrap()
                .push(format!("open {device_id} {} ports", ports.len()));
            let (sender, _receiver) = mpsc::sync_channel(1);
            let name = format!("Midistage/{}", lease.token);
            Ok(FakeBridge {
                fence: Arc::new(IoFence::new(device_id.into(), lease)),
                ports: NativePorts {
                    inputs: vec![name.clone()],
                    outputs: vec![name.clone()],
                },
                sinks: BTreeMap::from([(
                    name,
                    OutputSink {
                        destination: 2,
                        sender,
                    },
                )]),
                log: self.log.clone(),
                fail_release: self.fail_release.clone(),
            })
        }
    }
    impl Bridge for FakeBridge {
        fn fence(&self) -> &Arc<IoFence> {
            &self.fence
        }
        fn ports(&self) -> &NativePorts {
            &self.ports
        }
        fn output_sinks(&self) -> &BTreeMap<String, OutputSink> {
            &self.sinks
        }
        fn finish_release(&self, _runtime: &Shared) -> anyhow::Result<()> {
            self.log.lock().unwrap().push("finish_release".into());
            anyhow::ensure!(!self.fail_release.load(Ordering::SeqCst), "flush failed");
            Ok(())
        }
    }
    impl Drop for FakeBridge {
        fn drop(&mut self) {
            self.log.lock().unwrap().push("close".into());
        }
    }

    fn nano() -> BTreeMap<String, DevicePorts> {
        let port = |uid, input| Port {
            uid,
            physical_device: 7,
            name: "nanoKONTROL2".into(),
            input,
        };
        BTreeMap::from([(
            "nanokontrol".into(),
            DevicePorts {
                ports: vec![port(1, true), port(2, false)],
                error: None,
            },
        )])
    }
    fn hello(client: &str, native_midi: bool) -> Hello {
        Hello {
            protocol_version: PROTOCOL_VERSION,
            client_id: client.into(),
            display_name: client.into(),
            auth_token: "test".into(),
            native_midi,
            initial_enabled_profiles: vec!["nanokontrol".into()],
        }
    }
    fn setup(dir: &std::path::Path, native_midi: bool) -> (Shared, NativeRuntime<Fake>, Log) {
        let mut broker = Broker::new("boot");
        broker.add_device("nanokontrol", "nanokontrol", false);
        let mut runtime = Runtime::new(broker, SettingsStore::new(dir)).unwrap();
        runtime.hello("s1", &hello("vp", native_midi)).unwrap();
        let shared = Arc::new(Mutex::new(runtime));
        let fake = Fake {
            devices: Arc::new(Mutex::new(Ok(nano()))),
            log: Log::default(),
            fail_release: Arc::default(),
        };
        let log = fake.log.clone();
        (shared.clone(), NativeRuntime::new(shared, fake), log)
    }
    fn phase(shared: &Shared) -> Phase {
        shared.lock().unwrap().broker.devices["nanokontrol"]
            .phase
            .clone()
    }
    fn quiesce(shared: &Shared) {
        let mut r = shared.lock().unwrap();
        let lease = r.broker.devices["nanokontrol"].lease.clone().unwrap();
        r.broker
            .quiesced("nanokontrol", &lease.session_id, &lease.token)
            .unwrap();
    }

    #[test]
    fn an_active_native_lease_gets_one_bridge_and_its_ports_are_registered() {
        let dir = tempfile::tempdir().unwrap();
        let (shared, mut native, log) = setup(dir.path(), true);
        native.poll();
        native.poll();
        assert_eq!(phase(&shared), Phase::Active);
        assert_eq!(*log.lock().unwrap(), ["open nanokontrol 2 ports"]);
        let r = shared.lock().unwrap();
        let binding = &r.native_bindings["nanokontrol"];
        assert_eq!(binding.ports.outputs.len(), 1);
        assert!(r.output_sinks.contains_key(&binding.ports.outputs[0]));
    }

    #[test]
    fn a_client_without_native_midi_gets_no_bridge() {
        let dir = tempfile::tempdir().unwrap();
        let (shared, mut native, log) = setup(dir.path(), false);
        native.poll();
        assert_eq!(phase(&shared), Phase::Active);
        assert!(log.lock().unwrap().is_empty());
    }

    #[test]
    fn unplug_waits_for_quiesced_then_finishes_and_closes_the_bridge() {
        let dir = tempfile::tempdir().unwrap();
        let (shared, mut native, log) = setup(dir.path(), true);
        native.poll();
        *native.backend.devices.lock().unwrap() = Ok(BTreeMap::new());
        native.poll();
        // Quiesced の前は閉じない
        assert_eq!(phase(&shared), Phase::Releasing);
        assert_eq!(log.lock().unwrap().len(), 1);
        quiesce(&shared);
        native.poll();
        assert_eq!(
            *log.lock().unwrap(),
            ["open nanokontrol 2 ports", "finish_release", "close"]
        );
        let r = shared.lock().unwrap();
        assert!(r.native_bindings.is_empty());
        assert!(r.output_sinks.is_empty());
        assert_ne!(r.broker.devices["nanokontrol"].phase, Phase::Releasing);
    }

    #[test]
    fn a_failed_finish_keeps_releasing_and_reports_the_driver_error() {
        let dir = tempfile::tempdir().unwrap();
        let (shared, mut native, log) = setup(dir.path(), true);
        native.poll();
        native.backend.fail_release.store(true, Ordering::SeqCst);
        *native.backend.devices.lock().unwrap() = Ok(BTreeMap::new());
        native.poll();
        quiesce(&shared);
        native.poll();
        assert_eq!(phase(&shared), Phase::Releasing);
        assert!(!log.lock().unwrap().contains(&"close".to_string()));
        assert_eq!(
            shared.lock().unwrap().driver_errors["nanokontrol"],
            "flush failed"
        );
        // 直れば先へ進む
        native.backend.fail_release.store(false, Ordering::SeqCst);
        native.poll();
        assert!(log.lock().unwrap().contains(&"close".to_string()));
    }

    #[test]
    fn a_listing_failure_means_no_devices_not_a_crash() {
        let dir = tempfile::tempdir().unwrap();
        let (shared, mut native, log) = setup(dir.path(), true);
        *native.backend.devices.lock().unwrap() = Err(anyhow::anyhow!("dll missing"));
        native.poll();
        native.poll();
        assert!(!shared.lock().unwrap().broker.devices["nanokontrol"].present);
        assert!(log.lock().unwrap().is_empty());
        *native.backend.devices.lock().unwrap() = Ok(nano());
        native.poll();
        assert_eq!(*log.lock().unwrap(), ["open nanokontrol 2 ports"]);
    }
}

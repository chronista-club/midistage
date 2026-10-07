//! OS 境界。実機を開くのは NativeBridge::open だけで、列挙は read-only。
use crate::{
    inventory::{self, DevicePorts, Port},
    io_fence::{InFlight, IoFence, OutputRequest, OutputSink},
    midi_stream::MidiStream,
    runtime::{NativeBinding, Runtime},
};
use coremidi::{
    Client, InputPort, OutputPort, PacketBuffer, Source, VirtualDestination, VirtualSource,
};
use midistage_protocol::{Lease, Phase, wire::NativePorts};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, mpsc},
    thread,
};

type Shared = Arc<Mutex<Runtime>>;

fn os<T>(result: Result<T, i32>) -> anyhow::Result<T> {
    result.map_err(|status| anyhow::anyhow!("CoreMIDI status {status}"))
}
fn raw_endpoint(uid: u32) -> anyhow::Result<u32> {
    let mut object = 0;
    let mut kind = 0;
    os(unsafe {
        match coremidi_sys::MIDIObjectFindByUniqueID(uid as i32, &mut object, &mut kind) {
            0 => Ok(object),
            status => Err(status),
        }
    })
}
fn physical_device(uid: u32) -> Option<u32> {
    let endpoint = raw_endpoint(uid).ok()?;
    let mut entity = 0;
    let mut device = 0;
    let mut id = 0;
    unsafe {
        if coremidi_sys::MIDIEndpointGetEntity(endpoint, &mut entity) != 0 || entity == 0 {
            return None;
        }
        if coremidi_sys::MIDIEntityGetDevice(entity, &mut device) != 0 || device == 0 {
            return None;
        }
        if coremidi_sys::MIDIObjectGetIntegerProperty(
            device,
            coremidi_sys::kMIDIPropertyUniqueID,
            &mut id,
        ) != 0
        {
            return None;
        }
    }
    Some(id as u32)
}
pub fn discover() -> BTreeMap<String, DevicePorts> {
    let mut ports = Vec::new();
    for source in coremidi::Sources {
        if source.get_property_boolean("offline").unwrap_or(false) {
            continue;
        }
        if let Some(uid) = source.unique_id()
            && let Some(device) = physical_device(uid)
        {
            ports.push(Port {
                uid,
                physical_device: device,
                name: source.display_name().unwrap_or_default(),
                input: true,
            });
        }
    }
    for destination in coremidi::Destinations {
        if destination.get_property_boolean("offline").unwrap_or(false) {
            continue;
        }
        if let Some(uid) = destination.unique_id()
            && let Some(device) = physical_device(uid)
        {
            ports.push(Port {
                uid,
                physical_device: device,
                name: destination.display_name().unwrap_or_default(),
                input: false,
            });
        }
    }
    inventory::inventory(&ports)
}

struct SysExSend {
    request: coremidi_sys::MIDISysexSendRequest,
    _bytes: Vec<u8>,
    _permit: InFlight,
    done: mpsc::Sender<()>,
}
unsafe extern "C" fn sysex_completed(request: *mut coremidi_sys::MIDISysexSendRequest) {
    // CoreMIDI は成功時に一度だけ callback。Box と byte buffer はそれまで保持する。
    let context = unsafe { Box::from_raw((*request).completionRefCon.cast::<SysExSend>()) };
    let _ = context.done.send(());
}
fn send_sysex(destination: u32, bytes: Vec<u8>, permit: InFlight) -> anyhow::Result<()> {
    let (done, completed) = mpsc::channel();
    let mut context = Box::new(SysExSend {
        request: coremidi_sys::MIDISysexSendRequest {
            destination,
            data: bytes.as_ptr(),
            bytesToSend: bytes.len() as u32,
            complete: 0,
            reserved: [0; 3],
            completionProc: Some(sysex_completed),
            completionRefCon: std::ptr::null_mut(),
        },
        _bytes: bytes,
        _permit: permit,
        done,
    });
    context.request.completionRefCon = (&mut *context as *mut SysExSend).cast();
    let raw = Box::into_raw(context);
    let status = unsafe { coremidi_sys::MIDISendSysex(&mut (*raw).request) };
    if status != 0 {
        // 失敗時は所有権が OS に渡らず completion は来ない。
        unsafe {
            drop(Box::from_raw(raw));
        }
        anyhow::bail!("MIDISendSysex status {status}");
    }
    completed
        .recv()
        .map_err(|_| anyhow::anyhow!("SysEx completion lost"))?;
    Ok(())
}
fn fault(runtime: &Shared, fence: &IoFence, message: String) {
    let mut r = runtime.lock().expect("runtime mutex poisoned");
    if r.broker
        .devices
        .get(&fence.device_id)
        .and_then(|d| d.lease.as_ref())
        == Some(&fence.lease)
    {
        r.driver_errors.insert(fence.device_id.clone(), message);
        let _ = r.broker.set_present(&fence.device_id, false);
    }
}
fn output_worker(
    runtime: Shared,
    fence: Arc<IoFence>,
    output: OutputPort,
    receiver: mpsc::Receiver<OutputRequest>,
) {
    while let Ok(message) = receiver.recv() {
        let permit = {
            let r = runtime.lock().expect("runtime mutex poisoned");
            fence.begin(&r.broker)
        };
        let Ok(permit) = permit else {
            if let Some(completion) = message.completion {
                let _ = completion.send(Err("stale_lease".into()));
            }
            continue;
        }; // 古い仮想ポートに溜まった送信は破棄。
        let result = if message.bytes.first() == Some(&0xf0) {
            raw_endpoint(message.destination)
                .and_then(|destination| send_sysex(destination, message.bytes, permit))
        } else {
            let result = coremidi::Destinations
                .into_iter()
                .find(|d| d.unique_id() == Some(message.destination))
                .ok_or_else(|| anyhow::anyhow!("destination disappeared"))
                .and_then(|destination| {
                    os(output.send(&destination, &PacketBuffer::new(0, &message.bytes)))
                });
            drop(permit);
            result
        };
        if let Err(error) = &result {
            fault(&runtime, &fence, error.to_string());
        }
        if let Some(completion) = message.completion {
            let _ = completion.send(result.map_err(|e| e.to_string()));
        }
    }
}

pub struct NativeBridge {
    client: Client,
    inputs: Vec<InputPort>,
    sources: Vec<Arc<VirtualSource>>,
    outputs: Vec<VirtualDestination>,
    sender: Option<mpsc::SyncSender<OutputRequest>>,
    worker: Option<thread::JoinHandle<()>>,
    pub fence: Arc<IoFence>,
    pub ports: NativePorts,
    output_sinks: BTreeMap<String, OutputSink>,
    physical_ports: Vec<Port>,
}
impl NativeBridge {
    pub fn open(
        runtime: Shared,
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
        let client = os(Client::new("Midistage"))?;
        let output = os(client.output_port("Midistage physical output"))?;
        let fence = Arc::new(IoFence::new(device_id.into(), lease.clone()));
        let (sender, receiver) = mpsc::sync_channel(256);
        let worker_runtime = runtime.clone();
        let worker_fence = fence.clone();
        let worker =
            thread::spawn(move || output_worker(worker_runtime, worker_fence, output, receiver));
        let mut bridge = Self {
            client,
            inputs: vec![],
            sources: vec![],
            outputs: vec![],
            sender: Some(sender),
            worker: Some(worker),
            fence,
            ports: NativePorts::default(),
            output_sinks: BTreeMap::new(),
            physical_ports: ports.to_vec(),
        };
        for port in ports {
            let name = format!(
                "Midistage/{}/{}/{} | {}",
                client_id, lease.token, port.uid, port.name
            );
            if port.input {
                let source = Source::from_unique_id(port.uid)
                    .ok_or_else(|| anyhow::anyhow!("source disappeared"))?;
                let virtual_source = Arc::new(os(bridge.client.virtual_source(&name))?);
                bridge
                    .ports
                    .inputs
                    .push(virtual_source.display_name().unwrap_or(name));
                let forward = virtual_source.clone();
                let input_runtime = runtime.clone();
                let input_fence = bridge.fence.clone();
                let input =
                    os(bridge
                        .client
                        .input_port("Midistage physical input", move |packets| {
                            let permit = {
                                let r = input_runtime.lock().expect("runtime mutex poisoned");
                                input_fence.begin(&r.broker)
                            };
                            if let Ok(_permit) = permit {
                                // 元の host timestamp / packet 境界をそのまま仮想 source に渡す。
                                if let Err(status) = forward.received(packets) {
                                    fault(
                                        &input_runtime,
                                        &input_fence,
                                        format!("MIDIReceived status {status}"),
                                    );
                                }
                            }
                        }))?;
                os(input.connect_source(&source))?;
                bridge.inputs.push(input);
                bridge.sources.push(virtual_source);
            } else {
                let sender = bridge.sender.as_ref().expect("sender exists").clone();
                let output_runtime = runtime.clone();
                let output_fence = bridge.fence.clone();
                let uid = port.uid;
                let mut stream = MidiStream::default();
                let destination = os(bridge.client.virtual_destination(&name, move |packets| {
                    for packet in packets.iter() {
                        match stream.push(packet.data()) {
                            Ok(messages) => {
                                for bytes in messages {
                                    if sender
                                        .try_send(OutputRequest {
                                            destination: uid,
                                            bytes,
                                            completion: None,
                                        })
                                        .is_err()
                                    {
                                        fault(
                                            &output_runtime,
                                            &output_fence,
                                            "MIDI output queue overflow; lease stopped".into(),
                                        );
                                        return;
                                    }
                                }
                            }
                            Err(error) => {
                                fault(&output_runtime, &output_fence, error.to_string());
                                return;
                            }
                        }
                    }
                }))?;
                let display_name = destination.display_name().unwrap_or(name);
                bridge.output_sinks.insert(
                    display_name.clone(),
                    OutputSink {
                        destination: uid,
                        sender: bridge.sender.as_ref().expect("sender exists").clone(),
                    },
                );
                bridge.ports.outputs.push(display_name);
                bridge.outputs.push(destination);
            }
        }
        Ok(bridge)
    }
    fn flush(&self) -> anyhow::Result<()> {
        for port in self.physical_ports.iter().filter(|p| !p.input) {
            // 抜線済み endpoint は送信先自体が消えている。
            if let Ok(endpoint) = raw_endpoint(port.uid) {
                let status = unsafe { coremidi_sys::MIDIFlushOutput(endpoint) };
                if status != 0 {
                    anyhow::bail!("MIDIFlushOutput status {status}");
                }
            }
        }
        Ok(())
    }
}
impl Drop for NativeBridge {
    fn drop(&mut self) {
        self.inputs.clear();
        self.outputs.clear();
        self.sources.clear();
        self.output_sinks.clear();
        self.sender.take();
        // runtime mutex の外で drop すること。worker は認可時に同じ mutex を取る。
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

pub struct NativeRuntime {
    runtime: Shared,
    bridges: BTreeMap<String, NativeBridge>,
    inventory: BTreeMap<String, DevicePorts>,
}
impl Drop for NativeRuntime {
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
        self.bridges.clear();
    }
}
impl NativeRuntime {
    pub fn new(runtime: Shared) -> Self {
        Self {
            runtime,
            bridges: BTreeMap::new(),
            inventory: BTreeMap::new(),
        }
    }
    /// service の control loop と別 thread で呼ぶ。CoreMIDI open/close は mutex 外。
    pub fn poll(&mut self) {
        let found = discover();
        {
            let mut r = self.runtime.lock().expect("runtime mutex poisoned");
            for (id, device) in &found {
                if self.inventory.get(id) != Some(device) {
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
        }
        self.inventory = found;
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
                let mut released = false;
                {
                    let mut r = self.runtime.lock().expect("runtime mutex poisoned");
                    if r.broker.is_quiescent(&state.device_id) {
                        let result = if let Some(bridge) = self.bridges.get(&state.device_id) {
                            if bridge.fence.is_idle() {
                                match bridge.flush() {
                                    Ok(()) => bridge.fence.drained(&mut r.broker),
                                    Err(error) => {
                                        r.driver_errors
                                            .insert(state.device_id.clone(), error.to_string());
                                        Err(midistage_protocol::AccessError::Pending)
                                    }
                                }
                            } else {
                                Err(midistage_protocol::AccessError::Pending)
                            }
                        } else {
                            r.broker.drained(&state.device_id)
                        };
                        if result.is_ok() {
                            if let Some(binding) = r.native_bindings.remove(&state.device_id) {
                                for name in binding.ports.outputs {
                                    r.output_sinks.remove(&name);
                                }
                            }
                            released = true;
                        }
                    }
                }
                if released {
                    self.bridges.remove(&state.device_id);
                }
            } else if state.phase == Phase::Active && !self.bridges.contains_key(&state.device_id) {
                let Some(lease) = state.lease else {
                    continue;
                };
                let native = self
                    .runtime
                    .lock()
                    .expect("runtime mutex poisoned")
                    .native_requested(&lease.session_id);
                if !native {
                    continue;
                }
                let Some(device) = self.inventory.get(&state.device_id) else {
                    continue;
                };
                match NativeBridge::open(
                    self.runtime.clone(),
                    &state.device_id,
                    lease.clone(),
                    &device.ports,
                ) {
                    Ok(bridge) => {
                        {
                            let mut r = self.runtime.lock().expect("runtime mutex poisoned");
                            r.driver_errors.remove(&state.device_id);
                            r.output_sinks.extend(bridge.output_sinks.clone());
                            r.native_bindings.insert(
                                state.device_id.clone(),
                                NativeBinding {
                                    lease,
                                    ports: bridge.ports.clone(),
                                },
                            );
                        }
                        self.bridges.insert(state.device_id, bridge);
                    }
                    Err(error) => {
                        self.runtime
                            .lock()
                            .expect("runtime mutex poisoned")
                            .driver_errors
                            .insert(state.device_id, error.to_string());
                    }
                }
            }
        }
    }
}

use crate::runtime::Runtime;
use midistage_protocol::{AccessError, PROTOCOL_VERSION, wire::*};
use std::sync::{Arc, Mutex};
use unison::network::{
    InternalMeshKeypair, MessageType, ProtocolServer, TrustAnchors, channel::UnisonChannel,
    server::ServerHandle,
};

pub struct Service {
    pub endpoint: Endpoint,
    pub runtime: Arc<Mutex<Runtime>>,
    handle: ServerHandle,
}
impl Service {
    pub async fn start(runtime: Runtime) -> anyhow::Result<Self> {
        let epoch = runtime.broker.epoch().to_owned();
        let runtime = Arc::new(Mutex::new(runtime));
        let token = uuid::Uuid::new_v4().to_string();
        let pair = InternalMeshKeypair::generate(["localhost".into(), "::1".into()])?;
        let TrustAnchors::Custom(certs) = pair.client_trust_anchors else {
            unreachable!()
        };
        let server = Arc::new(ProtocolServer::with_identity(
            "midistage",
            "1.0.0",
            "club.chronista.midistage",
        ));
        server
            .enable_discovery(include_str!("../../../schemas/midistage.kdl"))
            .await?;
        let shared = runtime.clone();
        let auth = token.clone();
        server.register_channel(CHANNEL, move |_ctx, stream| {
            let shared = shared.clone();
            let auth = auth.clone();
            async move {
                let session = uuid::Uuid::new_v4().to_string();
                let channel = UnisonChannel::new(stream);
                let mut ticker = tokio::time::interval(std::time::Duration::from_millis(200));
                ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                let mut previous = None;
                let mut ticks = 0_u64;
                loop {
                    tokio::select! {
                        message = channel.recv() => {
                            let Ok(message) = message else { break; };
                            if message.msg_type != MessageType::Request { continue; }
                            let reply = if message.method == "SendMidi" {
                                send_midi(&shared, &session, message.payload_as_value().unwrap_or_default()).await
                            } else {
                                let mut runtime = shared.lock().expect("runtime mutex poisoned");
                                dispatch(&mut runtime, &session, &auth, &message.method, message.payload_as_value().unwrap_or_default())
                            };
                            if channel.send_response(message.id, &message.method, &reply).await.is_err() { break; }
                        }
                        _ = ticker.tick() => {
                            let current = {
                                let runtime = shared.lock().expect("runtime mutex poisoned");
                                runtime.broker.client_for_session(&session).map(|_| snapshot(&runtime, &session))
                            };
                            let Some(current) = current else { continue; };
                            ticks += 1;
                            // Unison events は混雑時 best-effort。状態と停止要求は再送可能にする。
                            if previous.as_ref() != Some(&current) || ticks.is_multiple_of(5) {
                                if channel.send_event("Event", &Event::State { snapshot: current.clone() }).await.is_err() { break; }
                                previous = Some(current.clone());
                            }
                            let mut closed = false;
                            for d in &current.devices {
                                if d.state.phase == midistage_protocol::Phase::Releasing
                                    && let Some(lease) = &d.state.lease
                                        && lease.session_id == session && channel.send_event("Event", &Event::Quiesce {
                                            device_id: d.state.device_id.clone(), profile_id: d.state.profile_id.clone(), lease_token: lease.token.clone(),
                                        }).await.is_err() { closed = true; break; }
                            }
                            if closed { break; }
                        }
                    }
                }
                shared.lock().expect("runtime mutex poisoned").disconnect(&session);
                Ok(())
            }
        }).await;
        let handle = server
            .listener("[::1]:0")
            .cert(pair.server_cert_source)
            .spawn()
            .await?;
        let endpoint = Endpoint {
            protocol_version: PROTOCOL_VERSION,
            port: handle.local_addr().port(),
            server_epoch: epoch,
            certificate_der: certs[0].as_ref().to_vec(),
            auth_token: token,
        };
        Ok(Self {
            endpoint,
            runtime,
            handle,
        })
    }
    pub async fn shutdown(self) -> anyhow::Result<()> {
        self.handle.shutdown().await?;
        Ok(())
    }
}

pub fn snapshot(runtime: &Runtime, session: &str) -> Snapshot {
    Snapshot {
        sequence: runtime
            .snapshot_sequence
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        protocol_version: PROTOCOL_VERSION,
        server_epoch: runtime.broker.epoch().into(),
        session_id: session.into(),
        devices: runtime
            .broker
            .devices
            .values()
            .map(|device| {
                let mut state = device.clone();
                let binding = runtime
                    .native_bindings
                    .get(&state.device_id)
                    .filter(|b| Some(&b.lease) == state.lease.as_ref());
                let mut ports = NativePorts::default();
                if state.phase == midistage_protocol::Phase::Active
                    && let Some(lease) = &state.lease
                {
                    if let Some(binding) = binding {
                        if lease.session_id == session {
                            ports = binding.ports.clone();
                        }
                    } else if runtime.native_requested(&lease.session_id) {
                        // 所有権の予約後、実ポートの準備ができるまで active を公開しない。
                        state.phase = midistage_protocol::Phase::Waiting;
                        state.lease = None;
                    }
                }
                let error = runtime.driver_errors.get(&state.device_id).cloned();
                if error.is_some()
                    && matches!(
                        state.phase,
                        midistage_protocol::Phase::Active | midistage_protocol::Phase::Waiting
                    )
                {
                    state.phase = midistage_protocol::Phase::Error;
                    ports = NativePorts::default();
                }
                DeviceView {
                    state,
                    controls: vec![],
                    native_ports: ports,
                    error,
                }
            })
            .collect(),
    }
}

async fn send_midi(
    shared: &Arc<Mutex<Runtime>>,
    session: &str,
    payload: serde_json::Value,
) -> Reply {
    let prepared = (|| -> anyhow::Result<_> {
        let runtime = shared.lock().expect("runtime mutex poisoned");
        let request: SendMidi = serde_json::from_value(payload)?;
        runtime
            .broker
            .authorize_output(&request.device_id, session, &request.lease_token)?;
        let binding = runtime
            .native_bindings
            .get(&request.device_id)
            .filter(|b| b.lease.session_id == session && b.lease.token == request.lease_token)
            .ok_or_else(|| anyhow::anyhow!("native output unavailable"))?;
        anyhow::ensure!(
            binding.ports.outputs.contains(&request.port_name),
            "output does not belong to device"
        );
        anyhow::ensure!(
            !request.bytes.is_empty() && request.bytes.len() <= 131_072,
            "invalid MIDI message size"
        );
        let messages = crate::midi_stream::MidiStream::default().push(&request.bytes)?;
        anyhow::ensure!(
            messages == [request.bytes.clone()],
            "expected one complete MIDI message"
        );
        let sink = runtime
            .output_sinks
            .get(&request.port_name)
            .ok_or_else(|| anyhow::anyhow!("native output unavailable"))?;
        let (completion, received) = tokio::sync::oneshot::channel();
        sink.sender
            .try_send(crate::io_fence::OutputRequest {
                destination: sink.destination,
                bytes: request.bytes,
                completion: Some(completion),
            })
            .map_err(|_| anyhow::anyhow!("output queue unavailable"))?;
        Ok(received)
    })();
    let completion = match prepared {
        Ok(completion) => completion,
        Err(reason) => return failure(&reason),
    };
    // Driver completion must not hold the broker lock; the driver rechecks its lease.
    match completion.await {
        Ok(Ok(())) => Reply {
            snapshot: Some(snapshot(
                &shared.lock().expect("runtime mutex poisoned"),
                session,
            )),
            error: None,
        },
        Ok(Err(message)) => error(
            if message == "stale_lease" {
                "stale_lease"
            } else {
                "driver_error"
            },
            &message,
        ),
        Err(_) => error("driver_error", "output worker stopped before completion"),
    }
}

fn dispatch(
    runtime: &mut Runtime,
    session: &str,
    auth: &str,
    method: &str,
    payload: serde_json::Value,
) -> Reply {
    let authenticated = runtime.broker.client_for_session(session).is_some();
    if !authenticated && method != "Hello" {
        return error("unauthenticated", "Hello is required");
    }
    let result = (|| -> anyhow::Result<()> {
        match method {
            "Hello" => {
                let request: Hello = serde_json::from_value(payload)?;
                anyhow::ensure!(request.auth_token == auth, "unauthenticated");
                runtime.hello(session, &request)?;
            }
            "Snapshot" => {}
            "SetEnabled" => runtime.set_enabled(session, &serde_json::from_value(payload)?)?,
            "Quiesced" => {
                let request: Quiesced = serde_json::from_value(payload)?;
                runtime
                    .broker
                    .quiesced(&request.device_id, session, &request.lease_token)?;
                // I/O adapter の in-flight 完了を別途確認するまで drained は呼ばない。
            }
            "Present" => {
                let request: Present = serde_json::from_value(payload)?;
                runtime.broker.authorize_output(
                    &request.device_id,
                    session,
                    &request.lease_token,
                )?;
                anyhow::bail!("presentation adapter unavailable");
            }
            _ => anyhow::bail!("unknown method"),
        }
        Ok(())
    })();
    match result {
        Ok(()) => Reply {
            snapshot: Some(snapshot(runtime, session)),
            error: None,
        },
        Err(reason) => failure(&reason),
    }
}
fn failure(reason: &anyhow::Error) -> Reply {
    let code = match reason.downcast_ref::<AccessError>() {
        Some(AccessError::Conflict) => "conflict",
        Some(AccessError::StaleRevision) => "stale_revision",
        Some(AccessError::StaleLease) => "stale_lease",
        Some(AccessError::Pending) => "pending",
        Some(AccessError::AlreadyConnected) => "already_connected",
        Some(AccessError::UnknownDevice) => "unknown_device",
        Some(AccessError::UnknownSession) => "unauthenticated",
        None if reason.to_string() == "unauthenticated" => "unauthenticated",
        None => "invalid_request",
    };
    error(code, &reason.to_string())
}

fn error(code: &str, message: &str) -> Reply {
    Reply {
        snapshot: None,
        error: Some(ProtocolError {
            code: code.into(),
            message: message.into(),
        }),
    }
}

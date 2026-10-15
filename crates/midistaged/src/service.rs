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
    handle_v4: ServerHandle,
}
impl Service {
    pub async fn start(runtime: Runtime) -> anyhow::Result<Self> {
        let epoch = runtime.broker.epoch().to_owned();
        let runtime = Arc::new(Mutex::new(runtime));
        let token = uuid::Uuid::new_v4().to_string();
        let pair =
            InternalMeshKeypair::generate(["localhost".into(), "::1".into(), "127.0.0.1".into()])?;
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
        // IPv6 と IPv4 の両方の loopback で同じポートを待つ。client（Rust SDK / VP）は `localhost` に
        // 繋ぎ、unison の client は名前解決の最初の候補しか試さないので、IPv6 の無いネットワークでは
        // 127.0.0.1 に向かう。[::1]:0 で動的ポートを取り、同じポートを 127.0.0.1 でも bind する。
        // 取れなければ両方捨てて取り直す（Swift SDK は ::1 固定なので [::1] 側は必ず要る）。
        let (handle, handle_v4) = {
            let mut found = None;
            let mut last_error = None;
            for _ in 0..8 {
                let h6 = server
                    .clone()
                    .listener("[::1]:0")
                    .cert(pair.server_cert_source.clone())
                    .spawn()
                    .await?;
                let port = h6.local_addr().port();
                match server
                    .clone()
                    .listener(&format!("127.0.0.1:{port}"))
                    .cert(pair.server_cert_source.clone())
                    .spawn()
                    .await
                {
                    Ok(h4) => {
                        found = Some((h6, h4));
                        break;
                    }
                    Err(e) => {
                        h6.shutdown().await?;
                        last_error = Some(e);
                    }
                }
            }
            found.ok_or_else(|| anyhow::anyhow!("loopback の bind に失敗: {last_error:?}"))?
        };
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
            handle_v4,
        })
    }
    pub async fn shutdown(self) -> anyhow::Result<()> {
        self.handle.shutdown().await?;
        self.handle_v4.shutdown().await?;
        Ok(())
    }
}

/// サービス自身の版と実行ファイル。実行ファイルは一度だけ引く
fn service_info() -> midistage_protocol::wire::ServiceInfo {
    static EXECUTABLE: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    let executable = EXECUTABLE.get_or_init(|| {
        std::env::current_exe()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default()
    });
    midistage_protocol::wire::ServiceInfo {
        version: env!("CARGO_PKG_VERSION").into(),
        executable: executable.clone(),
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
        service: service_info(),
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
    let device = payload
        .get("device_id")
        .and_then(|v| v.as_str())
        .map(crate::logging::printable)
        .unwrap_or_else(|| "?".into());
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
        Err(reason) => {
            warn_send_rejection(shared, session, &device, &reason.to_string());
            return failure(&reason);
        }
    };
    // Driver completion must not hold the broker lock; the driver rechecks its lease.
    match completion.await {
        Ok(Ok(())) => {
            let mut runtime = shared.lock().expect("runtime mutex poisoned");
            runtime.send_succeeded(session, &device);
            Reply {
                snapshot: Some(snapshot(&runtime, session)),
                error: None,
            }
        }
        Ok(Err(message)) => {
            warn_send_rejection(shared, session, &device, &message);
            error(
                if message == "stale_lease" {
                    "stale_lease"
                } else {
                    "driver_error"
                },
                &message,
            )
        }
        Err(_) => {
            let message = "output worker stopped before completion";
            warn_send_rejection(shared, session, &device, message);
            error("driver_error", message)
        }
    }
}

/// SendMidi の拒否を warn に。毎メッセージ来うるので、同じ拒否は直るまで 1 回だけ
fn warn_send_rejection(shared: &Arc<Mutex<Runtime>>, session: &str, device: &str, reason: &str) {
    let mut runtime = shared.lock().expect("runtime mutex poisoned");
    if runtime.first_send_rejection(session, device, reason) {
        let who = runtime.client_name(session);
        let reason = crate::logging::printable(reason);
        tracing::warn!(
            "SendMidi to {device} from {who} rejected: {reason} (repeats are not logged)"
        );
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
        tracing::warn!(
            "request {} from an unknown session rejected: Hello is required",
            crate::logging::printable(method)
        );
        return error("unauthenticated", "Hello is required");
    }
    let hello_client = (method == "Hello")
        .then(|| {
            payload
                .get("client_id")
                .and_then(|v| v.as_str())
                .map(crate::logging::printable)
        })
        .flatten();
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
    if method != "Snapshot" {
        if let Err(reason) = &result {
            // 拒否は開発中も常に見たい（stale_revision、未認証、版違い …）。
            // Hello はまだ名前を覚えていないので、名乗った client_id を出す
            let who = if method == "Hello" {
                hello_client.clone().unwrap_or_else(|| "?".into())
            } else {
                runtime.client_name(session)
            };
            // 理由には要求の中身が混ざりうる（serde のエラーは値を引用する）
            let method = crate::logging::printable(method);
            let reason = crate::logging::printable(&reason.to_string());
            tracing::warn!("request {method} from {who} rejected: {reason}");
        }
        runtime.log_flow();
    }
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

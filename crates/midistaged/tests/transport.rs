// mem_1CfnVkncPrmfdQet3bNSmx — 実 QUIC、実保存領域。実機 MIDI は開かない。
use midistage_protocol::{Broker, PROTOCOL_VERSION, wire::*};
use midistaged::{runtime::Runtime, service::Service, settings::SettingsStore};
use unison::network::{ProtocolClient, QuicClient, TrustAnchors, channel::UnisonChannel};

async fn connect(service: &Service) -> (ProtocolClient, UnisonChannel) {
    let transport = QuicClient::builder()
        .trust_anchors(TrustAnchors::Custom(vec![
            service.endpoint.certificate_der.clone().into(),
        ]))
        .build()
        .unwrap();
    let client = ProtocolClient::new(transport);
    client
        .connect(&format!("https://localhost:{}", service.endpoint.port))
        .await
        .unwrap();
    let channel = client.open_channel(CHANNEL).await.unwrap();
    (client, channel)
}
fn hello(service: &Service, client: &str) -> Hello {
    Hello {
        protocol_version: PROTOCOL_VERSION,
        client_id: client.into(),
        display_name: client.into(),
        auth_token: service.endpoint.auth_token.clone(),
        native_midi: false,
        initial_enabled_profiles: vec![],
    }
}
async fn start(path: &std::path::Path) -> Service {
    let mut broker = Broker::new("test-epoch");
    broker.add_device("nano", "nanokontrol", true);
    Service::start(Runtime::new(broker, SettingsStore::new(path)).unwrap())
        .await
        .unwrap()
}
#[tokio::test]
async fn unauthenticated_requests_and_wrong_token_cannot_observe_devices() {
    let dir = tempfile::tempdir().unwrap();
    let service = start(dir.path()).await;
    let (client, channel) = connect(&service).await;
    let reply: Reply = channel
        .request("Snapshot", &serde_json::json!({}))
        .await
        .unwrap();
    assert!(reply.snapshot.is_none());
    assert_eq!(reply.error.unwrap().code, "unauthenticated");
    let mut request = hello(&service, "ladyland");
    request.auth_token = "wrong".into();
    let reply: Reply = channel.request("Hello", &request).await.unwrap();
    assert!(reply.snapshot.is_none());
    assert_eq!(reply.error.unwrap().code, "unauthenticated");
    client.disconnect().await.unwrap();
}
#[tokio::test]
async fn native_clients_wait_for_driver_readiness_and_only_see_their_own_ports() {
    let dir = tempfile::tempdir().unwrap();
    let service = start(dir.path()).await;
    let (a, ca) = connect(&service).await;
    let (b, cb) = connect(&service).await;
    let mut request = hello(&service, "ladyland");
    request.native_midi = true;
    request.initial_enabled_profiles = vec!["nanokontrol".into()];
    let reply: Reply = ca.request("Hello", &request).await.unwrap();
    assert_eq!(
        reply.snapshot.unwrap().devices[0].state.phase,
        midistage_protocol::Phase::Waiting
    );
    {
        let mut runtime = service.runtime.lock().unwrap();
        let lease = runtime.broker.devices["nano"].lease.clone().unwrap();
        runtime.native_bindings.insert(
            "nano".into(),
            midistaged::runtime::NativeBinding {
                lease,
                ports: NativePorts {
                    inputs: vec!["owned source".into()],
                    outputs: vec![],
                },
            },
        );
    }
    let reply: Reply = ca
        .request("Snapshot", &serde_json::json!({}))
        .await
        .unwrap();
    let view = reply.snapshot.unwrap().devices.remove(0);
    assert_eq!(view.state.phase, midistage_protocol::Phase::Active);
    assert_eq!(view.native_ports.inputs, vec!["owned source"]);
    let reply: Reply = cb.request("Hello", &hello(&service, "vp")).await.unwrap();
    assert!(
        reply.snapshot.unwrap().devices[0]
            .native_ports
            .inputs
            .is_empty()
    );
    a.disconnect().await.unwrap();
    b.disconnect().await.unwrap();
    service.shutdown().await.unwrap();
}
#[tokio::test]
async fn rust_sdk_uses_pinned_endpoint_and_surfaces_protocol_errors() {
    let dir = tempfile::tempdir().unwrap();
    let service = start(dir.path()).await;
    let (client, snapshot) =
        midistage_client::Client::connect(&service.endpoint, hello(&service, "sdk"))
            .await
            .unwrap();
    assert_eq!(snapshot.server_epoch, service.endpoint.server_epoch);
    assert!(client.snapshot().await.unwrap().sequence > snapshot.sequence);
    let enabled = client
        .set_enabled(&SetEnabled {
            device_id: "nano".into(),
            enabled: true,
            expected_revision: 0,
            takeover: false,
        })
        .await
        .unwrap();
    assert_eq!(
        enabled.devices[0].state.assignment.client_id.as_deref(),
        Some("sdk")
    );
    let error = client
        .set_enabled(&SetEnabled {
            device_id: "nano".into(),
            enabled: false,
            expected_revision: 0,
            takeover: false,
        })
        .await
        .unwrap_err();
    assert!(error.to_string().contains("stale_revision"));
    let rejected = client
        .send_midi(&SendMidi {
            device_id: "nano".into(),
            lease_token: "old".into(),
            port_name: "output".into(),
            bytes: vec![0x90, 60, 100],
        })
        .await
        .unwrap_err();
    assert!(rejected.to_string().contains("stale_lease"));
    let old = enabled.devices[0].state.lease.clone().unwrap();
    let (peer, cp) = connect(&service).await;
    let _: Reply = cp.request("Hello", &hello(&service, "peer")).await.unwrap();
    let _: Reply = cp
        .request(
            "SetEnabled",
            &SetEnabled {
                device_id: "nano".into(),
                enabled: true,
                expected_revision: 1,
                takeover: true,
            },
        )
        .await
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            if let Event::Quiesce { lease_token, .. } = client.recv_event().await.unwrap() {
                assert_eq!(lease_token, old.token);
                break;
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(
        client.snapshot().await.unwrap().devices[0].state.phase,
        midistage_protocol::Phase::Releasing
    );
    assert!(!service.runtime.lock().unwrap().broker.is_quiescent("nano"));
    client
        .quiesced(&Quiesced {
            device_id: "nano".into(),
            lease_token: old.token,
        })
        .await
        .unwrap();
    assert!(service.runtime.lock().unwrap().broker.is_quiescent("nano"));
    peer.disconnect().await.unwrap();
    client.close().await.unwrap();
    service.shutdown().await.unwrap();
}

#[tokio::test]
async fn output_reply_waits_for_physical_completion_and_rejects_old_lease() {
    let dir = tempfile::tempdir().unwrap();
    let service = start(dir.path()).await;
    let (client, channel) = connect(&service).await;
    let _: Reply = channel
        .request("Hello", &hello(&service, "sender"))
        .await
        .unwrap();
    let reply: Reply = channel
        .request(
            "SetEnabled",
            &SetEnabled {
                device_id: "nano".into(),
                enabled: true,
                expected_revision: 0,
                takeover: false,
            },
        )
        .await
        .unwrap();
    let lease = reply.snapshot.unwrap().devices[0]
        .state
        .lease
        .clone()
        .unwrap();
    let (sender, receiver) = std::sync::mpsc::sync_channel(8);
    {
        let mut r = service.runtime.lock().unwrap();
        r.native_bindings.insert(
            "nano".into(),
            midistaged::runtime::NativeBinding {
                lease: lease.clone(),
                ports: NativePorts {
                    inputs: vec![],
                    outputs: vec!["native output".into()],
                },
            },
        );
        r.output_sinks.insert(
            "native output".into(),
            midistaged::io_fence::OutputSink {
                destination: 123,
                sender,
            },
        );
    }
    let request = SendMidi {
        device_id: "nano".into(),
        lease_token: lease.token,
        port_name: "native output".into(),
        bytes: vec![0xf0, 0x7d, 1, 0xf7],
    };
    let channel = std::sync::Arc::new(channel);
    let send_channel = channel.clone();
    let send_request = request.clone();
    let reply = tokio::spawn(async move {
        send_channel
            .request::<_, Reply>("SendMidi", &send_request)
            .await
            .unwrap()
    });
    let command = tokio::task::spawn_blocking(move || {
        receiver.recv_timeout(std::time::Duration::from_secs(1))
    })
    .await
    .unwrap()
    .expect("driver receives output");
    assert_eq!(command.bytes, request.bytes);
    assert!(!reply.is_finished());
    command.completion.unwrap().send(Ok(())).unwrap();
    assert!(reply.await.unwrap().error.is_none());
    let stale = SendMidi {
        lease_token: "old".into(),
        ..request
    };
    let reply: Reply = channel.request("SendMidi", &stale).await.unwrap();
    assert_eq!(reply.error.unwrap().code, "stale_lease");
    client.disconnect().await.unwrap();
    service.shutdown().await.unwrap();
}
#[tokio::test]
async fn two_apps_must_confirm_takeover_and_stale_output_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let service = start(dir.path()).await;
    let (a, ca) = connect(&service).await;
    let (b, cb) = connect(&service).await;
    for (channel, name) in [(&ca, "ladyland"), (&cb, "vp")] {
        let reply: Reply = channel
            .request("Hello", &hello(&service, name))
            .await
            .unwrap();
        assert!(reply.error.is_none());
    }
    let request = SetEnabled {
        device_id: "nano".into(),
        enabled: true,
        expected_revision: 0,
        takeover: false,
    };
    let reply: Reply = ca.request("SetEnabled", &request).await.unwrap();
    let old = reply.snapshot.unwrap().devices[0]
        .state
        .lease
        .clone()
        .unwrap();
    let mut request = SetEnabled {
        expected_revision: 1,
        ..request
    };
    let reply: Reply = cb.request("SetEnabled", &request).await.unwrap();
    assert_eq!(reply.error.unwrap().code, "conflict");
    request.takeover = true;
    let reply: Reply = cb.request("SetEnabled", &request).await.unwrap();
    assert_eq!(
        reply.snapshot.unwrap().devices[0].state.phase,
        midistage_protocol::Phase::Releasing
    );
    let event = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let message = ca.recv().await.unwrap();
            let event: Event = serde_json::from_value(message.payload_as_value().unwrap()).unwrap();
            if let Event::Quiesce { lease_token, .. } = event {
                break lease_token;
            }
        }
    })
    .await
    .expect("old owner receives cleanup request");
    assert_eq!(event, old.token);
    let reply: Reply = ca
        .request(
            "Present",
            &Present {
                device_id: "nano".into(),
                lease_token: old.token.clone(),
                controls: vec![],
            },
        )
        .await
        .unwrap();
    assert_eq!(reply.error.unwrap().code, "stale_lease");
    assert!(
        service
            .runtime
            .lock()
            .unwrap()
            .broker
            .drained("nano")
            .is_err()
    );
    let reply: Reply = ca
        .request(
            "Quiesced",
            &Quiesced {
                device_id: "nano".into(),
                lease_token: old.token,
            },
        )
        .await
        .unwrap();
    assert!(reply.error.is_none());
    assert_eq!(
        reply.snapshot.unwrap().devices[0].state.phase,
        midistage_protocol::Phase::Releasing
    );
    // 実機 adapter の完了境界をここではテストから明示的に進める。
    service
        .runtime
        .lock()
        .unwrap()
        .broker
        .drained("nano")
        .unwrap();
    let reply: Reply = cb
        .request("Snapshot", &serde_json::json!({}))
        .await
        .unwrap();
    let next = reply.snapshot.unwrap();
    assert_eq!(
        next.devices[0].state.lease.as_ref().unwrap().session_id,
        next.session_id
    );
    a.disconnect().await.unwrap();
    b.disconnect().await.unwrap();
    service.shutdown().await.unwrap();
}

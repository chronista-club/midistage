//! schemas/midistage.kdl の wire 型。ドメイン固有の Track / lane は含めない。
use crate::DeviceState;
use serde::{Deserialize, Serialize};

pub const CHANNEL: &str = "midistage";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Hello {
    pub protocol_version: u32,
    pub client_id: String,
    pub display_name: String,
    pub auth_token: String,
    #[serde(default)]
    pub native_midi: bool,
    #[serde(default)]
    pub initial_enabled_profiles: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SetEnabled {
    pub device_id: String,
    pub enabled: bool,
    pub expected_revision: u64,
    #[serde(default)]
    pub takeover: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Quiesced {
    pub device_id: String,
    pub lease_token: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Control {
    pub id: String,
    pub kind: String,
    #[serde(default)]
    pub outputs: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct NativePorts {
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct DeviceView {
    #[serde(flatten)]
    pub state: DeviceState,
    pub controls: Vec<Control>,
    pub native_ports: NativePorts,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Snapshot {
    pub sequence: u64,
    pub protocol_version: u32,
    pub server_epoch: String,
    pub session_id: String,
    pub devices: Vec<DeviceView>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProtocolError {
    pub code: String,
    pub message: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Reply {
    pub snapshot: Option<Snapshot>,
    pub error: Option<ProtocolError>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Event {
    State {
        snapshot: Snapshot,
    },
    Quiesce {
        device_id: String,
        profile_id: String,
        lease_token: String,
    },
    Input {
        device_id: String,
        lease_token: String,
        sequence: u64,
        timestamp_ns: u64,
        control_id: String,
        /// absolute / relative / button / touch / note の意味を明示する。
        value_kind: String,
        value: f64,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Present {
    pub device_id: String,
    pub lease_token: String,
    pub controls: Vec<ControlPresentation>,
}

/// native_midi 拡張。仮想ポート受付ではなく物理 driver 完了を待つ送信。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SendMidi {
    pub device_id: String,
    pub lease_token: String,
    pub port_name: String,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ControlPresentation {
    pub id: String,
    pub name: Option<String>,
    pub value: Option<f32>,
    pub color: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Endpoint {
    pub protocol_version: u32,
    pub port: u16,
    pub server_epoch: String,
    pub certificate_der: Vec<u8>,
    pub auth_token: String,
}

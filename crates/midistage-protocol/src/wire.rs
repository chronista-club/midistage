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
    /// 機材の設定ポート（Roto の Serial）を開いて用事をしている間 true。その間 Roto の MIDI は止まる（design 06）
    #[serde(default)]
    pub configuring: bool,
    /// midistage が最後に選んだ setup。本体の SEL で変えられた分は知らない
    #[serde(default)]
    pub current_setup: Option<u8>,
    /// setup の持ち主（client_id → setup）
    #[serde(default)]
    pub setup_owners: std::collections::BTreeMap<String, Vec<u8>>,
    /// 設定ポートの用事が失敗した理由（他のプロセスがポートを開いている など）
    #[serde(default)]
    pub config_error: Option<String>,
}

/// 自分の setup を登録する（design 06）。空の配列で取り消し。他のアプリの setup は取れない
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SetDeviceSetups {
    pub device_id: String,
    pub setups: Vec<u8>,
}

/// lease を持つアプリが、自分の setup の中で切り替える（design 06）
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SelectSetup {
    pub device_id: String,
    pub lease_token: String,
    pub setup: u8,
}

/// サービス自身の事実（design 05「常駐と配布」）。app が「同梱版が動いているか」「.app を更新したが
/// 古い版のままか」を表示するために使う。入れ替えは人の操作なので、ここは事実を載せるだけ。
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ServiceInfo {
    /// midistaged の crate version（`CARGO_PKG_VERSION`）
    pub version: String,
    /// 実行ファイルの絶対パス。取れなければ空
    pub executable: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Snapshot {
    pub sequence: u64,
    pub protocol_version: u32,
    pub server_epoch: String,
    pub session_id: String,
    pub devices: Vec<DeviceView>,
    /// 旧い client / fixture には無いので省略可（protocol_version は上げない）
    #[serde(default)]
    pub service: ServiceInfo,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Assignment, Lease, Phase};

    /// design 06 の欄を含む snapshot。Swift SDK（clients/swift/Tests/.../snapshot-setups.json）が同じものを読む。
    /// 形を意図して変えたら `UPDATE_FIXTURE=1 cargo test -p midistage-protocol` で書き直し、Swift のテストも通す
    #[test]
    fn device_setup_fields_match_the_swift_fixture() {
        let snapshot = Snapshot {
            sequence: 7,
            protocol_version: 1,
            server_epoch: "boot".into(),
            session_id: "v".into(),
            devices: vec![DeviceView {
                state: DeviceState {
                    device_id: "roto".into(),
                    profile_id: "roto".into(),
                    name: "Roto-Control".into(),
                    present: true,
                    assignment: Assignment {
                        client_id: Some("vp".into()),
                        revision: 3,
                        expected: false,
                    },
                    phase: Phase::Active,
                    lease: Some(Lease {
                        session_id: "v".into(),
                        token: "boot:4".into(),
                    }),
                    releasing_to: None,
                },
                controls: vec![],
                native_ports: NativePorts::default(),
                error: None,
                configuring: true,
                current_setup: Some(6),
                setup_owners: [
                    ("ladyland".to_string(), vec![0, 1, 2, 3]),
                    ("vp".to_string(), vec![6]),
                ]
                .into(),
                config_error: None,
            }],
            service: ServiceInfo::default(),
        };
        let got = serde_json::to_string_pretty(&snapshot).unwrap() + "\n";
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../clients/swift/Tests/MidistageClientTests/Fixtures/snapshot-setups.json");
        if std::env::var_os("UPDATE_FIXTURE").is_some() {
            std::fs::write(&path, &got).unwrap();
        }
        assert_eq!(got, std::fs::read_to_string(&path).unwrap());
        // 旧いサービスの snapshot（欄が無い）も読める
        let old: DeviceView = serde_json::from_str(
            r#"{"device_id":"nano","profile_id":"nanokontrol","name":"n","present":true,
                "assignment":{"client_id":null,"revision":0,"expected":false},"phase":"off","lease":null,
                "releasing_to":null,"controls":[],"native_ports":{"inputs":[],"outputs":[]},"error":null}"#,
        )
        .unwrap();
        assert!(!old.configuring && old.current_setup.is_none() && old.setup_owners.is_empty());
    }
}

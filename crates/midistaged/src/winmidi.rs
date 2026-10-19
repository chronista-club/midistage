//! Windows の OS 境界（design 07）。Windows MIDI Services（Windows.Devices.Midi2）を使う。
//! いまは列挙だけ（read-only。何も開かず、何も送らない）。
use crate::inventory::{self, DevicePorts, Endpoint};
use crate::winmidi_bindings::{MidiApi, MidiEndpointDeviceInformation};
use std::collections::BTreeMap;

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

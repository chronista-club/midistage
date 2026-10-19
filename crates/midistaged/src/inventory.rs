use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Port {
    pub uid: u32,
    pub physical_device: u32,
    pub name: String,
    pub input: bool,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DevicePorts {
    pub ports: Vec<Port>,
    pub error: Option<String>,
}
pub const PROFILES: &[(&str, &str)] = &[
    ("roto", "Roto-Control"),
    ("lpd8", "LPD8 mk2"),
    ("nanokontrol", "nanoKONTROL2"),
    ("keystage", "Keystage"),
    ("numa", "Numa"),
    ("minilab", "MiniLab"),
    ("fgdp", "FGDP"),
    ("xtouch", "X-Touch"),
    // 2026-10-11: gear 資産（design 04）と机（spec 01）の ID を inventory で正にするため追加
    ("l6max", "L6max"),
    ("zenith2", "Zenith 2"),
];
/// ポート名から profile を引く（大文字小文字を区別しない部分一致。純粋）
pub fn profile_for_name(name: &str) -> Option<&'static str> {
    let name = name.to_lowercase();
    PROFILES
        .iter()
        .find(|(id, pattern)| {
            name.contains(&pattern.to_lowercase()) || (*id == "numa" && name.contains("ncxse"))
        })
        .map(|(id, _)| *id)
}

/// 保存済みの `generic:<ポート名>` が、今は profile で認識される名前か。
/// profile ができる前に汎用として覚えた記録は、今は profile の ID で出るので重複になる（純粋）
pub fn superseded_generic(id: &str) -> bool {
    id.strip_prefix("generic:")
        .is_some_and(|name| profile_for_name(name).is_some())
}

pub fn inventory(ports: &[Port]) -> BTreeMap<String, DevicePorts> {
    let mut found: BTreeMap<String, DevicePorts> = PROFILES
        .iter()
        .map(|(id, _)| ((*id).into(), DevicePorts::default()))
        .collect();
    let mut unknown: BTreeMap<u32, Vec<Port>> = BTreeMap::new();
    for port in ports {
        if port.physical_device == 0 || port.name.starts_with("Midistage/") {
            continue;
        }
        if let Some(id) = profile_for_name(&port.name) {
            found
                .get_mut(id)
                .expect("known profile")
                .ports
                .push(port.clone());
        } else {
            unknown
                .entry(port.physical_device)
                .or_default()
                .push(port.clone());
        }
    }
    for ports in unknown.into_values() {
        if let Some(name) = ports.iter().map(|p| p.name.to_lowercase()).min() {
            found
                .entry(format!("generic:{name}"))
                .or_default()
                .ports
                .extend(ports);
        }
    }
    for device in found.values_mut() {
        let physical: std::collections::BTreeSet<_> =
            device.ports.iter().map(|p| p.physical_device).collect();
        if physical.len() > 1 {
            device.error =
                Some("同一機種が複数接続されています。v1 は一機種につき一台を扱います。".into());
            device.ports.clear();
        } else {
            device
                .ports
                .sort_by_key(|p| (p.input, p.name.contains(" EXT"), p.name.clone(), p.uid));
        }
    }
    found
}

/// Windows MIDI Services の endpoint（双方向。design 07）。OS から読んだ素の値
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Endpoint {
    /// `\\?\swd#midisrv#…`（大文字小文字は区別されない）
    pub id: String,
    pub name: String,
    /// transport code。`KS` / `KSA` が実機のドライバ、`APP` / `LOOP` / `DIAG` などは仮想
    pub transport: String,
    /// 親デバイス（USB 機材）の ID。同じ親の endpoint は一台の機材
    pub parent: String,
}

/// endpoint を CoreMIDI と同じ形の IN / OUT の組にする（純粋）。
/// uid と physical_device は ID の FNV-1a。実機のドライバ以外は物理ポートとして扱わない
pub fn endpoint_ports(endpoints: &[Endpoint]) -> Vec<Port> {
    endpoints
        .iter()
        .filter(|e| matches!(e.transport.as_str(), "KS" | "KSA"))
        .flat_map(|e| {
            let id = e.id.to_lowercase();
            let physical_device = fnv1a32(e.parent.to_lowercase().as_bytes()).max(1);
            [(true, "#in"), (false, "#out")].map(|(input, suffix)| Port {
                uid: fnv1a32(format!("{id}{suffix}").as_bytes()),
                physical_device,
                name: e.name.clone(),
                input,
            })
        })
        .collect()
}

/// FNV-1a 32 bit。保存や比較に使うので版をまたいで変えない
pub fn fnv1a32(bytes: &[u8]) -> u32 {
    bytes.iter().fold(0x811c_9dc5, |hash, byte| {
        (hash ^ u32::from(*byte)).wrapping_mul(0x0100_0193)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn port(uid: u32, physical_device: u32, name: &str, input: bool) -> Port {
        Port {
            uid,
            physical_device,
            name: name.into(),
            input,
        }
    }
    #[test]
    fn hub_replacement_and_port_reordering_keep_the_profile_identity() {
        let before = inventory(&[port(1, 11, "nanoKONTROL2 SLIDER/KNOB", true)]);
        let after = inventory(&[
            port(33, 22, "nanoKONTROL2 CTRL", false),
            port(44, 22, "nanoKONTROL2 SLIDER/KNOB", true),
        ]);
        assert_eq!(before["nanokontrol"].ports.len(), 1);
        assert_eq!(after["nanokontrol"].ports.len(), 2);
        assert!(after["nanokontrol"].error.is_none());
    }
    #[test]
    fn multiport_device_is_one_device_but_two_units_are_ambiguous() {
        let ports = [
            port(1, 11, "Keystage KBD/CTRL", true),
            port(2, 11, "Keystage DAW IN", true),
            port(3, 11, "Keystage CTRL", false),
        ];
        assert_eq!(inventory(&ports)["keystage"].ports.len(), 3);
        let mut duplicates = ports.to_vec();
        duplicates.push(port(4, 22, "Keystage KBD/CTRL", true));
        let found = inventory(&duplicates);
        assert!(found["keystage"].error.is_some());
        assert!(found["keystage"].ports.is_empty());
    }
    #[test]
    fn virtual_ports_are_ignored_and_unknown_physical_ports_keep_their_identity() {
        let found = inventory(&[
            port(1, 0, "Midistage/vp/lease | Roto-Control", true),
            port(2, 0, "Ladyland RotoInject", false),
            port(3, 55, "Unknown Box", true),
        ]);
        assert_eq!(found.len(), PROFILES.len() + 1);
        assert_eq!(found["generic:unknown box"].ports.len(), 1);
        assert!(
            found
                .iter()
                .filter(|(id, _)| !id.starts_with("generic:"))
                .map(|(_, d)| d)
                .all(|d| d.ports.is_empty() && d.error.is_none())
        );
    }
    #[test]
    fn l6max_and_zenith2_are_recognized_by_their_port_names() {
        // L6max は複数ポート（Mixer Control / MIDI I/O / Editor）、Zenith 2 は 1 ポート。どちらも 1 台
        let found = inventory(&[
            port(1, 61, "L6max Mixer Control Port", true),
            port(2, 61, "L6max Mixer Control Port", false),
            port(3, 61, "L6max MIDI I/O Port", true),
            port(4, 62, "Zenith 2", true),
            port(5, 62, "Zenith 2", false),
        ]);
        assert_eq!(found["l6max"].ports.len(), 3);
        assert_eq!(found["zenith2"].ports.len(), 2);
        assert!(found.keys().all(|k| !k.starts_with("generic:")));
    }
    #[test]
    fn numa_usb_name_and_xtouch_internal_port_are_recognized() {
        let found = inventory(&[
            port(1, 11, "NCXse-keyboard", true),
            port(2, 11, "NCXse-controller", true),
            port(3, 22, "X-Touch EXT", true),
            port(99, 22, "X-Touch INT", true),
        ]);
        assert_eq!(found["numa"].ports.len(), 2);
        assert_eq!(found["xtouch"].ports[0].name, "X-Touch INT");
    }
    #[test]
    fn a_port_name_tells_which_profile_it_belongs_to() {
        assert_eq!(profile_for_name("Zenith 2"), Some("zenith2"));
        assert_eq!(
            profile_for_name("zenith 2"),
            Some("zenith2"),
            "generic: の ID は小文字"
        );
        assert_eq!(
            profile_for_name("nanoKONTROL2 SLIDER/KNOB"),
            Some("nanokontrol")
        );
        assert_eq!(profile_for_name("NCXSE"), Some("numa"));
        assert_eq!(profile_for_name("Studio Keyboard MIDI"), None);
    }
    #[test]
    fn a_generic_id_is_superseded_once_a_profile_claims_its_name() {
        assert!(superseded_generic("generic:zenith 2"));
        assert!(!superseded_generic("generic:studio keyboard midi"));
        assert!(
            !superseded_generic("zenith2"),
            "profile の ID そのものは対象外"
        );
    }

    #[test]
    fn unknown_physical_keyboard_remains_available_across_hub_ids() {
        let before = inventory(&[port(1, 11, "Studio Keyboard MIDI", true)]);
        let after = inventory(&[port(88, 99, "Studio Keyboard MIDI", true)]);
        assert_eq!(before["generic:studio keyboard midi"].ports.len(), 1);
        assert_eq!(after["generic:studio keyboard midi"].ports.len(), 1);
        let duplicate = inventory(&[
            port(1, 11, "Studio Keyboard MIDI", true),
            port(2, 22, "Studio Keyboard MIDI", true),
        ]);
        assert!(duplicate["generic:studio keyboard midi"].error.is_some());
    }

    // ---- Windows MIDI Services の endpoint（design 07）----
    fn endpoint(id: &str, name: &str, transport: &str, parent: &str) -> Endpoint {
        Endpoint {
            id: id.into(),
            name: name.into(),
            transport: transport.into(),
            parent: parent.into(),
        }
    }
    const NANO: &str = r"\?\swd#midisrv#midiu_ks_7510086354376457165_outpin.0_inpin.2#{e7cce071-3c03-423f-88d3-f1045d02552b}";

    #[test]
    fn a_bidirectional_endpoint_becomes_one_input_and_one_output_of_one_device() {
        let ports = endpoint_ports(&[endpoint(
            NANO,
            "nanoKONTROL2",
            "KS",
            r"USB\VID_0944&PID_0117\5&1",
        )]);
        assert_eq!(ports.len(), 2);
        assert!(ports[0].input && !ports[1].input);
        assert_ne!(ports[0].uid, ports[1].uid);
        assert_eq!(ports[0].physical_device, ports[1].physical_device);
        assert_ne!(ports[0].physical_device, 0);
        assert!(ports.iter().all(|p| p.name == "nanoKONTROL2"));
        assert_eq!(inventory(&ports)["nanokontrol"].ports.len(), 2);
    }

    #[test]
    fn endpoint_ids_hash_to_fixed_values_regardless_of_case() {
        let lower = endpoint_ports(&[endpoint(NANO, "n", "KS", "p")]);
        let upper = endpoint_ports(&[endpoint(&NANO.to_uppercase(), "n", "KS", "P")]);
        assert_eq!(lower, upper);
        // 版をまたいで変えない（FNV-1a 32 bit）
        assert_eq!(fnv1a32(b""), 0x811c_9dc5);
        assert_eq!(fnv1a32(b"a"), 0xe40c_292c);
    }

    #[test]
    fn only_hardware_transports_are_physical() {
        let ports = endpoint_ports(&[
            endpoint("ks", "A", "KS", "usb-a"),
            endpoint("ksa", "B", "KSA", "usb-b"),
            endpoint("app", "Midistage/vp/x", "APP", "swd"),
            endpoint("loop", "Loopback A", "LOOP", "swd"),
            endpoint("diag", "Diagnostics", "DIAG", "swd"),
        ]);
        let names: std::collections::BTreeSet<_> = ports.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["A", "B"].into());
    }

    #[test]
    fn endpoints_of_one_parent_are_one_physical_device() {
        let ports = endpoint_ports(&[
            endpoint("e1", "Keystage", "KS", "usb-1"),
            endpoint("e2", "Keystage CTRL", "KS", "usb-1"),
            endpoint("e3", "Other", "KS", "usb-2"),
        ]);
        assert_eq!(ports[0].physical_device, ports[2].physical_device);
        assert_ne!(ports[0].physical_device, ports[4].physical_device);
    }
}

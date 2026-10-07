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
];
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
        let name = port.name.to_lowercase();
        if let Some((id, _)) = PROFILES.iter().find(|(id, pattern)| {
            name.contains(&pattern.to_lowercase()) || (*id == "numa" && name.contains("ncxse"))
        }) {
            found
                .get_mut(*id)
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
            port(3, 55, "Zenith 2", true),
        ]);
        assert_eq!(found.len(), PROFILES.len() + 1);
        assert_eq!(found["generic:zenith 2"].ports.len(), 1);
        assert!(
            found
                .iter()
                .filter(|(id, _)| !id.starts_with("generic:"))
                .map(|(_, d)| d)
                .all(|d| d.ports.is_empty() && d.error.is_none())
        );
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
}

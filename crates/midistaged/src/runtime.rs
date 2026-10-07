use crate::settings::{SavedDevice, SavedSettings, SettingsStore};
use midistage_protocol::{
    Broker, PROTOCOL_VERSION,
    wire::{Hello, SetEnabled},
};
use std::collections::BTreeMap;

#[derive(Clone)]
pub struct NativeBinding {
    pub lease: midistage_protocol::Lease,
    pub ports: midistage_protocol::wire::NativePorts,
}

pub struct Runtime {
    pub snapshot_sequence: std::sync::atomic::AtomicU64,
    pub broker: Broker,
    pub native_bindings: BTreeMap<String, NativeBinding>,
    pub output_sinks: BTreeMap<String, crate::io_fence::OutputSink>,
    pub driver_errors: BTreeMap<String, String>,
    native_sessions: BTreeMap<String, bool>,
    store: SettingsStore,
    saved: SavedSettings,
}
impl Runtime {
    pub fn new(mut broker: Broker, store: SettingsStore) -> anyhow::Result<Self> {
        let saved = store.load()?;
        broker.restore(
            &saved
                .devices
                .iter()
                .map(|(id, d)| (id.clone(), d.assignment.clone()))
                .collect(),
        );
        Ok(Self {
            snapshot_sequence: std::sync::atomic::AtomicU64::new(1),
            broker,
            store,
            saved,
            native_bindings: BTreeMap::new(),
            output_sinks: BTreeMap::new(),
            driver_errors: BTreeMap::new(),
            native_sessions: BTreeMap::new(),
        })
    }
    pub fn hello(&mut self, session: &str, hello: &Hello) -> anyhow::Result<()> {
        anyhow::ensure!(
            hello.protocol_version == PROTOCOL_VERSION,
            "incompatible protocol version"
        );
        anyhow::ensure!(
            !hello.client_id.is_empty() && hello.client_id.len() <= 128,
            "invalid client id"
        );
        let mut candidate = self.broker.clone();
        candidate.register_session(session, &hello.client_id)?;
        let mut saved = self.saved.clone();
        if saved.initialized_clients.insert(hello.client_id.clone()) {
            let initial: Vec<_> = candidate
                .devices
                .iter()
                .filter(|(_, d)| {
                    d.assignment.revision == 0
                        && d.assignment.client_id.is_none()
                        && hello.initial_enabled_profiles.contains(&d.profile_id)
                })
                .map(|(id, _)| id.clone())
                .collect();
            for id in initial {
                candidate.enable(&id, session, 0, false)?;
            }
        }
        self.commit(candidate, saved)?;
        self.native_sessions
            .insert(session.into(), hello.native_midi);
        Ok(())
    }
    pub fn set_enabled(&mut self, session: &str, request: &SetEnabled) -> anyhow::Result<()> {
        let mut candidate = self.broker.clone();
        if request.enabled {
            candidate.enable(
                &request.device_id,
                session,
                request.expected_revision,
                request.takeover,
            )?;
        } else {
            candidate.disable(&request.device_id, session, request.expected_revision)?;
        }
        self.commit(candidate, self.saved.clone())
    }
    fn commit(&mut self, candidate: Broker, mut saved: SavedSettings) -> anyhow::Result<()> {
        for (id, d) in &candidate.devices {
            saved.devices.insert(
                id.clone(),
                SavedDevice {
                    profile_id: d.profile_id.clone(),
                    name: d.name.clone(),
                    assignment: d.assignment.clone(),
                },
            );
        }
        // 永続化に失敗した変更は、実行中の所有権に一切適用しない。
        self.store.save(&saved)?;
        self.saved = saved;
        self.broker = candidate;
        Ok(())
    }
    pub fn native_requested(&self, session: &str) -> bool {
        self.native_sessions.get(session) == Some(&true)
    }
    pub fn disconnect(&mut self, session: &str) {
        self.native_sessions.remove(session);
        self.broker.disconnect_session(session);
    }
}

#[cfg(test)]
mod tests {
    // mem_1CfnVkncPrmfdQet3bNSmx
    use super::*;
    use midistage_protocol::{PROTOCOL_VERSION, Phase};
    fn hello(client: &str) -> Hello {
        Hello {
            protocol_version: PROTOCOL_VERSION,
            client_id: client.into(),
            display_name: client.into(),
            auth_token: "test".into(),
            native_midi: false,
            initial_enabled_profiles: vec!["nanokontrol".into()],
        }
    }
    fn runtime(path: &std::path::Path) -> Runtime {
        let mut broker = Broker::new("boot");
        broker.add_device("nano", "nanokontrol", true);
        Runtime::new(broker, SettingsStore::new(path)).unwrap()
    }
    #[test]
    fn initial_preferences_apply_once_and_never_steal() {
        let dir = tempfile::tempdir().unwrap();
        let mut r = runtime(dir.path());
        r.hello("a", &hello("ladyland")).unwrap();
        r.hello("b", &hello("vp")).unwrap();
        assert_eq!(
            r.broker.devices["nano"].assignment.client_id.as_deref(),
            Some("ladyland")
        );
        r.set_enabled(
            "a",
            &SetEnabled {
                device_id: "nano".into(),
                enabled: false,
                expected_revision: 1,
                takeover: false,
            },
        )
        .unwrap();
        let mut restarted = runtime(dir.path());
        restarted.hello("a2", &hello("ladyland")).unwrap();
        assert_eq!(restarted.broker.devices["nano"].phase, Phase::Off);
    }
    #[test]
    fn failed_persistence_cannot_revoke_the_current_owner() {
        let dir = tempfile::tempdir().unwrap();
        let mut r = runtime(dir.path());
        r.hello("a", &hello("ladyland")).unwrap();
        r.hello("b", &hello("vp")).unwrap();
        let before = r.broker.devices["nano"].clone();
        std::fs::remove_file(dir.path().join("settings.json")).unwrap();
        std::fs::create_dir(dir.path().join("settings.json")).unwrap();
        assert!(
            r.set_enabled(
                "b",
                &SetEnabled {
                    device_id: "nano".into(),
                    enabled: true,
                    expected_revision: 1,
                    takeover: true
                }
            )
            .is_err()
        );
        assert_eq!(r.broker.devices["nano"], before);
    }
    #[test]
    fn incompatible_protocol_does_not_register_a_session() {
        let dir = tempfile::tempdir().unwrap();
        let mut r = runtime(dir.path());
        let mut request = hello("ladyland");
        request.protocol_version = 99;
        assert!(r.hello("a", &request).is_err());
        request.protocol_version = PROTOCOL_VERSION;
        r.hello("a", &request).unwrap();
    }
}

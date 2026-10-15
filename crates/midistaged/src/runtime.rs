use crate::settings::{SavedDevice, SavedSettings, SettingsStore};
use midistage_protocol::{
    Broker, PROTOCOL_VERSION,
    wire::{Hello, SetEnabled},
};
use std::collections::{BTreeMap, BTreeSet};

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
    /// 最後に info ログへ出した状態。`log_flow` がここからの差分だけを出す（重複も取りこぼしもしない）
    flow_seen: Broker,
    /// session → app 名。切断後も lease の手放しを名前で出せるよう、lease が残る間は覚えておく
    session_names: BTreeMap<String, String>,
    /// warn 済みの SendMidi の拒否（session, device, 理由）。同じ拒否を毎メッセージ出さない
    send_rejections: BTreeSet<(String, String, String)>,
}
impl Runtime {
    pub fn new(mut broker: Broker, store: SettingsStore) -> anyhow::Result<Self> {
        let saved = store.load()?;
        for (id, device) in &saved.devices {
            if !broker.devices.contains_key(id) {
                broker.add_device(id, &device.profile_id, false);
                broker.devices.get_mut(id).expect("restored device").name = device.name.clone();
            }
        }
        broker.restore(
            &saved
                .devices
                .iter()
                .map(|(id, d)| (id.clone(), d.assignment.clone()))
                .collect(),
        );
        Ok(Self {
            snapshot_sequence: std::sync::atomic::AtomicU64::new(1),
            flow_seen: broker.clone(),
            session_names: BTreeMap::new(),
            send_rejections: BTreeSet::new(),
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
            !hello.client_id.is_empty()
                && hello.client_id.len() <= 128
                && !hello.client_id.chars().any(char::is_control),
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
        self.session_names
            .insert(session.into(), hello.client_id.clone());
        tracing::info!(
            "session connected: {} ({}) native_midi={}",
            hello.client_id,
            crate::logging::printable(&hello.display_name),
            hello.native_midi
        );
        Ok(())
    }

    /// 前回ログに出してからの状態の変化を info に出す（flow.rs）。状態を変える処理の直後に呼ぶ
    pub fn log_flow(&mut self) {
        let names = &self.session_names;
        let name_of = |s: &str| names.get(s).cloned();
        let lines = crate::flow::changes(&self.flow_seen, &self.broker, &name_of);
        // 変化が無ければ写しは要らない（差分は changes が見る項目だけで決まる）
        if !lines.is_empty() {
            for line in lines {
                tracing::info!("{line}");
            }
            self.flow_seen = self.broker.clone();
        }
        // 接続も lease も無くなったセッションの名前は、もう行に出ることがない
        let broker = &self.broker;
        self.session_names.retain(|session, _| {
            broker.client_for_session(session).is_some()
                || broker
                    .devices
                    .values()
                    .any(|d| d.lease.as_ref().is_some_and(|l| &l.session_id == session))
        });
    }

    /// MIDI I/O の障害を記録する。今の lease の障害で、その機材で最初のものだけ warn して true。
    /// 壊れたポートは毎パケット失敗するので、2 回目以降は黙る（最初の理由を残す）
    pub fn record_fault(
        &mut self,
        device_id: &str,
        lease: &midistage_protocol::Lease,
        message: String,
    ) -> bool {
        let current = self
            .broker
            .devices
            .get(device_id)
            .and_then(|d| d.lease.as_ref())
            == Some(lease);
        if !current || self.driver_errors.contains_key(device_id) {
            return false;
        }
        tracing::warn!(device = %device_id, token = %lease.token, "MIDI I/O fault: {message}");
        self.driver_errors.insert(device_id.into(), message);
        let _ = self.broker.set_present(device_id, false);
        self.log_flow();
        true
    }

    /// SendMidi の拒否を warn すべきか（同じ session・機材・理由は、直るか切断されるまで 1 回だけ）
    pub fn first_send_rejection(&mut self, session: &str, device_id: &str, reason: &str) -> bool {
        self.send_rejections
            .insert((session.into(), device_id.into(), reason.into()))
    }

    /// SendMidi が通った。次に同じ拒否が起きたらまた出す
    pub fn send_succeeded(&mut self, session: &str, device_id: &str) {
        self.send_rejections
            .retain(|(s, d, _)| !(s == session && d == device_id));
    }

    pub fn client_name(&self, session: &str) -> String {
        self.session_names
            .get(session)
            .cloned()
            .unwrap_or_else(|| "?".into())
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
        if self.broker.client_for_session(session).is_some() {
            tracing::info!("session disconnected: {}", self.client_name(session));
        }
        self.native_sessions.remove(session);
        self.send_rejections.retain(|(s, _, _)| s != session);
        self.broker.disconnect_session(session);
        self.log_flow();
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
    #[test]
    fn a_repeating_io_fault_is_reported_once_per_device() {
        let dir = tempfile::tempdir().unwrap();
        let mut r = runtime(dir.path());
        r.hello("a", &hello("ladyland")).unwrap();
        let lease = midistage_protocol::Lease {
            session_id: "a".into(),
            token: "boot:1".into(),
        };
        r.broker.devices.get_mut("nano").unwrap().lease = Some(lease.clone());
        assert!(r.record_fault("nano", &lease, "MIDIReceived status -1".into()));
        assert!(!r.broker.devices["nano"].present);
        // 毎パケット来ても 2 回目以降は黙る。最初の理由を残す
        assert!(!r.record_fault("nano", &lease, "MIDIReceived status -2".into()));
        assert_eq!(r.driver_errors["nano"], "MIDIReceived status -1");
    }
    #[test]
    fn a_fault_from_an_old_lease_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let mut r = runtime(dir.path());
        let old = midistage_protocol::Lease {
            session_id: "gone".into(),
            token: "boot:0".into(),
        };
        assert!(!r.record_fault("nano", &old, "late".into()));
        assert!(r.driver_errors.is_empty());
        assert!(r.broker.devices["nano"].present);
    }
    #[test]
    fn names_of_live_sessions_survive_many_short_sessions() {
        let dir = tempfile::tempdir().unwrap();
        let mut r = runtime(dir.path());
        r.hello("a", &hello("ladyland")).unwrap();
        for i in 0..300 {
            let s = format!("s{i}");
            r.hello(&s, &hello(&format!("probe{i}"))).unwrap();
            r.disconnect(&s);
        }
        assert_eq!(r.client_name("a"), "ladyland");
        assert!(
            r.session_names.len() <= 2,
            "去ったセッションの名前は残らない: {}",
            r.session_names.len()
        );
    }
    #[test]
    fn a_disconnected_session_keeps_its_name_while_it_still_holds_a_lease() {
        let dir = tempfile::tempdir().unwrap();
        let mut r = runtime(dir.path());
        r.hello("a", &hello("ladyland")).unwrap();
        r.broker.devices.get_mut("nano").unwrap().lease = Some(midistage_protocol::Lease {
            session_id: "a".into(),
            token: "boot:1".into(),
        });
        r.log_flow();
        r.disconnect("a");
        // releasing の間は lease が残る。手放しの行に名前を出すため覚えておく
        assert!(
            r.broker.devices["nano"].lease.is_some(),
            "{:?}",
            r.broker.devices["nano"]
        );
        assert_eq!(r.client_name("a"), "ladyland");
    }
    #[test]
    fn client_ids_with_control_characters_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let mut r = runtime(dir.path());
        assert!(r.hello("a", &hello("vp\nFAKE line")).is_err());
        assert!(r.hello("a", &hello("vp\u{1b}[31m")).is_err());
        r.hello("a", &hello("ラジオ")).unwrap();
    }
    #[test]
    fn the_same_send_rejection_is_warned_once_until_it_recovers() {
        let dir = tempfile::tempdir().unwrap();
        let mut r = runtime(dir.path());
        assert!(r.first_send_rejection("a", "nano", "stale_lease"));
        assert!(!r.first_send_rejection("a", "nano", "stale_lease"));
        assert!(r.first_send_rejection("a", "nano", "invalid MIDI message size"));
        assert!(r.first_send_rejection("b", "nano", "stale_lease"));
        r.send_succeeded("a", "nano");
        assert!(
            r.first_send_rejection("a", "nano", "stale_lease"),
            "直った後の再発はまた出す"
        );
        r.disconnect("b");
        assert!(
            r.first_send_rejection("b", "nano", "stale_lease"),
            "切断で忘れる"
        );
    }
    #[test]
    fn saved_generic_keyboard_is_restored_even_before_os_enumeration() {
        let dir = tempfile::tempdir().unwrap();
        let mut r = runtime(dir.path());
        r.broker
            .add_device("generic:studio keyboard midi", "generic", true);
        r.hello("s", &hello("ladyland")).unwrap();
        r.set_enabled(
            "s",
            &SetEnabled {
                device_id: "generic:studio keyboard midi".into(),
                enabled: true,
                expected_revision: 0,
                takeover: false,
            },
        )
        .unwrap();
        let restored = runtime(dir.path());
        let device = &restored.broker.devices["generic:studio keyboard midi"];
        assert_eq!(device.assignment.client_id.as_deref(), Some("ladyland"));
        assert_eq!(device.phase, Phase::Waiting);
    }
}

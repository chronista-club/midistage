use crate::settings::{SavedDevice, SavedSettings, SettingsStore};
use midistage_protocol::{
    Broker, PROTOCOL_VERSION,
    wire::{Hello, SetEnabled},
};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

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
    /// design 06: 設定ポート（Roto の Serial）でやる用事の列。1 つずつ 開く → 済ませる → 閉じる
    setup_jobs: VecDeque<SetupJob>,
    /// 切り替えを済ませた（または積んだ）lease。device → token。同じ lease に 2 回切り替えない
    switched_for: BTreeMap<String, String>,
    /// 設定ポートを開いて用事をしている機材（その間 MIDI は止まる）
    pub configuring: BTreeSet<String>,
    /// midistage が最後に選んだ setup
    pub current_setup: BTreeMap<String, u8>,
    /// 設定ポートの用事が失敗した理由
    pub config_errors: BTreeMap<String, String>,
}

/// 機材の setup にする用事（design 06）
#[derive(Clone, Debug, PartialEq)]
pub struct SetupJob {
    pub device_id: String,
    pub client_id: String,
    /// この lease の間だけ有効。lease が移っていたら捨てる
    pub token: String,
    pub setup: u8,
    pub action: SetupAction,
}
#[derive(Clone, Debug, PartialEq)]
pub enum SetupAction {
    /// 表に出す setup を選ぶ（SET SETUP）
    Select,
    /// setup の中身を書く（段 C。検査済みの席）
    Apply(crate::roto_setup::Desired),
}

/// setup を持つ機材（Serial で切り替えられる）
const SETUP_PROFILES: &[&str] = &["roto"];
impl Runtime {
    pub fn new(mut broker: Broker, store: SettingsStore) -> anyhow::Result<Self> {
        let mut saved = store.load()?;
        // profile ができる前に汎用として覚えた機材は、今は profile の ID で出る。残すと同じ機材が 2 つ並ぶ。
        // 担当は profile 側を正とし、汎用の記録は保存先からも消す
        let stale: Vec<String> = saved
            .devices
            .keys()
            .filter(|id| crate::inventory::superseded_generic(id))
            .cloned()
            .collect();
        if !stale.is_empty() {
            saved.devices.retain(|id, _| !stale.contains(id));
            match store.save(&saved) {
                Ok(()) => {
                    tracing::info!("dropped generic devices now recognized by a profile: {stale:?}")
                }
                // 保存できなくても、この起動では出さない（次の起動でまた消す）
                Err(error) => tracing::warn!(
                    "could not drop generic devices {stale:?} from settings: {error}"
                ),
            }
        }
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
            setup_jobs: VecDeque::new(),
            switched_for: BTreeMap::new(),
            configuring: BTreeSet::new(),
            current_setup: BTreeMap::new(),
            config_errors: BTreeMap::new(),
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

    /// setup の持ち主（client_id → setup）
    pub fn setup_owners(&self, device_id: &str) -> BTreeMap<String, Vec<u8>> {
        self.saved
            .device_setups
            .get(device_id)
            .map(|owners| {
                owners
                    .iter()
                    .map(|(client, s)| (client.clone(), s.setups.clone()))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// 自分の setup を登録する。空なら取り消し。他のアプリの setup は取れない
    pub fn set_device_setups(
        &mut self,
        session: &str,
        request: &midistage_protocol::wire::SetDeviceSetups,
    ) -> anyhow::Result<()> {
        let client = self
            .broker
            .client_for_session(session)
            .ok_or(midistage_protocol::AccessError::UnknownSession)?
            .to_owned();
        let device = self
            .broker
            .devices
            .get(&request.device_id)
            .ok_or(midistage_protocol::AccessError::UnknownDevice)?;
        anyhow::ensure!(
            SETUP_PROFILES.contains(&device.profile_id.as_str()),
            "device has no setups"
        );
        anyhow::ensure!(request.setups.iter().all(|s| *s < 64), "setup must be 0-63");
        let taken: Vec<u8> = self
            .setup_owners(&request.device_id)
            .into_iter()
            .filter(|(other, _)| *other != client)
            .flat_map(|(_, s)| s)
            .filter(|s| request.setups.contains(s))
            .collect();
        if !taken.is_empty() {
            return Err(midistage_protocol::AccessError::Conflict.into());
        }
        let mut saved = self.saved.clone();
        let owners = saved
            .device_setups
            .entry(request.device_id.clone())
            .or_default();
        if request.setups.is_empty() {
            owners.remove(&client);
        } else {
            let mut list = request.setups.clone();
            list.dedup();
            let last = owners
                .get(&client)
                .and_then(|o| o.last)
                .filter(|l| list.contains(l));
            owners.insert(
                client.clone(),
                crate::settings::ClientSetups { setups: list, last },
            );
        }
        if owners.is_empty() {
            saved.device_setups.remove(&request.device_id);
        }
        self.store.save(&saved)?;
        self.saved = saved;
        // いま lease を持っていれば、次の計画で切り替える
        self.switched_for.remove(&request.device_id);
        tracing::info!(
            "{}: setups of {client} = {:?}",
            request.device_id,
            request.setups
        );
        Ok(())
    }

    /// lease を持つアプリが、自分の setup の中で切り替える（列に積む）
    pub fn select_setup(
        &mut self,
        session: &str,
        request: &midistage_protocol::wire::SelectSetup,
    ) -> anyhow::Result<()> {
        self.broker
            .authorize_output(&request.device_id, session, &request.lease_token)?;
        let client = self
            .broker
            .client_for_session(session)
            .unwrap_or_default()
            .to_owned();
        anyhow::ensure!(
            self.setup_owners(&request.device_id)
                .get(&client)
                .is_some_and(|s| s.contains(&request.setup)),
            "setup is not registered by this app"
        );
        self.switched_for
            .insert(request.device_id.clone(), request.lease_token.clone());
        self.enqueue_setup_job(SetupJob {
            device_id: request.device_id.clone(),
            client_id: client,
            token: request.lease_token.clone(),
            setup: request.setup,
            action: SetupAction::Select,
        });
        Ok(())
    }

    /// setup の中身を書く（段 C）。lease を持つアプリが、自分の台帳の setup にだけ。
    /// 中身はここで検査して、壊れていれば受け付けない（列には積まない）
    pub fn apply_device_setup(
        &mut self,
        session: &str,
        request: &midistage_protocol::wire::ApplyDeviceSetup,
    ) -> anyhow::Result<()> {
        self.broker
            .authorize_output(&request.device_id, session, &request.lease_token)?;
        let client = self
            .broker
            .client_for_session(session)
            .unwrap_or_default()
            .to_owned();
        anyhow::ensure!(
            self.setup_owners(&request.device_id)
                .get(&client)
                .is_some_and(|s| s.contains(&request.setup)),
            "setup is not registered by this app"
        );
        let desired = crate::roto_setup::desired(request.setup, &request.content)?;
        self.enqueue_setup_job(SetupJob {
            device_id: request.device_id.clone(),
            client_id: client,
            token: request.lease_token.clone(),
            setup: request.setup,
            action: SetupAction::Apply(desired),
        });
        Ok(())
    }

    /// 用事を列に積む。**受け付けた時点から** その機材は configuring（列で待つ間も）。
    /// アプリは「受付の応答のあと、configuring が false になり config_error が無い」を完了の印にする（design 06）
    fn enqueue_setup_job(&mut self, job: SetupJob) {
        self.configuring.insert(job.device_id.clone());
        self.setup_jobs.push_back(job);
    }

    /// lease が新しい持ち主に移った機材について、その持ち主の setup へ切り替える用事を積む（同じ lease に 1 回）
    pub fn plan_setup_switches(&mut self) {
        let mut jobs = Vec::new();
        for (id, device) in &self.broker.devices {
            let Some(lease) = device.lease.as_ref() else {
                continue;
            };
            if device.phase != midistage_protocol::Phase::Active
                || self.switched_for.get(id) == Some(&lease.token)
            {
                continue;
            }
            let Some(client) = self.broker.client_for_session(&lease.session_id) else {
                continue;
            };
            let Some(owned) = self.saved.device_setups.get(id).and_then(|o| o.get(client)) else {
                continue;
            };
            let Some(setup) = owned.last.or_else(|| owned.setups.first().copied()) else {
                continue;
            };
            jobs.push(SetupJob {
                device_id: id.clone(),
                client_id: client.to_owned(),
                token: lease.token.clone(),
                setup,
                action: SetupAction::Select,
            });
        }
        for job in jobs {
            self.switched_for
                .insert(job.device_id.clone(), job.token.clone());
            self.enqueue_setup_job(job);
        }
    }

    /// 次の用事。lease が移っていたものは捨てる（捨てた機材に用事が残らなければ configuring を外す）
    pub fn next_setup_job(&mut self) -> Option<SetupJob> {
        while let Some(job) = self.setup_jobs.pop_front() {
            let current = self
                .broker
                .devices
                .get(&job.device_id)
                .and_then(|d| d.lease.as_ref())
                .is_some_and(|l| l.token == job.token);
            if current {
                return Some(job);
            }
            self.settle_configuring(&job.device_id);
        }
        None
    }

    /// 列に用事が残っていなければ configuring を外す
    fn settle_configuring(&mut self, device_id: &str) {
        if !self.setup_jobs.iter().any(|j| j.device_id == device_id) {
            self.configuring.remove(device_id);
        }
    }

    /// 用事が済んだ。成功なら current_setup と last を記録、失敗なら理由を出す（繰り返さない）
    pub fn finish_setup_job(&mut self, job: &SetupJob, result: Result<(), String>) {
        self.settle_configuring(&job.device_id);
        match result {
            Ok(()) if matches!(job.action, SetupAction::Apply(_)) => {
                // 中身を書いただけ。表に出す setup は変わらない
                self.config_errors.remove(&job.device_id);
                tracing::info!(
                    "{}: setup {} written for {}",
                    job.device_id,
                    job.setup,
                    job.client_id
                );
            }
            Ok(()) => {
                self.config_errors.remove(&job.device_id);
                self.current_setup.insert(job.device_id.clone(), job.setup);
                tracing::info!(
                    "{}: setup {} selected for {}",
                    job.device_id,
                    job.setup,
                    job.client_id
                );
                let mut saved = self.saved.clone();
                if let Some(owned) = saved
                    .device_setups
                    .get_mut(&job.device_id)
                    .and_then(|o| o.get_mut(&job.client_id))
                    && owned.last != Some(job.setup)
                {
                    owned.last = Some(job.setup);
                    match self.store.save(&saved) {
                        Ok(()) => self.saved = saved,
                        Err(error) => tracing::warn!("could not save last setup: {error}"),
                    }
                }
            }
            Err(reason) => {
                tracing::warn!(
                    "{}: could not {} setup {} for {}: {reason}",
                    job.device_id,
                    if matches!(job.action, SetupAction::Apply(_)) {
                        "write"
                    } else {
                        "select"
                    },
                    job.setup,
                    job.client_id
                );
                self.config_errors.insert(job.device_id.clone(), reason);
            }
        }
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
    fn a_generic_entry_left_from_before_its_profile_existed_is_dropped() {
        use crate::settings::{SavedDevice, SavedSettings};
        let dir = tempfile::tempdir().unwrap();
        let saved = |profile: &str, name: &str, client: Option<&str>, revision| SavedDevice {
            profile_id: profile.into(),
            name: name.into(),
            assignment: midistage_protocol::Assignment {
                client_id: client.map(Into::into),
                revision,
                expected: false,
            },
        };
        SettingsStore::new(dir.path())
            .save(&SavedSettings {
                version: 1,
                devices: [
                    ("zenith2".into(), saved("zenith2", "Zenith 2", None, 0)),
                    // profile ができる前の記録。担当が残っていても profile 側を正とする
                    (
                        "generic:zenith 2".into(),
                        saved("generic", "Zenith 2", Some("vp"), 4),
                    ),
                    (
                        "generic:studio keyboard midi".into(),
                        saved("generic", "Studio Keyboard MIDI", Some("ladyland"), 1),
                    ),
                ]
                .into(),
                initialized_clients: Default::default(),
                device_setups: Default::default(),
            })
            .unwrap();
        let r = runtime(dir.path());
        assert!(!r.broker.devices.contains_key("generic:zenith 2"));
        assert!(r.broker.devices.contains_key("zenith2"));
        assert_eq!(r.broker.devices["zenith2"].assignment.client_id, None);
        assert_eq!(
            r.broker.devices["generic:studio keyboard midi"]
                .assignment
                .client_id
                .as_deref(),
            Some("ladyland"),
            "profile の無い汎用の機材はそのまま"
        );
        // 保存先からも消える（次の起動でも出ない）
        let reloaded = SettingsStore::new(dir.path()).load().unwrap();
        assert!(!reloaded.devices.contains_key("generic:zenith 2"));
        assert!(
            reloaded
                .devices
                .contains_key("generic:studio keyboard midi")
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

/// design 06: Roto の setup の台帳と、持ち主に合わせた切り替え
#[cfg(test)]
mod setup_tests {
    use super::*;
    use midistage_protocol::{
        PROTOCOL_VERSION, Phase,
        wire::{ApplyDeviceSetup, SelectSetup, SetDeviceSetups, SetupContent},
    };

    fn hello(client: &str) -> Hello {
        Hello {
            protocol_version: PROTOCOL_VERSION,
            client_id: client.into(),
            display_name: client.into(),
            auth_token: "test".into(),
            native_midi: true,
            initial_enabled_profiles: vec![],
        }
    }
    fn runtime(path: &std::path::Path) -> Runtime {
        let mut broker = Broker::new("boot");
        broker.add_device("roto", "roto", true);
        broker.add_device("nano", "nanokontrol", true);
        Runtime::new(broker, SettingsStore::new(path)).unwrap()
    }
    fn setups(device: &str, list: &[u8]) -> SetDeviceSetups {
        SetDeviceSetups {
            device_id: device.into(),
            setups: list.to_vec(),
        }
    }
    /// session に機材を取らせ、lease の token を返す
    fn take(r: &mut Runtime, session: &str, device: &str) -> String {
        let revision = r.broker.devices[device].assignment.revision;
        r.set_enabled(
            session,
            &SetEnabled {
                device_id: device.into(),
                enabled: true,
                expected_revision: revision,
                takeover: true,
            },
        )
        .unwrap();
        // 引き継ぎ中なら、前の持ち主が静かになって手放したことにする
        if r.broker.devices[device].phase == Phase::Releasing {
            let lease = r.broker.devices[device].lease.clone().unwrap();
            r.broker
                .quiesced(device, &lease.session_id, &lease.token)
                .unwrap();
            r.broker.drained(device).unwrap();
        }
        assert_eq!(r.broker.devices[device].phase, Phase::Active);
        r.broker.devices[device].lease.clone().unwrap().token
    }

    #[test]
    fn each_app_registers_its_own_setups_and_cannot_take_another_apps() {
        let dir = tempfile::tempdir().unwrap();
        let mut r = runtime(dir.path());
        r.hello("l", &hello("ladyland")).unwrap();
        r.hello("v", &hello("vp")).unwrap();
        r.set_device_setups("l", &setups("roto", &[0, 1, 2, 3]))
            .unwrap();
        r.set_device_setups("v", &setups("roto", &[6])).unwrap();
        assert!(
            r.set_device_setups("v", &setups("roto", &[3, 6])).is_err(),
            "3 は ladyland のもの"
        );
        assert!(
            r.set_device_setups("v", &setups("roto", &[64])).is_err(),
            "setup は 0〜63"
        );
        assert!(
            r.set_device_setups("v", &setups("nano", &[0])).is_err(),
            "setup を持たない機材"
        );
        assert_eq!(r.setup_owners("roto")["vp"], vec![6]);
        // 保存され、再起動しても残る
        let again = runtime(dir.path());
        assert_eq!(again.setup_owners("roto")["ladyland"], vec![0, 1, 2, 3]);
        // 空で取り消し
        r.set_device_setups("v", &setups("roto", &[])).unwrap();
        assert!(!r.setup_owners("roto").contains_key("vp"));
    }

    #[test]
    fn when_the_lease_moves_the_new_owners_setup_is_selected_once() {
        let dir = tempfile::tempdir().unwrap();
        let mut r = runtime(dir.path());
        r.hello("l", &hello("ladyland")).unwrap();
        r.hello("v", &hello("vp")).unwrap();
        r.set_device_setups("l", &setups("roto", &[2, 3])).unwrap();
        r.set_device_setups("v", &setups("roto", &[6])).unwrap();

        let ladyland = take(&mut r, "l", "roto");
        r.plan_setup_switches();
        let job = r.next_setup_job().expect("ladyland の最初の setup へ");
        assert_eq!((job.setup, job.token.as_str()), (2, ladyland.as_str()));
        assert!(r.configuring.contains("roto"), "開いている間は configuring");
        r.finish_setup_job(&job, Ok(()));
        assert!(!r.configuring.contains("roto"));
        assert_eq!(r.current_setup.get("roto"), Some(&2));
        r.plan_setup_switches();
        assert!(r.next_setup_job().is_none(), "同じ lease には 1 回だけ");

        let vp = take(&mut r, "v", "roto");
        r.plan_setup_switches();
        let job = r.next_setup_job().unwrap();
        assert_eq!((job.setup, job.token.as_str()), (6, vp.as_str()));
        r.finish_setup_job(&job, Ok(()));
        assert_eq!(r.current_setup.get("roto"), Some(&6));
    }

    #[test]
    fn returning_to_an_app_restores_its_last_setup() {
        let dir = tempfile::tempdir().unwrap();
        let mut r = runtime(dir.path());
        r.hello("l", &hello("ladyland")).unwrap();
        r.hello("v", &hello("vp")).unwrap();
        r.set_device_setups("l", &setups("roto", &[0, 1, 2, 3]))
            .unwrap();
        r.set_device_setups("v", &setups("roto", &[6])).unwrap();
        let token = take(&mut r, "l", "roto");
        r.plan_setup_switches();
        let first = r.next_setup_job().unwrap();
        r.finish_setup_job(&first, Ok(()));
        // ladyland が自分の中で 3 へ
        r.select_setup(
            "l",
            &SelectSetup {
                device_id: "roto".into(),
                lease_token: token,
                setup: 3,
            },
        )
        .unwrap();
        let job = r.next_setup_job().unwrap();
        r.finish_setup_job(&job, Ok(()));
        // vp に渡して、また ladyland に戻る
        take(&mut r, "v", "roto");
        r.plan_setup_switches();
        let job = r.next_setup_job().unwrap();
        r.finish_setup_job(&job, Ok(()));
        take(&mut r, "l", "roto");
        r.plan_setup_switches();
        assert_eq!(
            r.next_setup_job().unwrap().setup,
            3,
            "最後に選んだ 3 へ戻る"
        );
    }

    #[test]
    fn only_the_lease_holder_can_select_and_only_its_own_setups() {
        let dir = tempfile::tempdir().unwrap();
        let mut r = runtime(dir.path());
        r.hello("l", &hello("ladyland")).unwrap();
        r.hello("v", &hello("vp")).unwrap();
        r.set_device_setups("l", &setups("roto", &[2, 3])).unwrap();
        r.set_device_setups("v", &setups("roto", &[6])).unwrap();
        let token = take(&mut r, "l", "roto");
        let select = |setup, token: &str| SelectSetup {
            device_id: "roto".into(),
            lease_token: token.into(),
            setup,
        };
        assert!(
            r.select_setup("v", &select(6, &token)).is_err(),
            "lease を持たない"
        );
        assert!(
            r.select_setup("l", &select(6, &token)).is_err(),
            "vp の setup"
        );
        assert!(
            r.select_setup("l", &select(3, "old")).is_err(),
            "古い token"
        );
        r.select_setup("l", &select(3, &token)).unwrap();
    }

    #[test]
    fn a_job_for_a_lease_that_already_moved_is_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let mut r = runtime(dir.path());
        r.hello("l", &hello("ladyland")).unwrap();
        r.hello("v", &hello("vp")).unwrap();
        r.set_device_setups("l", &setups("roto", &[2])).unwrap();
        take(&mut r, "l", "roto");
        r.plan_setup_switches();
        // 切り替える前に vp が取った（vp は setup を登録していない）
        take(&mut r, "v", "roto");
        assert!(
            r.next_setup_job().is_none(),
            "古い lease の切り替えはしない"
        );
        assert!(!r.configuring.contains("roto"));
    }

    #[test]
    fn a_failed_switch_is_shown_and_not_retried() {
        let dir = tempfile::tempdir().unwrap();
        let mut r = runtime(dir.path());
        r.hello("l", &hello("ladyland")).unwrap();
        r.set_device_setups("l", &setups("roto", &[2])).unwrap();
        take(&mut r, "l", "roto");
        r.plan_setup_switches();
        let job = r.next_setup_job().unwrap();
        r.finish_setup_job(&job, Err("port busy".into()));
        assert_eq!(
            r.config_errors.get("roto").map(String::as_str),
            Some("port busy")
        );
        assert_eq!(r.current_setup.get("roto"), None);
        r.plan_setup_switches();
        assert!(r.next_setup_job().is_none());
    }

    #[test]
    fn configuring_is_true_from_acceptance_until_the_last_queued_job_finishes() {
        let dir = tempfile::tempdir().unwrap();
        let mut r = runtime(dir.path());
        r.hello("l", &hello("ladyland")).unwrap();
        r.set_device_setups("l", &setups("roto", &[2, 3])).unwrap();
        let token = take(&mut r, "l", "roto");
        r.plan_setup_switches();
        // まだ誰も列を取り出していない = 受け付けただけ。それでも configuring
        assert!(r.configuring.contains("roto"), "受付の直後から configuring");
        let select = |setup| SelectSetup {
            device_id: "roto".into(),
            lease_token: token.clone(),
            setup,
        };
        r.select_setup("l", &select(3)).unwrap();
        let first = r.next_setup_job().unwrap();
        r.finish_setup_job(&first, Ok(()));
        assert!(
            r.configuring.contains("roto"),
            "2 つ目が列に残っている間は configuring のまま"
        );
        let second = r.next_setup_job().unwrap();
        assert_eq!(second.setup, 3);
        r.finish_setup_job(&second, Ok(()));
        assert!(
            !r.configuring.contains("roto"),
            "列が空になって初めて外れる"
        );
        assert!(r.next_setup_job().is_none());
    }
    #[test]
    fn apply_is_for_the_lease_holder_on_its_own_setup_with_valid_content() {
        let dir = tempfile::tempdir().unwrap();
        let mut r = runtime(dir.path());
        r.hello("l", &hello("ladyland")).unwrap();
        r.hello("v", &hello("vp")).unwrap();
        r.set_device_setups("l", &setups("roto", &[2, 3])).unwrap();
        r.set_device_setups("v", &setups("roto", &[6])).unwrap();
        let token = take(&mut r, "v", "roto");
        r.plan_setup_switches();
        let _ = r.next_setup_job().map(|j| r.finish_setup_job(&j, Ok(())));
        let apply = |setup, token: &str, content: SetupContent| ApplyDeviceSetup {
            device_id: "roto".into(),
            lease_token: token.into(),
            setup,
            content,
        };
        let ok = SetupContent {
            name: "VP".into(),
            ..Default::default()
        };
        assert!(
            r.apply_device_setup("l", &apply(2, &token, ok.clone()))
                .is_err(),
            "lease を持たない"
        );
        assert!(
            r.apply_device_setup("v", &apply(3, &token, ok.clone()))
                .is_err(),
            "ladyland の setup"
        );
        let bad = SetupContent {
            name: "name too long!".into(),
            ..Default::default()
        };
        assert!(
            r.apply_device_setup("v", &apply(6, &token, bad)).is_err(),
            "壊れた中身は受け付けない"
        );
        assert!(!r.configuring.contains("roto"), "断ったものは列に積まない");
        r.apply_device_setup("v", &apply(6, &token, ok)).unwrap();
        assert!(
            r.configuring.contains("roto"),
            "受け付けた瞬間から configuring"
        );
        let job = r.next_setup_job().unwrap();
        assert!(matches!(job.action, SetupAction::Apply(ref d) if d.name == "VP"));
        r.finish_setup_job(&job, Ok(()));
        assert!(!r.configuring.contains("roto"));
        assert_eq!(
            r.current_setup.get("roto"),
            Some(&6),
            "書いても表の setup は変わらない（前に選んだ 6 のまま）"
        );
        assert!(!r.config_errors.contains_key("roto"));
    }
    #[test]
    fn registering_while_holding_the_lease_switches_now() {
        let dir = tempfile::tempdir().unwrap();
        let mut r = runtime(dir.path());
        r.hello("v", &hello("vp")).unwrap();
        take(&mut r, "v", "roto");
        r.plan_setup_switches();
        assert!(r.next_setup_job().is_none(), "まだ登録が無い");
        r.set_device_setups("v", &setups("roto", &[6])).unwrap();
        r.plan_setup_switches();
        assert_eq!(r.next_setup_job().unwrap().setup, 6);
    }
}

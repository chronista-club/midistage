//! mem_1CfnVkncPrmfdQet3bNSmx — 使用意思と実行中 lease を分離する契約。
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const PROTOCOL_VERSION: u32 = 1;
pub mod wire;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Assignment {
    pub client_id: Option<String>,
    pub revision: u64,
    pub expected: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lease {
    pub session_id: String,
    pub token: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Off,
    Waiting,
    Active,
    Releasing,
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceState {
    pub device_id: String,
    pub profile_id: String,
    pub name: String,
    pub present: bool,
    pub assignment: Assignment,
    pub phase: Phase,
    pub lease: Option<Lease>,
    pub releasing_to: Option<String>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum AccessError {
    #[error("unknown device")]
    UnknownDevice,
    #[error("unknown session")]
    UnknownSession,
    #[error("another application is assigned")]
    Conflict,
    #[error("state changed since confirmation")]
    StaleRevision,
    #[error("stale lease")]
    StaleLease,
    #[error("handoff is pending")]
    Pending,
    #[error("application already has a live session")]
    AlreadyConnected,
}

#[derive(Clone)]
pub struct Broker {
    pub devices: BTreeMap<String, DeviceState>,
    sessions: BTreeMap<String, String>,
    quiescent: BTreeSet<String>,
    boot_id: String,
    sequence: u64,
}

impl Broker {
    /// boot_id はサービス起動ごとの UUID。再起動前の token を再発行しない。
    pub fn new(boot_id: impl Into<String>) -> Self {
        Self {
            devices: BTreeMap::new(),
            sessions: BTreeMap::new(),
            quiescent: BTreeSet::new(),
            boot_id: boot_id.into(),
            sequence: 0,
        }
    }
    pub fn register_session(&mut self, session: &str, client: &str) -> Result<(), AccessError> {
        if self.sessions.contains_key(session) || self.sessions.values().any(|c| c == client) {
            return Err(AccessError::AlreadyConnected);
        }
        self.sessions.insert(session.into(), client.into());
        let waiting: Vec<_> = self
            .devices
            .iter()
            .filter(|(_, d)| {
                d.phase == Phase::Waiting && d.assignment.client_id.as_deref() == Some(client)
            })
            .map(|(id, _)| id.clone())
            .collect();
        for id in waiting {
            self.activate(&id);
        }
        Ok(())
    }
    pub fn add_device(&mut self, id: &str, profile: &str, present: bool) {
        if self.devices.contains_key(id) {
            return;
        }
        self.devices.insert(
            id.into(),
            DeviceState {
                device_id: id.into(),
                profile_id: profile.into(),
                name: id.into(),
                present,
                assignment: Assignment {
                    client_id: None,
                    revision: 0,
                    expected: matches!(profile, "roto" | "lpd8" | "nanokontrol"),
                },
                phase: Phase::Off,
                lease: None,
                releasing_to: None,
            },
        );
    }
    pub fn enable(
        &mut self,
        id: &str,
        session: &str,
        revision: u64,
        takeover: bool,
    ) -> Result<(), AccessError> {
        let client = self
            .sessions
            .get(session)
            .ok_or(AccessError::UnknownSession)?
            .clone();
        let device = self.devices.get_mut(id).ok_or(AccessError::UnknownDevice)?;
        if device.assignment.revision != revision {
            return Err(AccessError::StaleRevision);
        }
        if device.phase == Phase::Releasing {
            return Err(AccessError::Pending);
        }
        if device.assignment.client_id.as_deref() == Some(&client) {
            return Ok(());
        }
        if device.assignment.client_id.is_some() && !takeover {
            return Err(AccessError::Conflict);
        }
        device.assignment.client_id = Some(client.clone());
        device.assignment.revision += 1;
        if device.lease.is_some() {
            device.phase = Phase::Releasing;
            device.releasing_to = Some(client);
            self.quiescent.remove(id);
        } else {
            self.activate(id);
        }
        Ok(())
    }
    pub fn disable(&mut self, id: &str, session: &str, revision: u64) -> Result<(), AccessError> {
        let client = self
            .sessions
            .get(session)
            .ok_or(AccessError::UnknownSession)?;
        let device = self.devices.get_mut(id).ok_or(AccessError::UnknownDevice)?;
        if device.assignment.revision != revision {
            return Err(AccessError::StaleRevision);
        }
        if device
            .assignment
            .client_id
            .as_ref()
            .is_some_and(|c| c != client)
        {
            return Err(AccessError::Conflict);
        }
        if device.assignment.client_id.is_none() {
            return Ok(());
        }
        device.assignment.client_id = None;
        device.assignment.revision += 1;
        device.releasing_to = None;
        if device.lease.is_some() {
            device.phase = Phase::Releasing;
        } else {
            device.phase = Phase::Off;
        }
        Ok(())
    }
    pub fn quiesced(&mut self, id: &str, session: &str, token: &str) -> Result<(), AccessError> {
        let device = self.devices.get(id).ok_or(AccessError::UnknownDevice)?;
        if device.phase != Phase::Releasing
            || !device
                .lease
                .as_ref()
                .is_some_and(|l| l.session_id == session && l.token == token)
        {
            return Err(AccessError::StaleLease);
        }
        self.quiescent.insert(id.into());
        Ok(())
    }
    /// adapter の送信キュー停止・in-flight 完了後に runtime が呼ぶ。
    pub fn drained(&mut self, id: &str) -> Result<(), AccessError> {
        let device = self.devices.get_mut(id).ok_or(AccessError::UnknownDevice)?;
        if device.phase != Phase::Releasing || !self.quiescent.remove(id) {
            return Err(AccessError::Pending);
        }
        device.lease = None;
        device.releasing_to = None;
        self.activate(id);
        Ok(())
    }
    pub fn authorize_output(
        &self,
        id: &str,
        session: &str,
        token: &str,
    ) -> Result<(), AccessError> {
        let device = self.devices.get(id).ok_or(AccessError::UnknownDevice)?;
        if device.phase == Phase::Active
            && device
                .lease
                .as_ref()
                .is_some_and(|l| l.session_id == session && l.token == token)
        {
            Ok(())
        } else {
            Err(AccessError::StaleLease)
        }
    }
    pub fn disconnect_session(&mut self, session: &str) {
        self.sessions.remove(session);
        for (id, device) in &mut self.devices {
            if device
                .lease
                .as_ref()
                .is_some_and(|l| l.session_id == session)
            {
                device.phase = Phase::Releasing;
                self.quiescent.insert(id.clone());
            }
        }
    }
    pub fn set_present(&mut self, id: &str, present: bool) -> Result<(), AccessError> {
        let device = self.devices.get_mut(id).ok_or(AccessError::UnknownDevice)?;
        if device.present == present {
            return Ok(());
        }
        device.present = present;
        if device.phase == Phase::Releasing {
            return Ok(());
        }
        if !present && device.lease.is_some() {
            device.phase = Phase::Releasing;
            self.quiescent.remove(id);
        } else {
            self.activate(id);
        }
        Ok(())
    }
    /// 起動時だけ呼ぶ。lease / セッションは保存状態から復元しない。
    pub fn restore(&mut self, saved: &BTreeMap<String, Assignment>) {
        for (id, assignment) in saved {
            if let Some(device) = self.devices.get_mut(id) {
                device.assignment = assignment.clone();
                device.lease = None;
                self.activate(id);
            }
        }
    }
    pub fn assignments(&self) -> BTreeMap<String, Assignment> {
        self.devices
            .iter()
            .map(|(id, device)| (id.clone(), device.assignment.clone()))
            .collect()
    }
    pub fn client_for_session(&self, session: &str) -> Option<&str> {
        self.sessions.get(session).map(String::as_str)
    }
    pub fn is_quiescent(&self, id: &str) -> bool {
        self.quiescent.contains(id)
    }
    pub fn epoch(&self) -> &str {
        &self.boot_id
    }
    fn activate(&mut self, id: &str) {
        let device = self
            .devices
            .get_mut(id)
            .expect("device validated by caller");
        device.lease = None;
        device.phase = if device.assignment.client_id.is_some() {
            Phase::Waiting
        } else {
            Phase::Off
        };
        if !device.present {
            return;
        }
        let Some(session) = self
            .sessions
            .iter()
            .find(|(_, client)| Some(*client) == device.assignment.client_id.as_ref())
            .map(|(session, _)| session.clone())
        else {
            return;
        };
        self.sequence += 1;
        device.lease = Some(Lease {
            session_id: session,
            token: format!("{}:{}", self.boot_id, self.sequence),
        });
        device.phase = Phase::Active;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rig() -> Broker {
        let mut b = Broker::new("test-boot");
        b.register_session("a1", "ladyland").unwrap();
        b.register_session("b1", "vp").unwrap();
        b.add_device("nano-1", "nanokontrol", true);
        b.add_device("lpd-1", "lpd8", true);
        b
    }

    #[test]
    fn enabling_one_device_does_not_enable_the_other() {
        let mut b = rig();
        b.enable("nano-1", "a1", 0, false).unwrap();
        assert_eq!(b.devices["nano-1"].phase, Phase::Active);
        assert_eq!(b.devices["lpd-1"].phase, Phase::Off);
        assert_eq!(
            b.devices["nano-1"].assignment.client_id.as_deref(),
            Some("ladyland")
        );
    }
    #[test]
    fn repeated_discovery_does_not_reset_assignment_or_lease() {
        let mut b = rig();
        b.enable("nano-1", "a1", 0, false).unwrap();
        let before = b.devices["nano-1"].clone();
        b.add_device("nano-1", "nanokontrol", true);
        assert_eq!(b.devices["nano-1"], before);
    }

    #[test]
    fn absent_device_remembers_intent_without_claiming_active() {
        let mut b = rig();
        b.add_device("fgdp-1", "fgdp", false);
        b.enable("fgdp-1", "a1", 0, false).unwrap();
        assert_eq!(b.devices["fgdp-1"].phase, Phase::Waiting);
        assert!(b.devices["fgdp-1"].lease.is_none());
    }

    #[test]
    fn handoff_requires_confirmation_and_two_sided_quiescence() {
        let mut b = rig();
        b.enable("nano-1", "a1", 0, false).unwrap();
        let old = b.devices["nano-1"].lease.clone().expect("old lease");
        assert_eq!(
            b.enable("nano-1", "b1", 1, false),
            Err(AccessError::Conflict)
        );
        b.enable("nano-1", "b1", 1, true).unwrap();
        assert_eq!(b.devices["nano-1"].phase, Phase::Releasing);
        assert_eq!(
            b.authorize_output("nano-1", "a1", &old.token),
            Err(AccessError::StaleLease)
        );
        assert_eq!(b.drained("nano-1"), Err(AccessError::Pending));
        b.quiesced("nano-1", "a1", &old.token).unwrap();
        b.drained("nano-1").unwrap();
        let next = b.devices["nano-1"].lease.clone().expect("successor lease");
        assert_eq!(next.session_id, "b1");
        assert_ne!(next.token, old.token);
        assert_eq!(
            b.authorize_output("nano-1", "a1", &old.token),
            Err(AccessError::StaleLease)
        );
        assert_eq!(b.authorize_output("nano-1", "b1", &next.token), Ok(()));
    }

    #[test]
    fn stale_confirmation_cannot_take_over_a_changed_device() {
        let mut b = rig();
        b.enable("nano-1", "a1", 0, false).unwrap();
        assert_eq!(
            b.enable("nano-1", "b1", 0, true),
            Err(AccessError::StaleRevision)
        );
        assert_eq!(
            b.devices["nano-1"].assignment.client_id.as_deref(),
            Some("ladyland")
        );
    }

    #[test]
    fn disconnect_revokes_runtime_lease_but_retains_saved_assignment() {
        let mut b = rig();
        b.enable("nano-1", "a1", 0, false).unwrap();
        b.disconnect_session("a1");
        assert_eq!(b.devices["nano-1"].phase, Phase::Releasing);
        b.drained("nano-1").unwrap();
        assert_eq!(b.devices["nano-1"].phase, Phase::Waiting);
        assert_eq!(
            b.devices["nano-1"].assignment.client_id.as_deref(),
            Some("ladyland")
        );
    }

    #[test]
    fn off_survives_restart_and_hotplug() {
        let mut b = rig();
        b.enable("nano-1", "a1", 0, false).unwrap();
        let lease = b.devices["nano-1"].lease.clone().unwrap();
        b.disable("nano-1", "a1", 1).unwrap();
        b.quiesced("nano-1", "a1", &lease.token).unwrap();
        b.drained("nano-1").unwrap();
        let saved = b.assignments();
        assert_eq!(saved["nano-1"].revision, 2);
        let mut restored = rig();
        restored.restore(&saved);
        restored.set_present("nano-1", false).unwrap();
        restored.set_present("nano-1", true).unwrap();
        assert_eq!(restored.devices["nano-1"].assignment.revision, 2);
        assert_eq!(restored.devices["nano-1"].phase, Phase::Off);
        assert!(restored.devices["nano-1"].lease.is_none());
    }

    #[test]
    fn unplug_requests_cleanup_and_replug_gets_fresh_lease() {
        let mut b = rig();
        b.enable("nano-1", "a1", 0, false).unwrap();
        let old = b.devices["nano-1"].lease.clone().unwrap();
        b.set_present("nano-1", false).unwrap();
        assert_eq!(b.devices["nano-1"].phase, Phase::Releasing);
        b.quiesced("nano-1", "a1", &old.token).unwrap();
        b.drained("nano-1").unwrap();
        assert_eq!(b.devices["nano-1"].phase, Phase::Waiting);
        b.set_present("nano-1", true).unwrap();
        assert_eq!(b.devices["nano-1"].phase, Phase::Active);
        assert_ne!(b.devices["nano-1"].lease.as_ref().unwrap().token, old.token);
    }
}

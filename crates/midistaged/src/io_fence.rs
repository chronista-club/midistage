use midistage_protocol::{AccessError, Broker, Lease, Phase};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

pub struct OutputRequest {
    pub destination: u32,
    pub bytes: Vec<u8>,
    pub completion: Option<tokio::sync::oneshot::Sender<Result<(), String>>>,
}
#[derive(Clone)]
pub struct OutputSink {
    pub destination: u32,
    pub sender: std::sync::mpsc::SyncSender<OutputRequest>,
}

pub struct IoFence {
    pub device_id: String,
    pub lease: Lease,
    pending: Arc<AtomicUsize>,
}
pub struct InFlight {
    pending: Arc<AtomicUsize>,
}
impl Clone for InFlight {
    fn clone(&self) -> Self {
        self.pending.fetch_add(1, Ordering::SeqCst);
        Self {
            pending: self.pending.clone(),
        }
    }
}
impl Drop for InFlight {
    fn drop(&mut self) {
        self.pending.fetch_sub(1, Ordering::SeqCst);
    }
}
impl IoFence {
    pub fn new(device_id: String, lease: Lease) -> Self {
        Self {
            device_id,
            lease,
            pending: Arc::new(AtomicUsize::new(0)),
        }
    }
    /// 呼び出し側は runtime mutex を保持。状態変更と送信開始を同一境界にする。
    pub fn begin(&self, broker: &Broker) -> Result<InFlight, AccessError> {
        broker.authorize_output(&self.device_id, &self.lease.session_id, &self.lease.token)?;
        self.pending.fetch_add(1, Ordering::SeqCst);
        Ok(InFlight {
            pending: self.pending.clone(),
        })
    }
    /// Private service finalization only; never exposed as a client write privilege.
    pub fn begin_cleanup(&self, broker: &Broker) -> Result<InFlight, AccessError> {
        let device = broker
            .devices
            .get(&self.device_id)
            .ok_or(AccessError::UnknownDevice)?;
        if device.lease.as_ref() != Some(&self.lease) {
            return Err(AccessError::StaleLease);
        }
        if device.phase != Phase::Releasing
            || !broker.is_quiescent(&self.device_id)
            || !self.is_idle()
        {
            return Err(AccessError::Pending);
        }
        self.pending.fetch_add(1, Ordering::SeqCst);
        Ok(InFlight {
            pending: self.pending.clone(),
        })
    }
    pub fn drained(&self, broker: &mut Broker) -> Result<(), AccessError> {
        if self.pending.load(Ordering::SeqCst) != 0 {
            return Err(AccessError::Pending);
        }
        if broker
            .devices
            .get(&self.device_id)
            .and_then(|d| d.lease.as_ref())
            != Some(&self.lease)
        {
            return Err(AccessError::StaleLease);
        }
        broker.drained(&self.device_id)
    }
    pub fn is_idle(&self) -> bool {
        self.pending.load(Ordering::SeqCst) == 0
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cleanup_requires_quiescent_old_lease_and_waits_for_bookkeeping() {
        let mut b = Broker::new("test");
        b.register_session("a", "ladyland").unwrap();
        b.register_session("b", "vp").unwrap();
        b.add_device("key", "keystage", true);
        b.enable("key", "a", 0, false).unwrap();
        let lease = b.devices["key"].lease.clone().unwrap();
        let fence = IoFence::new("key".into(), lease.clone());
        assert!(fence.begin_cleanup(&b).is_err());
        let driver = fence.begin(&b).unwrap();
        let bookkeeping = driver.clone();
        b.enable("key", "b", 1, true).unwrap();
        assert!(fence.begin_cleanup(&b).is_err());
        b.quiesced("key", "a", &lease.token).unwrap();
        drop(driver);
        assert!(!fence.is_idle());
        assert!(fence.begin_cleanup(&b).is_err());
        drop(bookkeeping);
        let cleanup = fence.begin_cleanup(&b).unwrap();
        assert!(fence.begin(&b).is_err());
        assert_eq!(fence.drained(&mut b), Err(AccessError::Pending));
        drop(cleanup);
        fence.drained(&mut b).unwrap();
        assert!(fence.begin_cleanup(&b).is_err());
    }
    #[test]
    fn handoff_waits_for_driver_completion_and_rejects_late_old_messages() {
        let mut b = Broker::new("test");
        b.register_session("a", "ladyland").unwrap();
        b.register_session("b", "vp").unwrap();
        b.add_device("roto", "roto", true);
        b.enable("roto", "a", 0, false).unwrap();
        let lease = b.devices["roto"].lease.clone().unwrap();
        let fence = IoFence::new("roto".into(), lease.clone());
        let sending = fence.begin(&b).unwrap();
        b.enable("roto", "b", 1, true).unwrap();
        b.quiesced("roto", "a", &lease.token).unwrap();
        assert_eq!(fence.drained(&mut b), Err(AccessError::Pending));
        assert!(fence.begin(&b).is_err());
        drop(sending);
        fence.drained(&mut b).unwrap();
        assert_eq!(b.devices["roto"].lease.as_ref().unwrap().session_id, "b");
        assert!(fence.begin(&b).is_err());
    }
}

//! 戻ってこないかもしれない OS の呼び出しを、別 thread で時限付きで行う（design 07）。
//! Windows MIDI Services の仮想デバイス作成は、midisrv が固まると戻らない（実測 2026-10-19）。
//! 時限切れはエラーにする。止まった thread は止められないので残し、それが戻るまでは次を始めない
//! （止まった thread を積み重ねない）。引き継ぎの完了待ちには使わない（design 03 の不変条件）
use std::{
    sync::mpsc,
    thread::{self, JoinHandle},
    time::Duration,
};

#[derive(Default)]
pub struct Watchdog {
    stuck: Option<JoinHandle<()>>,
}
impl Watchdog {
    pub fn call<T: Send + 'static>(
        &mut self,
        what: &str,
        limit: Duration,
        f: impl FnOnce() -> anyhow::Result<T> + Send + 'static,
    ) -> anyhow::Result<T> {
        if self.stuck.as_ref().is_some_and(|t| !t.is_finished()) {
            anyhow::bail!("a previous {what} still has not returned");
        }
        self.stuck = None;
        let (done, result) = mpsc::sync_channel(1);
        let thread = thread::Builder::new()
            .name(format!("watchdog: {what}"))
            .spawn(move || {
                // 時限切れの後に戻った結果は受け手がいない。T の drop で後片付けされる
                let _ = done.send(f());
            })?;
        match result.recv_timeout(limit) {
            Ok(result) => {
                let _ = thread.join();
                result
            }
            Err(_) => {
                self.stuck = Some(thread);
                anyhow::bail!("{what} did not return within {limit:?}")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[test]
    fn a_call_that_returns_in_time_gives_its_result() {
        let mut dog = Watchdog::default();
        assert_eq!(
            dog.call("open", Duration::from_secs(5), || Ok(7)).unwrap(),
            7
        );
        let error = dog
            .call::<()>("open", Duration::from_secs(5), || {
                anyhow::bail!("no device")
            })
            .unwrap_err();
        assert_eq!(error.to_string(), "no device");
    }

    #[test]
    fn a_stuck_call_times_out_and_blocks_the_next_until_it_returns() {
        let mut dog = Watchdog::default();
        let gate = Arc::new(Mutex::new(()));
        let held = gate.lock().unwrap();
        let inner = gate.clone();
        let error = dog
            .call("open", Duration::from_millis(50), move || {
                drop(inner.lock());
                Ok(())
            })
            .unwrap_err();
        assert!(error.to_string().contains("did not return"), "{error}");
        // 止まった呼び出しが残っている間は、次を始めずに断る
        let ran = Arc::new(Mutex::new(false));
        let flag = ran.clone();
        let error = dog
            .call("open", Duration::from_secs(5), move || {
                *flag.lock().unwrap() = true;
                Ok(())
            })
            .unwrap_err();
        assert!(
            error.to_string().contains("still has not returned"),
            "{error}"
        );
        assert!(!*ran.lock().unwrap());
        // 戻れば、また呼べる
        drop(held);
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            if dog.call("open", Duration::from_secs(5), || Ok(())).is_ok() {
                break;
            }
            assert!(std::time::Instant::now() < deadline);
            thread::sleep(Duration::from_millis(10));
        }
    }
}

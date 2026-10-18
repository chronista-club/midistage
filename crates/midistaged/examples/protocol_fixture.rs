//! Swift SDK 連携試験用。CoreMIDI を開かず一台の架空デバイスを提供する。
use midistage_protocol::Broker;
use midistaged::{
    runtime::Runtime,
    service::Service,
    settings::{SettingsStore, atomic_write},
};
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let directory = std::path::PathBuf::from(
        std::env::args_os()
            .nth(1)
            .expect("isolated state directory"),
    );
    let mut broker = Broker::new(uuid::Uuid::new_v4().to_string());
    broker.add_device("nano", "nanokontrol", true);
    // ROTO の偽物（design 06）。setup の用事は Serial を開かずに成功したことにする —
    // SetDeviceSetups / SelectSetup / 引き継ぎでの切り替えを、SDK から loopback で確かめるため
    broker.add_device("roto", "roto", true);
    let service = Service::start(Runtime::new(broker, SettingsStore::new(&directory))?).await?;
    atomic_write(
        &directory,
        "endpoint.json",
        &serde_json::to_vec(&service.endpoint)?,
    )?;
    let mut ticker = tokio::time::interval(std::time::Duration::from_millis(20));
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => break,
            _ = ticker.tick() => {
                let mut runtime = service.runtime.lock().unwrap();
                for id in ["nano", "roto"] {
                    if runtime.broker.is_quiescent(id) { runtime.broker.drained(id)?; }
                }
                runtime.plan_setup_switches();
                if let Some(job) = runtime.next_setup_job() {
                    runtime.finish_setup_job(&job, Ok(()));
                }
            }
        }
    }
    service.shutdown().await
}

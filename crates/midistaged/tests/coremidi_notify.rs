//! 後から別 process に生えた MIDI endpoint が、常駐中の midistaged から見えるか（実機の USB 抜き差しの代わり）。
//! CoreMIDI は endpoint 一覧の更新通知を、process で最初に作った client の run loop にだけ届ける。
//! run loop が回っていないと一覧は凍る（VP PR #1211 の実測、2026-10-16 LPD8 の差し直しを拾わなかった件）
#![cfg(target_os = "macos")]

use std::time::{Duration, Instant};

const HELPER_ENV: &str = "MIDISTAGED_TEST_HELPER_SOURCE";

fn source_names() -> Vec<String> {
    coremidi::Sources
        .into_iter()
        .filter_map(|s| s.display_name())
        .collect()
}

/// 子 process として呼ばれたときだけ動く: 仮想 source を作って数秒持つ
#[test]
#[ignore = "helper: sources_created_later_by_another_process_become_visible から起動される"]
fn helper_publish_virtual_source() {
    let Ok(name) = std::env::var(HELPER_ENV) else {
        return;
    };
    let client = coremidi::Client::new("midistaged-test-helper").unwrap();
    let _source = client.virtual_source(&name).unwrap();
    std::thread::sleep(Duration::from_secs(4));
}

#[test]
fn sources_created_later_by_another_process_become_visible() {
    midistaged::native::start_notify_thread();
    // 常駐中の midistaged と同じく、先に一度列挙しておく（ここで一覧が凍るかが問題）
    let name = format!("midistaged-test-{}", std::process::id());
    assert!(!source_names().contains(&name));

    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "helper_publish_virtual_source",
            "--exact",
            "--ignored",
            "--quiet",
        ])
        .env(HELPER_ENV, &name)
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut seen = false;
    while Instant::now() < deadline {
        if source_names().contains(&name) {
            seen = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let _ = child.kill();
    let _ = child.wait();
    assert!(
        seen,
        "別 process が後から作った source {name} が見えない（一覧が凍っている）"
    );
}

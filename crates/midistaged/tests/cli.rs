#[test]
fn cli_exposes_explicit_serve_and_read_only_inventory_commands() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_midistaged"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("serve"));
    assert!(text.contains("list-ports"));
}

/// MIDI の backend が無い OS では、列挙を空の成功にしない（design 07）
#[cfg(not(target_os = "macos"))]
#[test]
fn list_ports_fails_where_no_midi_backend_exists() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_midistaged"))
        .arg("list-ports")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("design 07"));
}

/// serve は止める合図が来るまで居続ける。endpoint.json を書いたあとも生きている
#[cfg(not(target_os = "macos"))]
#[test]
fn serve_stays_up_after_publishing_its_endpoint() {
    let dir = tempfile::tempdir().unwrap();
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_midistaged"))
        .arg("serve")
        .arg("--state-dir")
        .arg(dir.path().join("state"))
        .arg("--log-dir")
        .arg(dir.path().join("logs"))
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let endpoint = dir.path().join("state").join("endpoint.json");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while !endpoint.exists() {
        assert!(
            child.try_wait().unwrap().is_none(),
            "serve exited before publishing its endpoint"
        );
        assert!(std::time::Instant::now() < deadline, "no endpoint.json");
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    std::thread::sleep(std::time::Duration::from_millis(500));
    let still_running = child.try_wait().unwrap().is_none();
    let _ = child.kill();
    let _ = child.wait();
    assert!(
        still_running,
        "serve exited right after publishing its endpoint"
    );
}

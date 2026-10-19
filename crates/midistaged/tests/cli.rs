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

/// Windows: Windows.Devices.Midi2 の dll が隣に無ければ、理由を言って失敗する（空の成功にしない）
#[cfg(windows)]
#[test]
fn list_ports_without_the_midi2_dll_names_what_is_missing() {
    let dir = tempfile::tempdir().unwrap();
    let exe = dir.path().join("midistaged.exe");
    std::fs::copy(env!("CARGO_BIN_EXE_midistaged"), &exe).unwrap();
    let output = std::process::Command::new(&exe)
        .arg("list-ports")
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Windows.Devices.Midi2.dll"), "{stderr}");
}

/// Windows: dll があれば（scripts/winmidi.sh fetch 済みなら build.rs が写す）列挙が成功する
#[cfg(windows)]
#[test]
fn list_ports_with_the_midi2_dll_succeeds() {
    let exe = std::path::Path::new(env!("CARGO_BIN_EXE_midistaged"));
    if !exe.with_file_name("Windows.Devices.Midi2.dll").exists() {
        eprintln!(
            "skip: Windows.Devices.Midi2.dll is not next to {}",
            exe.display()
        );
        return;
    }
    let output = std::process::Command::new(exe)
        .arg("list-ports")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// MIDI の backend が無い OS では、列挙を空の成功にしない（design 07）
#[cfg(not(any(target_os = "macos", windows)))]
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

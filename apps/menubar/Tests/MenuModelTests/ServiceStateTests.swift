import Testing
@testable import MenuModel

let bundled = Bundled(version: "0.1.0", executable: "/Applications/midistage.app/Contents/MacOS/midistaged")

@Test func stoppedAndUnregisteredIsQuietGray() {
    let s = summarize(ServiceState(registration: .notRegistered, running: .stopped, bundled: bundled))
    #expect(s.icon == .stopped)
    #expect(s.title.contains("停止"))
    #expect(s.lines.contains { $0.contains("常駐は無効") })
    #expect(!s.needsApproval && !s.foreignBinary && !s.versionMismatch)
}

@Test func approvalPendingIsAttentionWithSettingsHint() {
    let s = summarize(ServiceState(registration: .requiresApproval, running: .stopped, bundled: bundled))
    #expect(s.icon == .attention)
    #expect(s.needsApproval)
    #expect(s.lines.contains { $0.contains("承認待ち") })
}

@Test func healthyBundledServiceIsNormal() {
    let running = Running.connected(version: "0.1.0", executable: bundled.executable, devices: DeviceSummary(present: 7))
    let s = summarize(ServiceState(registration: .enabled, running: running, bundled: bundled, appAtLogin: true))
    #expect(s.icon == .normal)
    #expect(s.title.contains("7 台"))
    #expect(s.lines.isEmpty)
}

@Test func serviceWithoutAppAtLoginGetsAHintNotAnAlarm() {
    let running = Running.connected(version: "0.1.0", executable: bundled.executable, devices: DeviceSummary(present: 7))
    let s = summarize(ServiceState(registration: .enabled, running: running, bundled: bundled, appAtLogin: false))
    #expect(s.icon == .normal)
    #expect(s.lines == ["ログイン時にこのアプリは起動しません（メニューで有効にできます）"])
}

@Test func foreignBinaryIsNamedButNotAlarming() {
    // cargo run の開発版が動いている。事実を出すだけで、止めない（design 03）
    let running = Running.connected(version: "0.1.0", executable: "/Users/x/repos/midistage/target/debug/midistaged", devices: DeviceSummary(present: 2))
    let s = summarize(ServiceState(registration: .enabled, running: running, bundled: bundled))
    #expect(s.foreignBinary)
    #expect(s.icon == .normal)
    #expect(s.lines.contains { $0.contains("別のバイナリ") })
}

@Test func versionMismatchAsksForRestart() {
    let running = Running.connected(version: "0.0.9", executable: bundled.executable, devices: DeviceSummary(present: 2))
    let s = summarize(ServiceState(registration: .enabled, running: running, bundled: bundled))
    #expect(s.versionMismatch)
    #expect(s.icon == .attention)
    #expect(s.lines.contains { $0.contains("再起動") })
}

@Test func stuckHandoffAndErrorsAreAttention() {
    let running = Running.connected(version: "0.1.0", executable: bundled.executable, devices: DeviceSummary(present: 3, releasingStuck: 1, errors: 1))
    let s = summarize(ServiceState(registration: .enabled, running: running, bundled: bundled, appAtLogin: true))
    #expect(s.icon == .attention)
    #expect(s.lines.count == 2)
}

@Test func oldServiceWithoutServiceInfoIsNotTreatedAsMismatch() {
    // service を載せない旧い midistaged: 版も実行ファイルも不明 → 嘘を言わない
    let running = Running.connected(version: nil, executable: nil, devices: DeviceSummary(present: 1))
    let s = summarize(ServiceState(registration: .enabled, running: running, bundled: bundled))
    #expect(!s.versionMismatch && !s.foreignBinary)
    #expect(s.icon == .normal)
}

@Test func tracingIsVisibleSoItIsNotLeftOn() {
    let running = Running.connected(version: "0.1.0", executable: bundled.executable, devices: DeviceSummary(present: 7))
    let s = summarize(ServiceState(registration: .enabled, running: running, bundled: bundled, appAtLogin: true, tracing: true))
    #expect(s.icon == .attention)
    #expect(s.lines.contains { $0.contains("MIDI トレース中") })
}

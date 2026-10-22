import Testing
@testable import MenuModel

// REQ-APP-062: アプリが新しくなったあと、動いている midistaged をいつ入れ替えるか
private let newBundle = Bundled(version: "0.2.0", executable: "/Applications/midistage.app/Contents/MacOS/midistaged")

private func state(running version: String, executable: String? = nil, holders: [String] = [], registration: Registration = .enabled) -> ServiceState {
    var s = ServiceState(registration: registration,
                         running: .connected(version: version, executable: executable ?? newBundle.executable, devices: DeviceSummary(present: 3)),
                         bundled: newBundle, appAtLogin: true)
    s.leaseHolders = holders
    return s
}

@Test func anOldServiceNobodyUsesIsRestartedNow() {
    #expect(serviceUpdate(state(running: "0.1.0"), tried: nil) == .restartNow)
}

@Test func anOldServiceInUseWaitsAndSaysWho() {
    #expect(serviceUpdate(state(running: "0.1.0", holders: ["vp", "ladyland", "vp"]), tried: nil) == .waitForQuiet(holders: ["ladyland", "vp"]))
    let s = summarize(state(running: "0.1.0", holders: ["vp"]))
    #expect(s.lines.contains { $0.contains("更新待ち") && $0.contains("vp") })
}

@Test func theSameVersionNeedsNothing() {
    #expect(serviceUpdate(state(running: "0.2.0"), tried: nil) == .none)
}

@Test func aDevelopmentBinaryIsNeverReplacedAutomatically() {
    let dev = state(running: "0.1.0", executable: "/Users/me/repos/midistage/target/debug/midistaged")
    #expect(serviceUpdate(dev, tried: nil) == .none)
}

@Test func anAutomaticRestartIsTriedOncePerVersion() {
    #expect(serviceUpdate(state(running: "0.1.0"), tried: "0.2.0") == .none)
    #expect(serviceUpdate(state(running: "0.1.0"), tried: "0.1.5") == .restartNow, "前の版で試したのは数えない")
}

@Test func nothingHappensWhenTheServiceIsNotRegisteredOrNotConnected() {
    #expect(serviceUpdate(state(running: "0.1.0", registration: .notRegistered), tried: nil) == .none)
    var stopped = state(running: "0.1.0")
    stopped.running = .stopped
    #expect(serviceUpdate(stopped, tried: nil) == .none)
    var old = state(running: "0.1.0")
    old.running = .connected(version: nil, executable: newBundle.executable, devices: DeviceSummary(present: 0))
    #expect(serviceUpdate(old, tried: nil) == .none, "版を名乗らない古いサービスは比べられない")
}

@Test func anOlderBundleNeverDowngradesTheService() {
    // 古いアプリが入っていても、新しいサービスを古い版に入れ替えない
    #expect(serviceUpdate(state(running: "0.3.0"), tried: nil) == .none)
    #expect(serviceUpdate(state(running: "0.10.0"), tried: nil) == .none, "0.10 は 0.2 より新しい（文字の順ではない）")
}

@Test func versionsCompareByNumbers() {
    #expect(isNewer("0.2.0", than: "0.1.9"))
    #expect(isNewer("0.10.0", than: "0.9.0"))
    #expect(isNewer("1.0", than: "0.99.1"))
    #expect(!isNewer("0.1.0", than: "0.1.0"))
    #expect(!isNewer("dev", than: "0.1.0"), "数字で読めない版は新しいとみなさない")
}

@Test func anOlderBundleIsMentionedQuietlyAndNeverPromisesASwap() {
    let s = summarize(state(running: "0.3.0", holders: ["vp"]))
    #expect(s.icon == .normal)
    #expect(s.versionMismatch)
    #expect(s.lines.contains { $0.contains("同梱の方が古い") })
    #expect(!s.lines.contains { $0.contains("更新待ち") || $0.contains("入れ替わります") })
}

import Foundation
import Testing
@testable import MidistageClient

@Test func rustSnapshotDecodesNativeLeaseAndExplicitOff() throws {
    let url = Bundle.module.url(forResource: "snapshot", withExtension: "json", subdirectory: "Fixtures")!
    let snapshot = try JSONDecoder().decode(Snapshot.self, from: Data(contentsOf: url))
    #expect(snapshot.protocolVersion == 1)
    #expect(snapshot.devices.count == 2)
    guard snapshot.devices.count == 2 else { return }
    #expect(snapshot.devices[0].deviceID == "nanokontrol")
    #expect(snapshot.devices[0].phase == "active")
    #expect(snapshot.devices[0].nativeInputs == ["Midistage/ladyland/boot:1 | nanoKONTROL2 SLIDER/KNOB"])
    #expect(snapshot.devices[1].phase == "off")
}

@Test func nativeAccessRevokesOneDeviceAndNeverAllowsPhysicalOrPeerPorts() throws {
    let url = Bundle.module.url(forResource: "snapshot", withExtension: "json", subdirectory: "Fixtures")!
    var snapshot = try JSONDecoder().decode(Snapshot.self, from: Data(contentsOf: url))
    snapshot.devices[1].phase = "active"
    snapshot.devices[1].lease = Lease(sessionID: "a", token: "boot:2")
    snapshot.devices[1].nativePorts = NativePorts(inputs: ["lpd owned input"], outputs: ["lpd owned output"])
    let access = NativeAccess()
    access.update(snapshot)
    let nano = snapshot.devices[0].nativeInputs[0]
    #expect(access.allowsInput(nano))
    #expect(access.allowsOutput("lpd owned output"))
    #expect(!access.allowsInput("nanoKONTROL2 SLIDER/KNOB"))
    access.revoke(profileID: "nanokontrol")
    #expect(!access.allowsInput(nano))
    #expect(access.allowsInput("lpd owned input"))
    snapshot.devices[1].lease?.sessionID = "peer"
    snapshot.sequence += 1
    access.update(snapshot)
    #expect(!access.allowsOutput("lpd owned output"))
    access.clear()
    #expect(!access.allowsInput(nano))
}

@Test func delayedSnapshotCannotReenableARevokedLease() throws {
    let url = Bundle.module.url(forResource: "snapshot", withExtension: "json", subdirectory: "Fixtures")!
    let active = try JSONDecoder().decode(Snapshot.self, from: Data(contentsOf: url))
    let access = NativeAccess()
    access.update(active)
    var released = active
    released.sequence += 1
    released.devices[0].phase = "releasing"
    access.update(released)
    access.update(active)
    #expect(!access.allowsInput(active.devices[0].nativeInputs[0]))
}

@Test func revocationWaitsForAnAlreadyAcceptedInputCallback() throws {
    let url = Bundle.module.url(forResource: "snapshot", withExtension: "json", subdirectory: "Fixtures")!
    let active = try JSONDecoder().decode(Snapshot.self, from: Data(contentsOf: url))
    let access = NativeAccess()
    access.update(active)
    let entered = DispatchSemaphore(value: 0)
    let finish = DispatchSemaphore(value: 0)
    let revoked = DispatchSemaphore(value: 0)
    DispatchQueue.global().async {
        access.withInput(active.devices[0].nativeInputs[0]) { entered.signal(); finish.wait() }
    }
    #expect(entered.wait(timeout: .now() + 2) == .success)
    DispatchQueue.global().async { access.revoke(profileID: "nanokontrol"); revoked.signal() }
    #expect(revoked.wait(timeout: .now() + 0.05) == .timedOut)
    finish.signal()
    #expect(revoked.wait(timeout: .now() + 2) == .success)
    #expect(!access.withInput(active.devices[0].nativeInputs[0]) {})
}

@Test func snapshotCarriesServiceVersionAndExecutable() throws {
    let url = Bundle.module.url(forResource: "snapshot", withExtension: "json", subdirectory: "Fixtures")!
    let snapshot = try JSONDecoder().decode(Snapshot.self, from: Data(contentsOf: url))
    #expect(snapshot.service?.version == "0.0.1")
    #expect(snapshot.service?.executable == "/Applications/midistage.app/Contents/MacOS/midistaged")
}
@Test func snapshotWithoutServiceStillDecodes() throws {
    // 旧いサービス（service を載せない版）の snapshot も読める。protocol_version は変えていない
    let url = Bundle.module.url(forResource: "snapshot", withExtension: "json", subdirectory: "Fixtures")!
    var json = try JSONSerialization.jsonObject(with: Data(contentsOf: url)) as! [String: Any]
    json.removeValue(forKey: "service")
    let snapshot = try JSONDecoder().decode(Snapshot.self, from: JSONSerialization.data(withJSONObject: json))
    #expect(snapshot.service == nil)
    #expect(snapshot.devices.count == 2)
}

/// design 06: setup の欄（Rust の wire テストが書く fixture）
@Test func setupFieldsDecodeAndOldServicesStillDecode() throws {
    let url = Bundle.module.url(forResource: "snapshot-setups", withExtension: "json", subdirectory: "Fixtures")!
    let snapshot = try JSONDecoder().decode(Snapshot.self, from: Data(contentsOf: url))
    let roto = try #require(snapshot.devices.first)
    #expect(roto.configuring == true)
    #expect(roto.currentSetup == 6)
    #expect(roto.setupOwners?["vp"] == [6])
    #expect(roto.setupOwners?["ladyland"] == [0, 1, 2, 3])
    #expect(roto.configError == nil)
    // 旧いサービス（欄が無い）
    let old = Bundle.module.url(forResource: "snapshot", withExtension: "json", subdirectory: "Fixtures")!
    let before = try JSONDecoder().decode(Snapshot.self, from: Data(contentsOf: old))
    #expect(before.devices[0].configuring == nil && before.devices[0].currentSetup == nil)
}

/// design 06: Rust が書く fixture（midistage-protocol の device_setup_fields_match_the_swift_fixture）を読める
@Test func rustSnapshotDecodesDeviceSetups() throws {
    let url = Bundle.module.url(forResource: "snapshot-setups", withExtension: "json", subdirectory: "Fixtures")!
    let snapshot = try JSONDecoder().decode(Snapshot.self, from: Data(contentsOf: url))
    let roto = try #require(snapshot.devices.first)
    #expect(roto.configuring == true)
    #expect(roto.currentSetup == 6)
    #expect(roto.setupOwners == ["ladyland": [0, 1, 2, 3], "vp": [6]])
    #expect(roto.configError == nil)
}

/// 旧いサービスの snapshot（setup の欄が無い）も読める
@Test func olderSnapshotHasNoSetupFields() throws {
    let url = Bundle.module.url(forResource: "snapshot", withExtension: "json", subdirectory: "Fixtures")!
    let snapshot = try JSONDecoder().decode(Snapshot.self, from: Data(contentsOf: url))
    #expect(snapshot.devices[0].configuring == nil && snapshot.devices[0].setupOwners == nil)
}

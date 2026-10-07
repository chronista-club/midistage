import Foundation
import Testing
@testable import MidistageClient

// scripts/test-sdk-interop.sh から実行する。fixture は物理 MIDI を一切開かない。
@Test(.enabled(if: ProcessInfo.processInfo.environment["MIDISTAGE_TEST_SERVER"] != nil))
func swiftSDKConnectsToRustAndAcknowledgesOnlyAfterAppCleanup() async throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent("midistage-sdk-\(UUID().uuidString)")
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: directory) }
    let process = Process()
    process.executableURL = URL(fileURLWithPath: ProcessInfo.processInfo.environment["MIDISTAGE_TEST_SERVER"]!)
    process.arguments = [directory.path]
    try process.run()
    defer { if process.isRunning { process.terminate(); process.waitUntilExit() } }
    let endpoint = directory.appendingPathComponent("endpoint.json")
    for _ in 0..<100 {
        if FileManager.default.fileExists(atPath: endpoint.path) { break }
        try await Task.sleep(for: .milliseconds(20))
    }
    #expect(FileManager.default.fileExists(atPath: endpoint.path))
    let (a, first) = try await Client.connect(endpointURL: endpoint, clientID: "swift-a", displayName: "Swift A", nativeMIDI: false)
    #expect(first.protocolVersion == 1)
    let (b, _) = try await Client.connect(endpointURL: endpoint, clientID: "swift-b", displayName: "Swift B", nativeMIDI: false)
    let enabled = try await a.setEnabled(deviceID: "nano", enabled: true, expectedRevision: 0)
    let token = try #require(enabled.devices.first?.lease?.token)
    do {
        _ = try await a.sendMIDI(deviceID: "nano", leaseToken: "old", portName: "output", bytes: [0x90, 60, 100])
        Issue.record("stale output was accepted")
    } catch let error as ProtocolFailure { #expect(error.code == "stale_lease") }
    do {
        _ = try await b.setEnabled(deviceID: "nano", enabled: true, expectedRevision: 1)
        Issue.record("takeover without confirmation was accepted")
    } catch let error as ProtocolFailure { #expect(error.code == "conflict") }
    _ = try await b.setEnabled(deviceID: "nano", enabled: true, expectedRevision: 1, takeover: true)
    for await event in a.events {
        if case let .quiesce(deviceID, _, leaseToken) = event {
            #expect(deviceID == "nano")
            #expect(leaseToken == token)
            break
        }
    }
    let pending = try await b.snapshot()
    #expect(pending.devices[0].phase == "releasing")
    _ = try await a.quiesced(deviceID: "nano", leaseToken: token)
    var successor = try await b.snapshot()
    for _ in 0..<50 {
        if successor.devices[0].phase == "active" { break }
        try await Task.sleep(for: .milliseconds(20))
        successor = try await b.snapshot()
    }
    #expect(successor.owns(successor.devices[0]))
    await a.close(); await b.close()
}

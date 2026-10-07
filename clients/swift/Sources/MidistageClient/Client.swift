import Foundation
import UnisonClient

public final class Client: Sendable {
    private let connection: Connection
    private let channel: StreamChannel<MidistageChannel>
    private init(connection: Connection, channel: StreamChannel<MidistageChannel>) {
        self.connection = connection
        self.channel = channel
    }
    public static var defaultEndpointURL: URL {
        FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Application Support/Midistage/endpoint.json")
    }
    /// 常駐サービスの版違いを検出しても停止・上書きしない。
    public static func connect(endpointURL: URL = defaultEndpointURL, clientID: String, displayName: String, nativeMIDI: Bool = true, initialEnabledProfiles: [String] = []) async throws -> (Client, Snapshot) {
        let endpoint = try JSONDecoder().decode(ServiceEndpoint.self, from: Data(contentsOf: endpointURL))
        guard endpoint.protocolVersion == 1 else { throw ProtocolFailure(code: "incompatible_protocol", message: "service protocol version differs") }
        let connection = try await UnisonClient.connect(to: .localDaemon(port: endpoint.port), trust: .pinned(Data(endpoint.certificateDER)))
        do {
            let channel = try await deadline(operation: { try await connection.openChannel(MidistageChannel()) }, onTimeout: { await connection.disconnect() })
            let client = Client(connection: connection, channel: channel)
            let snapshot = try await client.request(HelloRequest(protocolVersion: 1, clientID: clientID, displayName: displayName, authToken: endpoint.authToken, nativeMIDI: nativeMIDI, initialEnabledProfiles: initialEnabledProfiles))
            guard snapshot.serverEpoch == endpoint.serverEpoch && snapshot.protocolVersion == 1 else {
                await client.close()
                throw ProtocolFailure(code: "epoch_changed", message: "service changed during connection")
            }
            return (client, snapshot)
        } catch { await connection.disconnect(); throw error }
    }
    public var events: AsyncStream<Event> { channel.events }
    public func setEnabled(deviceID: String, enabled: Bool, expectedRevision: UInt64, takeover: Bool = false) async throws -> Snapshot {
        try await request(SetEnabledRequest(deviceID: deviceID, enabled: enabled, expectedRevision: expectedRevision, takeover: takeover))
    }
    /// アプリの入力停止・発音整理・出力キュー停止が完了してから呼ぶ。
    public func quiesced(deviceID: String, leaseToken: String) async throws -> Snapshot {
        try await request(QuiescedRequest(deviceID: deviceID, leaseToken: leaseToken))
    }
    /// SysEx は物理ドライバの completion まで待つ。
    public func sendMIDI(deviceID: String, leaseToken: String, portName: String, bytes: [UInt8]) async throws -> Snapshot {
        try await request(SendMIDIRequest(deviceID: deviceID, leaseToken: leaseToken, portName: portName, bytes: bytes))
    }
    public func snapshot() async throws -> Snapshot { try await request(SnapshotRequest()) }
    public func close() async { await channel.close(); await connection.disconnect() }
    private func request<R: UnisonRequest & Sendable>(_ request: R) async throws -> Snapshot where R.Response == Reply {
        let reply = try await Self.deadline(operation: { try await self.channel.request(request) }, onTimeout: { await self.close() })
        return try reply.value()
    }
    private static func deadline<T: Sendable>(operation: @escaping @Sendable () async throws -> T, onTimeout: @escaping @Sendable () async -> Void) async throws -> T {
        try await withTaskCancellationHandler {
            try await withThrowingTaskGroup(of: T.self) { group in
                group.addTask { try await operation() }
                group.addTask {
                    try await Task.sleep(for: .seconds(3))
                    // task の cancel だけでは Unison の pending continuation は解放されない。
                    await onTimeout()
                    throw ProtocolFailure(code: "timeout", message: "service response timed out")
                }
                defer { group.cancelAll() }
                return try await group.next()!
            }
        } onCancel: { Task { await onTimeout() } }
    }
}

private struct MidistageChannel: StreamChannelMeta {
    static let name = "midistage"
    typealias Event = MidistageClient.Event
}
private struct HelloRequest: UnisonRequest, Sendable {
    static let method = "Hello"
    typealias Response = Reply
    var protocolVersion: UInt32
    var clientID: String
    var displayName: String
    var authToken: String
    var nativeMIDI: Bool
    var initialEnabledProfiles: [String]
    enum CodingKeys: String, CodingKey {
        case protocolVersion = "protocol_version", clientID = "client_id", displayName = "display_name", authToken = "auth_token"
        case nativeMIDI = "native_midi", initialEnabledProfiles = "initial_enabled_profiles"
    }
}
private struct SetEnabledRequest: UnisonRequest, Sendable {
    static let method = "SetEnabled"
    typealias Response = Reply
    var deviceID: String
    var enabled: Bool
    var expectedRevision: UInt64
    var takeover: Bool
    enum CodingKeys: String, CodingKey { case deviceID = "device_id", enabled, expectedRevision = "expected_revision", takeover }
}
private struct QuiescedRequest: UnisonRequest, Sendable {
    static let method = "Quiesced"
    typealias Response = Reply
    var deviceID: String
    var leaseToken: String
    enum CodingKeys: String, CodingKey { case deviceID = "device_id", leaseToken = "lease_token" }
}
private struct SnapshotRequest: UnisonRequest, Sendable {
    static let method = "Snapshot"
    typealias Response = Reply
}

private struct SendMIDIRequest: UnisonRequest, Sendable {
    static let method = "SendMidi"
    typealias Response = Reply
    var deviceID: String
    var leaseToken: String
    var portName: String
    var bytes: [UInt8]
    enum CodingKeys: String, CodingKey { case deviceID = "device_id", leaseToken = "lease_token", portName = "port_name", bytes }
}

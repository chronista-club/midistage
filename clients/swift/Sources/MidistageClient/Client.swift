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
    /// 自分の setup（Roto 本体の保存枠 0〜63）を登録する。空で取り消し。lease が移ってきたとき、
    /// midistage がこの中の最後に選んだもの（無ければ先頭）へ切り替える（design 06）
    public func setDeviceSetups(deviceID: String, setups: [Int]) async throws -> Snapshot {
        try await request(SetDeviceSetupsRequest(deviceID: deviceID, setups: setups))
    }
    /// lease を持つ間、自分の setup の中で切り替える。受け付けたら返り、完了は snapshot の `configuring` / `currentSetup` で見る
    public func selectSetup(deviceID: String, leaseToken: String, setup: Int) async throws -> Snapshot {
        try await request(SelectSetupRequest(deviceID: deviceID, leaseToken: leaseToken, setup: setup))
    }
    /// setup の中身をまるごと書く（design 06 の段 C）。lease を持つ間、自分の台帳の setup にだけ。受け付けたら返り、
    /// 進み具合は snapshot の `configuring` / `configError`。本体ができるまでは `not_implemented` が返る
    public func applyDeviceSetup(deviceID: String, leaseToken: String, setup: Int, content: SetupContent) async throws -> Snapshot {
        try await request(ApplyDeviceSetupRequest(deviceID: deviceID, leaseToken: leaseToken, setup: setup, content: content))
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

private struct SetDeviceSetupsRequest: UnisonRequest, Sendable {
    static let method = "SetDeviceSetups"
    typealias Response = Reply
    var deviceID: String
    var setups: [Int]
    enum CodingKeys: String, CodingKey { case deviceID = "device_id", setups }
}
private struct SelectSetupRequest: UnisonRequest, Sendable {
    static let method = "SelectSetup"
    typealias Response = Reply
    var deviceID: String
    var leaseToken: String
    var setup: Int
    enum CodingKeys: String, CodingKey { case deviceID = "device_id", leaseToken = "lease_token", setup }
}
private struct ApplyDeviceSetupRequest: UnisonRequest, Sendable {
    static let method = "ApplyDeviceSetup"
    typealias Response = Reply
    var deviceID: String
    var leaseToken: String
    var setup: Int
    var content: SetupContent
    enum CodingKeys: String, CodingKey { case deviceID = "device_id", leaseToken = "lease_token", setup, content }
}

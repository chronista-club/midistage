import Foundation

public struct Assignment: Codable, Sendable, Equatable {
    public var clientID: String?
    public var revision: UInt64
    public var expected: Bool
    enum CodingKeys: String, CodingKey { case clientID = "client_id", revision, expected }
}
public struct Lease: Codable, Sendable, Equatable {
    public var sessionID: String
    public var token: String
    enum CodingKeys: String, CodingKey { case sessionID = "session_id", token }
}
public struct Control: Codable, Sendable, Equatable {
    public var id: String
    public var kind: String
    public var outputs: [String]
}
public struct NativePorts: Codable, Sendable, Equatable {
    public var inputs: [String]
    public var outputs: [String]
}
public struct DeviceView: Codable, Sendable, Equatable, Identifiable {
    public var deviceID: String
    public var profileID: String
    public var name: String
    public var present: Bool
    public var assignment: Assignment
    public var phase: String
    public var lease: Lease?
    public var releasingTo: String?
    public var controls: [Control]
    public var nativePorts: NativePorts
    public var error: String?
    public var id: String { deviceID }
    public var nativeInputs: [String] { nativePorts.inputs }
    public var nativeOutputs: [String] { nativePorts.outputs }
    enum CodingKeys: String, CodingKey {
        case deviceID = "device_id", profileID = "profile_id", name, present, assignment, phase, lease
        case releasingTo = "releasing_to", controls, nativePorts = "native_ports", error
    }
}
public struct Snapshot: Codable, Sendable, Equatable {
    public var sequence: UInt64
    public var protocolVersion: UInt32
    public var serverEpoch: String
    public var sessionID: String
    public var devices: [DeviceView]
    enum CodingKeys: String, CodingKey {
        case sequence, protocolVersion = "protocol_version", serverEpoch = "server_epoch", sessionID = "session_id", devices
    }
    public func owns(_ device: DeviceView) -> Bool { device.phase == "active" && device.lease?.sessionID == sessionID }
}
public struct ServiceEndpoint: Codable, Sendable {
    public var protocolVersion: UInt32
    public var port: UInt16
    public var serverEpoch: String
    public var certificateDER: [UInt8]
    public var authToken: String
    enum CodingKeys: String, CodingKey {
        case protocolVersion = "protocol_version", port, serverEpoch = "server_epoch"
        case certificateDER = "certificate_der", authToken = "auth_token"
    }
}
public struct ProtocolFailure: Codable, Sendable, Error, LocalizedError {
    public var code: String
    public var message: String
    public var errorDescription: String? { "\(code): \(message)" }
}
public struct Reply: Decodable, Sendable {
    public var snapshot: Snapshot?
    public var error: ProtocolFailure?
    public func value() throws -> Snapshot {
        if let error { throw error }
        guard let snapshot else { throw ProtocolFailure(code: "invalid_reply", message: "snapshot missing") }
        return snapshot
    }
}
public enum Event: Decodable, Sendable {
    case state(Snapshot)
    case quiesce(deviceID: String, profileID: String, leaseToken: String)
    case input(deviceID: String, leaseToken: String, sequence: UInt64, timestampNS: UInt64, controlID: String, valueKind: String, value: Double)
    enum CodingKeys: String, CodingKey {
        case kind, snapshot, deviceID = "device_id", profileID = "profile_id", leaseToken = "lease_token"
        case sequence, timestampNS = "timestamp_ns", controlID = "control_id", valueKind = "value_kind", value
    }
    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        switch try c.decode(String.self, forKey: .kind) {
        case "state": self = .state(try c.decode(Snapshot.self, forKey: .snapshot))
        case "quiesce": self = .quiesce(deviceID: try c.decode(String.self, forKey: .deviceID), profileID: try c.decode(String.self, forKey: .profileID), leaseToken: try c.decode(String.self, forKey: .leaseToken))
        case "input": self = .input(deviceID: try c.decode(String.self, forKey: .deviceID), leaseToken: try c.decode(String.self, forKey: .leaseToken), sequence: try c.decode(UInt64.self, forKey: .sequence), timestampNS: try c.decode(UInt64.self, forKey: .timestampNS), controlID: try c.decode(String.self, forKey: .controlID), valueKind: try c.decode(String.self, forKey: .valueKind), value: try c.decode(Double.self, forKey: .value))
        default: throw ProtocolFailure(code: "unknown_event", message: "unsupported event kind")
        }
    }
}

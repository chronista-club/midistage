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
    /// 設定ポート（Roto の Serial）を開いて用事をしている間 true。その間 Roto の MIDI は止まる（design 06）。旧いサービスには無い
    public var configuring: Bool?
    /// midistage が最後に選んだ setup
    public var currentSetup: Int?
    /// setup の持ち主（client_id → setup）
    public var setupOwners: [String: [Int]]?
    /// 設定ポートの用事が失敗した理由
    public var configError: String?
    public var id: String { deviceID }
    public var nativeInputs: [String] { nativePorts.inputs }
    public var nativeOutputs: [String] { nativePorts.outputs }
    enum CodingKeys: String, CodingKey {
        case deviceID = "device_id", profileID = "profile_id", name, present, assignment, phase, lease
        case releasingTo = "releasing_to", controls, nativePorts = "native_ports", error
        case configuring, currentSetup = "current_setup", setupOwners = "setup_owners", configError = "config_error"
    }
}
// MARK: - setup の中身（design 06 の段 C）。形の正は Fixtures/apply-device-setup.json（Rust が書く）

public enum ControlMode: String, Codable, Sendable, Equatable {
    case cc7, cc14, nrpn7, nrpn14, programChange = "program_change", note
}
public enum KnobHaptics: Codable, Sendable, Equatable {
    /// 300 度のなめらかな回転。クリック位置（0–127）を 2 つまで
    case smooth(indents: [Int])
    /// 2–16 段のクリック。段ごとの名前
    case steps(names: [String])
    /// 300 度 + 中央のクリック
    case centreIndent
    enum CodingKeys: String, CodingKey { case kind, indents, names }
    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        switch try c.decode(String.self, forKey: .kind) {
        case "smooth": self = .smooth(indents: try c.decodeIfPresent([Int].self, forKey: .indents) ?? [])
        case "steps": self = .steps(names: try c.decode([String].self, forKey: .names))
        case "centre_indent": self = .centreIndent
        case let other: throw ProtocolFailure(code: "invalid_request", message: "unknown haptics kind \(other)")
        }
    }
    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: CodingKeys.self)
        switch self {
        case .smooth(let indents): try c.encode("smooth", forKey: .kind); try c.encode(indents, forKey: .indents)
        case .steps(let names): try c.encode("steps", forKey: .kind); try c.encode(names, forKey: .names)
        case .centreIndent: try c.encode("centre_indent", forKey: .kind)
        }
    }
}
public enum SwitchAction: String, Codable, Sendable, Equatable { case push, toggle }
/// つまみ 1 席（control 00–1F = 8 × 4 ページ）
public struct KnobConfig: Codable, Sendable, Equatable {
    public var control: Int
    public var mode: ControlMode
    public var channel: Int
    public var param: Int
    public var nrpnAddress: Int
    public var min: Int
    public var max: Int
    public var name: String
    public var color: Int
    public var haptics: KnobHaptics
    public init(control: Int, mode: ControlMode, channel: Int, param: Int = 0, nrpnAddress: Int = 0, min: Int = 0, max: Int,
                name: String, color: Int, haptics: KnobHaptics) {
        self.control = control; self.mode = mode; self.channel = channel; self.param = param; self.nrpnAddress = nrpnAddress
        self.min = min; self.max = max; self.name = name; self.color = color; self.haptics = haptics
    }
    enum CodingKeys: String, CodingKey { case control, mode, channel, param, nrpnAddress = "nrpn_address", min, max, name, color, haptics }
}
/// ボタン 1 席
public struct SwitchConfig: Codable, Sendable, Equatable {
    public var control: Int
    public var mode: ControlMode
    public var channel: Int
    public var param: Int
    public var nrpnAddress: Int
    public var min: Int
    public var max: Int
    public var name: String
    public var color: Int
    public var ledOn: Int
    public var ledOff: Int
    public var action: SwitchAction
    public var stepNames: [String]
    public init(control: Int, mode: ControlMode, channel: Int, param: Int = 0, nrpnAddress: Int = 0, min: Int = 0, max: Int,
                name: String, color: Int, ledOn: Int, ledOff: Int, action: SwitchAction, stepNames: [String] = []) {
        self.control = control; self.mode = mode; self.channel = channel; self.param = param; self.nrpnAddress = nrpnAddress
        self.min = min; self.max = max; self.name = name; self.color = color; self.ledOn = ledOn; self.ledOff = ledOff
        self.action = action; self.stepNames = stepNames
    }
    enum CodingKeys: String, CodingKey {
        case control, mode, channel, param, nrpnAddress = "nrpn_address", min, max, name, color
        case ledOn = "led_on", ledOff = "led_off", action, stepNames = "step_names"
    }
}
/// setup 1 つの中身。載っていない席は未設定（CLEAR）にする
public struct SetupContent: Codable, Sendable, Equatable {
    /// 12 文字までの ASCII
    public var name: String
    public var knobs: [KnobConfig]
    public var switches: [SwitchConfig]
    public init(name: String, knobs: [KnobConfig] = [], switches: [SwitchConfig] = []) {
        self.name = name; self.knobs = knobs; self.switches = switches
    }
}

/// サービス自身の版と実行ファイル（design 05「常駐と配布」）。旧いサービスには無い
public struct ServiceInfo: Codable, Sendable, Equatable {
    public var version: String
    public var executable: String
    public init(version: String, executable: String) { self.version = version; self.executable = executable }
}
public struct Snapshot: Codable, Sendable, Equatable {
    public var sequence: UInt64
    public var protocolVersion: UInt32
    public var serverEpoch: String
    public var sessionID: String
    public var devices: [DeviceView]
    public var service: ServiceInfo?
    enum CodingKeys: String, CodingKey {
        case sequence, protocolVersion = "protocol_version", serverEpoch = "server_epoch", sessionID = "session_id", devices, service
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

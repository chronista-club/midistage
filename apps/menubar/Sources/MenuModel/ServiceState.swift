// メニューバーの状態行は 3 つの事実の合成（design 05「アプリの形」）。それぞれ別に持ち、潰さない。
// ここは純粋: 入力は事実、出力は表示。SMAppService や endpoint.json には触れない。

/// 1. 登録（SMAppService.status の写し）
public enum Registration: Sendable, Equatable {
    case enabled
    case requiresApproval   // システム設定 › ログイン項目 で承認待ち
    case notRegistered
    case notFound           // 同梱 plist が無い（ビルドの欠陥）
}

/// 3. 機材の要約（snapshot から）
public struct DeviceSummary: Sendable, Equatable {
    public var present: Int
    public var releasingStuck: Int   // releasing が表示上の閾値を超えた台数
    public var errors: Int
    public init(present: Int, releasingStuck: Int = 0, errors: Int = 0) {
        self.present = present; self.releasingStuck = releasingStuck; self.errors = errors
    }
}

/// 2. 稼働（endpoint.json → SDK 接続の結果）
public enum Running: Sendable, Equatable {
    case stopped                                  // endpoint.json が無い
    case unreachable(reason: String)              // endpoint.json はあるが繋がらない / 版違い / epoch 違い
    case connected(version: String?, executable: String?, devices: DeviceSummary)
}

/// アプリが知っている同梱版の事実
public struct Bundled: Sendable, Equatable {
    public var version: String
    public var executable: String
    public init(version: String, executable: String) { self.version = version; self.executable = executable }
}

public struct ServiceState: Sendable, Equatable {
    public var registration: Registration
    public var running: Running
    public var bundled: Bundled
    public init(registration: Registration, running: Running, bundled: Bundled) {
        self.registration = registration; self.running = running; self.bundled = bundled
    }
}

public enum Icon: Sendable, Equatable { case normal, attention, stopped }

/// メニューに出すもの
public struct MenuSummary: Sendable, Equatable {
    public var icon: Icon
    public var title: String          // 1 行目
    public var lines: [String]        // 補足（承認待ちの導線、別バイナリ、版違い、機材の注意）
    public var needsApproval: Bool    // 「システム設定を開く」を出す
    public var foreignBinary: Bool    // 同梱版ではない midistaged が動いている
    public var versionMismatch: Bool  // 同梱版と稼働中の版が違う（「サービスを再起動」を促す）
}

public func summarize(_ s: ServiceState) -> MenuSummary {
    var lines: [String] = []
    var needsApproval = false
    var foreign = false
    var mismatch = false
    var icon: Icon
    var title: String

    switch s.running {
    case .stopped:
        icon = .stopped
        title = "midistaged は停止しています"
    case .unreachable(let reason):
        icon = .attention
        title = "midistaged に繋がりません"
        lines.append(reason)
    case .connected(let version, let executable, let devices):
        icon = .normal
        title = "midistaged 稼働中（\(devices.present) 台）"
        if let executable, executable != s.bundled.executable {
            foreign = true
            lines.append("別のバイナリが動いています: \(executable)")
        }
        if let version, version != s.bundled.version {
            mismatch = true
            icon = .attention
            lines.append("稼働中 \(version) / 同梱 \(s.bundled.version) — 再起動で入れ替わります")
        }
        if devices.releasingStuck > 0 {
            icon = .attention
            lines.append("引き継ぎが止まっている機材: \(devices.releasingStuck)")
        }
        if devices.errors > 0 {
            icon = .attention
            lines.append("エラーの機材: \(devices.errors)")
        }
    }

    switch s.registration {
    case .enabled:
        break
    case .requiresApproval:
        // 人が動かないと常駐が始まらない。停止中でも注意で出す
        needsApproval = true
        icon = .attention
        lines.append("ログイン項目の承認待ち（システム設定で許可）")
    case .notRegistered:
        lines.append("常駐は無効（ログイン時に起動しない）")
        if case .stopped = s.running { icon = .stopped }
    case .notFound:
        icon = .attention
        lines.append("同梱の LaunchAgent が見つかりません（アプリの不具合）")
    }

    return MenuSummary(icon: icon, title: title, lines: lines,
                       needsApproval: needsApproval, foreignBinary: foreign, versionMismatch: mismatch)
}

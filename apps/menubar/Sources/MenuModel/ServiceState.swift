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
    /// 4. アプリ自体をログイン時に起こすか（SMAppService.mainApp）。サービスとは別の登録
    public var appAtLogin: Bool
    /// 5. MIDI トレース（開発用）がオンか（フラグファイルの有無）
    public var tracing: Bool
    /// 6. いま lease を持っているアプリ（snapshot から。サービスの入れ替えで切れる相手）
    public var leaseHolders: [String]
    public init(registration: Registration, running: Running, bundled: Bundled, appAtLogin: Bool = false, tracing: Bool = false,
                leaseHolders: [String] = []) {
        self.registration = registration; self.running = running; self.bundled = bundled; self.appAtLogin = appAtLogin
        self.tracing = tracing; self.leaseHolders = leaseHolders
    }
}

/// アプリが新しくなったあと、動いているサービスをどうするか（REQ-APP-062）
public enum ServiceUpdate: Sendable, Equatable {
    case none
    /// 誰も使っていない。その場で入れ替えても何も切れない
    case restartNow
    /// 使っているアプリがある。空いたら入れ替える
    case waitForQuiet(holders: [String])
}

/// 同梱版と稼働中の版が違い、稼働中が同梱のバイナリ（開発用の別バイナリではない）で、
/// まだこの版で自動の入れ替えを試していないときだけ動く（純粋）
public func serviceUpdate(_ s: ServiceState, tried: String?) -> ServiceUpdate {
    guard s.registration == .enabled,
          case .connected(let version?, let executable, _) = s.running,
          isNewer(s.bundled.version, than: version),   // 古いアプリで新しいサービスを置き換えない
          executable == nil || executable == s.bundled.executable,
          tried != s.bundled.version
    else { return .none }
    let holders = Array(Set(s.leaseHolders)).sorted()
    return holders.isEmpty ? .restartNow : .waitForQuiet(holders: holders)
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
            let holders = Array(Set(s.leaseHolders)).sorted()
            if holders.isEmpty || foreign {
                lines.append("稼働中 \(version) / 同梱 \(s.bundled.version) — 再起動で入れ替わります")
            } else {
                lines.append("サービスの更新待ち（\(holders.joined(separator: "、")) が使用中）— 空いたら \(s.bundled.version) に入れ替えます")
            }
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
    if s.tracing {
        // 開発用。付けっぱなしに気づけるように注意で出す
        if icon == .normal { icon = .attention }
        lines.append("MIDI トレース中（開発用。止め忘れに注意）")
    }
    if s.registration == .enabled && !s.appAtLogin {
        // サービスは起きるがアイコンは出ない状態。注意ではなく案内
        lines.append("ログイン時にこのアプリは起動しません（メニューで有効にできます）")
    }

    return MenuSummary(icon: icon, title: title, lines: lines,
                       needsApproval: needsApproval, foreignBinary: foreign, versionMismatch: mismatch)
}

/// 版を数字の並びとして比べる（"0.10.0" は "0.9.0" より新しい）。数字で読めない版は新しいとみなさない（純粋）
public func isNewer(_ a: String, than b: String) -> Bool {
    func parts(_ v: String) -> [Int]? {
        let ps = v.split(separator: ".").map { Int($0) }
        return ps.contains(nil) || ps.isEmpty ? nil : ps.compactMap { $0 }
    }
    guard let x = parts(a), let y = parts(b) else { return false }
    for i in 0..<max(x.count, y.count) {
        let l = i < x.count ? x[i] : 0, r = i < y.count ? y[i] : 0
        if l != r { return l > r }
    }
    return false
}

// spec 01 のステータス一覧（REQ-APP-001〜003）。snapshot の事実 → 画面の行。純粋。
// 「挿さっている」と「担当」と「握っている」は別の事実なので別の列に出し、潰さない（design 05）。

/// snapshot の DeviceView から要る分だけ写した事実（MidistageClient に依存しないための写し）
public struct DeviceFacts: Sendable, Equatable {
    public var id: String
    public var name: String
    public var present: Bool
    public var assignedTo: String?       // assignment.client_id
    public var revision: UInt64
    public var held: Bool                // lease があるか
    public var heldByMe: Bool            // lease.session_id が自分のセッション
    public var phase: String             // off / waiting / active / releasing / error
    public var releasingTo: String?
    public var error: String?
    public var releasingStuck: Bool      // releasing が表示上の閾値を超えた（probe が計る）
    public var hasGearAsset: Bool        // 3D 資産の有無（REQ-APP-011）。無ければ「模型なし」
    public var maker: String?            // gear.json の maker（「どこの」）。資産が無ければ nil
    public var model: String?            // gear.json の model（「何か」）
    public init(id: String, name: String, present: Bool, assignedTo: String?, revision: UInt64,
                held: Bool, heldByMe: Bool = false, phase: String, releasingTo: String? = nil,
                error: String? = nil, releasingStuck: Bool = false, hasGearAsset: Bool = true,
                maker: String? = nil, model: String? = nil) {
        self.id = id; self.name = name; self.present = present; self.assignedTo = assignedTo
        self.revision = revision; self.held = held; self.heldByMe = heldByMe; self.phase = phase
        self.releasingTo = releasingTo; self.error = error; self.releasingStuck = releasingStuck
        self.hasGearAsset = hasGearAsset; self.maker = maker; self.model = model
    }
    /// 一覧に出す名前: 「KORG Keystage 61」のように どこの何か。gear.json が無ければ snapshot の名前
    public var displayName: String {
        if let maker, let model { return "\(maker) \(model)" }
        return model ?? name
    }
}

public enum RowTone: Sendable, Equatable {
    case normal, quiet, attention
    var rank: Int { switch self { case .attention: 0; case .normal: 1; case .quiet: 2 } }
}

/// 一覧の 1 行。文言はここで決める（画面は並べるだけ）
public struct DeviceRow: Sendable, Equatable, Identifiable {
    public var id: String
    public var name: String          // 並べ替え・重複判定用の全体名（maker + model、重複時は ID 付き）
    public var maker: String?        // 控えめに添える「どこの」（mako 2026-10-12）
    public var model: String         // 主役の「何か」（重複時は ID 付き）
    public var presence: String      // 挿さっている / 見当たらない
    public var assignment: String    // 担当 app / なし
    public var holder: String        // 握っている app の表現
    public var phase: String
    public var note: String?         // 注意（引き継ぎが止まっている / エラー / 模型なし）
    public var tone: RowTone
    public var revision: UInt64
}

public func statusRows(_ devices: [DeviceFacts]) -> [DeviceRow] {
    // 同じ名前が複数あるとき（認識前の generic:<name> の登録が残っている等）は ID を添えて区別する
    var nameCount: [String: Int] = [:]
    for d in devices { nameCount[d.displayName, default: 0] += 1 }
    return devices.map { d in
        var notes: [String] = []
        var tone: RowTone = .normal
        if d.phase == "error" || d.error != nil {
            tone = .attention
            notes.append(d.error ?? "エラー")
        }
        if d.releasingStuck {
            tone = .attention
            notes.append("引き継ぎが止まっています（\(d.releasingTo ?? "?") へ）")
        }
        if let to = d.assignedTo, d.present, !d.held, d.phase == "waiting" {
            // 担当はあるのに握られていない = その app が繋がっていない
            notes.append("\(to) が接続していません")
        }
        if !d.present { tone = tone == .attention ? .attention : .quiet }
        if !d.hasGearAsset { notes.append("模型なし") }

        let holder: String
        if d.held {
            holder = d.heldByMe ? "この画面" : (d.assignedTo ?? "？")
        } else {
            holder = "—"
        }
        let phase = d.phase == "releasing" ? "releasing → \(d.releasingTo ?? "?")" : d.phase
        let dup = (nameCount[d.displayName] ?? 0) > 1
        let shownName = dup ? "\(d.displayName)（\(d.id)）" : d.displayName
        let baseModel = d.model ?? d.name
        return DeviceRow(
            id: d.id, name: shownName, maker: d.maker, model: dup ? "\(baseModel)（\(d.id)）" : baseModel,
            presence: d.present ? "挿さっている" : "見当たらない",
            assignment: d.assignedTo ?? "なし",
            holder: holder, phase: phase,
            note: notes.isEmpty ? nil : notes.joined(separator: " / "),
            tone: tone, revision: d.revision)
    }
    // 注意 → 挿さっている → 名前 の順。見当たらない機材は下へ（tone は順位で比べる。真偽の比較だと順序が決まらない）
    .sorted { a, b in
        if a.tone != b.tone { return a.tone.rank < b.tone.rank }
        if (a.presence == "挿さっている") != (b.presence == "挿さっている") { return a.presence == "挿さっている" }
        return a.name < b.name
    }
}

/// gear.json（配布先の <id>.json）から maker / model を読む。純粋: JSON の辞書 → 2 値
public func makerAndModel(fromGearJSON json: [String: Any]) -> (maker: String?, model: String?) {
    (json["maker"] as? String, json["model"] as? String)
}

/// phase の見せ方（mako 2026-10-12「Phase はポイントと色で表してもいいね」）。色の名前は画面が決める
public enum PhaseTone: Sendable, Equatable { case off, waiting, active, releasing, error }

public func phaseTone(_ phase: String) -> PhaseTone {
    switch phase {
    case "active": return .active
    case "waiting": return .waiting
    case "error": return .error
    default:
        return phase.hasPrefix("releasing") ? .releasing : .off
    }
}

/// 担当 app のアイコンを引くための対応（mako 2026-10-12「担当はアプリアイコンも隣に表示できないかな？」）。
/// client_id（`vp` / `ladyland`）は bundle id の末尾（`tech.anycreative.vp` / `club.chronista.ladyland`）と
/// 一致するので、名前を埋め込まずに照合できる。正しくは wire で bundle id を運ぶ（design 05 未決）
public func bundleMatchesClient(bundleID: String, clientID: String) -> Bool {
    let last = bundleID.split(separator: ".").last.map(String.init) ?? bundleID
    return last.lowercased() == clientID.lowercased()
}

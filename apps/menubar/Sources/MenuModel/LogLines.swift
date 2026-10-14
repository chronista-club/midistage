// midistaged のログ（midistaged.log / trace.log）を window に流すための読み取り。純粋。
// 行の形（tracing の fmt、ANSI なし）:
//   2026-10-13T06:32:11.558367Z  INFO midistaged::runtime: keystage: plugged in
//   2026-10-13T14:36:49.093712Z  INFO FWD | B7 4F 1F dir="dev→app" device=lpd8 app=vp port=LPD8 mk2 kind="cc"
// trace.log は target を出さない（with_target(false)）。項目の値は空白を含みうる（port=LPD8 mk2）ので
// 「次の 既知のキー= の手前まで」を値とする。
import Foundation

public enum LogLevel: String, Sendable, Equatable {
    case trace = "TRACE", debug = "DEBUG", info = "INFO", warn = "WARN", error = "ERROR"
}

public struct LogLine: Sendable, Equatable, Identifiable {
    public var id: Int                 // 読んだ順の番号（表示の安定した ID）
    public var time: Date
    public var level: LogLevel
    public var target: String?         // midistaged::runtime など。trace.log では nil
    public var message: String
    public var fields: [String: String]
}

/// trace.log の行を表にするための形
public struct TraceRow: Sendable, Equatable, Identifiable {
    public var id: Int
    public var time: Date
    public var direction: String       // dev→app / app→svc / svc→dev / （開閉は空）
    public var device: String
    public var app: String
    public var verdict: String         // FWD / DROP no lease / QUEUE / SENT / OPEN bridge …
    public var kind: String            // note-on / cc / sysex …
    public var detail: String          // バイト列、または開閉したポート
}

/// トレースの項目として知っているキー（バイト列などと取り違えないため、これだけを項目とみなす）
let knownKeys = ["dir", "device", "app", "port", "kind", "via", "token"]

public func parseLogLine(_ raw: String, id: Int) -> LogLine? {
    // 時刻 + 空白 + レベル + 空白 + 残り
    let parts = raw.split(separator: " ", maxSplits: 1, omittingEmptySubsequences: true)
    guard parts.count == 2, let time = parseTime(String(parts[0])) else { return nil }
    let rest = parts[1].drop { $0 == " " }
    let levelAndRest = rest.split(separator: " ", maxSplits: 1, omittingEmptySubsequences: true)
    guard levelAndRest.count == 2, let level = LogLevel(rawValue: String(levelAndRest[0])) else { return nil }
    var body = String(levelAndRest[1])

    // target（"midistaged::runtime: " のように `::` を含む最初の語 + ": "）
    var target: String?
    if let colon = body.range(of: ": "), body[..<colon.lowerBound].contains("::"),
       !body[..<colon.lowerBound].contains(" ") {
        target = String(body[..<colon.lowerBound])
        body = String(body[colon.upperBound...])
    }

    // 項目: " key=" の位置を集め、値は次の項目の手前まで
    var marks: [(key: String, start: String.Index, valueStart: String.Index)] = []
    for key in knownKeys {
        var searchFrom = body.startIndex
        while let r = body.range(of: " \(key)=", range: searchFrom..<body.endIndex) {
            marks.append((key, r.lowerBound, r.upperBound))
            searchFrom = r.upperBound
        }
    }
    marks.sort { $0.start < $1.start }
    var fields: [String: String] = [:]
    for (i, m) in marks.enumerated() {
        let end = i + 1 < marks.count ? marks[i + 1].start : body.endIndex
        var value = String(body[m.valueStart..<end]).trimmingCharacters(in: .whitespaces)
        if value.hasPrefix("\""), value.hasSuffix("\""), value.count >= 2 { value = String(value.dropFirst().dropLast()) }
        fields[m.key] = value
    }
    let message = marks.first.map { String(body[..<$0.start]) } ?? body
    return LogLine(id: id, time: time, level: level, target: target,
                   message: message.trimmingCharacters(in: .whitespaces), fields: fields)
}

/// "2026-10-13T06:32:11.558367Z" → Date（UTC、ミリ秒まで）
public func parseTime(_ s: String) -> Date? {
    guard s.hasSuffix("Z"), s.count >= 20 else { return nil }
    let chars = Array(s)
    func int(_ a: Int, _ b: Int) -> Int? { Int(String(chars[a..<b])) }
    guard let y = int(0, 4), let mo = int(5, 7), let d = int(8, 10),
          let h = int(11, 13), let mi = int(14, 16), let se = int(17, 19) else { return nil }
    var ms = 0
    if chars.count > 21, chars[19] == "." {
        let frac = String(chars[20..<min(23, chars.count - 1)])
        ms = Int(frac.padding(toLength: 3, withPad: "0", startingAt: 0)) ?? 0
    }
    var c = DateComponents()
    c.year = y; c.month = mo; c.day = d; c.hour = h; c.minute = mi; c.second = se
    c.nanosecond = ms * 1_000_000
    var cal = Calendar(identifier: .gregorian)
    cal.timeZone = TimeZone(identifier: "UTC")!
    return cal.date(from: c)
}

/// 通信ライブラリ（unison）の細かい行。流れのタブでは既定で隠す
public func isLibraryNoise(_ line: LogLine) -> Bool {
    line.target?.hasPrefix("unison") == true
}

public func traceRow(_ line: LogLine) -> TraceRow? {
    // 開始・停止の区切り行（---- MIDI trace on ----）は表に出さない
    if line.message.hasPrefix("----") { return nil }
    let pieces = line.message.components(separatedBy: " | ")
    let verdict = pieces.first ?? line.message
    let detail = pieces.dropFirst().joined(separator: " | ")
    return TraceRow(id: line.id, time: line.time,
                    direction: line.fields["dir"] ?? "",
                    device: line.fields["device"] ?? "",
                    app: line.fields["app"] ?? "",
                    verdict: verdict,
                    kind: line.fields["kind"] ?? "",
                    detail: detail)
}

/// 絞り込み（大文字小文字を区別しない部分一致。空なら全部）
public func matches(_ line: LogLine, query: String) -> Bool {
    let q = query.trimmingCharacters(in: .whitespaces)
    if q.isEmpty { return true }
    let hay = ([line.message, line.target ?? ""] + line.fields.values).joined(separator: " ")
    return hay.localizedCaseInsensitiveContains(q)
}

/// 末尾を追いかけるときの行の切り出し。読んだ塊の境界は UTF-8 の文字の途中に来うる（→ は 3 バイト）ので、
/// 文字列にする前にバイトのまま改行（0x0A）で切り、途中で切れた最後の行はバイトのまま次の塊とつなぐ
public struct LineSplitter: Sendable {
    private var pending = Data()

    public init() {}

    /// 塊を足し、完結した行（空行は除く）を返す
    public mutating func push(_ chunk: Data) -> [String] {
        pending.append(chunk)
        var lines: [String] = []
        var start = pending.startIndex
        while let newline = pending[start...].firstIndex(of: 0x0A) {
            if newline > start { lines.append(String(decoding: pending[start..<newline], as: UTF8.self)) }
            start = pending.index(after: newline)
        }
        pending = Data(pending[start...])
        return lines
    }

    /// 別のファイルに切り替わった（日付が変わった・切り詰められた）
    public mutating func reset() { pending = Data() }
}

/// 最新のファイル（`midistaged.log.2026-10-13` のように日付が付く）を名前から選ぶ
public func newestLogFile(_ names: [String], prefix: String) -> String? {
    names.filter { $0.hasPrefix(prefix + ".") }.max()
}

/// クロック / Active Sensing の行（表で既定で隠す。サービスは記録時点で既定で省くが、古いファイルには残る）
public func isClockRow(_ row: TraceRow) -> Bool {
    row.kind == "clock" || row.kind == "active-sensing"
}

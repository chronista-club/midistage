import Foundation
import Testing
@testable import MenuModel

@Test func flowLineSplitsTimeLevelTargetAndMessage() throws {
    let l = try #require(parseLogLine("2026-10-13T06:32:11.558367Z  INFO midistaged::runtime: keystage: plugged in", id: 1))
    #expect(l.level == .info)
    #expect(l.target == "midistaged::runtime")
    #expect(l.message == "keystage: plugged in")
    #expect(l.fields.isEmpty)
    var cal = Calendar(identifier: .gregorian); cal.timeZone = TimeZone(identifier: "UTC")!
    let c = cal.dateComponents([.hour, .minute, .second, .nanosecond], from: l.time)
    #expect(c.hour == 6 && c.minute == 32 && c.second == 11)
    #expect(abs((c.nanosecond ?? 0) - 558_000_000) < 1_000_000)
}

@Test func warnLevelIsRecognized() throws {
    let l = try #require(parseLogLine("2026-10-13T06:32:11.5Z  WARN midistaged::service: request SetEnabled from vp rejected: state changed since confirmation", id: 2))
    #expect(l.level == .warn)
    #expect(l.message.hasPrefix("request SetEnabled from vp rejected"))
}

@Test func traceLineKeepsSpacesInsideValuesAndBecomesARow() throws {
    let raw = #"2026-10-13T14:36:49.093712Z  INFO FWD | B7 4F 1F dir="dev→app" device=lpd8 app=vp port=LPD8 mk2 kind="cc""#
    let l = try #require(parseLogLine(raw, id: 3))
    #expect(l.target == nil)
    #expect(l.message == "FWD | B7 4F 1F")
    #expect(l.fields["port"] == "LPD8 mk2")
    #expect(l.fields["kind"] == "cc")
    let row = try #require(traceRow(l))
    #expect(row.direction == "dev→app" && row.device == "lpd8" && row.app == "vp")
    #expect(row.verdict == "FWD" && row.kind == "cc" && row.detail == "B7 4F 1F")
}

@Test func bridgeOpenRowKeepsThePortListAsDetail() throws {
    let raw = #"2026-10-13T06:32:12.1Z  INFO OPEN bridge | in ["Midistage/vp/x:1/1 | LPD8 mk2"] | out ["Midistage/vp/x:1/2 | LPD8 mk2"] device=lpd8 app=vp token=x:1"#
    let line = try #require(parseLogLine(raw, id: 4))
    let row = try #require(traceRow(line))
    #expect(row.verdict == "OPEN bridge")
    #expect(row.detail.hasPrefix("in [") && row.detail.contains("out ["))
    #expect(row.device == "lpd8")
}

@Test func markerLinesAndGarbageAreSkipped() {
    let marker = parseLogLine("2026-10-13T06:00:00.0Z  INFO ---- MIDI trace on ----", id: 5)!
    #expect(traceRow(marker) == nil)
    #expect(parseLogLine("not a log line", id: 6) == nil)
    #expect(parseLogLine("", id: 7) == nil)
}

@Test func libraryNoiseAndQueryFiltering() throws {
    let noise = try #require(parseLogLine("2026-10-13T06:00:00.0Z  INFO unison::network::quic: New QUIC connection from: [::1]:1", id: 8))
    #expect(isLibraryNoise(noise))
    let line = try #require(parseLogLine("2026-10-13T06:00:00.0Z  INFO midistaged::runtime: roto: lease granted to vp", id: 9))
    #expect(!isLibraryNoise(line))
    #expect(matches(line, query: "ROTO"))
    #expect(matches(line, query: ""))
    #expect(!matches(line, query: "keystage"))
}

@Test func tailingKeepsTheUnfinishedLastLine() {
    var splitter = LineSplitter()
    #expect(splitter.push(Data("a\nb\nc-par".utf8)) == ["a", "b"])
    #expect(splitter.push(Data("tial\n".utf8)) == ["c-partial"])
    #expect(splitter.push(Data("\n\n".utf8)) == [], "空行は捨てる")
}

/// 読んだ塊の境界が UTF-8 の文字の途中に来ても、行は壊れない（→ は 3 バイト）
@Test func aChunkBoundaryInsideAMultibyteCharacterDoesNotBreakTheLine() {
    let line = #"2026-10-14T01:02:03.4Z  INFO FWD | B7 dir="dev→app" device=lpd8"#
    let bytes = Array((line + "\n").utf8)
    for cut in 1..<bytes.count {
        var splitter = LineSplitter()
        let first = splitter.push(Data(bytes[..<cut]))
        let second = splitter.push(Data(bytes[cut...]))
        #expect(first + second == [line], "cut at \(cut)")
    }
}

/// サービスが書いた行の形（Rust の contract テストが作る fixture）を、app が読める
@Test func theServiceLogContractIsReadable() throws {
    let lines = try fixtureLines("log-contract.midistaged.log")
    #expect(lines.count == 5, "全行が読める")
    #expect(lines[0].target == "midistaged::runtime" && lines[0].level == .info)
    #expect(lines[0].message == "session connected: vp (Vantage Point) native_midi=true")
    #expect(lines[2].message.hasPrefix("lpd8: virtual ports opened for vp"))
    #expect(lines[3].level == .warn && lines[3].target == "midistaged::service")
    #expect(isLibraryNoise(lines[4]) && !isLibraryNoise(lines[0]))
}

@Test func theTraceContractIsReadable() throws {
    let lines = try fixtureLines("log-contract.trace.log")
    let rows = lines.compactMap(traceRow)
    #expect(lines.count == 10 && rows.count == 8, "区切り 2 行以外は表の行になる")
    let fwd = rows[0]
    #expect(fwd.direction == "dev→app" && fwd.device == "lpd8" && fwd.app == "vp")
    #expect(fwd.verdict == "FWD" && fwd.kind == "cc" && fwd.detail == "B7 4F 1F")
    #expect(lines[1].fields["port"] == "LPD8 mk2")
    #expect(rows[1].verdict == "DROP no lease" && isClockRow(rows[1]))
    #expect(rows[2].direction == "app→svc" && rows[2].verdict == "QUEUE" && rows[2].kind == "note-on")
    #expect(rows[3].verdict == "SENT" && rows[3].kind == "sysex" && rows[3].detail.hasSuffix("(44 bytes)"))
    #expect(lines[4].fields["via"] == "SendMidi")
    #expect(rows[4].verdict == "DROP stale lease" && lines[5].fields["via"] == "port")
    #expect(rows[5].verdict == "OVERFLOW queue full; lease stopped" && rows[5].device == "keystage")
    #expect(rows[6].verdict == "OPEN bridge" && rows[6].detail.contains("out [") && lines[7].fields["token"] == "boot:1")
    #expect(rows[7].verdict == "CLOSE bridge" && rows[7].device == "lpd8")
}

private func fixtureLines(_ name: String) throws -> [LogLine] {
    let url = URL(fileURLWithPath: #filePath).deletingLastPathComponent()
        .appendingPathComponent("Fixtures").appendingPathComponent(name)
    var splitter = LineSplitter()
    let raw = splitter.push(try Data(contentsOf: url))
    return raw.enumerated().compactMap { parseLogLine($1, id: $0) }
}

@Test func newestDailyFileIsChosenByName() {
    let names = ["midistaged.log.2026-10-12", "trace.log.2026-10-13", "midistaged.log.2026-10-13", "other.txt"]
    #expect(newestLogFile(names, prefix: "midistaged.log") == "midistaged.log.2026-10-13")
    #expect(newestLogFile(names, prefix: "trace.log") == "trace.log.2026-10-13")
    #expect(newestLogFile([], prefix: "trace.log") == nil)
}

@Test func clockRowsCanBeHidden() throws {
    let clock = try #require(parseLogLine(#"2026-10-13T06:24:41.8Z  INFO FWD | F8 dir="dev→app" device=keystage app=ladyland port=Keystage KBD/CTRL kind="clock""#, id: 10))
    let cc = try #require(parseLogLine(#"2026-10-13T14:36:49.0Z  INFO FWD | B7 4F 1F dir="dev→app" device=lpd8 app=vp port=LPD8 mk2 kind="cc""#, id: 11))
    let clockRow = try #require(traceRow(clock))
    let ccRow = try #require(traceRow(cc))
    #expect(isClockRow(clockRow))
    #expect(!isClockRow(ccRow))
}

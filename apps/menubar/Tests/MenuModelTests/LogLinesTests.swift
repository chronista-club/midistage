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
    let (lines, rest) = splitCompleteLines("a\nb\nc-partial")
    #expect(lines == ["a", "b"])
    #expect(rest == "c-partial")
    #expect(splitCompleteLines("a\n").remainder == "")
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

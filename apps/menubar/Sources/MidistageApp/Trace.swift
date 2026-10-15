// MIDI トレース（開発用）のオン／オフ。midistaged は <state_dir>/trace の有無を毎秒見て切り替える。
// 記録は ~/Library/Logs/midistaged/trace.log.<日付>（midistaged/src/trace.rs）。
import AppKit
import Foundation

enum MidiTrace {
    static var stateDir: URL {
        FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Application Support/Midistage")
    }
    static var flag: URL { stateDir.appendingPathComponent("trace") }
    static var logDir: URL {
        FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Logs/midistaged")
    }

    static func isOn() -> Bool { FileManager.default.fileExists(atPath: flag.path) }

    static func start() throws {
        try FileManager.default.createDirectory(at: stateDir, withIntermediateDirectories: true)
        try Data().write(to: flag)
    }

    static func stop() throws {
        if isOn() { try FileManager.default.removeItem(at: flag) }
    }

    /// 最新の trace.log.<日付> を Console で開く。無ければフォルダを開く
    static func openLatest() {
        let files = (try? FileManager.default.contentsOfDirectory(at: logDir, includingPropertiesForKeys: [.contentModificationDateKey])) ?? []
        let latest = files
            .filter { $0.lastPathComponent.hasPrefix("trace.log") }
            .max { a, b in
                let da = (try? a.resourceValues(forKeys: [.contentModificationDateKey]).contentModificationDate) ?? .distantPast
                let db = (try? b.resourceValues(forKeys: [.contentModificationDateKey]).contentModificationDate) ?? .distantPast
                return da < db
            }
        NSWorkspace.shared.open(latest ?? logDir)
    }
}

// ~/Library/Logs/midistaged/ の最新ファイル（日付つき）の末尾を追いかける。
// サービスへの wire は使わない（同じユーザーのファイルを読むだけ）。日付が変わって新しいファイルに
// 切り替わったら、そちらを頭から読む。行の読み取りは MenuModel（純粋）。
import Foundation
import MenuModel

@MainActor @Observable
final class LogTail {
    let prefix: String                    // "midistaged.log" / "trace.log"
    private(set) var lines: [LogLine] = []
    var following = true                  // 追従（新しい行が来たら末尾へ）
    let limit = 3000

    private var file: String?
    private var offset: UInt64 = 0
    private var remainder = ""
    private var nextID = 0
    private var timer: Timer?
    private var dir: URL { FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Logs/midistaged") }

    init(prefix: String) { self.prefix = prefix }

    func start() {
        guard timer == nil else { return }
        read(initial: true)
        timer = Timer.scheduledTimer(withTimeInterval: 0.5, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated { self?.read(initial: false) }
        }
    }

    func stop() { timer?.invalidate(); timer = nil }

    func clear() { lines.removeAll() }

    private func read(initial: Bool) {
        let names = (try? FileManager.default.contentsOfDirectory(atPath: dir.path)) ?? []
        guard let newest = newestLogFile(names, prefix: prefix) else { return }
        let url = dir.appendingPathComponent(newest)
        let size = ((try? FileManager.default.attributesOfItem(atPath: url.path))?[.size] as? NSNumber)?.uint64Value ?? 0
        if newest != file || size < offset {
            // 新しい日付のファイル、または切り詰められた。開いた直後は末尾 256 KiB だけ読む
            file = newest
            offset = initial ? (size > 262_144 ? size - 262_144 : 0) : 0
            remainder = ""
        }
        guard size > offset, let handle = try? FileHandle(forReadingFrom: url) else { return }
        defer { try? handle.close() }
        try? handle.seek(toOffset: offset)
        guard let data = try? handle.readToEnd() else { return }
        offset += UInt64(data.count)
        let (raw, rest) = splitCompleteLines(remainder + String(decoding: data, as: UTF8.self))
        remainder = rest
        var added: [LogLine] = []
        for r in raw {
            nextID += 1
            if let l = parseLogLine(r, id: nextID) { added.append(l) }
        }
        guard !added.isEmpty else { return }
        lines.append(contentsOf: added)
        if lines.count > limit { lines.removeFirst(lines.count - limit) }
    }
}

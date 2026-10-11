// 担当 / 握っている app のアイコン。client_id → bundle id は末尾一致（MenuModel.bundleMatchesClient）。
// /Applications と稼働中の app から一度だけ引いて覚える。見つからなければ nil（文字だけ出す）。
import AppKit
import MenuModel

@MainActor
final class AppIcons {
    static let shared = AppIcons()
    private var cache: [String: NSImage?] = [:]
    private lazy var installed: [(bundleID: String, url: URL)] = {
        let fm = FileManager.default
        let dirs = ["/Applications", NSHomeDirectory() + "/Applications"]
        var out: [(String, URL)] = []
        for dir in dirs {
            for name in (try? fm.contentsOfDirectory(atPath: dir)) ?? [] where name.hasSuffix(".app") {
                let url = URL(fileURLWithPath: dir).appendingPathComponent(name)
                if let id = Bundle(url: url)?.bundleIdentifier { out.append((id, url)) }
            }
        }
        return out
    }()

    func icon(for clientID: String) -> NSImage? {
        if let hit = cache[clientID] { return hit }
        var found: NSImage?
        if let app = NSWorkspace.shared.runningApplications.first(where: {
            ($0.bundleIdentifier).map { bundleMatchesClient(bundleID: $0, clientID: clientID) } == true
        }) {
            found = app.icon
        } else if let hit = installed.first(where: { bundleMatchesClient(bundleID: $0.bundleID, clientID: clientID) }) {
            found = NSWorkspace.shared.icon(forFile: hit.url.path)
        }
        cache[clientID] = found
        return found
    }
}

// SMAppService の薄い包み（design 05「midistaged の同梱と LaunchAgent」）。
// 登録・解除は人の操作でだけ呼ぶ。初回起動で勝手に register しない。
import Foundation
import MenuModel
import ServiceManagement

enum Residency {
    static let plistName = "club.chronista.midistaged.plist"
    static var agent: SMAppService { SMAppService.agent(plistName: plistName) }

    static func registration() -> Registration {
        switch agent.status {
        case .enabled: return .enabled
        case .requiresApproval: return .requiresApproval
        case .notRegistered: return .notRegistered
        case .notFound: return .notFound
        @unknown default: return .notFound
        }
    }

    static func enable() throws { try agent.register() }
    static func disable() throws { try agent.unregister() }
    /// 「サービスを再起動」= 解除 → 登録。動いているプロセスは launchd が止めて新しい同梱版を起こす
    static func restart() throws { try? agent.unregister(); try agent.register() }
    static func openSettings() { SMAppService.openSystemSettingsLoginItems() }

    /// 同梱版の事実。版は Info.plist、実行ファイルは .app の中の midistaged
    static func bundled() -> Bundled {
        let version = Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "dev"
        let exe = Bundle.main.bundleURL.appendingPathComponent("Contents/MacOS/midistaged").path
        return Bundled(version: version, executable: exe)
    }
}

// 自動アップデート（spec REQ-APP-060〜062、design 05「自動アップデート」）。アプリ本体は Sparkle 2 が更新する。
// 動いている midistaged の入れ替えは別（AppModel が serviceUpdate で決める）。
import Foundation
import Sparkle

@MainActor
final class Updates: NSObject, SPUStandardUserDriverDelegate {
    /// Info.plist に SUFeedURL が無い（swift run の開発ビルド）なら作らない
    private var controller: SPUStandardUpdaterController?

    override init() {
        super.init()
        guard Bundle.main.object(forInfoDictionaryKey: "SUFeedURL") != nil else { return }
        controller = SPUStandardUpdaterController(startingUpdater: true, updaterDelegate: nil, userDriverDelegate: self)
    }

    var available: Bool { controller != nil }

    func checkForUpdates() { controller?.checkForUpdates(nil) }

    // メニューバーに常駐するアプリなので、定期の確認では作業中にウィンドウを割り込ませない（控えめに知らせる）
    nonisolated var supportsGentleScheduledUpdateReminders: Bool { true }
}

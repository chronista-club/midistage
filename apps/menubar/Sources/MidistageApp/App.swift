// midistage — メニューバーから始める（docs/spec/01 REQ-APP-040〜046、docs/design/05「常駐と配布」）。
// Dock に出ない（Info.plist の LSUIElement）。状態は 登録 / 稼働 / 機材 の 3 事実を MenuModel で合成する。
import AppKit
import MenuModel
import SwiftUI

@main
struct MidistageApp: App {
    @State private var model = AppModel()

    var body: some Scene {
        MenuBarExtra {
            MenuContent(model: model)
        } label: {
            Image(systemName: model.summary.icon.symbol)
        }
        .menuBarExtraStyle(.menu)
        Window("midistage — ステータス", id: "status") {
            StatusPlaceholder(model: model)
        }
    }
}

extension Icon {
    var symbol: String {
        switch self {
        case .normal: return "pianokeys"
        case .attention: return "exclamationmark.triangle"
        case .stopped: return "pianokeys.inverse"
        }
    }
}

@MainActor @Observable
final class AppModel {
    var state = ServiceState(registration: Residency.registration(), running: .stopped, bundled: Residency.bundled())
    var summary: MenuSummary { summarize(state) }
    var lastError: String?
    private let probe = ServiceProbe()
    private var task: Task<Void, Never>?

    init() { start() }

    func start() {
        task = Task { [weak self] in
            while !Task.isCancelled {
                await self?.refresh()
                try? await Task.sleep(for: .seconds(3))
            }
        }
    }

    func refresh() async {
        let running = await probe.probe()
        state = ServiceState(registration: Residency.registration(), running: running, bundled: Residency.bundled())
    }

    func act(_ f: () throws -> Void) {
        do { try f(); lastError = nil } catch { lastError = "\(error)" }
        Task { await refresh() }
    }
}

struct MenuContent: View {
    var model: AppModel
    @Environment(\.openWindow) private var openWindow

    var body: some View {
        let s = model.summary
        Text(s.title)
        ForEach(s.lines, id: \.self) { Text($0) }
        if let e = model.lastError { Text("失敗: \(e)") }
        Divider()
        if s.needsApproval {
            Button("システム設定でログイン項目を許可…") { Residency.openSettings() }
        }
        if model.state.registration == .enabled {
            Button("常駐を無効にする") { model.act(Residency.disable) }
            Button(s.versionMismatch ? "サービスを再起動（同梱版に入れ替え）" : "サービスを再起動") { model.act(Residency.restart) }
        } else {
            Button("常駐を有効にする") { model.act(Residency.enable) }
        }
        Divider()
        Button("ステータスを開く") { openWindow(id: "status"); NSApp.activate(ignoringOtherApps: true) }
        Button("ログを開く") {
            let logs = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Logs/midistaged")
            NSWorkspace.shared.open(logs)
        }
        Divider()
        Button("midistage を終了") { NSApp.terminate(nil) }
    }
}

/// spec 01 のステータス一覧の置き場。今は要約だけ（2 ビューは後から育てる）
struct StatusPlaceholder: View {
    var model: AppModel
    var body: some View {
        let s = model.summary
        VStack(alignment: .leading, spacing: 8) {
            Text(s.title).font(.headline)
            ForEach(s.lines, id: \.self) { Text($0) }
            Text("同梱 midistaged \(model.state.bundled.version)").foregroundStyle(.secondary)
        }
        .padding(20)
        .frame(minWidth: 360, minHeight: 160, alignment: .topLeading)
    }
}

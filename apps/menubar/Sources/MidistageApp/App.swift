// midistage — メニューバーから始める（docs/spec/01 REQ-APP-040〜046、docs/design/05「常駐と配布」）。
// Dock に出ない（Info.plist の LSUIElement）。状態は 登録 / 稼働 / 機材 の 3 事実を MenuModel で合成する。
import AppKit
import MenuModel
import SwiftUI
import os

let log = Logger(subsystem: "club.chronista.midistage", category: "menubar")

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
            StatusView(model: model)
        }
        .defaultSize(width: 760, height: 420)
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
    var rows: [DeviceRow] = []
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
        let result = await probe.probe()
        rows = statusRows(result.devices)
        let next = ServiceState(registration: Residency.registration(), running: result.running, bundled: Residency.bundled())
        if next != state {
            // 状態が変わったときだけ。理由をそのまま残す（spec REQ-APP-002: 黙って空にしない）
            log.notice("registration=\(String(describing: next.registration), privacy: .public) running=\(String(describing: next.running), privacy: .public) summary=\(summarize(next).title, privacy: .public)")
        }
        state = next
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

/// spec 01 のステータス一覧（REQ-APP-001〜003）。挿さっている / 担当 / 握っている / phase を別の列で
struct StatusView: View {
    var model: AppModel
    var body: some View {
        let s = model.summary
        VStack(alignment: .leading, spacing: 10) {
            HStack {
                Image(systemName: s.icon.symbol)
                Text(s.title).font(.headline)
                Spacer()
                Text("同梱 midistaged \(model.state.bundled.version)").foregroundStyle(.secondary).font(.caption)
            }
            ForEach(s.lines, id: \.self) { Text($0).font(.callout).foregroundStyle(.secondary) }
            if model.rows.isEmpty {
                Text(model.state.running == .stopped ? "サービスが停止しているため、機材の状態は分かりません" : "機材の情報を待っています…")
                    .foregroundStyle(.secondary)
                Spacer()
            } else {
                Table(model.rows) {
                    TableColumn("機材") { row in
                        Text(row.name).fontWeight(row.tone == .attention ? .semibold : .regular)
                            .foregroundStyle(row.tone == .quiet ? .secondary : .primary)
                    }
                    TableColumn("挿さっている") { row in Text(row.presence).foregroundStyle(row.presence == "挿さっている" ? .primary : .secondary) }
                    TableColumn("担当", value: \.assignment)
                    TableColumn("握っている", value: \.holder)
                    TableColumn("phase", value: \.phase)
                    TableColumn("注意") { row in
                        Text(row.note ?? "").foregroundStyle(row.tone == .attention ? Color.orange : Color.secondary)
                    }
                }
            }
        }
        .padding(16)
        .frame(minWidth: 640, minHeight: 320, alignment: .topLeading)
    }
}

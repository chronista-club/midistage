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
    @State private var logs = LogViewModel()

    var body: some Scene {
        MenuBarExtra {
            MenuContent(model: model, logs: logs)
        } label: {
            Image(systemName: model.summary.icon.symbol)
        }
        .menuBarExtraStyle(.menu)
        Window("midistage — ステータス", id: "status") {
            StatusView(model: model)
        }
        .defaultSize(width: 920, height: 480)
        Window("midistage — ログ", id: "log") {
            LogWindow(app: model, vm: logs)
        }
        .defaultSize(width: 980, height: 520)
    }
}

/// phase の点の色（mako「Phase はポイントと色で表してもいいね」）
func phaseColor(_ tone: PhaseTone) -> Color {
    switch tone {
    case .active: return .green
    case .waiting: return .yellow
    case .releasing: return .orange
    case .error: return .red
    case .off: return Color.secondary.opacity(0.5)
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
    var state = ServiceState(registration: Residency.registration(), running: .stopped, bundled: Residency.bundled(), appAtLogin: Residency.appAtLogin(), tracing: MidiTrace.isOn())
    var rows: [DeviceRow] = []
    var summary: MenuSummary { summarize(state) }
    var lastError: String?
    private let probe = ServiceProbe()
    private var task: Task<Void, Never>?

    init() {
        start()
        // 終了時に接続を閉じる（閉じないとサービスに約 30 秒セッションが残る）
        NotificationCenter.default.addObserver(forName: NSApplication.willTerminateNotification, object: nil, queue: nil) { [probe] _ in
            let done = DispatchSemaphore(value: 0)
            Task.detached { await probe.close(); done.signal() }
            _ = done.wait(timeout: .now() + 1.0)
        }
    }

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
        let next = ServiceState(registration: Residency.registration(), running: result.running, bundled: Residency.bundled(), appAtLogin: Residency.appAtLogin(), tracing: MidiTrace.isOn())
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
    var logs: LogViewModel
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
        if model.state.appAtLogin {
            Button("ログイン時にアプリを起動しない") { model.act(Residency.disableAppAtLogin) }
        } else {
            Button("ログイン時にアプリも起動する") { model.act(Residency.enableAppAtLogin) }
        }
        Divider()
        Button("ステータスを開く") { openWindow(id: "status"); NSApp.activate(ignoringOtherApps: true) }
        Divider()
        Button("ログを開く") {
            logs.tab = .flow
            openWindow(id: "log"); NSApp.activate(ignoringOtherApps: true)
        }
        if model.state.tracing {
            Button("MIDI トレースを見る") {
                logs.tab = .trace
                openWindow(id: "log"); NSApp.activate(ignoringOtherApps: true)
            }
            Button("MIDI トレースを止める") { model.act(MidiTrace.stop) }
        } else {
            Button("MIDI トレースを開始（開発用）") {
                model.act(MidiTrace.start)
                logs.tab = .trace
                openWindow(id: "log"); NSApp.activate(ignoringOtherApps: true)
            }
        }
        Button("ログのフォルダを開く") {
            let logs = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Logs/midistaged")
            NSWorkspace.shared.open(logs)
        }
        Divider()
        Button("midistage を終了") { NSApp.terminate(nil) }
    }
}

/// spec 01 のステータス一覧（REQ-APP-001〜003）。担当 / 握っている / phase を別の列で。
/// 「挿さっている / 見当たらない」は機材名の濃淡で表す（mako 2026-10-12「タイトルですでに表現できてるね」）
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
                        // メーカー + 機材名。メーカーは控えめ（mako「メーカーは、控えめの色と表示」）
                        HStack(alignment: .firstTextBaseline, spacing: 6) {
                            if let maker = row.maker {
                                Text(maker).font(.caption).foregroundStyle(.tertiary)
                            }
                            Text(row.model).fontWeight(row.tone == .attention ? .semibold : .regular)
                                .foregroundStyle(row.tone == .quiet ? .secondary : .primary)
                        }
                        .help(row.id)
                    }
                    .width(min: 180, ideal: 230)
                    TableColumn("担当") { row in AppLabel(name: row.assignment) }.width(min: 90, ideal: 120)
                    TableColumn("握っている") { row in AppLabel(name: row.holder) }.width(min: 90, ideal: 120)
                    TableColumn("phase") { row in
                        HStack(spacing: 6) {
                            Circle().fill(phaseColor(phaseTone(row.phase))).frame(width: 8, height: 8)
                            Text(row.phase)
                        }
                    }
                    .width(min: 110, ideal: 150)
                    TableColumn("注意") { row in
                        Text(row.note ?? "").foregroundStyle(row.tone == .attention ? Color.orange : Color.secondary)
                            .help(row.note ?? "")
                    }
                    .width(min: 220, ideal: 320)
                }
            }
        }
        .padding(16)
        .frame(minWidth: 640, minHeight: 320, alignment: .topLeading)
    }
}

/// app 名の左にアイコン（引けたときだけ）。「なし」「—」「この画面」は文字のまま
struct AppLabel: View {
    var name: String
    var body: some View {
        HStack(spacing: 5) {
            if let icon = AppIcons.shared.icon(for: name), !["なし", "—", "この画面", "？"].contains(name) {
                Image(nsImage: icon).resizable().frame(width: 16, height: 16)
            }
            Text(name)
        }
    }
}

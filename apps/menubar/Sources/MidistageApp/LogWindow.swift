// 「ログ」window（mako 2026-10-14「そのログを window に出せる？」）。
// 流れ = midistaged.log（info / warn、常時）、MIDI トレース = trace.log（開発用、オンのときだけ増える）。
import AppKit
import MenuModel
import SwiftUI

enum LogTab: String, CaseIterable, Identifiable {
    case flow = "流れ", trace = "MIDI トレース"
    var id: String { rawValue }
}

@MainActor @Observable
final class LogViewModel {
    let flow = LogTail(prefix: "midistaged.log")
    let trace = LogTail(prefix: "trace.log")
    var tab: LogTab = .flow
    var query = ""
    var hideLibrary = true
    var hideClock = true      // クロック / Active Sensing の行（古いファイルに残っている分も）を隠す
}

private let timeFormat: DateFormatter = {
    let f = DateFormatter()
    f.dateFormat = "HH:mm:ss.SSS"
    return f
}()

struct LogWindow: View {
    var app: AppModel
    @Bindable var vm: LogViewModel

    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: 12) {
                Picker("", selection: $vm.tab) {
                    ForEach(LogTab.allCases) { Text($0.rawValue).tag($0) }
                }
                .pickerStyle(.segmented)
                .frame(width: 240)
                TextField("絞り込み（機材・app・文字）", text: $vm.query)
                    .textFieldStyle(.roundedBorder)
                    .frame(maxWidth: 260)
                if vm.tab == .flow {
                    Toggle("通信ライブラリの行を隠す", isOn: $vm.hideLibrary).toggleStyle(.checkbox)
                } else {
                    Toggle("クロックを隠す", isOn: $vm.hideClock).toggleStyle(.checkbox)
                }
                Spacer()
                Toggle("追従", isOn: Binding(
                    get: { (vm.tab == .flow ? vm.flow : vm.trace).following },
                    set: { (vm.tab == .flow ? vm.flow : vm.trace).following = $0 }
                )).toggleStyle(.checkbox)
                Button("消去") { (vm.tab == .flow ? vm.flow : vm.trace).clear() }
                if vm.tab == .trace {
                    if app.state.tracing {
                        Button("トレースを止める") { app.act(MidiTrace.stop) }
                    } else {
                        Button("トレースを開始") { app.act(MidiTrace.start) }
                    }
                }
            }
            .padding(10)
            Divider()
            switch vm.tab {
            case .flow: FlowList(tail: vm.flow, query: vm.query, hideLibrary: vm.hideLibrary)
            case .trace: TraceTable(tail: vm.trace, query: vm.query, tracing: app.state.tracing, hideClock: vm.hideClock)
            }
        }
        .frame(minWidth: 820, minHeight: 360)
        .onAppear { vm.flow.start(); vm.trace.start() }
        .onDisappear { vm.flow.stop(); vm.trace.stop() }
    }
}

struct FlowList: View {
    var tail: LogTail
    var query: String
    var hideLibrary: Bool

    var body: some View {
        let shown = tail.lines.filter { (!hideLibrary || !isLibraryNoise($0)) && matches($0, query: query) }
        ScrollViewReader { proxy in
            List(shown) { line in
                HStack(alignment: .firstTextBaseline, spacing: 10) {
                    Text(timeFormat.string(from: line.time)).foregroundStyle(.secondary)
                    Text(line.level.rawValue)
                        .foregroundStyle(levelColor(line.level))
                        .frame(width: 44, alignment: .leading)
                    Text(line.message).foregroundStyle(line.level == .info ? .primary : levelColor(line.level))
                        .textSelection(.enabled)
                }
                .font(.system(.callout, design: .monospaced))
                .id(line.id)
            }
            .onChange(of: shown.last?.id) { _, last in
                if tail.following, let last { proxy.scrollTo(last, anchor: .bottom) }
            }
            .onAppear { if let last = shown.last?.id { proxy.scrollTo(last, anchor: .bottom) } }
        }
    }
}

struct TraceTable: View {
    var tail: LogTail
    var query: String
    var tracing: Bool
    var hideClock: Bool

    var body: some View {
        let rows = tail.lines.filter { matches($0, query: query) }.compactMap(traceRow)
            .filter { !hideClock || !isClockRow($0) }
        if rows.isEmpty {
            VStack(spacing: 8) {
                Spacer()
                Text(tracing ? "MIDI を待っています…（機材を触ると行が流れます）" : "トレースは止まっています。上の「トレースを開始」で記録が始まります")
                    .foregroundStyle(.secondary)
                Spacer()
            }
            .frame(maxWidth: .infinity)
        } else {
            ScrollViewReader { proxy in
                Table(rows) {
                    TableColumn("時刻") { r in Text(timeFormat.string(from: r.time)).foregroundStyle(.secondary) }
                        .width(min: 90, ideal: 100)
                    TableColumn("向き", value: \.direction).width(min: 60, ideal: 70)
                    TableColumn("機材", value: \.device).width(min: 70, ideal: 90)
                    TableColumn("app", value: \.app).width(min: 60, ideal: 80)
                    TableColumn("判定") { r in Text(r.verdict).foregroundStyle(verdictColor(r.verdict)) }
                        .width(min: 70, ideal: 110)
                    TableColumn("種類", value: \.kind).width(min: 60, ideal: 80)
                    TableColumn("バイト列") { r in Text(r.detail).textSelection(.enabled) }
                }
                .font(.system(.callout, design: .monospaced))
                .onChange(of: rows.last?.id) { _, last in
                    if tail.following, let last { proxy.scrollTo(last, anchor: .bottom) }
                }
                .onAppear { if let last = rows.last?.id { proxy.scrollTo(last, anchor: .bottom) } }
            }
        }
    }
}

func levelColor(_ l: LogLevel) -> Color {
    switch l {
    case .error: return .red
    case .warn: return .orange
    case .info: return .secondary
    case .debug, .trace: return Color.secondary.opacity(0.6)
    }
}

/// 判定の色: 届いた（FWD / SENT / QUEUE）は通常、捨てた・失敗は注意、開閉は控えめ
func verdictColor(_ v: String) -> Color {
    if v.hasPrefix("DROP") || v.hasPrefix("FAIL") || v.hasPrefix("OVERFLOW") { return .orange }
    if v.hasPrefix("OPEN") || v.hasPrefix("CLOSE") { return .secondary }
    return .primary
}

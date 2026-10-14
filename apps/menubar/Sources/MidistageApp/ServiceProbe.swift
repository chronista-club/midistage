// 稼働の事実を集める: endpoint.json → SDK 接続 → snapshot。観察専用（lease を取らない）。
// 失敗の理由はそのまま Running.unreachable に載せる（spec REQ-APP-002）。
import Foundation
import MenuModel
import MidistageClient

struct ProbeResult: Sendable {
    var running: Running
    var devices: [DeviceFacts]
}

actor ServiceProbe {
    private var releasingSince: [String: Date] = [:]
    /// 繋ぎっぱなしにする（3 秒ごとに接続・切断すると、サービスの info ログが毎回 connected / disconnected で埋まる）。
    /// 状態は Snapshot 要求で読み直す。失敗したら捨てて、次回に繋ぎ直す
    private var client: Client?
    /// 接続中の 1 本。actor は await の間に次の probe を受け付けるので、繋ぎ始めを共有しないと
    /// 2 本目が「同じ app のセッションがある」で断られる
    private var connecting: Task<(Client, Snapshot), Error>?
    /// close の後に接続が終わったら、その接続は閉じる（終了時にセッションを残さない）
    private var closed = false
    /// releasing がこれより長く続いたら「止まっている」とみなす（表示上の閾値。design 05）
    let stuckAfter: TimeInterval = 5

    /// 終了時に呼ぶ。閉じずに落ちると、サービスは QUIC のタイムアウト（約 30 秒）までセッションを
    /// 生きているとみなし、その間の再接続は「同じ app のセッションがある」で断られる（実測 2026-10-14）
    func close() async {
        closed = true
        connecting?.cancel()
        await drop()
    }

    /// 持っている client を捨てて閉じる。await の前に外す（その間に来た probe が同じ client を使わない）。
    /// `only` を渡したら、それが今の client のときだけ（遅れて失敗した probe が、別の probe の新しい接続を閉じない）
    private func drop(only: Client? = nil) async {
        guard let stale = client, only == nil || only === stale else { return }
        client = nil
        await stale.close()
    }

    /// 接続を 1 本だけ作る（同時に来た probe は同じ接続を待つ）
    private func connect(_ url: URL) async throws -> (Client, Snapshot) {
        let task = connecting ?? Task {
            try await Client.connect(
                endpointURL: url, clientID: "club.chronista.midistage.app",
                displayName: "midistage", nativeMIDI: false)
        }
        connecting = task
        defer { if connecting == task { connecting = nil } }
        let (connected, first) = try await task.value
        if closed {
            // 同じ接続を待っていた probe が複数いても、閉じるのは最初の 1 つだけ
            if connecting == task {
                connecting = nil
                await connected.close()
            }
            throw CancellationError()
        }
        if client == nil { client = connected }
        return (connected, first)
    }

    func probe() async -> ProbeResult {
        let url = Client.defaultEndpointURL
        guard !closed else { return ProbeResult(running: .stopped, devices: []) }
        guard FileManager.default.fileExists(atPath: url.path) else {
            await drop()
            return ProbeResult(running: .stopped, devices: [])
        }
        let using = client
        do {
            let snapshot: Snapshot
            if let client {
                snapshot = try await client.snapshot()
            } else {
                snapshot = try await connect(url).1
            }
            let summary = summarize(snapshot)
            let running = Running.connected(version: snapshot.service?.version,
                                            executable: snapshot.service?.executable,
                                            devices: summary)
            return ProbeResult(running: running, devices: facts(snapshot))
        } catch {
            // 繋がっていた client は捨てる（サービスの再起動・入れ替えで epoch が変わった等）。次回に繋ぎ直す
            if let using { await drop(only: using) }
            return ProbeResult(running: .unreachable(reason: "\(error)"), devices: [])
        }
    }

    /// 3D 資産の有無: design 04 の配布先に <id>.usdz があるか（REQ-APP-011）
    private func hasAsset(_ id: String) -> Bool {
        let dist = FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent("Library/Application Support/midistage/gear/\(id).usdz")
        return FileManager.default.fileExists(atPath: dist.path)
    }

    /// 配布先の <id>.json から maker / model（REQ-APP-001 の「どこの何か」）
    private func makerModel(_ id: String) -> (String?, String?) {
        let url = FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent("Library/Application Support/midistage/gear/\(id).json")
        guard let data = try? Data(contentsOf: url),
              let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else { return (nil, nil) }
        return makerAndModel(fromGearJSON: json)
    }

    private func facts(_ snapshot: Snapshot) -> [DeviceFacts] {
        let now = Date()
        return snapshot.devices.map { d in
            let stuck = d.phase == "releasing" && releasingSince[d.deviceID].map { now.timeIntervalSince($0) > stuckAfter } == true
            return DeviceFacts(
                id: d.deviceID, name: d.name, present: d.present,
                assignedTo: d.assignment.clientID, revision: d.assignment.revision,
                held: d.lease != nil, heldByMe: d.lease?.sessionID == snapshot.sessionID,
                phase: d.phase, releasingTo: d.releasingTo, error: d.error,
                releasingStuck: stuck, hasGearAsset: hasAsset(d.profileID),
                maker: makerModel(d.profileID).0, model: makerModel(d.profileID).1)
        }
    }

    private func summarize(_ snapshot: Snapshot) -> DeviceSummary {
        let now = Date()
        var stuck = 0
        var errors = 0
        var present = 0
        var seen: Set<String> = []
        for d in snapshot.devices {
            if d.present { present += 1 }
            if d.error != nil { errors += 1 }
            if d.phase == "releasing" {
                seen.insert(d.deviceID)
                let since = releasingSince[d.deviceID] ?? now
                releasingSince[d.deviceID] = since
                if now.timeIntervalSince(since) > stuckAfter { stuck += 1 }
            }
        }
        releasingSince = releasingSince.filter { seen.contains($0.key) }
        return DeviceSummary(present: present, releasingStuck: stuck, errors: errors)
    }
}

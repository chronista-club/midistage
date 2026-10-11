// 稼働の事実を集める: endpoint.json → SDK 接続 → snapshot。観察専用（lease を取らない）。
// 失敗の理由はそのまま Running.unreachable に載せる（spec REQ-APP-002）。
import Foundation
import MenuModel
import MidistageClient

actor ServiceProbe {
    private var releasingSince: [String: Date] = [:]
    /// releasing がこれより長く続いたら「止まっている」とみなす（表示上の閾値。design 05）
    let stuckAfter: TimeInterval = 5

    func probe() async -> Running {
        let url = Client.defaultEndpointURL
        guard FileManager.default.fileExists(atPath: url.path) else { return .stopped }
        do {
            let (client, snapshot) = try await Client.connect(
                endpointURL: url, clientID: "club.chronista.midistage.app",
                displayName: "midistage", nativeMIDI: false)
            await client.close()
            return .connected(version: snapshot.service?.version,
                              executable: snapshot.service?.executable,
                              devices: summarize(snapshot))
        } catch {
            return .unreachable(reason: "\(error)")
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

import Testing
@testable import MenuModel

private func facts(_ id: String, present: Bool = true, assigned: String? = "ladyland", held: Bool = true,
                   phase: String = "active", releasingTo: String? = nil, error: String? = nil,
                   stuck: Bool = false, asset: Bool = true, me: Bool = false) -> DeviceFacts {
    DeviceFacts(id: id, name: id.uppercased(), present: present, assignedTo: assigned, revision: 3,
                held: held, heldByMe: me, phase: phase, releasingTo: releasingTo, error: error,
                releasingStuck: stuck, hasGearAsset: asset)
}

@Test func presenceAssignmentAndHolderAreSeparateColumns() {
    let rows = statusRows([facts("keystage")])
    #expect(rows[0].presence == "挿さっている")
    #expect(rows[0].assignment == "ladyland")
    #expect(rows[0].holder == "ladyland")
    #expect(rows[0].phase == "active")
    #expect(rows[0].note == nil && rows[0].tone == .normal)
}

@Test func assignedButNotHeldSaysTheAppIsNotConnected() {
    let rows = statusRows([facts("xtouch", held: false, phase: "waiting")])
    #expect(rows[0].holder == "—")
    #expect(rows[0].note == "ladyland が接続していません")
}

@Test func missingDeviceIsQuietAndSortedLast() {
    let rows = statusRows([facts("fgdp", present: false, held: false, phase: "waiting"), facts("roto")])
    #expect(rows.last?.id == "fgdp")
    #expect(rows.last?.presence == "見当たらない")
    #expect(rows.last?.tone == .quiet)
}

@Test func stuckHandoffAndErrorsAreAttentionAndSortedFirst() {
    let rows = statusRows([
        facts("roto"),
        facts("lpd8", phase: "releasing", releasingTo: "vp", stuck: true),
        facts("numa", phase: "error", error: "two units"),
    ])
    #expect(rows[0].tone == .attention && rows[1].tone == .attention)
    #expect(rows.map(\.id).contains("lpd8") && rows[2].id == "roto")
    let lpd8 = rows.first { $0.id == "lpd8" }!
    #expect(lpd8.phase == "releasing → vp")
    #expect(lpd8.note?.contains("引き継ぎが止まっています") == true)
}

@Test func thisWindowIsNamedWhenItHoldsTheLease() {
    let rows = statusRows([facts("maru", assigned: "midistage", me: true)])
    #expect(rows[0].holder == "この画面")
}

@Test func missingGearAssetIsNotedButNotAlarming() {
    let rows = statusRows([facts("maru", asset: false)])
    #expect(rows[0].note == "模型なし")
    #expect(rows[0].tone == .normal)
}

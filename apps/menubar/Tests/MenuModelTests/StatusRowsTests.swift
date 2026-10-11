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

@Test func duplicateNamesCarryTheirIDs() {
    // 認識前の generic:zenith 2 の登録が settings に残っていると、zenith2 と同名で並ぶ
    let rows = statusRows([
        DeviceFacts(id: "zenith2", name: "Zenith 2", present: true, assignedTo: nil, revision: 0, held: false, phase: "off"),
        DeviceFacts(id: "generic:zenith 2", name: "Zenith 2", present: false, assignedTo: nil, revision: 4, held: false, phase: "off"),
        DeviceFacts(id: "roto", name: "Roto-Control", present: true, assignedTo: "vp", revision: 1, held: true, phase: "active"),
    ])
    #expect(rows.map(\.name).contains("Zenith 2（zenith2）"))
    #expect(rows.map(\.name).contains("Zenith 2（generic:zenith 2）"))
    #expect(rows.map(\.name).contains("Roto-Control"))
}

@Test func phaseToneFollowsThePhaseWord() {
    #expect(phaseTone("active") == .active)
    #expect(phaseTone("waiting") == .waiting)
    #expect(phaseTone("releasing → vp") == .releasing)
    #expect(phaseTone("error") == .error)
    #expect(phaseTone("off") == .off)
}

@Test func clientIDMatchesTheLastComponentOfABundleID() {
    #expect(bundleMatchesClient(bundleID: "tech.anycreative.vp", clientID: "vp"))
    #expect(bundleMatchesClient(bundleID: "club.chronista.ladyland", clientID: "ladyland"))
    #expect(!bundleMatchesClient(bundleID: "club.chronista.midistage", clientID: "vp"))
    #expect(!bundleMatchesClient(bundleID: "vp", clientID: "ladyland"))
}

@Test func rowsSayWhoseAndWhichWhenTheGearJSONKnows() {
    // mako「ユーザが見たいのは、どこの何か」
    let rows = statusRows([
        DeviceFacts(id: "keystage", name: "Keystage", present: true, assignedTo: nil, revision: 0, held: false, phase: "off",
                    maker: "KORG", model: "Keystage 61"),
        DeviceFacts(id: "generic:maru", name: "maru", present: true, assignedTo: nil, revision: 0, held: false, phase: "off",
                    hasGearAsset: false),
    ])
    #expect(rows.map(\.name).contains("KORG Keystage 61"))
    #expect(rows.map(\.name).contains("maru"))
    let keystage = rows.first { $0.id == "keystage" }!
    #expect(keystage.maker == "KORG" && keystage.model == "Keystage 61")   // メーカーは別の欄（控えめに描く）
    let maru = rows.first { $0.id == "generic:maru" }!
    #expect(maru.maker == nil && maru.model == "maru")
    #expect(makerAndModel(fromGearJSON: ["maker": "AKAI", "model": "LPD8 mk2"]).maker == "AKAI")
    #expect(makerAndModel(fromGearJSON: [:]).model == nil)
}

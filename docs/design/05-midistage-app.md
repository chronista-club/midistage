# 05. midistage app — 構成・机のデータモデル・実装順

> **Status**: Draft
> **Related**: spec 01、起票 `mem_1Cfu4Bg8fsANpHGAoTMhT7`、design 03、design 04、火花 `mem_1Cfu3gwPdVa818Txp7FqCw`
> **対象**: `apps/midistage-app/`、`schemas/desk.kdl`、`gear/`

## Abstract

Swift package のアプリ 1 本（SwiftUI + RealityKit、macOS 13+）。midistaged には `MidistageClient` で観察専用に繋ぐ。機材の 3D 資産は design 04 の配布先から USDZ を読む。机の配置は KDL を正本にし、純粋な計算層（Swift、I/O なし）で検証と変換を行う。

## Architecture

```
apps/midistage-app/
  Package.swift
  Sources/
    DeskModel/        # 純粋: 机 KDL ↔ 構造、配置の検証、Ladyland desk_layout.json の取り込み
    StatusModel/      # 純粋: snapshot → 表示モデル、releasing 滞留・error の判定、takeover の前提確認
    GearAssets/       # 配布先の読み出し: manifest.json、gear.json、USDZ の所在。check 結果の読み出し
    MidistageApp/     # SwiftUI。StatusView / DeskEditorView / GearInspector。RealityKit の薄い wrapper
  Tests/
    DeskModelTests/   # KDL 往復、面の上に載る、はみ出し警告、取り込み
    StatusModelTests/ # fixture snapshot（midistaged の共通 fixture を流用）→ 表示モデル
```

- **接続**: `MidistageClient.connect(clientID: "club.chronista.midistage.app", displayName: "midistage", nativeMIDI: false)`。lease を取らない。`events` の `State` で一覧を更新する。接続失敗・epoch 変更は StatusModel の状態として持ち、画面の先頭に出す
- **資産**: `~/Library/Application Support/midistage/gear/<id>.usdz` と `<id>.json`（鍵盤展開済み）、`manifest.json`。USDZ のノード名 = 部品名（design 04 の契約）なので、RealityKit の `Entity.findEntity(named:)` でそのまま掴む。ポインタの hit test で部品名 → gear.json の `control` を引く
- **検査**: `check.py` の結果は build 時に `manifest.json` に入れる（design 04 に追記: 各 id の `check: { ok, issues[] }`）。アプリは読むだけで、自前で検査しない（spec REQ-APP-010）
- **机**: `~/Library/Application Support/midistage/desk/<name>.kdl`。既定 `default.kdl`。保存は一時ファイル → fsync → rename（midistaged の settings と同じ作法）
- **反転の扱い**: Ladyland は Jack 画面の机を midistage の `desk/` から読む側に回る（別 PR）。`studio.blend` の机・棚はそのまま Ladyland の Blender 資産として残し、配置値だけ KDL に移す。camera は Ladyland が持つ

## Data Model

### 机（`schemas/desk.kdl`）

単位 mm。座標系は gear と同じ: x = 右 +、z = 手前 +、y = 上。Ladyland `desk_layout.json` と同じ量を KDL に写す。

```kdl
// 場所ごとに 1 文書: desk/home.kdl（Zenith 2）、desk/studio.kdl（L6max）。例は default
desk "default" {
    // 床の上の机の足元。center は床面、size は机全体の占有
    footprint center=-200 -175 size=2050 1790

    // 面（天板・棚）。elevation = 床からの上面高さ、thickness = 板厚
    surface "front" center=0 -566.5 size=1650 917 elevation=740 thickness=40
    surface "shelf-upper" center=0 -900 size=1650 300 elevation=1120 thickness=25

    // 機材。id は midistage inventory の ID。surface に載せる。pos は面の上での中心、yaw は度
    gear "keystage" surface="front" pos=0 -400 yaw=0
    gear "nanokontrol" surface="shelf-upper" pos=-500 -900 yaw=0
    gear "l6max" surface="shelf-upper" pos=300 -900 yaw=0
    gear "zenith2" surface="front" pos=600 -300 yaw=-15

    // 任意の置き場（アプリが意味づけする名前つきの点）。Ladyland の tray に相当
    anchor "mixer" surface="front" pos=-120 -680
    anchor "track-knobs" surface="front" pos=120 -680
}
```

- `gear` の実寸と高さは gear.json から引く。KDL には位置と向きだけ（`size` を書かない。資産が正本）
- 同じ id を 2 回置ける（同一機種 2 台は v1 では状態表示の対象外。design 03）。区別が要るときは `gear "lpd8" tag="left"`
- **現在の場所**: `~/Library/Application Support/midistage/desk/current`（場所の名前 1 行 = 机の文書名）。無ければ `default`。場所 1 つ = 文書 1 枚。DeskModel は複数の机を同時に持てる純粋な層で、机の数で増えない — 増えるのは KDL 文書
- **机の一致度**（DeskModel、純粋）: `score(desk, present_ids) = |desk の gear ∩ present| / |desk の gear|`。現在の机より score が高い机があれば StatusModel が「切り替え提案」を出す。同点・0 件は提案しない。切り替えは人の操作だけ（spec REQ-APP-026）
- 読み込み時の検証（DeskModel）: surface が存在する、pos が surface の内側（はみ出しは警告）、機材同士の外形が重ならない（警告）
- 取り込み（Ladyland `desk_layout.json`）: `desk` → footprint、`surfaces[]` → surface、`gear[]` → gear（`elevation` から surface を逆引き、`id` は inventory ID へ写像: `ncxse` → `numa`、`fgdp50` → `fgdp`）、`tray` → anchor。`camera` は捨てる

### 状態の表示モデル（StatusModel）

snapshot の `DeviceView` から:

| 列 | 元 |
|---|---|
| 機材 | `device_id`、`name`、`profile_id`。資産の有無（GearAssets） |
| 挿さっている | `present` |
| 担当 | `assignment.client_id`（display_name は Hello の値を snapshot が持たないので、client_id をそのまま出す。表示名は将来の wire 拡張） |
| 握っている | `lease` の有無。`lease.session_id` と自分の `session_id` が同じなら「この画面」 |
| phase | `off` / `waiting` / `active` / `releasing` → `releasing_to`。`error` |
| 注意 | releasing が閾値（既定 5 秒）超、`error` あり、present なのに担当 app が接続していない |

takeover の前提確認: 表示した `assignment.revision` を持ち、送る直前の snapshot と一致するときだけ `SetEnabled { takeover: true, expected_revision }` を送る。

## Implementation

順番。各段は前の段のテストが通ってから。

1. **gear 資産の移設**（design 04 の段 3 を前倒し）: `gear/<id>/gear.json` + `.blend`、`gear/build.py`（USDZ / GLB / manifest、check 結果込み）、`gear/check.py`。Ladyland 側の L6max / Zenith 2（`wip/gear-l6max`）もここへ。これが無いと本アプリの資産読み出し先が無い
2. **StatusModel + StatusView**: fixture snapshot でテスト → 実サービスに繋いで一覧。接続失敗の表示。GearInspector（USDZ + 部品名 hover + check 結果）
3. **DeskModel**: KDL 往復と検証のテスト → Ladyland desk_layout.json の取り込みテスト
4. **DeskEditorView**: 面と機材の配置、drag で移動・回転、保存。机の上に REQ-APP-025 の状態バッジ
5. **Ladyland の読み出し切替**（別 repo、別 PR）: Jack 画面の机を `desk/default.kdl` から

検証の境界:
- 純粋層はユニットテストで固定。RealityKit の見た目はスクリーンショットで mako が確認（実機確認待ちの扱い）
- midistaged への接続は `crates/midistaged/examples/protocol_fixture.rs` を相手にした結合テスト（Swift SDK の interop テストと同じ型）

## 未決

- 表示名（display_name）を snapshot に載せる wire 拡張（design 03 側）。載せるなら `schemas/midistage.kdl` と protocol / Swift SDK / fixture を同じ PR で
- 机の KDL に「棚の段数」などの家具テンプレートを持つか（最初は surface の列挙だけ）
- VP の Devices 面の 3D 表示（`mem_1CfsA67B5KTNTQgChBcFbM`）との関係。VP は自分の面を持ち続け、資産と机は midistage から読む、で揃える

## Status log

- 2026-10-11 起工。Draft。spec 01 と同時

# 06. ROTO-CONTROL の setup を持ち主に合わせて切り替える

> **Status**: Draft
> **Related**: design 03（共通サービス）、mako 裁定 2026-10-16（creo `mem_1Cg4TThd7VWzafHywhSsVG`「MIDI モード + Serial で setup を焼く」「midistage側におこうか、ポートも」）、VP からの依頼（wire 2026-10-16）
> **対象**: `crates/midistage-profiles/src/roto_serial.rs`、`crates/midistaged/src/roto_admin.rs`、`schemas/midistage.kdl`、Swift SDK

## Abstract

ROTO-CONTROL を VP と Ladyland で切り替えながら使う（mako 2026-10-16「ROTOとVPと、Ladylandで、切り替えながら使いたい」）。Roto は MIDI モードで使い、各アプリは Roto 本体の **MIDI setup**（64 個の保存枠。名前 + つまみ 32 + ボタン 32）に自分の配置を持つ。lease が移ったら、midistage が新しい持ち主の setup へ Serial で切り替える。画面の名前・色・手応えと送る CC が、まるごと持ち主のものになる。

## Roto の setup と Serial

- Roto 本体は setup を 64 個持つ（電源を切っても残る）。表に出ているのは常に 1 個で、画面・手応え・送る CC はそれで決まる
- 操作は 2 種類: **切り替え**（`SET SETUP 02 03`。本体の SEL と同じ。一往復）と **書き込み**（setup の中身を書き換える。本体の保存領域に書く）
- Serial（admin port）を開いている間、Roto は MIDI の CC を送らない（全アプリぶん。Ladyland 実測 2026-08-12、close で即復帰）
- 1 要求 約 27 ms（実測 2026-10-16、4163 要求で 112.8 秒）

## 決めたこと

### Serial のポートは midistaged だけが開く

物理 MIDI ポートと同じ扱い。アプリは開かず、サービスに頼む。理由は、開くと全アプリの MIDI が止まる（= 仲介役の仕事）、lease を持つアプリだけが操作できる形で取り合いを防げる、の 2 つ。

- **開きっぱなしにしない**。用事（切り替え・読む・書く）を 1 列に並べ、1 つずつ 開く → 済ませる → 閉じる
- 用事の間、機材を **configuring** として snapshot に出す（MIDI が止まっている理由が見える）
- 他のプロセス（ROTO-SETUP、移行前の Ladyland）が開いていれば排他 open が失敗する。黙らず warn と snapshot の `config_error` に出す
- ポートは VID/PID（2E8A / F010）で Roto のものだけを選ぶ（maru など同じ VID の別機材に問い合わせない）

### 台帳: setup の持ち主は midistage が持つ

Roto 本体には「この setup は誰のもの」を書く場所が無い。名前で判断せず、midistage が `settings.json` に持つ。

```json
"device_setups": {
  "roto": {
    "ladyland": { "setups": [0, 1, 2, 3], "last": 2 },
    "vp":       { "setups": [6],          "last": null }
  }
}
```

- アプリは自分の setup を `SetDeviceSetups` で登録する。他のアプリの setup は取れない（`conflict`）。空の配列で取り消し
- `last` は最後に選んだ setup。持ち主に戻ったとき、そこへ戻す（Ladyland は自分の 4 つの間を行き来する）

### lease が移ったら切り替える

1. 引き継ぎ（design 03: `active(A) → releasing(A, B) → active(B)`）はそのまま
2. `active(B)` になり、B が Roto の setup を登録していれば、midistage が B の `last`（無ければ最初の setup）へ `SET SETUP`
3. B は snapshot の `configuring` が外れてから、自分の今の値を CC で送ってモーターを合わせる（Ladyland の `RotoMidiSync` が既にやっている）

同じ lease（token）に対しては 1 回だけ切り替える。失敗しても繰り返さない（warn と `config_error`）。

### アプリから切り替える

`SelectSetup { device_id, lease_token, setup }`。lease を持つアプリが、自分の登録した setup の中でだけ切り替えられる（Ladyland の「長押しで INST へ」の置き換え）。応答は受け付けた時点で返し、完了は snapshot の `configuring` / `current_setup` で見る。

## Wire（追加のみ。protocol_version は上げない）

- request `SetDeviceSetups { device_id, setups: [int] }`
- request `SelectSetup { device_id, lease_token, setup: int }`
- request `ApplyDeviceSetup { device_id, lease_token, setup: int, content: SetupContent }`（段 C。`content` の形は
  `wire.rs` の `SetupContent` = `clients/swift/Tests/MidistageClientTests/Fixtures/apply-device-setup.json` が正。
  載っていない席は CLEAR。本体ができるまで `not_implemented`）
- **完了の見方**（SelectSetup / ApplyDeviceSetup 共通）: 応答は受け付けた時点で返る。**受け付けた瞬間から**
  snapshot の `configuring` が true（列で待つ間も）。false に戻って `config_error` が無ければ完了、あれば失敗。
  速く終わっても busy を取り落とさないよう、受付の応答の snapshot 自体が `configuring=true` を含む
- `DeviceView` に省略可の欄: `configuring: bool`、`current_setup: int?`（midistage が最後に選んだもの。本体の SEL で変えられると知らない）、`setup_owners: { client_id: [int] }`、`config_error: string?`

## 気をつけること

- **本体の SEL で手動に切り替えられると、midistage には見えない**（ポートを開いていないと Roto は通知を送らない）。切り替えは midistage 経由を基本にする。保険としてアプリごとに MIDI チャンネルを分ける（Ladyland は ch1 / ch2）
- Roto をまだ DAW モード（MIX / PLUGIN）で使うアプリがあると、setup の切り替えだけでは足りない（SET MODE が要る）。今回は MIDI モードだけを扱う
- **書き込み（setup の中身を焼く）はこの段に入れない**。1 席で実機の反映と後遺症を確かめてから足す

## 段 C: 書き込み（ApplyDeviceSetup）

lease 保持者だけ、自分の台帳の setup にだけ。サービスが 全 64 席の backup（roto-dump と同じ JSON、
`~/Library/Application Support/Midistage/roto-backup/<日時>.json`）→ START CONFIG UPDATE → 差分だけ SET / CLEAR →
END → 読み戻し照合 をやる。実行中は configuring、失敗は config_error と warn。1 回の所要秒を実測して記録する
（Serial を開いている間 MIDI が止まる）。実機の最初の 1 回は VP の 1 席（6）だけ、mako の GO で。

## Status log

- 2026-10-16: 起草。読み取り（PR #26）の実測を前提に、切り替えと台帳を先に入れる
- 2026-10-16: 台帳 / SelectSetup / lease 移動での SET SETUP（PR #27）
- 2026-10-18: 段 C の契約（ApplyDeviceSetup の型、KDL、Rust / Swift SDK、共有 fixture）と、受付時点からの configuring の保証。本体は未実装（not_implemented）
- 2026-10-18: 段 C の本体（roto_serial の書き込み命令、roto_setup の検査と差分、roto_admin::apply_setup）。控えは `<state_dir>/roto-backup/<unix秒>-setup-NN.json`
- 2026-10-18: **実機で最初の 1 席を書いた**（mako GO。FW 3.3.0、MIDI モード、setup 6 に名前「VP」+ switch 7「MIC」）。
  2 op、ポートを開いていたのは 3508 ms / 135 要求（読み 65 + START/END + 2 + 読み戻し 65）、読み戻し一致。
  SET SETUP は 66 ms。同じ中身の再 apply は 0 op / 1669 ms（読むだけ）。**書いた後に挿し直しは要らない**
  （トレースで Roto の CC 250 件が VP に届くのを確認。ROTO-SETUP の Import で要った症状は出ない）。
  所要の目安: 約 1.7 秒 + 27 ms × 差分の席数 + 約 1.7 秒。その間 MIDI が止まる

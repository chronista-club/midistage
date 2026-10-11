# 01. midistage app — デスク構築エディターとステータス一覧

> **Status**: Draft
> **Related**: 起票 `mem_1Cfu4Bg8fsANpHGAoTMhT7`、design 05、design 03（共通サービス）、design 04（機材の 3D 資産）、火花 `mem_1Cfu3gwPdVa818Txp7FqCw`、ヴァーチャルスタジオ UX の GO `mem_1CfmRXq8Nq5sL425AS7AnC`、gear 集約 todo `mem_1CfsvTkUsCc3nJJn9GidaS`
> **対象**: `apps/midistage-app/`（新規）、`gear/`（design 04）、`~/Library/Application Support/midistage/desk/`

## Abstract

midistaged に GUI を付ける。ビューは 2 つ。**ステータス一覧**は、機材ごとに「挿さっているか・担当 app・今握っている app・phase」を midistaged の snapshot から描き、機材の 3D 資産（design 04）を部品名つきで見せる。**デスク構築エディター**は、机と棚の上に機材の 3D モデルを置いて配置を保存する。配置の正本は midistage が持ち、Ladyland / VP はそれを読む。

## Motivation

- 今、機材の状態を人が見る手段は `vp midi devices` の JSON だけ。引き継ぎが `releasing` で止まっている、サービスが落ちている、といったことが一目で分からない（2026-10-08〜10 の実機確認でそうだった。`mem_1CfrJGkydLLAzFP4RnYwLV`）
- design 04 で起こした機材の 3D 資産は、Blender でレンダーして目で確かめるしかない。部品名が gear.json の契約どおりかも、画面で触って確かめたい
- 机の配置は Ladyland の `desk_layout.json` / `studio.blend` にあり、VP は別に持っている。「標準的なデスク」を midistage が持てば、どのアプリも同じ机を見る。ヴァーチャルスタジオ UX（置く → 割り当てる → 操作する）の「置く」がここに来る

mako の言葉（原文は火花 `mem_1Cfu3gwPdVa818Txp7FqCw`）:

> 作ったアセットの確認や、機材の状態などがわかると良いかもね
> 標準的なデスク構築エディター的なのと、ステータス一覧的なビューは欲しいね

## Scope

### 含む

- macOS アプリ 1 本。ステータス一覧とデスク構築エディターの 2 ビュー
- midistaged への接続は **観察専用**（lease を取らない）。操作は「別 app への引き継ぎの確認」だけ
- 機材の 3D 資産は design 04 の配布先（`~/Library/Application Support/midistage/gear/`）を読む。模型の無い機材は箱で出す
- 机の配置ファイル（KDL）の読み書き。机・棚・機材の位置と向きだけを持つ

### 含まない

- 機材への MIDI 送信、`Present`、操作イベントの表示（design 03 の runtime adapter 後の話）
- Track / lane / dock への割り当て（アプリの意味づけ）。「割り当てる」「操作する」はアプリ側
- サービスの install / 起動停止（midistaged 自身の `install` subcommand。別件）
- iOS / visionOS。Ladyland の Field とは別物

### 設計の反転（design 04 からの変更）

design 04 は「机・棚・カメラはアプリに残る」とした。本 spec で **机と棚の配置は midistage の正本**に変わる。カメラはアプリに残る。Ladyland の `desk_layout.json` と `studio.blend` の机・棚部分は移設対象。design 04 の Status log に追記する。

## Requirements

### ステータス一覧

- REQ-APP-001: 機材ごとに `present` / 担当（assignment）/ 今握っている app（lease の client）/ phase を表示する。情報源は midistaged の snapshot のみ。CoreMIDI を直接見ない
- REQ-APP-002: サービスに接続できない（`endpoint.json` が無い、版が違う、epoch が変わった）ときは、機材一覧の前にその事実を出す。黙って空にしない
- REQ-APP-003: `releasing` が一定時間（design 03 のタイムアウトとは別に、表示上の閾値）続いた機材と、`error` の機材を目立たせる
- REQ-APP-004: 行を選ぶと、その機材の 3D モデルと gear.json の部品一覧を出す。部品にポインタを乗せると部品名と `control` ID が出る
- REQ-APP-005: 引き継ぎは、担当 app と revision を見せた確認の上で `SetEnabled(takeover)` を送る。確認後に状態が変わっていれば送らない（`stale_revision` を UI でも先に止める）
- REQ-APP-006: 本アプリは lease を取らない。snapshot の更新と `Quiesce` 以外の event を受けても機材に触らない

### 3D 資産の確認

- REQ-APP-010: design 04 の `check.py` が見るもの（部品集合 ⊇ gear.json の部品名、原点が底面中央、縮尺 1、外形が実寸 ±1 mm）の結果を機材ごとに ✓ / ✗ で出す。検査の実体は `check.py` と同じ規則で、アプリが別の規則を持たない
- REQ-APP-011: 模型の無い機材（inventory にあるが gear 資産が無い）は、実寸が分かれば箱、分からなければ既定の箱で出し、「模型なし」と書く
- REQ-APP-012: 配布先の `manifest.json`（schema version / sha256 / 生成元 commit）を読み、資産がどの commit から出たかを表示する

### デスク構築エディター

- REQ-APP-020: 机・棚（面）・機材を 3D で置く。機材は midistage の inventory ID で指定し、模型は gear 資産から引く
- REQ-APP-021: 機材は面の上に載る（面の elevation + 機材の底）。面からはみ出す・重なる配置は警告するが禁止しない
- REQ-APP-022: 配置は KDL で保存する（design 05 のスキーマ）。単位 mm、座標系はアプリと同じ（機材中心が原点、x = 右 +、z = 手前 +、y = 上）。保存は一時ファイル → rename
- REQ-APP-023: 切り替えの単位は**場所**。場所ごとに机の文書を 1 枚持つ（例: `home` に Zenith 2、`studio` に L6max）。場所が決まれば、机の形とそこにある機材が決まる。「現在の場所」はアプリが 1 つ持ち、起動時は前回の場所。場所ごとに違えたい設定が増えたら、同じ文書に節を足す（机の配置以外は今は持たない）
- REQ-APP-026: 挿さっている機材（snapshot の `present`）と各机の機材の一致度を出し、現在の机より合う机があれば**切り替えを提案する**。自動では切り替えない（両方挿さっている・どちらも無い場面で勝手に変わらない）。現在の机にあるのに挿さっていない機材は一覧で「見当たらない」と出す
- REQ-APP-024: Ladyland の `desk_layout.json`（`desk` / `surfaces` / `gear` / `tray`）から一度だけ取り込める。取り込み後の正本は KDL
- REQ-APP-025: 編集中の机にある機材の状態（REQ-APP-001）を、机の上の機材にも重ねて表示する（担当 app のバッジ、phase の色）

### 共通

- REQ-APP-030: 純粋な計算（配置の検証、KDL ↔ 構造、snapshot → 表示モデル）は I/O を持たない層に置き、テストで固定する。RealityKit / 画面は薄く保つ
- REQ-APP-031: 公開リポジトリに置く。非公開アプリ名を UI 文言や既定データに入れない（アプリ名は snapshot の `display_name` から出す）

# 04. 機材の 3D 資産 — 意味の層と見た目の層

**ステータス**: Draft（2026-10-10 起工、wip/gear-assets）
**起点**: アプリ B（Web、GLB 利用）の 3D lane からの handoff（creo mem_1Cfsv1jjEsr26BcBACweHx）。
mako「midistage に集めるか。3D モデルも。」→ Ladyland で Conception →「C で」

## なぜここに置くか

機材の形・実寸・操作子の対応は、アプリ（Ladyland / アプリ B）ではなく機材の事実。
midistage は「宣言ファイルが正本、機材上の状態は投影」の場所なので、機材の
3D 資産もここが正本になる。アプリは机の配置・タブ・カメラ・使用権の見せ方だけを持つ。

## 二層（mako 裁定「C で」）

| 層 | 正本 | 形式 | 役割 |
|---|---|---|---|
| **意味の層** | `gear/<id>/gear.json` | JSON（mm、差分が読める） | 実寸、部品の位置と**名前**、セクション、操作子 ID（`control`）、出典 URL |
| **見た目の層** | `gear/<id>/<id>.blend` | Blender | 材質・面取り・質感。**部品名は意味の層との契約**として守る |

- 意味の層から Blender で**初回生成**できる（`gear/build.py`、旧 Ladyland の
  gear_build.py / nanokontrol.py）。生成後は .blend を手で磨いてよい
- **検証**（`gear/check.py`）: .blend の部品集合 ⊇ gear.json の部品名、原点は底面中央、
  縮尺 1、外形が実寸 ±1mm。CI で回す。契約を人の注意ではなく機械で守る
- 再生成は手修正を消す。生成器を変えたら**別の一時シーン**で生成して比較する
  （Ladyland の assets/blender/README.md の作法をそのまま持ち越す）

### 棄てた案

- A「JSON だけが正本、.blend は常に生成物」— 質を手で上げる道が無い
- B「.blend が正本」— バイナリで差分が読めず、部品名の契約を人が守ることになる

## ID

- **機材 ID は midistage の inventory を正**とする: roto / lpd8 / nanokontrol / keystage /
  numa / minilab / fgdp / xtouch / maru。Ladyland の `ncxse` → `numa`、`fgdp50` → `fgdp`。
  機種名（Numa Compact X SE、FGDP-50）は `gear.json` の `model` に残す
- **部品名は触らない**（`pad_1` / `fader_3`、1 始まり。USDZ / GLB のノード名 = アプリが掴む名前）。
  各部品に `control: "pad.0"` を足して `ControlEvent`（0 始まり index）と繋ぐ。
  rename ではなく対応を書く

## 配布

- git に入れるのは**生成元だけ**（gear.json、.blend、スクリプト、プレビュー PNG）
- `gear/build.py` が `~/Library/Application Support/midistage/gear/` に書く:
  `<id>.usdz`（Ladyland）、`<id>.glb`（アプリ B、Web）、`<id>.json`（鍵盤展開済み）、
  `environment.exr`、`manifest.json`（schema version / 各ファイルの sha256 / 生成元 commit）
- Ladyland は midistage の置き場を優先し、無ければ従来の `ladyland/gear` にフォールバック。
  アプリ B は同じ置き場の GLB を読む
- 公開リポなので、出典（公式写真・PDF）は URL を metadata に書くだけ。画像を同梱しない

## 範囲外（別の仕事）

- 机・棚・カメラ（`studio.blend` / desk_layout.json）はアプリに残る
- CC 番号の二重定義（Ladyland の Jack 契約 vs midistage profiles）は
  「Ladyland が midistaged から ControlEvent を受ける」lane が解消する
- maru の模型（まだ無い）

## 進め方

1. Ladyland PR #36（.blend 8 台 + xtouch.json）が nightly に入るのを待つ
2. **GLB の spike**: 1 機種（nanokontrol）を Blender の glTF で出し、アプリ B で見る。
   品質が足りなければ変換経路を変える。ここで schema を固めてアプリ B に返す
3. 移設: gear/<id>/ に JSON と .blend、build.py / check.py、manifest
4. Ladyland の読み出しを midistage 優先 + フォールバックに
5. 旧 Ladyland の Gear/ は PR で削除（この handoff では削除しない）

## Status log

- 2026-10-10 起工。Draft
- 2026-10-10 GLB spike: Blender 5.2.2 の glTF 出力で nanokontrol.blend → nanokontrol.glb（1.43MB、ノード 118、材質 6、画像 1）。
  **部品名（fader_1 / knob_1 / m_1 / play …）は GLB のノード名にそのまま残る** = USDZ と同じ契約でアプリ B も掴める。
  見た目の品質はアプリ B（Three.js）側で未確認。spike のスクリプトは `spike-export-glb.py`（build.py に取り込む前の下書き）

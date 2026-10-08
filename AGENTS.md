# AGENTS.md

このファイルは、このリポジトリで作業する AI エージェント共通の開発規約です。

プロジェクト指示はこのファイルに集約する（CLAUDE.md は置かない）。Claude Code は AGENTS.md の読み込みを使う。

## プロジェクト概要

`midistage` は、**MIDI コントローラーを使いたい macOS アプリの共通の作業机**。常駐サービス `midistaged` だけが物理 MIDI ポートを開き、アプリ（アプリ A／B …）は対等な client としてつなぐ。機材ごとの担当と使用権（lease）を持ち、アプリ間の引き継ぎを安全に行う。

- 設計の正典: `docs/design/03-runtime-service.md`（共通サービス）
- 通信の契約: `schemas/midistage.kdl`（Unison protocol の `midistage` channel）
- KDL profile を `pull` / `push` / `diff` で機材に同期する設定 CLI `midistage`（Keystage から）は**計画中**。コマンドは雛形のみ

README は英語（`README.md`）が既定、日本語は `README.ja.md`。両方を同じ PR で揃える。公開リポジトリなので、非公開アプリの名前は README に出さず「アプリ A／B」と書く。

## creo-memories Atlas

このプロジェクトで `remember` するときは必ず `atlasId` を指定すること。

| キー | 値 |
|------|-----|
| Atlas 名 | midistage |
| Atlas ID | `atl_1CahBrpoKgKy1WbiNpzD8w` |

```
atlasId: "atl_1CahBrpoKgKy1WbiNpzD8w"
```

## crate 構造

```
crates/
  midistaged/          # サービス本体: CoreMIDI、アプリセッション、設定の保存、引き継ぎ、native_midi
  midistage-protocol/  # wire 型と状態遷移（I/O なし）
  midistage-client/    # Rust SDK
  midistage-profiles/  # 機材固有の入力・表示変換（I/O なし。VP から集約した正本）
  midistage-core/      # UMP、KORG SysEx、CoreMIDI FFI
  midistage-keystage/  # Keystage の設定モデル（設定 CLI 用）
  midistage-cli/       # 設定 CLI `midistage`（計画中）
clients/swift/         # Swift SDK（MidistageClient、ルートの Package.swift）
schemas/               # 通信の契約（KDL）
```

wire 型を変えるときは `schemas/midistage.kdl`、`midistage-protocol`、Swift SDK、共通 fixture を同じ PR で更新する。

## 開発コマンド

```bash
mise run check                              # mbx check --workspace --all-targets
mise run test                               # mbx test --workspace
mise run clippy                             # mbx clippy --workspace --all-targets
cargo fmt --all -- --check
scripts/test-sdk-interop.sh                 # Swift SDK と Rust サービスの相互テスト
cargo run -p midistaged -- list-ports       # 物理ポートの列挙（何も開かず、何も送らない）
cargo run -p midistaged -- serve            # サービス起動
```

Rust は `rust-toolchain.toml`（rustup）で固定し、mise では管理しない。dev loop の check / test / clippy は mbx 経由、install / release 系は素の cargo のまま。

## サービスの不変条件

詳細は design 03。崩しやすいものだけ挙げる。

- 物理ポートを開くのはサービスだけ。アプリは自分の bridge ポート以外を選ばない
- `active(A) → releasing(A, B) → active(B)`: A の `Quiesced` と送信中 SysEx の完了を確認するまで B を active にしない。タイムアウトだけで進めない
- 未実装の要求（`Present` など）を成功扱いしない
- profiles の純粋変換テストが通ることと、実機での確認を区別する。実機でしか確かめられないものは「実機確認待ち」と書く

## 設定 CLI（計画中）の protocol stack

| layer | path |
|-------|------|
| transport | UMP packet (MIDI 1.0/2.0 両対応、 SysEx7 packet で SysEx を運ぶ) |
| identify | MIDI-CI Discovery + Property Exchange GET (DeviceInfo / X-ParameterList) |
| read | KORG SysEx Func=0x10 (scene) / 0x0E (global) |
| write | KORG SysEx Func=0x40 (scene) / 0x51 (global) / 0x11 (memory write) |

MIDI-CI Property Exchange は **read-only** (Keystage 仕様)。 internal setting の write は KORG manufacturer SysEx 必須。

計画中のコマンド:

```
midistage list-ports
midistage list-profiles
midistage init <profile>
midistage pull [<profile>] [--scene N|all] [--global]
midistage push [<profile>] [--scene N|all] [--global] [--write-to-memory N]
midistage diff [<profile>]
midistage scene-change <N>            # Func=0x14
midistage mode <normal|native>        # Func=0x00
midistage controller-mode <name>      # Func=0x49
```

profile は KDL（`~/.config/midistage/<profile>.kdl`）。KORG Scene Data (500 byte) + Global Data (3667 byte) の階層構造を natural nesting で表現する。

## コーディング規約

- コメントは日本語
- data / calculations / actions を分離
- KORG protocol は `Keystage_MIDIimp.txt` (cplp-sound-system の docs/midi/Keystage/ にコピーあり) を SSOT として参照する。KORG 公式 docs そのものはこのリポジトリに入れない（公開リポジトリのため）

## 開発フロー (creo-memories first)

- 設計決定 / 調査結果 / 罠の知見は **creo-memories の midistage Atlas** に保存 (`atlasId: "atl_1CahBrpoKgKy1WbiNpzD8w"`)
- 共通サービスの設計: `mem_1CfnVkncPrmfdQet3bNSmx`（design 03）
- 設定 CLI の design 起草: `mem_1CahByDTFrNjzvpXfJRVgK`
- VP との cross-project link: `mem_1CahBzwD1Y3d1Z8BqJAbXz` (vantage-point Atlas、 HP の MIDI 設定機能を midistage に外部委譲した決定)

## 関連

- 起点 LANE: `vantage-point/mako/keystage` (VP HP capability 拡張から派生)
- Vendored: `cplp-sound-system/crates/cplp-midi` (RX 部分流用 + TX 新規実装、 attribution は `crates/midistage-core/src/{ump,coremidi_sys}.rs` の先頭コメント参照)
- Reference: `~/repos/cplp-sound-system/docs/midi/Keystage/` の KORG 公式 docs (Keystage_MIDIimp.txt 1115 行 / Keystage_PE_MIDIimp.txt 312 行 / Keystage_PE_ResourceList.txt 156 行)

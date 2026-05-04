# CLAUDE.md

## プロジェクト概要

`midistage` は **KORG MIDI 2.0 device configurator CLI**。 declarative KDL profile を push/pull で device に同期する。 起点は KORG Keystage (MIDI 2.0 / Polyphonic Aftertouch / 49-key)、 将来は LPD8 等の他 device も plugin module で扱う device-agnostic な framework に育てる。

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
  midistage-core/      # transport (UMP) + protocol (SysEx / MIDI-CI PE) + 7-bit encoding
  midistage-cli/       # binary `midistage` (pull / push / diff / init / list-* / scene-change / mode / controller-mode)
  midistage-keystage/  # Keystage device module (Scene / Global struct + KDL serde + binary serde)
```

## 開発コマンド

```bash
cargo build --workspace
cargo test --workspace
cargo install --path crates/midistage-cli   # binary install
cargo fmt --all -- --check
cargo clippy --workspace --all-targets
```

## protocol stack

| layer | path |
|-------|------|
| transport | UMP packet (MIDI 1.0/2.0 両対応、 SysEx7 packet で SysEx を運ぶ) |
| identify | MIDI-CI Discovery + Property Exchange GET (DeviceInfo / X-ParameterList) |
| read | KORG SysEx Func=0x10 (scene) / 0x0E (global) |
| write | KORG SysEx Func=0x40 (scene) / 0x51 (global) / 0x11 (memory write) |

MIDI-CI Property Exchange は **read-only** (Keystage 仕様)。 internal setting の write は KORG manufacturer SysEx 必須。

## v0 commands (計画)

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

## file format

KDL (`~/.config/midistage/<profile>.kdl`)。 KORG Scene Data (500 byte) + Global Data (3667 byte) の階層構造を natural nesting で表現。

## コーディング規約

- コメントは日本語
- data / calculations / actions を分離
- KORG protocol は `Keystage_MIDIimp.txt` (cplp-sound-system の docs/midi/Keystage/ にコピーあり) を SSOT として参照する

## 開発フロー (creo-memories first)

- 設計決定 / 調査結果 / 罠の知見は **creo-memories の midistage Atlas** に保存 (`atlasId: "atl_1CahBrpoKgKy1WbiNpzD8w"`)
- design 起草 SSOT: `mem_1CahByDTFrNjzvpXfJRVgK` (本 repo の design 全文、 protocol stack / Scene 500B / Global 3667B / commands / KDL / vendor in path)
- VP との cross-project link: `mem_1CahBzwD1Y3d1Z8BqJAbXz` (vantage-point Atlas、 HP の MIDI 設定機能を midistage に外部委譲した決定)

## 関連

- 起点 LANE: `vantage-point/mako/keystage` (VP HP capability 拡張から派生)
- Vendored: `cplp-sound-system/crates/cplp-midi` (RX 部分流用 + TX 新規実装、 attribution は `crates/midistage-core/src/{ump,coremidi_sys}.rs` の先頭コメント参照)
- Reference: `~/repos/cplp-sound-system/docs/midi/Keystage/` の KORG 公式 docs (Keystage_MIDIimp.txt 1115 行 / Keystage_PE_MIDIimp.txt 312 行 / Keystage_PE_ResourceList.txt 156 行)

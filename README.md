# midistage

> **KORG MIDI 2.0 device configurator CLI** — declarative KDL profile を push/pull で device に同期する tool。

起点は KORG **Keystage** (MIDI 2.0 / Polyphonic Aftertouch / 49-key)、 将来は LPD8 等の他 device も plugin module で扱う device-agnostic な framework に育てる。

## Status

**v0 in progress** (2026-05-04 開始). Keystage configurator が初期 dogfood target。 詳細は `docs/design/` 参照 (creo-memories `midistage` Atlas に design SSOT)。

## Usage (v0 計画、 未実装)

```bash
midistage list-ports                  # 接続中の MIDI device 列挙
midistage init keystage-vp-dev        # blank KDL profile template 生成
midistage pull keystage-vp-dev        # device → KDL
midistage push keystage-vp-dev        # KDL → device
midistage diff keystage-vp-dev        # 差分表示
midistage scene-change 0              # active scene 切替
```

## Architecture

```
crates/
  midistage-core/      # transport (UMP) + protocol (SysEx / MIDI-CI PE) + 7-bit encoding
  midistage-cli/       # binary `midistage`
  midistage-keystage/  # Keystage device module (Scene / Global / KDL serde)
```

## Protocol stack

| layer | path |
|-------|------|
| transport | UMP packet (MIDI 1.0/2.0 両対応、 SysEx7 packet で SysEx を運ぶ) |
| identify | MIDI-CI Discovery + Property Exchange GET (DeviceInfo / X-ParameterList) |
| read | KORG SysEx Func=0x10 (scene) / 0x0E (global) |
| write | KORG SysEx Func=0x40 (scene) / 0x51 (global) / 0x11 (memory write) |

MIDI-CI Property Exchange は **read-only** (Keystage 仕様)。 設定の write は KORG manufacturer SysEx 必須。

## License

Dual-licensed: **Apache-2.0 OR MIT** (publish 時に決定可能)。

## Origin

- 起点 LANE: `vantage-point/mako/keystage` (VP HP capability 拡張から派生、 2026-05-04)
- Vendored: `cplp-sound-system/crates/cplp-midi` (RX 部分流用 + TX 新規実装)
- Reference: KORG Keystage MIDI Implementation v1.00 (2023.8.31)

## Related projects

- [vantage-point](https://github.com/chronista-club/vantage-point) — AI ネイティブ開発環境、 HP container から `midistage` を spawn する path 予定
- [cplp-sound-system](https://github.com/chronista-club/cplp-sound-system) — 音楽 app、 `cplp-midi` の vendor 元

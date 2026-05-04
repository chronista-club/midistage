# KORG Keystage Protocol Detail

> **Layer 1 canonical doc** (repo-tracked、 Living Documentation primary source)
> 起源 spec: **KORG Keystage MIDI Implementation v1.00 (2023.8.31)**
> Reference copy: `~/repos/cplp-sound-system/docs/midi/Keystage/`

このドキュメントは KORG Keystage の MIDI 仕様を midistage 実装視点で抽出した SSOT。 公式 docs (KORG 配布の `.txt`) を一次資料、 本 doc は実装に必要な部分の構造化抽出。

## Device Identity

| field | value |
|-------|-------|
| Manufacturer ID | `0x42` (KORG) |
| Family ID | `0x0169` (`69 01` LSB-first) |
| Model ID (49-key) | `0x01` (Member ID) |
| Model ID (61-key) | `0x09` (Member ID) |

PE Discovery (MIDI-CI) では:
- Manufacturer ID: `42 00 00`
- Family ID: `69 01`
- Model Number: `01 00` (49) / `09 00` (61)

## SysEx Header Structure

KORG 独自 SysEx の共通構造:

```
+------+-------+----------------------------------------+
| Byte | Value | Description                            |
+------+-------+----------------------------------------+
|  1   | F0    | Exclusive status                       |
|  2   | 42    | KORG manufacturer ID                   |
|  3   | 4g    | g = Global MIDI Channel (0~F)          |
|  4   | 00    |                                        |
|  5   | 01    | Product ID = 0169H (Keystage)          |
|  6   | 69    |                                        |
|  7   | mm    | 01 = Keystage-49, 09 = Keystage-61     |
| 8~10 | ss    | [FunctionID]+[Data] length             |
|      |       | 7bit × 3 (Little Endian)               |
|  11  | nn    | Function ID                            |
|  12+ | dd    | Data (variable length)                 |
| last | F7    | End of Exclusive (EOX)                 |
+------+-------+----------------------------------------+
```

**Length encoding (byte 8~10)**: `0sss ssss  0sss ssss  0sss ssss` で 21-bit unsigned little-endian。 例えば length=2 なら `02 00 00`。

## Function ID Code List

### Transmitted (device → host)

| ID | Description | Triggered by |
|----|-------------|--------------|
| `0x4F` | Scene Change (notification) | scene 切替時 |
| `0x40` | Current Scene Data Dump | Func=0x10 受信時 |
| `0x51` | Global Data Dump | Func=0x0E 受信時 |
| `0x23` | Data Load Completed (ACK) | dump 受信成功時 |
| `0x24` | Data Load Error (NAK) | dump 受信失敗時 |
| `0x26` | Receive Data Format Error | format 不正時 |
| `0x21` | Write Completed | Func=0x11 成功時 |
| `0x22` | Write Error | Func=0x11 失敗時 |
| `0x42` | Mode Data | Func=0x12 受信時 |
| `0x01` | Native Mode Enter/Exit | Func=0x00 受信時 |
| `0x4A` | Knob Position Value Reply | Func=0x2A 受信時 |
| `0x5F` | Controller Mode Change | Func=0x49 受信時 |
| `0x41` | Parameter Change (Get Reply) | Func=0x2B 受信時 / parameter 変更時 |

### Recognized Receive (host → device)

| ID | Description | Response |
|----|-------------|----------|
| `0x10` | Current Scene Data Dump Request | Func=0x40 or 0x24 |
| `0x40` | Current Scene Data Dump | Func=0x23 or 0x24 |
| `0x0E` | Global Data Dump Request | Func=0x51 or 0x24 |
| `0x51` | Global Data Dump | Func=0x23 or 0x24 |
| `0x11` | Scene Data Write Request (ss=0~7) | Func=0x21 or 0x22 |
| `0x12` | Mode Request | Func=0x42 |
| `0x14` | Scene Change Request (ss=0~15) | Func=0x4F + 0x23 or 0x24 |
| `0x00` | Native Mode Enter/Exit Request | Func=0x01 |
| `0x49` | Controller Mode Change Request | Func=0x5F |
| `0x28` | Native Mode Display Message | Func=0x23 or 0x24 |
| `0x2A` | Knob Position Value Request | Func=0x4A |
| `0x2B` | Get Parameter Request | Func=0x41 |
| `0x41` | Parameter Change | (state 反映) |

## Scene Data Layout (TABLE 1, 500 byte)

1 scene = **500 byte**。 `Func=0x40` で host ↔ device。 device 内蔵 memory に **8 scene** 保存可能 (`Func=0x11` で memory slot ss=0~7 に焼く)。

| offset | size | parameter | range |
|--------|------|-----------|-------|
| 0~9 | 10 | Scene Name | null-terminated string |
| 10~17 | 8 | User Page (Knob 1~8) | 0~127 |
| 18~25 | 8 | Arp User Page (Knob 1~8) | 0~15 (Arp param 16 種から選択) |
| 26~28 | 3 | Modulation Mapping (ModWheel/Velocity/Aftertouch) | 0~127 |
| 29 | 1 | Property Exchange Enable | 0/1 (Disable/Enable) |
| 30 | 1 | Program Number Display | 0/1 |
| 31~46 | 16 | Arpeggiator (Mode/Octave/Latch/KeySync/Rate/Swing/Pattern/Ratchet×4/Gate×3/Velocity/Chance) | 各 0~N |
| 47~49 | 3 | Chord (SetNum/StrumTime/StrumDirection) | 各 0~N |
| 50~52 | 3 | Keyboard (MIDI Ch/Octave/Transpose) | 0~16 / 0~6 / 0~24 |
| 53~55 | 3 | Wheel (MIDI Ch/Lower/Upper) | 0~16 / 0~127 / 0~127 |
| 56~58 | 3 | Encoder REW (MIDI Ch/AssignType/CC-or-Note) | 0~16 / 0~2 / 0~127 |
| 59~61 | 3 | Encoder FF (同上) | 同上 |
| 62~64 | 3 | Knob 1 (MIDI Ch/Left/Right) | 0~16 / 0~127 / 0~127 |
| 65~445 | 381 | Knob 2~128 (各 3 byte) | 同上 |
| 446~451 | 6 | Play Button (MIDI Ch/AssignType/Behavior/CC-or-Note/Off/On) | 0~16 / 0~2 / 0~1 / 0~127 / 0~127 / 0~127 |
| 452~457 | 6 | Stop Button | 同上 |
| 458~463 | 6 | Rec Button | 同上 |
| 464~469 | 6 | Loop Button | 同上 |
| 470~475 | 6 | Tempo Button | 同上 |
| 476~481 | 6 | Metro Button | 同上 |
| 482~487 | 6 | Undo Button | 同上 |
| 488~493 | 6 | Track Down Button | 同上 |
| 494~499 | 6 | Track Up Button | 同上 |

**Total: 500 byte。** Knob は **128 個** (User Page で 16 page × 8 knob = 128 仮想 knob)、 Button は **9 個**。

### Arp Mode List

`00:Up / 01:Down / 02:Up-Down / 03:Down-Up / 04:Play / 05:Random / 06:Trigger`

### Arp Rate List

`00:1/1 / 01:1/2 / 02:1/3 / 03:1/4 / 04:1/6 / 05:1/8 / 06:1/12 / 07:1/16 / 08:1/24 / 09:1/32 / 10:1/48 / 11:1/64`

### Button Assign Type

`00:NoAssign / 01:CC / 02:Note`

### Button Behavior

`00:Toggle / 01:Momentary`

## Global Data Layout (TABLE 2, 3667 byte)

device 横断的な設定。 `Func=0x51` で host ↔ device。

| offset | size | parameter | range |
|--------|------|-----------|-------|
| 0 | 1 | Global MIDI Ch | 0~15 (= 1~16) |
| 1 | 1 | Controller Mode | 0~11 (Logic/Cubase/Live/...) |
| 2 | 1 | Reserved | - |
| 3 | 1 | Velocity Curve | 0~20 (-10 ~ +10) |
| 4 | 1 | Aftertouch Mode | 0~3 (Off/Channel/Polyphonic/MPE) |
| 5 | 1 | Aftertouch Curve | 0~20 |
| 6 | 1 | Aftertouch Threshold | 0~127 |
| 7 | 1 | Aftertouch Max | 0~127 |
| 8 | 1 | Pedal 1 Assign Type | 0~1 (Damper/Expression) |
| 9 | 1 | Pedal 1 Assign Mode | 0~4 (Damper/Expression/CC/ProgramUp/ProgramDown) |
| 10 | 1 | Pedal 1 CC Number | 0~95, 102~119 |
| 11 | 1 | Pedal 1 Curve | 0~20 |
| 12~15 | 4 | Pedal 2 (同上 4 fields) | 同上 |
| 16 | 1 | Contrast | 1~10 |
| 17 | 1 | Auto Power Off | 0~4 (Disabled/1h/2h/3h/4h) |
| 18 | 1 | Auto Screen Off | 0~8 (Disabled/1m/5m/15m/30m/1h/2h/3h/4h) |
| 19~24 | 6 | User Chord Set 1 Name | null-terminated |
| 25~210 | 186 | User Chord Set 2~32 Name (各 6 byte) | 同上 |
| 211~317 | 107 | User Chord Set 1 (Key 1~12、 各 9 byte: Size + 8 Note Numbers) | 0~8 / 0~127 |
| 318~3666 | 3349 | User Chord Set 2~32 (各 107 byte) | 同上 |

**Total: 3667 byte。**

### Controller Mode List

`00:Assignable / 01:Logic / 02:GarageBand / 04:Ableton Live / 05:FL Studio / 06:Cubase / 07:Studio One / 09:Digital Performer / 10:Pro Tools / 11:Cakewalk`

(03 と 08 は spec 上欠番)

## 7-bit Encoding

KORG SysEx は **8-bit data → 7-bit MIDI data** に encode して運ぶ。 MIDI message の data byte は MSB が 0 でなければならない (7-bit constraint) ため。

### Encoding (8 → 7)

```
input:  7 byte (8-bit each)
output: 1 high-bit byte + 7 data byte (7-bit each)

high-bit byte:
  bit 0 = data[0] の MSB
  bit 1 = data[1] の MSB
  ...
  bit 6 = data[6] の MSB
  bit 7 = 0 (未使用)

data byte:
  各 byte は元 data の下位 7 bit
```

### Wire format

```
Data (1set = 8bit × 7Byte):
  b7~b0  b7~b0  b7~b0  b7~b0  b7~b0  b7~b0  b7~b0
  [7n+0] [7n+1] [7n+2] [7n+3] [7n+4] [7n+5] [7n+6]

MIDI Data (1set = 7bit × 8Byte):
  b7b7b7b7b7b7b7   b6~b0   b6~b0   ...   b6~b0
  [hi-bits]        [7n+0]  [7n+1]  ...   [7n+6]
```

### Encoded sizes

| Data | Encoded |
|------|---------|
| Scene Data (500 byte) | 500 = 7×71+3 → 8×71+(3+1) = **572 byte** |
| Global Data (3667 byte) | 3667 = 7×523+6 → 8×523+(6+1) = **4191 byte** |

### Reference implementation

Swift: `~/repos/cplp-sound-system/apple/CplpSoundSystem/Sources/macOS/Midi/KeystageManager.swift::encode7bitData / decode7bitData / encode7bit3` (line 199~236)

Rust 移植は `midistage-core::encoding` (v0-α 実装予定、 todo `mem_1CahC1m36XLS6CcbZxCFj4`)。

## MIDI-CI / Property Exchange (Read-Only)

KORG Keystage の MIDI-CI 対応:

| Sub ID #2 | Description | T (Transmitted) | R (Recognized) |
|-----------|-------------|-----------------|----------------|
| `0x70` | Discovery | O | O |
| `0x71` | Reply to Discovery | O | O |
| `0x30` | Inquiry: PE Capabilities | O | O |
| `0x31` | Reply to PE Capabilities | O | O |
| `0x34` | Inquiry: Get Property Data | O | O |
| `0x35` | Reply to Get Property Data | O | O |
| `0x38` | Subscription | O | O |
| `0x39` | Reply to Subscription | O | O |
| `0x7E` | Invalidate MUID | O | O |

**重要: `0x36` Set Property Data / `0x37` Reply は Recognized 行に無い** = **Set 不可**。

### PE で取得可能な Property

| Resource | Direction | 用途 |
|----------|-----------|------|
| `ResourceList` | T,R | Resource 一覧の自己紹介 |
| `DeviceInfo` | T,R | manufacturer / family / model / version |
| `ChannelList` | T,R subscribable | Global MIDI Ch (変更通知) |
| `ProgramList` | -,R | (Keystage は transmit せず) |
| `X-ParameterList` | -,R | KORG 独自の "performance parameter 一覧 + CC mapping 宣言" |
| `X-ProgramEdit` | -,R | KORG 独自の "現在 parameter values + display value" |

### 重要な注意

X-ParameterList / X-ProgramEdit は KORG が想定している use case = **「鍵盤 Controller が DAW 側 synth に対して "私が触る CC は 24/25/26... でこういう parameter 群です" を宣言する」** path。 つまり **device の internal mapping を書き換える protocol ではない**。

midistage の v0 では PE は **identify 用途** (DeviceInfo / X-ParameterList を読んで device 側 default mapping を確認) として補助的に使い、 internal setting の write は KORG SysEx (Func=0x40 / 0x51) を使う。

## Wire format example: Scene Pull

### Request (host → device)

```
F0  42  40  00 01 69  01  01 00 00  10  F7
^   ^   ^   ^^^^^^^^  ^   ^^^^^^^^  ^   ^
|   |   |   |         |   |         |   End
|   |   |   |         |   |         Function ID = 0x10 (Scene Dump Request)
|   |   |   |         |   Length = 1 (= 1 Func + 0 Data, little-endian 21-bit)
|   |   |   |         Member ID = 0x01 (Keystage-49)
|   |   |   Family ID = 0x0169
|   |   Channel byte (4g, g=0)
|   KORG ID = 0x42
SysEx start
```

### Response (device → host)

```
F0  42  40  00 01 69  01  3C 04 00  40  [572 bytes 7-bit encoded scene data]  F7
^                              ^^^^^^^   ^                                     ^
                               |         |                                     End
                               |         Function ID = 0x40 (Scene Dump)
                               Length = 573 (= 1 Func + 572 Data, little-endian 21-bit)
                               (3C 04 00 = 0x0000043C = 1084 ... wait, recompute)
```

(注: length encoding は 21-bit little-endian、 `1084 = 0x43C`、 7-bit-encoded each byte で `3C 08 00` のような形になる。 詳細は KORG docs と実機検証で確定)

## References

- KORG 公式 docs (cplp-sound-system にコピーあり):
  - `Keystage_MIDIimp.txt` (1115 行) — 全 SysEx + Native Mode の SSOT
  - `Keystage_PE_MIDIimp.txt` (312 行) — PE protocol 詳細
  - `Keystage_PE_ResourceList.txt` (156 行) — PE で expose される Resource
- Swift 先行実装: `KeystageManager.swift` (250 行、 Func=0x10/0x40 の reference 実装)
- Rust UMP decoder: `crates/midistage-core/src/ump.rs` (vendored from cplp-midi)

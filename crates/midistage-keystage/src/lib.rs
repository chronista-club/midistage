//! midistage-keystage — KORG Keystage device module
//!
//! Scene Data (TABLE 1、 500 byte) / Global Data (TABLE 2、 3667 byte) の Rust struct と、
//! KDL profile との serde、 binary serde (KORG SysEx 内 7-bit-encoded byte stream) を提供する。
//!
//! ## 参照
//!
//! - KORG 公式 MIDI Implementation: `~/repos/cplp-sound-system/docs/midi/Keystage/Keystage_MIDIimp.txt` (1115 行)
//! - PE Resource List: `~/repos/cplp-sound-system/docs/midi/Keystage/Keystage_PE_ResourceList.txt`
//! - Swift 先行実装: `~/repos/cplp-sound-system/apple/CplpSoundSystem/Sources/macOS/Midi/KeystageManager.swift`
//!
//! ## Scene Data 構造 (500 byte、 TABLE 1)
//!
//! - 0~9: Scene Name (null terminated)
//! - 10~17: User Page (Knob 1~8)
//! - 18~25: Arp User Page (Knob 1~8)
//! - 26~28: Modulation Mapping (ModWheel/Velocity/Aftertouch)
//! - 29~30: PE Enable / Program Number Display
//! - 31~46: Arpeggiator (Mode/Octave/Latch/Rate/Swing/Pattern/Ratchet/Gate/Velocity/Chance)
//! - 47~49: Chord (Set Num / Strum Time / Strum Direction)
//! - 50~52: Keyboard (MIDI Ch / Octave / Transpose)
//! - 53~55: Wheel (MIDI Ch / Lower / Upper)
//! - 56~61: Encoders REW / FF
//! - 62~445: Knob 1~128 (User Page で 16 page × 8 knob)
//! - 446~499: Buttons (Play/Stop/Rec/Loop/Tempo/Metro/Undo/TrackDown/TrackUp、 各 6 byte)

// TODO v0-alpha: pub mod scene;       // Scene Data (TABLE 1、 500 byte) struct + binary serde
// TODO v0-alpha: pub mod kdl;         // KDL serializer / deserializer (knus)
// TODO v0-gamma: pub mod global;      // Global Data (TABLE 2、 3667 byte) struct + binary serde

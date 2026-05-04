//! midistage-core
//!
//! transport (UMP) + protocol (SysEx / MIDI-CI PE) + 7-bit encoding。
//! KORG Keystage 起点だが device-agnostic な abstraction を目指す。
//!
//! ## v0 module 構成
//!
//! - `coremidi_sys` — CoreMIDI FFI binding (RX 完備、 TX は v0-alpha で追加)
//! - `ump` — UMP packet decoder (encoder は v0-alpha で追加)
//! - `sysex` — KORG SysEx builder / parser (v0-alpha 実装予定)
//! - `encoding` — 7-bit encode/decode utilities (v0-alpha 実装予定)
//! - `midi_ci` — MIDI-CI Discovery + Property Exchange (v0-gamma 実装予定)
//!
//! ## vendor 由来
//!
//! `coremidi_sys` と `ump` は cplp-sound-system/crates/cplp-midi (RX 専用 537 行) から
//! vendor in (γ) されている。 各 file 先頭の attribution コメント参照。
//! v1+ で α 正規化 (cplp-sound-system 側を `midistage-core` 依存に切替) を計画。

#[allow(
    non_camel_case_types,
    non_upper_case_globals,
    non_snake_case,
    dead_code,
    unused_unsafe
)]
pub mod coremidi_sys;
pub mod ump;

// TODO v0-alpha: pub mod sysex;     // KORG SysEx (Func=0x10/0x40/0x0E/0x51 等)
// TODO v0-alpha: pub mod encoding;  // 7-bit encode/decode (8-byte block packing)
// TODO v0-gamma: pub mod midi_ci;   // MIDI-CI Discovery + PE GET

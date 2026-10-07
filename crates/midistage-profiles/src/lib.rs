//! midistage-profiles — MIDI コントロールサーフェスのランタイム制御プロトコル（純粋計算）
//!
//! Ladyland / VP / bikeboy 等のための共通 MIDI ライブラリ。
//! 責務は **data と calculations のみ**で、I/O（ポート接続・polling・送出）は含まない —
//! 接続の管理は midistaged runtime の仕事。
//!
//! - [`device_profile`] — 出力方向: アプリ state → 機材 byte 列（LED/LCD projection）
//! - [`device_input`] — 入力方向: 機材 raw byte → 論理 [`device_input::ControlEvent`]
//! - [`roto_palette`] — ROTO-CONTROL の色パレット量子化
//!
//! ## midistage ファミリーでの位置づけ
//!
//! midistage workspace 内の責務:
//! - `midistage-core`（midistage repo）= transport 基盤（CoreMIDI FFI / UMP / KORG SysEx）
//! - `midistage-profiles`（本 crate）= ランタイム制御の device protocol（ROTO / X-Touch / LPD8 mk2）
//!
//! 2026-10-07 に VP workspace から移植。以後、共通 profile の正本は本 crate。
//! ROTO の解読資料は vantage-point/docs/design/20-roto-control-sysex-protocol.md を参照。
//! スコープは MIDI / コントロールサーフェス系に限定する（汎用置き場にしない）。
//! 経緯: vantage-point 本体 crate からの切り出し（2026-07-15 mako 決定、
//! bikeboy design/03-midi-focus-model.md も参照）。

pub mod device_input;
pub mod device_profile;
pub mod roto_palette;

pub mod keystage;

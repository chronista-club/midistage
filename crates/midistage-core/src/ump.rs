//! UMP (Universal MIDI Packet) デコーダ
//!
//! MIDI 2.0 Channel Voice (MT=0x4) と MIDI 1.0 Channel Voice (MT=0x2) をデコードする。
//! CoreMIDI の MIDIEventPacket の words 配列を入力として受け取る。
//!
//! ## Vendored origin
//!
//! Vendored from `cplp-sound-system/crates/cplp-midi/src/ump.rs` (snapshot 2026-05-04)。
//! decoder のみ実装。 encoder と SysEx7 packet (MT=0x3) 構築は v0-alpha で追加する。
//! PitchBend / Aftertouch / Per-Note Controllers 等の未対応 MT も v0-alpha〜beta で順次拡張。

/// デコードされた MIDI イベント
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UmpEvent {
    NoteOn {
        channel: u8,
        note: u8,
        /// 7bit velocity (0-127)
        velocity: u8,
    },
    NoteOff {
        channel: u8,
        note: u8,
    },
    ControlChange {
        channel: u8,
        cc: u8,
        /// 7bit value (0-127)
        value: u8,
    },
}

/// UMP words をデコード
///
/// MIDI 2.0 Channel Voice (MT=0x4) と MIDI 1.0 Channel Voice (MT=0x2) に対応。
/// 対応外のメッセージタイプは None を返す。
pub fn decode(words: &[u32]) -> Option<UmpEvent> {
    let first = *words.first()?;
    let mt = (first >> 28) as u8;

    match mt {
        0x4 => decode_midi2_channel_voice(first, words),
        0x2 => decode_midi1_channel_voice(first),
        _ => None,
    }
}

/// MIDI 2.0 Channel Voice Message (MT=0x4, 2 words)
fn decode_midi2_channel_voice(word0: u32, words: &[u32]) -> Option<UmpEvent> {
    let channel = ((word0 >> 16) & 0xF) as u8;
    let status = ((word0 >> 20) & 0xF) as u8;
    let note = ((word0 >> 8) & 0x7F) as u8;

    match status {
        0x9 => {
            // Note On — velocity は word1 の上位 16bit (u16)
            let vel16 = if words.len() > 1 {
                (words[1] >> 16) as u16
            } else {
                0
            };
            let vel7 = (vel16 >> 9) as u8; // 16bit → 7bit
            let velocity = if vel16 > 0 { vel7.max(1) } else { 0 };

            if velocity == 0 {
                // velocity 0 の NoteOn は NoteOff として扱う
                Some(UmpEvent::NoteOff { channel, note })
            } else {
                Some(UmpEvent::NoteOn {
                    channel,
                    note,
                    velocity,
                })
            }
        }
        0x8 => {
            // Note Off
            Some(UmpEvent::NoteOff { channel, note })
        }
        0xB => {
            // Control Change — value は word1 の上位 32bit → 7bit
            let cc = note; // CC number is in the note field position
            let val32 = if words.len() > 1 { words[1] } else { 0 };
            let val7 = (val32 >> 25) as u8; // 32bit → 7bit
            Some(UmpEvent::ControlChange {
                channel,
                cc,
                value: val7,
            })
        }
        _ => None, // PitchBend, Aftertouch, etc. は将来対応
    }
}

/// MIDI 1.0 Channel Voice Message (MT=0x2, 1 word)
fn decode_midi1_channel_voice(word0: u32) -> Option<UmpEvent> {
    let channel = ((word0 >> 16) & 0xF) as u8;
    let status_byte = ((word0 >> 16) & 0xF0) as u8;
    let data1 = ((word0 >> 8) & 0x7F) as u8;
    let data2 = (word0 & 0x7F) as u8;

    match status_byte {
        0x90 => {
            if data2 > 0 {
                Some(UmpEvent::NoteOn {
                    channel,
                    note: data1,
                    velocity: data2,
                })
            } else {
                Some(UmpEvent::NoteOff {
                    channel,
                    note: data1,
                })
            }
        }
        0x80 => Some(UmpEvent::NoteOff {
            channel,
            note: data1,
        }),
        0xB0 => Some(UmpEvent::ControlChange {
            channel,
            cc: data1,
            value: data2,
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── MIDI 2.0 Channel Voice (MT=0x4) ──

    #[test]
    fn midi2_note_on() {
        // MT=4, status=9, channel=0, note=60 (C4)
        // word0: 0x4090_3C00, word1: velocity=0x8000 (16bit mid) → 7bit = 64
        let words = [0x4090_3C00u32, 0x8000_0000];
        let event = decode(&words).unwrap();
        assert_eq!(
            event,
            UmpEvent::NoteOn {
                channel: 0,
                note: 60,
                velocity: 64
            }
        );
    }

    #[test]
    fn midi2_note_on_full_velocity() {
        // velocity = 0xFFFF → 7bit = 127
        let words = [0x4090_4500u32, 0xFFFF_0000];
        let event = decode(&words).unwrap();
        assert_eq!(
            event,
            UmpEvent::NoteOn {
                channel: 0,
                note: 69,
                velocity: 127
            }
        );
    }

    #[test]
    fn midi2_note_on_velocity_zero_is_note_off() {
        let words = [0x4090_3C00u32, 0x0000_0000];
        let event = decode(&words).unwrap();
        assert_eq!(
            event,
            UmpEvent::NoteOff {
                channel: 0,
                note: 60
            }
        );
    }

    #[test]
    fn midi2_note_off() {
        // MT=4, status=8 (NoteOff), channel=0, note=60
        let words = [0x4080_3C00u32, 0x4000_0000];
        let event = decode(&words).unwrap();
        assert_eq!(
            event,
            UmpEvent::NoteOff {
                channel: 0,
                note: 60
            }
        );
    }

    #[test]
    fn midi2_cc() {
        // MT=4, status=B (CC), channel=0, cc=1 (mod wheel)
        // word1: value=0x7F000000 (32bit) → 7bit = 3 (0x7F >> 25 = 3)
        // Actually 0x7FFFFFFF >> 25 = 63
        let words = [0x40B0_0100u32, 0x7FFF_FFFF];
        let event = decode(&words).unwrap();
        assert_eq!(
            event,
            UmpEvent::ControlChange {
                channel: 0,
                cc: 1,
                value: 63
            }
        );
    }

    #[test]
    fn midi2_channel_1() {
        // channel=1
        let words = [0x4091_3C00u32, 0x8000_0000];
        let event = decode(&words).unwrap();
        assert_eq!(
            event,
            UmpEvent::NoteOn {
                channel: 1,
                note: 60,
                velocity: 64
            }
        );
    }

    // ── MIDI 1.0 Channel Voice (MT=0x2) ──

    #[test]
    fn midi1_note_on() {
        // MT=2, status=0x90, note=60, velocity=100
        // word0: 0x2090_3C64
        let words = [0x2090_3C64u32];
        let event = decode(&words).unwrap();
        assert_eq!(
            event,
            UmpEvent::NoteOn {
                channel: 0,
                note: 60,
                velocity: 100
            }
        );
    }

    #[test]
    fn midi1_note_on_velocity_zero() {
        let words = [0x2090_3C00u32];
        let event = decode(&words).unwrap();
        assert_eq!(
            event,
            UmpEvent::NoteOff {
                channel: 0,
                note: 60
            }
        );
    }

    #[test]
    fn midi1_note_off() {
        let words = [0x2080_3C40u32];
        let event = decode(&words).unwrap();
        assert_eq!(
            event,
            UmpEvent::NoteOff {
                channel: 0,
                note: 60
            }
        );
    }

    #[test]
    fn midi1_cc() {
        // CC#1 (mod wheel) value=127
        let words = [0x20B0_017Fu32];
        let event = decode(&words).unwrap();
        assert_eq!(
            event,
            UmpEvent::ControlChange {
                channel: 0,
                cc: 1,
                value: 127
            }
        );
    }

    // ── Edge cases ──

    #[test]
    fn unknown_message_type_returns_none() {
        // MT=0x5 (Data128) — 未対応
        let words = [0x5000_0000u32];
        assert_eq!(decode(&words), None);
    }

    #[test]
    fn empty_words_returns_none() {
        let words: &[u32] = &[];
        assert_eq!(decode(words), None);
    }

    #[test]
    fn midi2_unsupported_status_returns_none() {
        // MT=4, status=0xE (Pitch Bend) — 未対応
        let words = [0x40E0_0000u32, 0x4000_0000];
        assert_eq!(decode(&words), None);
    }

    #[test]
    fn midi1_unsupported_status_returns_none() {
        // 0xC0 (Program Change) — 未対応
        let words = [0x20C0_0500u32];
        assert_eq!(decode(&words), None);
    }

    #[test]
    fn midi2_note_on_minimum_velocity() {
        // velocity = 0x0001 (最小 non-zero) → 7bit = max(0>>9, 1) = 1
        let words = [0x4090_3C00u32, 0x0001_0000];
        let event = decode(&words).unwrap();
        assert_eq!(
            event,
            UmpEvent::NoteOn {
                channel: 0,
                note: 60,
                velocity: 1
            }
        );
    }
}

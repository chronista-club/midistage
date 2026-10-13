//! MIDI トレース（開発用）。既定はオフ。オンのときだけ、MIDI の流れを 1 本の時系列で
//! `~/Library/Logs/midistaged/trace.log.<日付>` に書く（mako 2026-10-13「通常は切って負荷かけないように
//! しつつ、開発でちゃんとどこまで接続がきてるかを確認できるように」）。
//!
//! - オン／オフは `<state_dir>/trace` の有無（再起動なしで切り替わる。メニューバーの app が置く・消す）。
//!   環境変数 `MIDISTAGED_TRACE=1` でも強制的にオン
//! - オフのときの負荷は 1 パケットあたり atomic の読み 1 回
//! - 記録点: 機材→app（転送 / 拒否）、app→サービス（受理 / 溢れ）、サービス→機材（送信 / 古い lease で破棄 / 失敗）、
//!   仮想ポートの開閉（lease の切り替え）

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

/// trace の記録先（tracing の target）。通常のログとはファイルを分ける
pub const TARGET: &str = "midistage_trace";

static ENABLED: AtomicBool = AtomicBool::new(false);
/// クロック（F8）と Active Sensing（FE）も記録するか。既定は省く（1 秒に数十行で埋もれるため）
static REALTIME: AtomicBool = AtomicBool::new(false);

#[inline]
pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// 記録するパケットか。realtime（F8 / FE）だけのパケットは、`realtime` 指定が無ければ省く（純粋 + フラグ）
pub fn keep(bytes: &[u8]) -> bool {
    REALTIME.load(Ordering::Relaxed) || !is_noise(bytes)
}

/// クロックと Active Sensing だけでできているか（純粋）
pub fn is_noise(bytes: &[u8]) -> bool {
    !bytes.is_empty() && bytes.iter().all(|b| *b == 0xf8 || *b == 0xfe)
}

/// フラグファイルと環境変数から状態を決め、変わったときだけ true を返す。
/// フラグファイルに `realtime` と書けば、クロックと Active Sensing も記録する
pub fn refresh(state_dir: &Path) -> bool {
    let flag = state_dir.join("trace");
    let want = std::env::var_os("MIDISTAGED_TRACE").is_some_and(|v| v == "1") || flag.exists();
    let realtime = std::fs::read_to_string(&flag).is_ok_and(|s| s.contains("realtime"));
    REALTIME.store(realtime, Ordering::Relaxed);
    let was = ENABLED.swap(want, Ordering::Relaxed);
    if was != want {
        tracing::info!("MIDI trace {}", if want { "on" } else { "off" });
        tracing::info!(target: TARGET, "---- MIDI trace {} ----", if want { "on" } else { "off" });
    }
    was != want
}

/// MIDI のバイト列を 1 行に。長い SysEx は頭と尾だけ残して長さを添える（純粋）
pub fn format_bytes(bytes: &[u8]) -> String {
    const HEAD: usize = 24;
    const TAIL: usize = 4;
    let hex = |b: &[u8]| {
        b.iter()
            .map(|x| format!("{x:02X}"))
            .collect::<Vec<_>>()
            .join(" ")
    };
    if bytes.len() <= HEAD + TAIL {
        hex(bytes)
    } else {
        format!(
            "{} … {} ({} bytes)",
            hex(&bytes[..HEAD]),
            hex(&bytes[bytes.len() - TAIL..]),
            bytes.len()
        )
    }
}

/// メッセージの種類を短く（人が流し読みするため。純粋）
pub fn describe(bytes: &[u8]) -> &'static str {
    match bytes.first() {
        Some(0xf0) => "sysex",
        Some(0xf8) => "clock",
        Some(0xfa) => "start",
        Some(0xfb) => "continue",
        Some(0xfc) => "stop",
        Some(0xfe) => "active-sensing",
        Some(s) => match s & 0xf0 {
            0x80 => "note-off",
            0x90 if bytes.get(2) == Some(&0) => "note-off",
            0x90 => "note-on",
            0xa0 => "poly-pressure",
            0xb0 => "cc",
            0xc0 => "program",
            0xd0 => "pressure",
            0xe0 => "pitch-bend",
            _ => "other",
        },
        None => "empty",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_messages_are_shown_whole() {
        assert_eq!(format_bytes(&[0xb0, 16, 127]), "B0 10 7F");
        assert_eq!(format_bytes(&[]), "");
    }

    #[test]
    fn long_sysex_keeps_head_tail_and_length() {
        let mut sysex = vec![0xf0, 0x42, 0x49];
        sysex.extend(std::iter::repeat_n(0x11, 60));
        sysex.push(0xf7);
        let line = format_bytes(&sysex);
        assert!(line.starts_with("F0 42 49 11"));
        assert!(line.ends_with("11 11 11 F7 (64 bytes)"), "{line}");
    }

    #[test]
    fn clock_and_active_sensing_are_noise_but_mixed_packets_are_not() {
        assert!(is_noise(&[0xf8]));
        assert!(is_noise(&[0xf8, 0xfe, 0xf8]));
        assert!(!is_noise(&[0xf8, 0x90, 60, 100]));
        assert!(!is_noise(&[0xb0, 1, 2]));
        assert!(!is_noise(&[]));
    }

    #[test]
    fn kinds_are_named() {
        assert_eq!(describe(&[0x90, 60, 100]), "note-on");
        assert_eq!(describe(&[0x90, 60, 0]), "note-off");
        assert_eq!(describe(&[0xb3, 1, 2]), "cc");
        assert_eq!(describe(&[0xf0, 0x7e, 0xf7]), "sysex");
        assert_eq!(describe(&[0xf8]), "clock");
    }

    #[test]
    fn flag_file_turns_trace_on_and_off_without_restart() {
        let dir = tempfile::tempdir().unwrap();
        // 環境変数での強制オンがこのテストを汚さないように
        if std::env::var_os("MIDISTAGED_TRACE").is_some() {
            return;
        }
        refresh(dir.path());
        assert!(!enabled());
        std::fs::write(dir.path().join("trace"), b"").unwrap();
        assert!(refresh(dir.path()), "変わったら true");
        assert!(enabled());
        assert!(!refresh(dir.path()), "変わらなければ false");
        std::fs::remove_file(dir.path().join("trace")).unwrap();
        refresh(dir.path());
        assert!(!enabled());
    }
}

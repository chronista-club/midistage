//! ログの出し先の組み立て（serve 用）。main.rs から切り出して、書き先と時計を差し替えてテストできるようにする。
//!
//! - stderr と `midistaged.log`: info / warn（`RUST_LOG` で絞れる）。MIDI トレースは混ぜない
//! - `trace.log`: target が `midistage_trace` の行だけ。`RUST_LOG` の影響を受けない
//!   （オン／オフはフラグファイルが決める。`RUST_LOG=warn` でトレースが消えると「オンなのに何も出ない」になる）

use crate::trace;
use tracing_subscriber::filter::{EnvFilter, FilterExt, filter_fn};
use tracing_subscriber::fmt::{MakeWriter, time::FormatTime};
use tracing_subscriber::{Layer, Registry};

pub type BoxedLayer = Box<dyn Layer<Registry> + Send + Sync>;

/// 3 本の layer を作る（純粋。`registry().with(layers(..)).init()` で使う）
pub fn layers<E, M, T, C>(
    directives: &str,
    stderr: E,
    main: M,
    trace_file: T,
    timer: C,
) -> Vec<BoxedLayer>
where
    E: for<'w> MakeWriter<'w> + Send + Sync + 'static,
    M: for<'w> MakeWriter<'w> + Send + Sync + 'static,
    T: for<'w> MakeWriter<'w> + Send + Sync + 'static,
    C: FormatTime + Clone + Send + Sync + 'static,
{
    // EnvFilter は layer ごとに付ける。registry 全体に付けると trace.log まで絞られる
    let usual = || {
        EnvFilter::try_new(directives)
            .unwrap_or_else(|_| EnvFilter::new("info"))
            .and(filter_fn(|m| m.target() != trace::TARGET))
    };
    vec![
        tracing_subscriber::fmt::layer()
            .with_timer(timer.clone())
            .with_writer(stderr)
            .with_filter(usual())
            .boxed(),
        tracing_subscriber::fmt::layer()
            .with_timer(timer.clone())
            .with_ansi(false)
            .with_writer(main)
            .with_filter(usual())
            .boxed(),
        tracing_subscriber::fmt::layer()
            .with_timer(timer)
            .with_ansi(false)
            .with_target(false)
            .with_writer(trace_file)
            .with_filter(filter_fn(|m| m.target() == trace::TARGET))
            .boxed(),
    ]
}

/// app から来た文字列をログに出す前に、制御文字（改行など）を `\n` の形に（行の偽造を防ぐ。純粋）
pub fn printable(s: &str) -> String {
    s.chars()
        .flat_map(|c| {
            let escaped: Vec<char> = if c.is_control() {
                c.escape_default().collect()
            } else {
                vec![c]
            };
            escaped
        })
        .collect()
}

#[cfg(test)]
pub(crate) mod testing {
    use std::io::Write;
    use std::sync::{Arc, Mutex};
    use tracing_subscriber::fmt::format::Writer;
    use tracing_subscriber::fmt::time::FormatTime;

    /// メモリに書く MakeWriter
    #[derive(Clone, Default)]
    pub struct Buffer(Arc<Mutex<Vec<u8>>>);
    impl Buffer {
        pub fn text(&self) -> String {
            String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
        }
    }
    impl Write for Buffer {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    impl<'w> tracing_subscriber::fmt::MakeWriter<'w> for Buffer {
        type Writer = Buffer;
        fn make_writer(&'w self) -> Buffer {
            self.clone()
        }
    }

    /// 固定の時刻（fixture を毎回同じにするため）
    #[derive(Clone)]
    pub struct FixedTime;
    impl FormatTime for FixedTime {
        fn format_time(&self, w: &mut Writer<'_>) -> std::fmt::Result {
            write!(w, "2026-10-14T01:02:03.456789Z")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testing::{Buffer, FixedTime};
    use super::*;
    use tracing_subscriber::layer::SubscriberExt;

    fn capture(directives: &str, emit: impl FnOnce()) -> (String, String, String) {
        let (stderr, main, trace_file) = (Buffer::default(), Buffer::default(), Buffer::default());
        let subscriber = Registry::default().with(layers(
            directives,
            stderr.clone(),
            main.clone(),
            trace_file.clone(),
            FixedTime,
        ));
        tracing::subscriber::with_default(subscriber, emit);
        (stderr.text(), main.text(), trace_file.text())
    }

    #[test]
    fn rust_log_narrows_the_usual_log_but_not_the_trace() {
        let (stderr, main, trace_file) = capture("warn", || {
            tracing::info!(target: "midistaged::runtime", "keystage: plugged in");
            tracing::warn!(target: "midistaged::service", "request X rejected");
            tracing::info!(target: trace::TARGET, "FWD | 90 3C 64");
        });
        assert!(!main.contains("plugged in"), "{main}");
        assert!(main.contains("request X rejected"), "{main}");
        assert!(stderr.contains("request X rejected") && !stderr.contains("plugged in"));
        assert!(
            trace_file.contains("FWD | 90 3C 64"),
            "trace は RUST_LOG に関係なく残る: {trace_file:?}"
        );
    }

    #[test]
    fn trace_lines_stay_out_of_the_usual_log() {
        let (stderr, main, trace_file) = capture("info", || {
            tracing::info!(target: "midistaged::runtime", "keystage: plugged in");
            tracing::info!(target: trace::TARGET, "FWD | 90 3C 64");
        });
        assert!(main.contains("plugged in") && !main.contains("FWD"));
        assert!(!stderr.contains("FWD"));
        assert!(!trace_file.contains("plugged in"));
    }

    #[test]
    fn broken_directives_fall_back_to_info() {
        let (_, main, _) = capture("=[", || {
            tracing::info!(target: "midistaged::runtime", "still here");
        });
        assert!(main.contains("still here"));
    }

    /// app のログ window（apps/menubar の LogLines.swift）が読む行の形を固定する。
    /// 本物の layer 設定と trace.rs の記録関数で書いた行を fixture と比べ、Swift 側のテストが同じ fixture を読む。
    /// 形を意図して変えたら `UPDATE_FIXTURE=1 cargo test -p midistaged contract` で書き直し、Swift のテストも通す
    #[test]
    fn log_lines_match_the_contract_the_app_reads() {
        let (_, main, trace_file) = capture("info", || {
            tracing::info!(target: "midistaged::runtime", "session connected: vp (Vantage Point) native_midi=true");
            tracing::info!(target: "midistaged::runtime", "lpd8: lease granted to vp (boot:1)");
            tracing::info!(target: "midistaged::native", "lpd8: virtual ports opened for vp (boot:1) | in [\"Midistage/vp/boot:1/11 | LPD8 mk2\"] | out []");
            tracing::warn!(target: "midistaged::service", "request SetEnabled from vp rejected: state changed since confirmation");
            tracing::info!(target: "unison::network::quic", "New QUIC connection from: [::1]:50000");
            trace::marker(true);
            trace::port_line(
                "dev→app",
                "lpd8",
                "vp",
                "LPD8 mk2",
                "FWD",
                &[0xb7, 0x4f, 0x1f],
            );
            trace::port_line(
                "dev→app",
                "keystage",
                "ladyland",
                "Keystage KBD/CTRL",
                "DROP no lease",
                &[0xf8],
            );
            trace::port_line(
                "app→svc",
                "keystage",
                "ladyland",
                "Keystage KBD/CTRL",
                "QUEUE",
                &[0x90, 60, 100],
            );
            let mut sysex = vec![0xf0, 0x42, 0x30];
            sysex.extend(std::iter::repeat_n(0x11, 40));
            sysex.push(0xf7);
            trace::send_line("keystage", "ladyland", "SendMidi", "SENT", &sysex);
            trace::send_line(
                "keystage",
                "ladyland",
                "port",
                "DROP stale lease",
                &[0xb0, 1, 2],
            );
            trace::overflow("keystage", "ladyland");
            trace::bridge_open(
                "lpd8",
                "vp",
                "boot:1",
                &["Midistage/vp/boot:1/11 | LPD8 mk2".into()],
                &["Midistage/vp/boot:1/12 | LPD8 mk2".into()],
            );
            trace::bridge_close("lpd8", "boot:1");
            trace::marker(false);
        });
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../apps/menubar/Tests/MenuModelTests/Fixtures");
        for (name, got) in [
            ("log-contract.midistaged.log", main),
            ("log-contract.trace.log", trace_file),
        ] {
            let path = dir.join(name);
            if std::env::var_os("UPDATE_FIXTURE").is_some() {
                std::fs::create_dir_all(&dir).unwrap();
                std::fs::write(&path, &got).unwrap();
                continue;
            }
            let want = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("{}: {e}（UPDATE_FIXTURE=1 で作る）", path.display()));
            assert_eq!(
                got, want,
                "{name} の形が変わった。app のログ window が読めるか確かめてから fixture を更新する"
            );
        }
    }

    #[test]
    fn control_characters_cannot_forge_a_line() {
        assert_eq!(printable("vp"), "vp");
        assert_eq!(printable("a\nb\r\tc\u{1b}[31m"), "a\\nb\\r\\tc\\u{1b}[31m");
        assert_eq!(printable("ラ→ン"), "ラ→ン");
    }
}

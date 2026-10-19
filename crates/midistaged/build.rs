//! Windows だけ: Windows.Devices.Midi2 の dll / pri を midistaged.exe の隣（target/<profile>/）に写す。
//! 取得は `scripts/winmidi.sh fetch`（.cache/winmidi/）か、`MIDISTAGE_WINMIDI_DIR`。design 07。
//! 無ければ警告だけ出す。そのときの list-ports は「dll が無い」で失敗する（空の成功にしない）。
use std::path::PathBuf;

const FILES: [&str; 2] = ["Windows.Devices.Midi2.dll", "Windows.Devices.Midi2.pri"];

fn main() {
    println!("cargo:rerun-if-env-changed=MIDISTAGE_WINMIDI_DIR");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("manifest dir"));
    let source = std::env::var_os("MIDISTAGE_WINMIDI_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| manifest.join("../../.cache/winmidi"));
    // OUT_DIR = target/<profile>/build/midistaged-<hash>/out
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("out dir"));
    let Some(profile) = out.ancestors().nth(3) else {
        return;
    };
    for name in FILES {
        let from = source.join(name);
        println!("cargo:rerun-if-changed={}", from.display());
        if !from.exists() {
            println!(
                "cargo:warning={} is missing; run scripts/winmidi.sh fetch (design 07)",
                from.display()
            );
            continue;
        }
        if let Err(e) = std::fs::copy(&from, profile.join(name)) {
            println!("cargo:warning=copy {name}: {e}");
        }
    }
}

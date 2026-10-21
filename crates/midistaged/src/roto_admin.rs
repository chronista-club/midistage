//! ROTO-CONTROL の admin port（USB シリアル）を開いて閉じる層。フレーミングは
//! `midistage_profiles::roto_serial`（純粋）、ここは線の世話だけ。
//!
//! Roto の Serial ポートを開くのは midistage だけ（mako 裁定 2026-10-16「midistage側におこうか、ポートも」）。
//!
//! ## 掟
//! - **開いている間、Roto は MIDI の CC を送らない**（全アプリぶん止まる。Ladyland 実測 2026-08-12、
//!   close で即復帰）。開く → 用事 → 閉じる を短く。開きっぱなしにしない — [`with_port`] が閉じる
//! - リクエストは 1 本ずつ
//! - ポートは VID/PID（0x2E8A / 0xF010）で Roto のものだけに絞る。maru など別の RP2040 機材
//!   （同じ VID でありうる）に問い合わせを撃たない。Roto は 2 本出し、若い USB インターフェースが admin、
//!   もう 1 本は文字を流すデバッグ用。**問い合わせるのは admin の 1 本だけ**
//! - Roto が 2 台以上あれば推測しない。`--port` で指定されない限りエラー
//! - 他のプロセスと同じポートに書かない。排他 open（O_EXLOCK + TIOCEXCL）。どちらも相手が守って
//!   いる前提の仕組みで、排他を使わずに先に開いているプロセス（ROTO-SETUP など）は検出できない
//! - **すべて同期（ブロックする）**。async の中から直接呼ばない — `spawn_blocking` か専用 thread で
//! - いまは **読み取りだけ**。書き込みは報告と GO の後

use midistage_profiles::roto_serial as rs;
// 線の世話（port）は macOS だけ。Windows は design 07 の 5 段目
#[cfg(target_os = "macos")]
use midistage_profiles::roto_serial::{Event, Parser};
use serde::Serialize;
#[cfg(target_os = "macos")]
use std::time::{Duration, Instant};

pub const VENDOR: u16 = 0x2E8A;
pub const PRODUCT: u16 = 0xF010;

/// シリアルポート 1 本の素性（IOKit から）
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SerialPort {
    pub path: String,
    pub vendor: Option<u16>,
    pub product: Option<u16>,
    pub interface: Option<u8>,
    /// USB の locationID（どの差し口の機材か。同じ機材のポートは同じ値）
    pub location: Option<u32>,
}

/// Roto の admin port を 1 本に決める（純粋）。Roto が無い・2 台以上ならエラー（推測しない）。
/// 1 台なら、その機材のいちばん若い USB インターフェースのポート（もう 1 本は debug console）
pub fn admin_port(ports: &[SerialPort]) -> Result<String, String> {
    let mut devices: std::collections::BTreeMap<Option<u32>, Vec<&SerialPort>> = Default::default();
    for p in ports
        .iter()
        .filter(|p| p.vendor == Some(VENDOR) && p.product == Some(PRODUCT))
    {
        devices.entry(p.location).or_default().push(p);
    }
    let lowest = |ps: &[&SerialPort]| {
        ps.iter()
            .min_by_key(|p| (p.interface.unwrap_or(u8::MAX), p.path.clone()))
            .map(|p| p.path.clone())
    };
    match devices.len() {
        0 => Err("no ROTO-CONTROL serial port (VID 2E8A / PID F010)".into()),
        1 => devices
            .values()
            .next()
            .and_then(|ps| lowest(ps))
            .ok_or_else(|| "no ROTO-CONTROL serial port".into()),
        n => Err(format!(
            "{n} ROTO-CONTROL devices are connected ({}); choose one with --port",
            devices
                .values()
                .filter_map(|ps| lowest(ps))
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

/// 席を読んだ応答コードの意味（純粋）。FD だけが「未設定」、それ以外の失敗はエラー
#[derive(Debug, PartialEq, Eq)]
pub enum Seat {
    Configured,
    Empty,
    Error(u8),
}
pub fn seat_outcome(code: u8) -> Seat {
    match code {
        rs::RC_OK => Seat::Configured,
        rs::RC_UNCONFIGURED => Seat::Empty,
        other => Seat::Error(other),
    }
}

/// 読み取りの結果（JSON に保存する形）
#[derive(Debug, Serialize)]
pub struct Dump {
    pub port: String,
    pub firmware: rs::FwVersion,
    pub mode: Option<rs::ModeState>,
    pub current_setup: Option<rs::Setup>,
    pub setups: Vec<SetupDump>,
    /// open から close まで（= Roto の MIDI が止まっていた時間）
    pub port_open_ms: u128,
    pub requests: usize,
}
#[derive(Debug, Serialize)]
pub struct SetupDump {
    pub index: u8,
    /// GET SETUP の応答コード（00 = 使用中、FD = 未設定）
    pub code: u8,
    pub name: Option<String>,
    pub knobs: Vec<rs::Knob>,
    pub switches: Vec<rs::Switch>,
    /// 未設定（FD）だった席の番号
    pub empty_knobs: Vec<u8>,
    pub empty_switches: Vec<u8>,
    /// FD 以外で読めなかったもの（一時的な失敗を空き席と取り違えないため）
    pub errors: Vec<String>,
}
impl SetupDump {
    /// 空き席: setup 自体が未設定か、席が全部空で名前が工場出荷のまま（`SETUP <番号+1>`）か空（純粋）
    pub fn is_free(&self) -> bool {
        let factory_name = format!("SETUP {:02}", self.index as u16 + 1);
        let name = self.name.as_deref().unwrap_or("");
        if !self.errors.is_empty() {
            return false;
        }
        self.code == rs::RC_UNCONFIGURED
            || (self.code == rs::RC_OK
                && self.knobs.is_empty()
                && self.switches.is_empty()
                && (name.is_empty() || name == factory_name))
    }
}

/// 書き込みの結果（ログと報告用）
#[derive(Debug, Serialize)]
pub struct Applied {
    pub ops: usize,
    /// 書く前の中身の控え
    pub backup: std::path::PathBuf,
    pub port_open_ms: u128,
    pub requests: usize,
}

#[cfg(target_os = "macos")]
pub use port::{Session, apply_setup, list_serial_ports, read_all, select_setup, with_port};
#[cfg(not(target_os = "macos"))]
pub use unavailable::{apply_setup, select_setup};

/// Serial の線の世話がまだ無い OS（Windows は design 07 の 5 段目）。setup の用事は受け付けたまま
/// 終わらせず、理由を言って失敗する（`configuring` が戻り、`config_error` に出る）
#[cfg(not(target_os = "macos"))]
mod unavailable {
    const REASON: &str = "ROTO serial is not available on this OS yet (design 07)";
    pub fn select_setup(_setup: u8) -> anyhow::Result<()> {
        anyhow::bail!(REASON)
    }
    pub fn apply_setup(
        _setup: u8,
        _desired: &crate::roto_setup::Desired,
        _backup_dir: &std::path::Path,
    ) -> anyhow::Result<super::Applied> {
        anyhow::bail!(REASON)
    }
}

#[cfg(target_os = "macos")]
mod port {
    use super::*;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

    /// 開いている間だけ生きる。drop で閉じる（閉じ忘れの形を作らない）
    pub struct Session {
        fd: OwnedFd,
        pub path: String,
        pub firmware: rs::FwVersion,
        parser: Parser,
        pub requests: usize,
        /// 応答待ちの間に Roto から届いた通知（本体で setup を選んだ など）
        pub notifications: Vec<(u8, u8, Vec<u8>)>,
    }

    impl Session {
        /// 1 往復。RC と、RC が 00 のときのデータを返す
        pub fn transact(&mut self, request: &[u8], expect: usize) -> anyhow::Result<(u8, Vec<u8>)> {
            self.requests += 1;
            self.parser.expect(expect);
            write_all(self.fd.as_raw_fd(), request)?;
            let deadline = Instant::now() + Duration::from_secs(2);
            while Instant::now() < deadline {
                for event in self.parser.feed(&read_available(self.fd.as_raw_fd())) {
                    match event {
                        Event::Reply { code, data } => return Ok((code, data)),
                        Event::Notification { kind, sub, data } => {
                            self.notifications.push((kind, sub, data))
                        }
                    }
                }
                std::thread::sleep(Duration::from_millis(1));
            }
            anyhow::bail!("{}: no reply within 2s", self.path)
        }
    }

    /// 開いて、用事を済ませて、必ず閉じる。`path` を渡さなければ Roto のポートを探す
    pub fn with_port<T>(
        path: Option<&str>,
        body: impl FnOnce(&mut Session) -> anyhow::Result<T>,
    ) -> anyhow::Result<T> {
        let mut session = open(path)?;
        body(&mut session)
        // session はここで drop され、ポートが閉じる
    }

    fn open(path: Option<&str>) -> anyhow::Result<Session> {
        let path = match path {
            Some(p) => p.to_owned(),
            None => admin_port(&list_serial_ports()).map_err(anyhow::Error::msg)?,
        };
        let fd = open_serial(&path)?;
        let mut parser = Parser::default();
        parser.expect(rs::reply_len::FW_VERSION);
        write_all(fd.as_raw_fd(), &rs::get_fw_version())?;
        let firmware = await_version(fd.as_raw_fd(), &mut parser).ok_or_else(|| {
            anyhow::anyhow!("{path}: no reply to GET FW VERSION (is this the admin port?)")
        })?;
        Ok(Session {
            fd,
            path,
            firmware,
            parser,
            requests: 1,
            notifications: Vec::new(),
        })
    }

    fn await_version(fd: i32, parser: &mut Parser) -> Option<rs::FwVersion> {
        let deadline = Instant::now() + Duration::from_millis(500);
        while Instant::now() < deadline {
            for event in parser.feed(&read_available(fd)) {
                if let Event::Reply {
                    code: rs::RC_OK,
                    data,
                } = event
                {
                    return rs::FwVersion::parse(&data);
                }
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        None
    }

    fn open_serial(path: &str) -> anyhow::Result<OwnedFd> {
        let c = std::ffi::CString::new(path)?;
        // O_EXLOCK: 同じく排他で開くプロセス（別の midistaged、roto-dump など）とは、後から来た方が EBUSY
        let raw = unsafe {
            libc::open(
                c.as_ptr(),
                libc::O_RDWR | libc::O_NOCTTY | libc::O_NONBLOCK | libc::O_EXLOCK,
            )
        };
        if raw < 0 {
            let error = std::io::Error::last_os_error();
            if matches!(
                error.raw_os_error(),
                Some(libc::EBUSY) | Some(libc::EWOULDBLOCK)
            ) {
                anyhow::bail!("{path} is in use by another process (EBUSY)");
            }
            anyhow::bail!("{path}: {error}");
        }
        let fd = unsafe { OwnedFd::from_raw_fd(raw) };
        // TIOCEXCL: この後の open（排他を使わないものも）を EBUSY に
        if unsafe { libc::ioctl(raw, libc::TIOCEXCL as libc::c_ulong) } != 0 {
            anyhow::bail!("{path}: TIOCEXCL: {}", std::io::Error::last_os_error());
        }
        let mut tty: libc::termios = unsafe { std::mem::zeroed() };
        if unsafe { libc::tcgetattr(raw, &mut tty) } != 0 {
            anyhow::bail!("tcgetattr: {}", std::io::Error::last_os_error());
        }
        unsafe {
            libc::cfmakeraw(&mut tty);
            libc::cfsetspeed(&mut tty, libc::B115200 as libc::speed_t);
        }
        tty.c_cflag |= libc::CLOCAL | libc::CREAD;
        if unsafe { libc::tcsetattr(raw, libc::TCSANOW, &tty) } != 0 {
            anyhow::bail!("tcsetattr: {}", std::io::Error::last_os_error());
        }
        // 前に残ったバイトを最初の応答と取り違えない
        unsafe { libc::tcflush(raw, libc::TCIOFLUSH) };
        Ok(fd)
    }

    fn write_all(fd: i32, bytes: &[u8]) -> anyhow::Result<()> {
        let mut sent = 0;
        let deadline = Instant::now() + Duration::from_secs(1);
        while sent < bytes.len() {
            let n = unsafe { libc::write(fd, bytes[sent..].as_ptr().cast(), bytes.len() - sent) };
            if n > 0 {
                sent += n as usize;
            } else if Instant::now() > deadline {
                anyhow::bail!("write: {}", std::io::Error::last_os_error());
            } else {
                std::thread::sleep(Duration::from_millis(1));
            }
        }
        Ok(())
    }

    fn read_available(fd: i32) -> Vec<u8> {
        let mut buf = [0u8; 1024];
        let n = unsafe { libc::read(fd, buf.as_mut_ptr().cast(), buf.len()) };
        if n > 0 {
            buf[..n as usize].to_vec()
        } else {
            Vec::new()
        }
    }

    /// 全部読む（読み取りだけ）。FW / MODE / 現在の setup / 64 setup の名前とつまみ・ボタン
    pub fn read_all(path: Option<&str>) -> anyhow::Result<Dump> {
        let started = Instant::now();
        let mut dump = with_port(path, |s| {
            let mode = match s.transact(&rs::get_mode(), rs::reply_len::MODE)? {
                (rs::RC_OK, d) => rs::ModeState::parse(&d),
                _ => None,
            };
            let current_setup = match s.transact(&rs::get_current_setup(), rs::reply_len::SETUP)? {
                (rs::RC_OK, d) => rs::Setup::parse(&d),
                _ => None,
            };
            let mut setups = Vec::new();
            for index in 0..rs::SETUPS {
                let (code, data) = s.transact(&rs::get_setup(index), rs::reply_len::SETUP)?;
                let mut setup = SetupDump {
                    index,
                    code,
                    name: (code == rs::RC_OK)
                        .then(|| rs::Setup::parse(&data).map(|x| x.name))
                        .flatten(),
                    knobs: Vec::new(),
                    switches: Vec::new(),
                    empty_knobs: Vec::new(),
                    empty_switches: Vec::new(),
                    errors: Vec::new(),
                };
                if let Seat::Error(rc) = seat_outcome(code) {
                    setup.errors.push(format!("setup: rc={rc:02X}"));
                }
                if code == rs::RC_OK {
                    for c in 0..rs::CONTROLS {
                        let (code, d) = s.transact(&rs::get_knob(index, c), rs::reply_len::KNOB)?;
                        match seat_outcome(code) {
                            Seat::Configured => match rs::Knob::parse(&d) {
                                Some(k) => setup.knobs.push(k),
                                None => setup.errors.push(format!("knob {c}: short reply")),
                            },
                            Seat::Empty => setup.empty_knobs.push(c),
                            Seat::Error(rc) => setup.errors.push(format!("knob {c}: rc={rc:02X}")),
                        }
                        let (code, d) =
                            s.transact(&rs::get_switch(index, c), rs::reply_len::SWITCH)?;
                        match seat_outcome(code) {
                            Seat::Configured => match rs::Switch::parse(&d) {
                                Some(w) => setup.switches.push(w),
                                None => setup.errors.push(format!("switch {c}: short reply")),
                            },
                            Seat::Empty => setup.empty_switches.push(c),
                            Seat::Error(rc) => {
                                setup.errors.push(format!("switch {c}: rc={rc:02X}"))
                            }
                        }
                    }
                }
                setups.push(setup);
            }
            Ok(Dump {
                port: s.path.clone(),
                firmware: s.firmware.clone(),
                mode,
                current_setup,
                setups,
                port_open_ms: 0,
                requests: s.requests,
            })
        })?;
        // with_port から戻った時点でポートは閉じている
        dump.port_open_ms = started.elapsed().as_millis();
        Ok(dump)
    }

    /// setup 1 つの中身（名前 + つまみ + ボタン）を読む。空き席は飛ばし、FD 以外の失敗はエラー
    fn read_setup(s: &mut Session, index: u8) -> anyhow::Result<crate::roto_setup::Current> {
        let (code, data) = s.transact(&rs::get_setup(index), rs::reply_len::SETUP)?;
        anyhow::ensure!(code == rs::RC_OK, "GET SETUP {index}: rc={code:02X}");
        let name = rs::Setup::parse(&data)
            .map(|x| x.name)
            .ok_or_else(|| anyhow::anyhow!("GET SETUP {index}: short reply"))?;
        let mut current = crate::roto_setup::Current {
            name,
            ..Default::default()
        };
        for c in 0..rs::CONTROLS {
            let (code, d) = s.transact(&rs::get_knob(index, c), rs::reply_len::KNOB)?;
            match seat_outcome(code) {
                Seat::Configured => current.knobs.push(
                    rs::Knob::parse(&d).ok_or_else(|| anyhow::anyhow!("knob {c}: short reply"))?,
                ),
                Seat::Empty => {}
                Seat::Error(rc) => anyhow::bail!("knob {c}: rc={rc:02X}"),
            }
            let (code, d) = s.transact(&rs::get_switch(index, c), rs::reply_len::SWITCH)?;
            match seat_outcome(code) {
                Seat::Configured => current.switches.push(
                    rs::Switch::parse(&d)
                        .ok_or_else(|| anyhow::anyhow!("switch {c}: short reply"))?,
                ),
                Seat::Empty => {}
                Seat::Error(rc) => anyhow::bail!("switch {c}: rc={rc:02X}"),
            }
        }
        Ok(current)
    }

    /// setup の中身を書く（段 C）。開く → 今の中身を読んで控える → 差分だけ START / SET・CLEAR / END →
    /// 読み戻して照合 → 閉じる。差分が無ければ何も書かない
    pub fn apply_setup(
        setup: u8,
        desired: &crate::roto_setup::Desired,
        backup_dir: &std::path::Path,
    ) -> anyhow::Result<Applied> {
        use crate::roto_setup::{diff, encode};
        let started = Instant::now();
        let mut applied = with_port(None, |s| {
            let current = read_setup(s, setup)?;
            std::fs::create_dir_all(backup_dir)?;
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let backup = backup_dir.join(format!("{stamp}-setup-{setup:02}.json"));
            std::fs::write(
                &backup,
                serde_json::to_vec_pretty(&serde_json::json!({
                    "setup": setup, "name": current.name, "knobs": current.knobs, "switches": current.switches,
                    "firmware": s.firmware,
                }))?,
            )?;
            let ops = diff(setup, &current, desired);
            if ops.is_empty() {
                return Ok(Applied {
                    ops: 0,
                    backup,
                    port_open_ms: 0,
                    requests: s.requests,
                });
            }
            let ok = |s: &mut Session, what: &str, msg: &[u8]| -> anyhow::Result<()> {
                match s.transact(msg, 0)? {
                    (rs::RC_OK, _) => Ok(()),
                    (code, _) => anyhow::bail!("ROTO rejected {what} (rc={code:02X})"),
                }
            };
            ok(s, "START CONFIG UPDATE", &rs::start_config_update())?;
            let mut written = Ok(());
            for op in &ops {
                written = ok(s, &format!("{op:?}"), &encode(setup, op));
                if written.is_err() {
                    break;
                }
            }
            // 失敗しても END は送る（本体を書き込みセッションのまま残さない）
            let ended = ok(s, "END CONFIG UPDATE", &rs::end_config_update());
            written?;
            ended?;
            let after = read_setup(s, setup)?;
            let remaining = diff(setup, &after, desired);
            anyhow::ensure!(
                remaining.is_empty(),
                "read-back differs after write: {} seat(s) ({:?})",
                remaining.len(),
                remaining
                    .iter()
                    .take(3)
                    .map(|o| format!("{o:?}").chars().take(40).collect::<String>())
                    .collect::<Vec<_>>()
            );
            Ok(Applied {
                ops: ops.len(),
                backup,
                port_open_ms: 0,
                requests: s.requests,
            })
        })?;
        applied.port_open_ms = started.elapsed().as_millis();
        Ok(applied)
    }

    /// setup を選ぶ（SET SETUP）。開く → 1 往復 → 閉じる
    pub fn select_setup(setup: u8) -> anyhow::Result<()> {
        with_port(None, |s| match s.transact(&rs::set_setup(setup), 0)? {
            (rs::RC_OK, _) => Ok(()),
            (code, _) => anyhow::bail!("ROTO rejected SET SETUP {setup} (rc={code:02X})"),
        })
    }

    // ---- IOKit: シリアルポートと USB の素性 ----

    use core_foundation::base::{CFType, TCFType};
    use core_foundation::number::CFNumber;
    use core_foundation::string::CFString;
    use core_foundation_sys::base::{CFAllocatorRef, CFTypeRef, kCFAllocatorDefault};
    use core_foundation_sys::dictionary::CFMutableDictionaryRef;
    use core_foundation_sys::string::CFStringRef;

    #[link(name = "IOKit", kind = "framework")]
    unsafe extern "C" {
        fn IOServiceMatching(name: *const libc::c_char) -> CFMutableDictionaryRef;
        fn IOServiceGetMatchingServices(
            main: u32,
            matching: CFMutableDictionaryRef,
            iter: *mut u32,
        ) -> i32;
        fn IOIteratorNext(iter: u32) -> u32;
        fn IOObjectRelease(object: u32) -> i32;
        fn IORegistryEntryCreateCFProperty(
            entry: u32,
            key: CFStringRef,
            alloc: CFAllocatorRef,
            options: u32,
        ) -> CFTypeRef;
        fn IORegistryEntrySearchCFProperty(
            entry: u32,
            plane: *const libc::c_char,
            key: CFStringRef,
            alloc: CFAllocatorRef,
            options: u32,
        ) -> CFTypeRef;
    }
    const ITERATE_RECURSIVELY: u32 = 1;
    const ITERATE_PARENTS: u32 = 2;

    fn property(entry: u32, key: &str, search_parents: bool) -> Option<CFType> {
        let key = CFString::new(key);
        let raw = unsafe {
            if search_parents {
                IORegistryEntrySearchCFProperty(
                    entry,
                    c"IOService".as_ptr(),
                    key.as_concrete_TypeRef(),
                    kCFAllocatorDefault,
                    ITERATE_RECURSIVELY | ITERATE_PARENTS,
                )
            } else {
                IORegistryEntryCreateCFProperty(
                    entry,
                    key.as_concrete_TypeRef(),
                    kCFAllocatorDefault,
                    0,
                )
            }
        };
        (!raw.is_null()).then(|| unsafe { CFType::wrap_under_create_rule(raw) })
    }
    fn number(value: Option<CFType>) -> Option<i64> {
        value?.downcast::<CFNumber>()?.to_i64()
    }

    /// 開かずに列挙する（IOSerialBSDClient の callout device と、親の USB の VID/PID/インターフェース）
    pub fn list_serial_ports() -> Vec<SerialPort> {
        let mut out = Vec::new();
        unsafe {
            let matching = IOServiceMatching(c"IOSerialBSDClient".as_ptr());
            let mut iter = 0;
            if matching.is_null() || IOServiceGetMatchingServices(0, matching, &mut iter) != 0 {
                return out;
            }
            loop {
                let entry = IOIteratorNext(iter);
                if entry == 0 {
                    break;
                }
                if let Some(path) = property(entry, "IOCalloutDevice", false)
                    .and_then(|v| v.downcast::<CFString>())
                    .map(|s| s.to_string())
                {
                    out.push(SerialPort {
                        path,
                        vendor: number(property(entry, "idVendor", true)).map(|v| v as u16),
                        product: number(property(entry, "idProduct", true)).map(|v| v as u16),
                        interface: number(property(entry, "bInterfaceNumber", true))
                            .map(|v| v as u8),
                        location: number(property(entry, "locationID", true)).map(|v| v as u32),
                    });
                }
                IOObjectRelease(entry);
            }
            IOObjectRelease(iter);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn port(path: &str, product: u16, interface: u8, location: u32) -> SerialPort {
        SerialPort {
            path: path.into(),
            vendor: Some(VENDOR),
            product: Some(product),
            interface: Some(interface),
            location: Some(location),
        }
    }

    #[test]
    fn the_admin_port_is_the_lowest_interface_of_the_one_roto() {
        let ports = [
            port("/dev/cu.usbmodem1403", PRODUCT, 3, 0x0140_0000),
            port("/dev/cu.usbmodem1401", PRODUCT, 1, 0x0140_0000),
            // 同じ Raspberry Pi の VID の別機材（maru など）には問い合わせない
            port("/dev/cu.usbmodem2101", 0x000A, 1, 0x0210_0000),
            SerialPort {
                path: "/dev/cu.Bluetooth-Incoming-Port".into(),
                vendor: None,
                product: None,
                interface: None,
                location: None,
            },
        ];
        // debug console（IF3）には問い合わせない — 候補は 1 本だけ
        assert_eq!(admin_port(&ports).unwrap(), "/dev/cu.usbmodem1401");
    }

    #[test]
    fn no_roto_or_two_rotos_is_an_error_not_a_guess() {
        assert!(admin_port(&[]).is_err());
        let two = [
            port("/dev/cu.usbmodem1401", PRODUCT, 1, 0x0140_0000),
            port("/dev/cu.usbmodem1403", PRODUCT, 3, 0x0140_0000),
            port("/dev/cu.usbmodem2201", PRODUCT, 1, 0x0220_0000),
        ];
        let error = admin_port(&two).unwrap_err();
        assert!(error.contains("2 ROTO-CONTROL"), "{error}");
        assert!(
            error.contains("--port"),
            "どう指定すればよいかを言う: {error}"
        );
    }

    #[test]
    fn only_unconfigured_counts_as_empty_other_codes_are_errors() {
        assert_eq!(
            seat_outcome(midistage_profiles::roto_serial::RC_UNCONFIGURED),
            Seat::Empty
        );
        assert_eq!(seat_outcome(0x01), Seat::Error(0x01));
        assert_eq!(seat_outcome(0x00), Seat::Configured);
    }

    #[test]
    fn a_setup_is_free_when_unconfigured_or_blank() {
        let blank = |code, name: Option<&str>| SetupDump {
            index: 0,
            code,
            name: name.map(Into::into),
            knobs: vec![],
            switches: vec![],
            empty_knobs: (0..32).collect(),
            empty_switches: (0..32).collect(),
            errors: vec![],
        };
        // 読み取りの失敗があれば、空き席とはみなさない（席選びに使うと危ない）
        let mut failed = blank(0, Some("SETUP 01"));
        failed.errors.push("knob 3: rc=01".into());
        assert!(!failed.is_free());
        assert!(
            !blank(0x01, None).is_free(),
            "FD 以外の失敗は空き席ではない"
        );
        assert!(blank(midistage_profiles::roto_serial::RC_UNCONFIGURED, None).is_free());
        assert!(blank(0, Some("")).is_free());
        assert!(!blank(0, Some("VP")).is_free(), "名前を付けた枠は使用中");
        // 工場出荷のままの枠は「SETUP <番号+1>」という名前で、席が全部空（実機 2026-10-16）
        let mut factory = blank(0, Some("SETUP 05"));
        factory.index = 4;
        assert!(factory.is_free());
        let mut renamed = blank(0, Some("SETUP 05"));
        renamed.index = 9;
        assert!(
            !renamed.is_free(),
            "別の番号の既定名は、誰かが付けた名前とみなす"
        );
    }

    /// Serial の無い OS では、setup の用事を受け付けたまま終わらせない。理由を言って失敗する
    #[cfg(not(target_os = "macos"))]
    #[test]
    fn setup_jobs_fail_with_a_reason_where_serial_is_not_available() {
        let error = select_setup(2).unwrap_err().to_string();
        assert!(error.contains("design 07"), "{error}");
        let dir = tempfile::tempdir().unwrap();
        let desired = crate::roto_setup::Desired::default();
        let error = apply_setup(2, &desired, dir.path())
            .unwrap_err()
            .to_string();
        assert!(error.contains("design 07"), "{error}");
    }
}

#[cfg(all(test, target_os = "macos"))]
mod macos_tests {
    /// 開かずに列挙するだけ（機材が無くても通る）。`--nocapture` で中身が見える
    #[test]
    fn listing_serial_ports_does_not_open_them() {
        let ports = super::list_serial_ports();
        for p in &ports {
            println!("{p:?}");
        }
        assert!(ports.iter().all(|p| p.path.starts_with("/dev/cu.")));
    }
}

/// 実機があるときだけ（`cargo test -p midistaged --lib roto_admin -- --ignored`）。
/// 開いている間、Roto の MIDI は止まる（1 秒未満）
#[cfg(all(test, target_os = "macos"))]
mod device_tests {
    #[test]
    #[ignore = "ROTO-CONTROL の実機が要る"]
    fn a_second_open_is_refused_while_the_port_is_held() {
        let (firmware, second) = super::with_port(None, |s| {
            let second = super::with_port(Some(&s.path), |_| Ok(()));
            Ok((s.firmware.clone(), second))
        })
        .unwrap();
        println!("firmware {firmware:?}");
        let error = second.expect_err("2 本目は断られる").to_string();
        assert!(error.contains("in use"), "{error}");
    }
}

//! ROTO-CONTROL の Serial API（USB シリアル、115200 8N1）のフレーミング。純粋層（I/O なし）。
//!
//! 正本は Melbourne Instruments 公式の `ROTO-CONTROL_SERIAL_API_v1.3.pdf`
//! （github.com/Melbourne-Instruments/roto_control/docs。PDF はこのリポジトリに入れない）。
//! 移植元は Ladyland の `RotoKit/RotoAdmin.swift`（ROTO-SETUP.app の解剖、2026-08-12 実機検証）。
//!
//! Roto は「MIDI モード + Serial で setup を焼く」で使う（mako 裁定 2026-10-16、
//! creo mem_1Cg4TThd7VWzafHywhSsVG）。ここは設定の **読み取り** だけを持つ。
//! 書き込み（START / END CONFIG UPDATE、SET_*、CLEAR_*、SET MODE、SET SETUP）は、
//! 読み取りの報告のあとで決める。
//!
//! 作法:
//! - コマンド `5A <type> <sub> <長さ:2 BE> <data…>`、応答 `A5 <RC> <data…>`
//! - 応答には長さの欄が無い。長さはリクエストごとに決まっていて、RC が 00 のときだけデータが続く
//! - リクエストは 1 本ずつ（詰めて送ると失う）
//! - Roto は自分からもコマンド（`5A …`。本体で setup を選んだ、LEARN した）を同じ線で送ってくる

/// コマンドの先頭
pub const COMMAND: u8 = 0x5A;
/// 応答の先頭
pub const RESPONSE: u8 = 0xA5;
/// 成功
pub const RC_OK: u8 = 0x00;
/// 未設定の枠を読んだ（エラーではない。空き席の印）
pub const RC_UNCONFIGURED: u8 = 0xFD;

/// 名前の欄の長さ（NULL 終端の ASCII、00 で埋める）
pub const NAME_LEN: usize = 13;
/// setup の数（00〜3F）
pub const SETUPS: u8 = 64;
/// 1 つの setup の中のつまみ・ボタンの数（それぞれ 00〜1F）
pub const CONTROLS: u8 = 32;
/// クリックの段ごとの名前の数
pub const STEP_NAMES: usize = 16;

const GENERAL: u8 = 0x01;
const MIDI: u8 = 0x02;

/// 応答のデータ長（RC を除く）。リクエストの種類で決まる
pub mod reply_len {
    use super::{NAME_LEN, STEP_NAMES};
    /// major / minor / patch + commit 7 文字
    pub const FW_VERSION: usize = 10;
    /// mode + page
    pub const MODE: usize = 2;
    /// setup index + 名前
    pub const SETUP: usize = 1 + NAME_LEN;
    /// `SI CI CM CC CP NA:2 MN:2 MX:2 CN:13 CS HM IP1 IP2 HS SN:16×13`
    pub const KNOB: usize = 29 + STEP_NAMES * NAME_LEN;
    /// `SI CI CM CC CP NA:2 MN:2 MX:2 CN:13 CS LN LF HM HS SN:16×13`
    pub const SWITCH: usize = 29 + STEP_NAMES * NAME_LEN;
}

// ---- リクエスト（読み取りだけ） ---------------------------------------------

fn request(kind: u8, sub: u8, data: &[u8]) -> Vec<u8> {
    let len = data.len() as u16;
    let mut out = vec![COMMAND, kind, sub, (len >> 8) as u8, len as u8];
    out.extend_from_slice(data);
    out
}

/// GET FW VERSION（01 01）
pub fn get_fw_version() -> Vec<u8> {
    request(GENERAL, 0x01, &[])
}
/// GET MODE（01 02）
pub fn get_mode() -> Vec<u8> {
    request(GENERAL, 0x02, &[])
}
/// GET CURRENT SETUP（02 01）
pub fn get_current_setup() -> Vec<u8> {
    request(MIDI, 0x01, &[])
}
/// GET SETUP（02 02）
pub fn get_setup(setup: u8) -> Vec<u8> {
    request(MIDI, 0x02, &[setup])
}
/// GET KNOB CONTROL CONFIG（02 05）
pub fn get_knob(setup: u8, control: u8) -> Vec<u8> {
    request(MIDI, 0x05, &[setup, control])
}
/// GET SWITCH CONTROL CONFIG（02 06）
pub fn get_switch(setup: u8, control: u8) -> Vec<u8> {
    request(MIDI, 0x06, &[setup, control])
}

// ---- 切り替え ----------------------------------------------------------------

/// SET SETUP（02 03）: 表に出す setup を選ぶ（本体の SEL と同じ。応答は RC だけ）。
/// 保存領域は書き換えない — 書き込み（SET_* / CLEAR_*）とは別物（design 06）
pub fn set_setup(setup: u8) -> Vec<u8> {
    request(MIDI, 0x03, &[setup])
}

// ---- 応答の読み取り ----------------------------------------------------------

/// 13 バイトの名前欄 → 文字列（NULL まで。ASCII 以外は置き換え）
fn name(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}
fn be16(bytes: &[u8]) -> u16 {
    ((bytes[0] as u16) << 8) | bytes[1] as u16
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct FwVersion {
    pub major: u8,
    pub minor: u8,
    pub patch: u8,
    pub commit: String,
}
impl FwVersion {
    pub fn parse(data: &[u8]) -> Option<Self> {
        (data.len() >= reply_len::FW_VERSION).then(|| Self {
            major: data[0],
            minor: data[1],
            patch: data[2],
            commit: name(&data[3..10]),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Midi,
    Plugin,
    Mix,
    Unknown(u8),
}
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct ModeState {
    pub mode: Mode,
    /// 8 の倍数（00 = 1 ページ目、08 = 2 ページ目 …）
    pub page_index: u8,
}
impl ModeState {
    pub fn parse(data: &[u8]) -> Option<Self> {
        (data.len() >= reply_len::MODE).then(|| Self {
            mode: match data[0] {
                0 => Mode::Midi,
                1 => Mode::Plugin,
                2 => Mode::Mix,
                other => Mode::Unknown(other),
            },
            page_index: data[1],
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Setup {
    pub index: u8,
    pub name: String,
}
impl Setup {
    pub fn parse(data: &[u8]) -> Option<Self> {
        (data.len() >= reply_len::SETUP).then(|| Self {
            index: data[0],
            name: name(&data[1..1 + NAME_LEN]),
        })
    }
}

/// 送る MIDI の種類（つまみは 00〜03、ボタンは 04 / 05 も）
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlMode {
    Cc7,
    Cc14,
    Nrpn7,
    Nrpn14,
    ProgramChange,
    Note,
    Unknown(u8),
}
impl ControlMode {
    fn from_byte(b: u8) -> Self {
        match b {
            0 => Self::Cc7,
            1 => Self::Cc14,
            2 => Self::Nrpn7,
            3 => Self::Nrpn14,
            4 => Self::ProgramChange,
            5 => Self::Note,
            other => Self::Unknown(other),
        }
    }
}

/// つまみ・ボタンに共通の前半
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Common {
    pub setup: u8,
    pub control: u8,
    pub mode: ControlMode,
    /// 01〜10（1 始まりのまま。公式仕様の表記どおり）
    pub channel: u8,
    /// CC 番号 / Program 番号 / Note。NRPN では FF
    pub param: u8,
    pub nrpn_address: u16,
    pub min: u16,
    pub max: u16,
    pub name: String,
    /// 83 色パレットの番号（roto_palette）
    pub color: u8,
}
fn common(data: &[u8]) -> Common {
    Common {
        setup: data[0],
        control: data[1],
        mode: ControlMode::from_byte(data[2]),
        channel: data[3],
        param: data[4],
        nrpn_address: be16(&data[5..7]),
        min: be16(&data[7..9]),
        max: be16(&data[9..11]),
        name: name(&data[11..11 + NAME_LEN]),
        color: data[24],
    }
}
/// 段ごとの名前（使っている段の数だけ。空の名前は除かない — 段の位置を保つ）
fn step_names(data: &[u8], steps: u8) -> Vec<String> {
    let base = 29;
    (0..(steps as usize).min(STEP_NAMES))
        .map(|i| name(&data[base + i * NAME_LEN..base + (i + 1) * NAME_LEN]))
        .collect()
}

/// つまみの手応え
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum KnobHaptics {
    /// 300 度のなめらかな回転。任意のクリック位置を 2 つまで（00〜7F）
    Smooth {
        indents: Vec<u8>,
    },
    /// 2〜16 段のクリック。段ごとに名前
    Steps {
        names: Vec<String>,
    },
    /// 300 度 + 中央のクリック
    CentreIndent,
    Unknown {
        mode: u8,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Knob {
    #[serde(flatten)]
    pub common: Common,
    pub haptics: KnobHaptics,
}
impl Knob {
    pub fn parse(data: &[u8]) -> Option<Self> {
        if data.len() < reply_len::KNOB {
            return None;
        }
        let haptics = match data[25] {
            0 => KnobHaptics::Smooth {
                indents: [data[26], data[27]]
                    .into_iter()
                    .filter(|p| *p != 0xFF)
                    .collect(),
            },
            1 => KnobHaptics::Steps {
                names: step_names(data, data[28]),
            },
            2 => KnobHaptics::CentreIndent,
            mode => KnobHaptics::Unknown { mode },
        };
        Some(Self {
            common: common(data),
            haptics,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SwitchAction {
    /// 押している間だけ ON
    Push,
    /// 押すたびに ON / OFF
    Toggle,
    Unknown(u8),
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Switch {
    #[serde(flatten)]
    pub common: Common,
    pub led_on: u8,
    pub led_off: u8,
    pub action: SwitchAction,
    /// 段の名前（段を使わないボタンは空）
    pub step_names: Vec<String>,
}
impl Switch {
    pub fn parse(data: &[u8]) -> Option<Self> {
        if data.len() < reply_len::SWITCH {
            return None;
        }
        let steps = data[28];
        Some(Self {
            common: common(data),
            led_on: data[25],
            led_off: data[26],
            action: match data[27] {
                0 => SwitchAction::Push,
                1 => SwitchAction::Toggle,
                other => SwitchAction::Unknown(other),
            },
            step_names: if steps >= 2 {
                step_names(data, steps)
            } else {
                Vec::new()
            },
        })
    }
}

// ---- 受信の切り出し ----------------------------------------------------------

/// 受信して切り出したもの。自分の要求への応答と、Roto が自分から送るコマンドが同じ線に混ざる
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Reply {
        code: u8,
        data: Vec<u8>,
    },
    /// Roto から（本体で setup を選んだ `02 03`、LEARN した `02 0B` など）
    Notification {
        kind: u8,
        sub: u8,
        data: Vec<u8>,
    },
}

/// 分かれて届いても、まとめて届いても切り出せる。
/// 応答の長さはリクエストしか知らないので、送るたびに [`Parser::expect`] を呼ぶ
#[derive(Default)]
pub struct Parser {
    buffer: Vec<u8>,
    expected: usize,
}
impl Parser {
    /// 次の応答のデータ長（RC を除く）を予約する。送信の直前に呼ぶ
    pub fn expect(&mut self, len: usize) {
        self.expected = len;
    }

    pub fn feed(&mut self, bytes: &[u8]) -> Vec<Event> {
        self.buffer.extend_from_slice(bytes);
        let mut events = Vec::new();
        while let Some(event) = self.next() {
            events.push(event);
        }
        events
    }

    fn next(&mut self) -> Option<Event> {
        // 先頭の印以外は読み飛ばす（公式実装と同じ）
        let start = self
            .buffer
            .iter()
            .position(|b| *b == COMMAND || *b == RESPONSE)
            .unwrap_or(self.buffer.len());
        self.buffer.drain(..start);
        match *self.buffer.first()? {
            RESPONSE => {
                let code = *self.buffer.get(1)?;
                let len = if code == RC_OK { self.expected } else { 0 };
                if self.buffer.len() < 2 + len {
                    return None;
                }
                let data = self.buffer[2..2 + len].to_vec();
                self.buffer.drain(..2 + len);
                self.expected = 0;
                Some(Event::Reply { code, data })
            }
            _ => {
                if self.buffer.len() < 5 {
                    return None;
                }
                let len = be16(&self.buffer[3..5]) as usize;
                if self.buffer.len() < 5 + len {
                    return None;
                }
                let event = Event::Notification {
                    kind: self.buffer[1],
                    sub: self.buffer[2],
                    data: self.buffer[5..5 + len].to_vec(),
                };
                self.buffer.drain(..5 + len);
                Some(event)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn padded(s: &str) -> Vec<u8> {
        let mut v = s.as_bytes().to_vec();
        v.resize(NAME_LEN, 0);
        v
    }
    /// つまみ・ボタン共通の前半（setup 3、control 5、CC7、ch1、CC 74、0〜127、"CUTOFF"、色 12）
    fn common_bytes() -> Vec<u8> {
        let mut d = vec![3, 5, 0, 1, 74, 0, 0, 0, 0, 0, 127];
        d.extend(padded("CUTOFF"));
        d.push(12);
        d
    }
    fn knob_bytes(mode: u8, ip1: u8, ip2: u8, steps: u8, names: &[&str]) -> Vec<u8> {
        let mut d = common_bytes();
        d.extend([mode, ip1, ip2, steps]);
        for i in 0..STEP_NAMES {
            d.extend(padded(names.get(i).copied().unwrap_or("")));
        }
        assert_eq!(d.len(), reply_len::KNOB);
        d
    }

    #[test]
    fn read_requests_follow_the_official_framing() {
        assert_eq!(get_fw_version(), [0x5A, 0x01, 0x01, 0x00, 0x00]);
        assert_eq!(get_mode(), [0x5A, 0x01, 0x02, 0x00, 0x00]);
        assert_eq!(get_current_setup(), [0x5A, 0x02, 0x01, 0x00, 0x00]);
        assert_eq!(get_setup(0x3F), [0x5A, 0x02, 0x02, 0x00, 0x01, 0x3F]);
        assert_eq!(get_knob(2, 0x1F), [0x5A, 0x02, 0x05, 0x00, 0x02, 2, 0x1F]);
        assert_eq!(get_switch(2, 7), [0x5A, 0x02, 0x06, 0x00, 0x02, 2, 7]);
    }

    #[test]
    fn selecting_a_setup_is_one_byte_of_data() {
        assert_eq!(set_setup(6), [0x5A, 0x02, 0x03, 0x00, 0x01, 6]);
    }

    #[test]
    fn reply_lengths_match_the_spec() {
        // 29 + 16 × 13（Ladyland の実機読み出しとも一致）
        assert_eq!(reply_len::KNOB, 237);
        assert_eq!(reply_len::SWITCH, 237);
        assert_eq!(reply_len::SETUP, 14);
    }

    #[test]
    fn firmware_version_from_the_real_device() {
        // 実機 2026-10-15（SysEx の 0A 0E と同じ中身）: 3.2.0 dcf8018
        let v = FwVersion::parse(&[3, 2, 0, b'd', b'c', b'f', b'8', b'0', b'1', b'8']).unwrap();
        assert_eq!((v.major, v.minor, v.patch), (3, 2, 0));
        assert_eq!(v.commit, "dcf8018");
        assert!(FwVersion::parse(&[3, 2]).is_none());
    }

    #[test]
    fn mode_and_setup() {
        assert_eq!(
            ModeState::parse(&[0, 8]).unwrap(),
            ModeState {
                mode: Mode::Midi,
                page_index: 8
            }
        );
        assert_eq!(ModeState::parse(&[2, 0]).unwrap().mode, Mode::Mix);
        let mut d = vec![0x3F];
        d.extend(padded("VP"));
        assert_eq!(
            Setup::parse(&d).unwrap(),
            Setup {
                index: 63,
                name: "VP".into()
            }
        );
        // 13 文字ちょうど（終端なし）も読める
        let mut full = vec![1];
        full.extend(b"ABCDEFGHIJKLM");
        assert_eq!(Setup::parse(&full).unwrap().name, "ABCDEFGHIJKLM");
    }

    #[test]
    fn smooth_knob_keeps_only_the_indents_in_use() {
        let k = Knob::parse(&knob_bytes(0, 0x40, 0xFF, 0, &[])).unwrap();
        assert_eq!(k.common.name, "CUTOFF");
        assert_eq!(
            (
                k.common.setup,
                k.common.control,
                k.common.channel,
                k.common.param
            ),
            (3, 5, 1, 74)
        );
        assert_eq!((k.common.min, k.common.max, k.common.color), (0, 127, 12));
        assert_eq!(k.common.mode, ControlMode::Cc7);
        assert_eq!(
            k.haptics,
            KnobHaptics::Smooth {
                indents: vec![0x40]
            }
        );
    }

    #[test]
    fn stepped_knob_names_each_step() {
        let k = Knob::parse(&knob_bytes(1, 0xFF, 0xFF, 3, &["SAW", "SQR", "TRI"])).unwrap();
        assert_eq!(
            k.haptics,
            KnobHaptics::Steps {
                names: vec!["SAW".into(), "SQR".into(), "TRI".into()]
            }
        );
        assert_eq!(
            Knob::parse(&knob_bytes(2, 0xFF, 0xFF, 0, &[]))
                .unwrap()
                .haptics,
            KnobHaptics::CentreIndent
        );
        assert!(Knob::parse(&knob_bytes(0, 0, 0, 0, &[])[..100]).is_none());
    }

    #[test]
    fn switch_reads_led_colours_and_action() {
        let mut d = common_bytes();
        d[2] = 5; // NOTE
        d.extend([14, 70, 1, 0]); // LED on 白 / off 黒、TOGGLE、段なし
        for _ in 0..STEP_NAMES {
            d.extend(padded(""));
        }
        let s = Switch::parse(&d).unwrap();
        assert_eq!(s.common.mode, ControlMode::Note);
        assert_eq!((s.led_on, s.led_off), (14, 70));
        assert_eq!(s.action, SwitchAction::Toggle);
        assert!(s.step_names.is_empty());
    }

    #[test]
    fn parser_waits_for_split_replies_and_skips_noise() {
        let mut p = Parser::default();
        p.expect(reply_len::MODE);
        assert!(
            p.feed(&[0x00, b'x', RESPONSE]).is_empty(),
            "印の前のごみは捨て、続きを待つ"
        );
        assert!(p.feed(&[RC_OK, 0]).is_empty());
        assert_eq!(
            p.feed(&[8]),
            vec![Event::Reply {
                code: RC_OK,
                data: vec![0, 8]
            }]
        );
    }

    #[test]
    fn an_unconfigured_seat_has_no_data() {
        let mut p = Parser::default();
        p.expect(reply_len::KNOB);
        assert_eq!(
            p.feed(&[RESPONSE, RC_UNCONFIGURED]),
            vec![Event::Reply {
                code: RC_UNCONFIGURED,
                data: vec![]
            }]
        );
    }

    #[test]
    fn switch_with_steps_names_each_step() {
        let mut d = common_bytes();
        d.extend([14, 70, 0, 3]); // PUSH、3 段
        for name in ["LOW", "MID", "HIGH"] {
            d.extend(padded(name));
        }
        for _ in 3..STEP_NAMES {
            d.extend(padded(""));
        }
        let s = Switch::parse(&d).unwrap();
        assert_eq!(s.action, SwitchAction::Push);
        assert_eq!(s.step_names, ["LOW", "MID", "HIGH"]);
    }

    #[test]
    fn unknown_haptic_mode_is_kept_as_unknown() {
        assert_eq!(
            Knob::parse(&knob_bytes(7, 0xFF, 0xFF, 0, &[]))
                .unwrap()
                .haptics,
            KnobHaptics::Unknown { mode: 7 }
        );
    }

    #[test]
    fn an_error_code_consumes_only_two_bytes() {
        // FD 以外の失敗（01）でもデータは続かない。次の応答を食わない
        let mut p = Parser::default();
        p.expect(reply_len::KNOB);
        let events = p.feed(&[RESPONSE, 0x01, 0x5A, 0x02, 0x03, 0x00, 0x01, 0x04]);
        assert_eq!(
            events,
            vec![
                Event::Reply {
                    code: 0x01,
                    data: vec![]
                },
                Event::Notification {
                    kind: 0x02,
                    sub: 0x03,
                    data: vec![4]
                },
            ]
        );
    }

    #[test]
    fn a_reply_shorter_than_expected_waits_instead_of_guessing() {
        let mut p = Parser::default();
        p.expect(reply_len::FW_VERSION);
        assert!(
            p.feed(&[RESPONSE, RC_OK, 3, 2, 0]).is_empty(),
            "10 バイト揃うまで待つ"
        );
        assert_eq!(
            p.feed(b"dcf8018"),
            vec![Event::Reply {
                code: RC_OK,
                data: vec![3, 2, 0, b'd', b'c', b'f', b'8', b'0', b'1', b'8']
            }]
        );
    }

    #[test]
    fn a_notification_split_across_reads_is_reassembled() {
        let mut p = Parser::default();
        assert!(p.feed(&[0x5A, 0x02]).is_empty());
        assert!(p.feed(&[0x03, 0x00]).is_empty());
        assert!(p.feed(&[0x01]).is_empty());
        assert_eq!(
            p.feed(&[0x09]),
            vec![Event::Notification {
                kind: 0x02,
                sub: 0x03,
                data: vec![9]
            }]
        );
    }

    #[test]
    fn a_name_with_invalid_utf8_is_still_read() {
        let mut d = vec![0];
        d.extend([b'A', 0xFF, b'B']);
        d.resize(1 + NAME_LEN, 0);
        assert_eq!(Setup::parse(&d).unwrap().name, "A\u{FFFD}B");
    }

    #[test]
    fn notifications_from_the_device_are_separated_from_replies() {
        let mut p = Parser::default();
        p.expect(0);
        // 本体で setup 2 を選んだ通知と、要求への応答が同時に届く
        let events = p.feed(&[0x5A, 0x02, 0x03, 0x00, 0x01, 0x02, RESPONSE, RC_OK]);
        assert_eq!(
            events,
            vec![
                Event::Notification {
                    kind: 0x02,
                    sub: 0x03,
                    data: vec![2]
                },
                Event::Reply {
                    code: RC_OK,
                    data: vec![]
                },
            ]
        );
    }
}

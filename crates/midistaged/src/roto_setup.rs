//! design 06 の段 C: wire の `SetupContent` を Roto の席（`roto_serial::Knob` / `Switch`）に直し、
//! 今の中身との差分だけを書く操作に落とす。純粋（I/O なし）。ポートの世話は `roto_admin`

use midistage_profiles::roto_serial::{
    self as rs, Common, Knob, KnobHaptics, SeatKind, Switch, SwitchAction,
};
use midistage_protocol::wire::{self, SetupContent};

/// 名前欄に入る文字数（13 バイト - NULL）
const NAME_MAX: usize = rs::NAME_LEN - 1;
/// 色（公式仕様 Appendix A: 00–52）
const COLOR_MAX: u8 = 0x52;

/// setup 1 つぶんの、Roto の席の形
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Desired {
    pub name: String,
    pub knobs: Vec<Knob>,
    pub switches: Vec<Switch>,
}

fn check_name(what: &str, name: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        name.len() <= NAME_MAX && name.chars().all(|c| c.is_ascii() && !c.is_ascii_control()),
        "{what}: name must be ASCII and at most {NAME_MAX} characters: {name:?}"
    );
    Ok(())
}
fn mode(m: wire::ControlMode) -> rs::ControlMode {
    match m {
        wire::ControlMode::Cc7 => rs::ControlMode::Cc7,
        wire::ControlMode::Cc14 => rs::ControlMode::Cc14,
        wire::ControlMode::Nrpn7 => rs::ControlMode::Nrpn7,
        wire::ControlMode::Nrpn14 => rs::ControlMode::Nrpn14,
        wire::ControlMode::ProgramChange => rs::ControlMode::ProgramChange,
        wire::ControlMode::Note => rs::ControlMode::Note,
    }
}
fn value_max(m: wire::ControlMode) -> u16 {
    match m {
        wire::ControlMode::Cc14 | wire::ControlMode::Nrpn14 => 16383,
        _ => 127,
    }
}
#[allow(clippy::too_many_arguments)]
fn common(
    what: &str,
    setup: u8,
    control: u8,
    m: wire::ControlMode,
    channel: u8,
    param: u8,
    nrpn_address: u16,
    min: u16,
    max: u16,
    name: &str,
    color: u8,
) -> anyhow::Result<Common> {
    anyhow::ensure!(control < rs::CONTROLS, "{what}: control must be 0-31");
    anyhow::ensure!(
        (1..=16).contains(&channel),
        "{what} {control}: channel must be 1-16"
    );
    anyhow::ensure!(param <= 127, "{what} {control}: param must be 0-127");
    anyhow::ensure!(
        min <= max && max <= value_max(m),
        "{what} {control}: min <= max <= {} for {m:?}",
        value_max(m)
    );
    anyhow::ensure!(
        color <= COLOR_MAX,
        "{what} {control}: color must be 0-{COLOR_MAX}"
    );
    check_name(&format!("{what} {control}"), name)?;
    Ok(Common {
        setup,
        control,
        mode: mode(m),
        channel,
        param,
        nrpn_address,
        min,
        max,
        name: name.into(),
        color,
    })
}

/// wire の中身を検査して Roto の席に直す。壊れていればどこが悪いかを言って断る
pub fn desired(setup: u8, content: &SetupContent) -> anyhow::Result<Desired> {
    check_name("setup", &content.name)?;
    let mut knobs = Vec::new();
    for k in &content.knobs {
        anyhow::ensure!(
            !matches!(
                k.mode,
                wire::ControlMode::ProgramChange | wire::ControlMode::Note
            ),
            "knob {}: knobs send cc or nrpn only",
            k.control
        );
        anyhow::ensure!(
            !knobs.iter().any(|x: &Knob| x.common.control == k.control),
            "knob {}: listed twice",
            k.control
        );
        let haptics = match &k.haptics {
            wire::KnobHaptics::Smooth { indents } => {
                anyhow::ensure!(
                    indents.len() <= 2 && indents.iter().all(|i| *i <= 127),
                    "knob {}: up to 2 indents, each 0-127",
                    k.control
                );
                KnobHaptics::Smooth {
                    indents: indents.clone(),
                }
            }
            wire::KnobHaptics::Steps { names } => {
                anyhow::ensure!(
                    (2..=rs::STEP_NAMES).contains(&names.len()),
                    "knob {}: steps need 2-{} names",
                    k.control,
                    rs::STEP_NAMES
                );
                for n in names {
                    check_name(&format!("knob {} step", k.control), n)?;
                }
                KnobHaptics::Steps {
                    names: names.clone(),
                }
            }
            wire::KnobHaptics::CentreIndent => KnobHaptics::CentreIndent,
        };
        knobs.push(Knob {
            common: common(
                "knob",
                setup,
                k.control,
                k.mode,
                k.channel,
                k.param,
                k.nrpn_address,
                k.min,
                k.max,
                &k.name,
                k.color,
            )?,
            haptics,
        });
    }
    let mut switches = Vec::new();
    for s in &content.switches {
        anyhow::ensure!(
            !switches
                .iter()
                .any(|x: &Switch| x.common.control == s.control),
            "switch {}: listed twice",
            s.control
        );
        anyhow::ensure!(
            s.step_names.is_empty() || (2..=rs::STEP_NAMES).contains(&s.step_names.len()),
            "switch {}: step_names is empty or 2-{} names",
            s.control,
            rs::STEP_NAMES
        );
        for n in &s.step_names {
            check_name(&format!("switch {} step", s.control), n)?;
        }
        anyhow::ensure!(
            s.led_on <= COLOR_MAX && s.led_off <= COLOR_MAX,
            "switch {}: led colours must be 0-{COLOR_MAX}",
            s.control
        );
        switches.push(Switch {
            common: common(
                "switch",
                setup,
                s.control,
                s.mode,
                s.channel,
                s.param,
                s.nrpn_address,
                s.min,
                s.max,
                &s.name,
                s.color,
            )?,
            led_on: s.led_on,
            led_off: s.led_off,
            action: match s.action {
                wire::SwitchAction::Push => SwitchAction::Push,
                wire::SwitchAction::Toggle => SwitchAction::Toggle,
            },
            step_names: s.step_names.clone(),
        });
    }
    Ok(Desired {
        name: content.name.clone(),
        knobs,
        switches,
    })
}

/// いま本体にある setup の中身（読み戻したもの）
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Current {
    pub name: String,
    pub knobs: Vec<Knob>,
    pub switches: Vec<Switch>,
}

/// 書く操作 1 つ（START と END の間に送る）
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Op {
    SetName(String),
    SetKnob(Knob),
    SetSwitch(Switch),
    Clear(SeatKind, u8),
}

/// 今の中身と欲しい中身の差分だけ（同じ席は触らない。欲しい側に無い席は CLEAR）
pub fn diff(setup: u8, current: &Current, desired: &Desired) -> Vec<Op> {
    let mut ops = Vec::new();
    if current.name != desired.name {
        ops.push(Op::SetName(desired.name.clone()));
    }
    for control in 0..rs::CONTROLS {
        let now = current.knobs.iter().find(|k| k.common.control == control);
        match desired.knobs.iter().find(|k| k.common.control == control) {
            Some(want) if now != Some(want) => ops.push(Op::SetKnob(want.clone())),
            None if now.is_some() => ops.push(Op::Clear(SeatKind::Knob, control)),
            _ => {}
        }
        let now = current
            .switches
            .iter()
            .find(|s| s.common.control == control);
        match desired
            .switches
            .iter()
            .find(|s| s.common.control == control)
        {
            Some(want) if now != Some(want) => ops.push(Op::SetSwitch(want.clone())),
            None if now.is_some() => ops.push(Op::Clear(SeatKind::Switch, control)),
            _ => {}
        }
    }
    let _ = setup;
    ops
}

/// 操作をバイト列に（送る順のまま）
pub fn encode(setup: u8, op: &Op) -> Vec<u8> {
    match op {
        Op::SetName(name) => rs::set_setup_name(setup, name),
        Op::SetKnob(k) => rs::set_knob(k),
        Op::SetSwitch(s) => rs::set_switch(s),
        Op::Clear(kind, control) => rs::clear_control(setup, *kind, *control),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use midistage_protocol::wire::{KnobConfig, SwitchConfig};

    fn knob(control: u8, name: &str) -> KnobConfig {
        KnobConfig {
            control,
            mode: wire::ControlMode::Cc7,
            channel: 3,
            param: 20 + control,
            nrpn_address: 0,
            min: 0,
            max: 127,
            name: name.into(),
            color: 12,
            haptics: wire::KnobHaptics::Smooth { indents: vec![] },
        }
    }
    fn switch(control: u8, name: &str) -> SwitchConfig {
        SwitchConfig {
            control,
            mode: wire::ControlMode::Cc7,
            channel: 3,
            param: 27,
            nrpn_address: 0,
            min: 0,
            max: 127,
            name: name.into(),
            color: 2,
            led_on: 2,
            led_off: 70,
            action: wire::SwitchAction::Push,
            step_names: vec![],
        }
    }
    fn content(knobs: Vec<KnobConfig>, switches: Vec<SwitchConfig>) -> SetupContent {
        SetupContent {
            name: "VP".into(),
            knobs,
            switches,
        }
    }

    #[test]
    fn valid_content_becomes_seats() {
        let d = desired(
            6,
            &content(vec![knob(0, "Lane 1")], vec![switch(7, "TALK")]),
        )
        .unwrap();
        assert_eq!(d.name, "VP");
        assert_eq!(d.knobs[0].common.setup, 6);
        assert_eq!(d.knobs[0].common.param, 20);
        assert_eq!(d.switches[0].led_off, 70);
    }

    #[test]
    fn broken_content_is_refused_with_the_reason() {
        let cases: Vec<(&str, SetupContent)> = vec![
            (
                "name",
                SetupContent {
                    name: "thirteen chars".into(),
                    ..Default::default()
                },
            ),
            ("control", content(vec![knob(32, "x")], vec![])),
            (
                "channel",
                content(
                    vec![KnobConfig {
                        channel: 0,
                        ..knob(0, "x")
                    }],
                    vec![],
                ),
            ),
            (
                "max",
                content(
                    vec![KnobConfig {
                        max: 200,
                        ..knob(0, "x")
                    }],
                    vec![],
                ),
            ),
            (
                "color",
                content(
                    vec![KnobConfig {
                        color: 83,
                        ..knob(0, "x")
                    }],
                    vec![],
                ),
            ),
            ("twice", content(vec![knob(0, "a"), knob(0, "b")], vec![])),
            (
                "cc or nrpn",
                content(
                    vec![KnobConfig {
                        mode: wire::ControlMode::Note,
                        ..knob(0, "x")
                    }],
                    vec![],
                ),
            ),
            (
                "indents",
                content(
                    vec![KnobConfig {
                        haptics: wire::KnobHaptics::Smooth {
                            indents: vec![1, 2, 3],
                        },
                        ..knob(0, "x")
                    }],
                    vec![],
                ),
            ),
            (
                "steps",
                content(
                    vec![KnobConfig {
                        haptics: wire::KnobHaptics::Steps {
                            names: vec!["one".into()],
                        },
                        ..knob(0, "x")
                    }],
                    vec![],
                ),
            ),
            ("ASCII", content(vec![knob(0, "ラジオ")], vec![])),
            (
                "led",
                content(
                    vec![],
                    vec![SwitchConfig {
                        led_on: 90,
                        ..switch(7, "x")
                    }],
                ),
            ),
        ];
        for (expect, c) in cases {
            let error = desired(6, &c).unwrap_err().to_string();
            assert!(error.contains(expect), "{expect}: {error}");
        }
    }

    #[test]
    fn diff_writes_only_what_changed_and_clears_what_is_gone() {
        let want = desired(
            6,
            &content(
                vec![knob(0, "Lane 1"), knob(1, "Lane 2")],
                vec![switch(7, "TALK")],
            ),
        )
        .unwrap();
        // 本体: 名前が既定、knob 0 は同じ、knob 1 は名前が違う、knob 5 は余計、switch 7 は無い
        let mut stale = want.knobs[1].clone();
        stale.common.name = "OLD".into();
        let mut extra = want.knobs[0].clone();
        extra.common.control = 5;
        let now = Current {
            name: "SETUP 07".into(),
            knobs: vec![want.knobs[0].clone(), stale, extra],
            switches: vec![],
        };
        let ops = diff(6, &now, &want);
        assert_eq!(
            ops,
            vec![
                Op::SetName("VP".into()),
                Op::SetKnob(want.knobs[1].clone()),
                Op::Clear(SeatKind::Knob, 5),
                Op::SetSwitch(want.switches[0].clone()),
            ]
        );
        // 同じなら何もしない
        let same = Current {
            name: "VP".into(),
            knobs: want.knobs.clone(),
            switches: want.switches.clone(),
        };
        assert!(diff(6, &same, &want).is_empty());
        // knobs = [] は全席 CLEAR（VP の第 1 段）
        let none = desired(6, &content(vec![], vec![switch(7, "TALK")])).unwrap();
        let ops = diff(6, &same, &none);
        assert_eq!(ops.len(), 2);
        assert!(matches!(ops[0], Op::Clear(SeatKind::Knob, 0)));
    }

    #[test]
    fn ops_encode_to_the_serial_commands() {
        let want = desired(6, &content(vec![knob(0, "Lane 1")], vec![])).unwrap();
        assert_eq!(
            encode(6, &Op::SetName("VP".into()))[..3],
            [0x5A, 0x02, 0x04]
        );
        assert_eq!(
            encode(6, &Op::SetKnob(want.knobs[0].clone()))[..3],
            [0x5A, 0x02, 0x07]
        );
        assert_eq!(
            encode(6, &Op::Clear(SeatKind::Switch, 7)),
            [0x5A, 0x02, 0x09, 0, 3, 6, 1, 7]
        );
    }
}

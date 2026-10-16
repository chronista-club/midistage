//! 流れの要所を info ログに出すための差分（mako 2026-10-13「このフローの中で大事なポイントを info log に
//! 出して欲しい。info は、開発中も常時見たいエラーレベル」）。
//!
//! Broker の変更前後を比べて、機材ごとに 1 行にまとめる（純粋）。呼ぶ側は状態を変える処理の直後に
//! `changes` を取り、`tracing::info!` に流す。MIDI のバイト列は扱わない（それは trace.rs）。
use midistage_protocol::{Broker, DeviceState, Phase};

fn phase_name(p: &Phase) -> &'static str {
    match p {
        Phase::Off => "off",
        Phase::Waiting => "waiting",
        Phase::Active => "active",
        Phase::Releasing => "releasing",
        Phase::Error => "error",
    }
}

fn phase_label(d: &DeviceState) -> String {
    match (&d.phase, &d.releasing_to) {
        (Phase::Releasing, Some(to)) => format!("releasing(→{to})"),
        (p, _) => phase_name(p).into(),
    }
}

fn owner(d: &DeviceState) -> &str {
    d.assignment.client_id.as_deref().unwrap_or("none")
}

fn holder(d: &DeviceState, name_of: &dyn Fn(&str) -> Option<String>) -> String {
    match &d.lease {
        Some(l) => name_of(&l.session_id).unwrap_or_else(|| "?".into()),
        None => "none".into(),
    }
}

/// 変わった機材ごとに 1 行。変わっていなければ空。
/// `name_of` は session → app 名。切断済みのセッションも引けるよう、呼ぶ側が覚えておいて渡す
pub fn changes(
    before: &Broker,
    after: &Broker,
    name_of: &dyn Fn(&str) -> Option<String>,
) -> Vec<String> {
    let mut lines = Vec::new();
    for (id, a) in &after.devices {
        let mut parts: Vec<String> = Vec::new();
        let Some(b) = before.devices.get(id) else {
            lines.push(format!(
                "{id}: added (present={}, owner={})",
                a.present,
                owner(a)
            ));
            continue;
        };
        if b.present != a.present {
            parts.push(if a.present { "plugged in" } else { "unplugged" }.into());
        }
        if b.assignment.client_id != a.assignment.client_id {
            parts.push(format!(
                "owner {} → {} (rev {} → {})",
                owner(b),
                owner(a),
                b.assignment.revision,
                a.assignment.revision
            ));
        }
        let (hb, ha) = (holder(b, name_of), holder(a, name_of));
        let token_changed =
            b.lease.as_ref().map(|l| &l.token) != a.lease.as_ref().map(|l| &l.token);
        if token_changed {
            parts.push(match (&b.lease, &a.lease) {
                (None, Some(l)) => format!("lease granted to {ha} ({})", l.token),
                (Some(_), None) => format!("lease released by {hb}"),
                (Some(_), Some(l)) => format!("lease {hb} → {ha} ({})", l.token),
                (None, None) => unreachable!(),
            });
        }
        if b.phase != a.phase || b.releasing_to != a.releasing_to {
            parts.push(format!("phase {} → {}", phase_label(b), phase_label(a)));
        }
        if !parts.is_empty() {
            lines.push(format!("{id}: {}", parts.join(", ")));
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(s: &str) -> Option<String> {
        match s {
            "s-lady" => Some("ladyland".into()),
            "s-vp" => Some("vp".into()),
            _ => None,
        }
    }

    fn broker() -> Broker {
        let mut b = Broker::new("boot");
        b.add_device("keystage", "keystage", true);
        b.add_device("roto", "roto", true);
        b.register_session("s-lady", "ladyland").unwrap();
        b.register_session("s-vp", "vp").unwrap();
        b
    }

    #[test]
    fn no_change_no_lines() {
        let b = broker();
        assert!(changes(&b, &b.clone(), &names).is_empty());
    }

    #[test]
    fn enable_reports_owner_lease_and_phase_in_one_line() {
        let before = broker();
        let mut after = before.clone();
        after.enable("keystage", "s-lady", 0, false).unwrap();
        let lines = changes(&before, &after, &names);
        assert_eq!(lines.len(), 1, "{lines:?}");
        let line = &lines[0];
        assert!(line.starts_with("keystage: "), "{line}");
        assert!(line.contains("owner none → ladyland"), "{line}");
        assert!(line.contains("lease granted to ladyland"), "{line}");
        assert!(line.contains("phase"), "{line}");
    }

    #[test]
    fn takeover_shows_releasing_target_then_handover() {
        let mut start = broker();
        start.enable("roto", "s-lady", 0, false).unwrap();
        let rev = start.devices["roto"].assignment.revision;
        let mut releasing = start.clone();
        releasing.enable("roto", "s-vp", rev, true).unwrap();
        let first = changes(&start, &releasing, &names);
        assert!(first[0].contains("owner ladyland → vp"), "{first:?}");
        assert!(first[0].contains("releasing(→vp)"), "{first:?}");

        let token = releasing.devices["roto"]
            .lease
            .as_ref()
            .unwrap()
            .token
            .clone();
        let mut done = releasing.clone();
        done.quiesced("roto", "s-lady", &token).unwrap();
        done.drained("roto").unwrap();
        let second = changes(&releasing, &done, &names);
        assert!(second[0].contains("lease ladyland → vp"), "{second:?}");
        assert!(
            second[0].contains("phase releasing(→vp) → active"),
            "{second:?}"
        );
    }

    #[test]
    fn unplug_and_session_loss_are_named() {
        let mut before = broker();
        before.enable("keystage", "s-lady", 0, false).unwrap();
        let mut after = before.clone();
        after.set_present("keystage", false).unwrap();
        assert!(changes(&before, &after, &names)[0].contains("unplugged"));

        // 切断は 2 段: まず releasing（lease は残る）、送信が片付いたら lease を手放す。担当は残る
        let mut gone = before.clone();
        gone.disconnect_session("s-lady");
        let first = &changes(&before, &gone, &names)[0];
        assert!(first.contains("phase active → releasing"), "{first}");
        let mut drained = gone.clone();
        drained.drained("keystage").unwrap();
        let second = &changes(&gone, &drained, &names)[0];
        assert!(
            second.contains("lease released by ladyland"),
            "切断後でも名前が引ける: {second}"
        );
        assert!(!second.contains("owner"), "担当は残る: {second}");
    }
}

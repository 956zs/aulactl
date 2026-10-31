//! actuation, deadband, switch, pollrate, travel, calibrate, reset.

use crate::args::{
    mm, mm_to_raw, parse_mm, parse_num, parse_poll_rate, parse_rt, parse_trigger_mode,
    trigger_mode_name, Args,
};
use crate::render::{board, key_list, table};
use crate::{bad_args, put, say, signal, Cli, Outcome};
use aula_core::hidraw::Link;
use aula_core::layout;
use aula_core::perkey::{key_name, PerKey};
use aula_core::travel::{CalibrationEvent, Confirm};
use aula_core::trigger::{
    switch_name, DeadBand, KeyTrigger, TravelLimits, TriggerMode, DEAD_BAND_MAX, SWITCHES,
};
use std::collections::BTreeMap;
use std::ops::ControlFlow;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Columns per 1 u in the keyboard views: room for `1.00r`.
const UNIT_COLS: usize = 6;

/// The selected keys, all keys when no selector is given.
pub fn select(sel: Option<&str>) -> Result<Vec<usize>, String> {
    Ok(layout::select(sel.unwrap_or("all"))?)
}

pub fn trigger_text(l: &TravelLimits, t: &KeyTrigger) -> String {
    match t.mode {
        TriggerMode::Fixed => format!("fixed at {} mm", mm(l, t.actuation)),
        TriggerMode::Rapid => format!(
            "rt from {} mm, sensitivity {} mm",
            mm(l, t.actuation),
            mm(l, t.press)
        ),
        TriggerMode::RapidSplit => format!(
            "rt-split from {} mm, press {} mm, release {} mm",
            mm(l, t.actuation),
            mm(l, t.press),
            mm(l, t.release)
        ),
    }
}

/// Keys grouped by identical trigger settings, largest group first.
pub fn groups(l: &TravelLimits, triggers: &PerKey<KeyTrigger>) -> Vec<(String, Vec<usize>)> {
    let mut by: BTreeMap<KeyTrigger, Vec<usize>> = BTreeMap::new();
    for (idx, t) in triggers.iter() {
        by.entry(t.normalized()).or_default().push(idx);
    }
    let mut v: Vec<(String, Vec<usize>)> = by
        .iter()
        .map(|(t, keys)| (trigger_text(l, t), keys.clone()))
        .collect();
    v.sort_by(|a, b| b.1.len().cmp(&a.1.len()));
    v
}

/// Keyboard view cell: actuation in mm, `r` for rapid trigger, `s` for split rapid trigger.
fn actuation_cell(l: &TravelLimits, t: Option<&KeyTrigger>) -> String {
    match t {
        None => String::new(),
        Some(t) => {
            let mark = match t.mode {
                TriggerMode::Fixed => "",
                TriggerMode::Rapid => "r",
                TriggerMode::RapidSplit => "s",
            };
            format!("{}{mark}", mm(l, t.actuation))
        }
    }
}

pub fn actuation_view(l: &TravelLimits, triggers: &PerKey<KeyTrigger>) -> String {
    let mut out = board(
        UNIT_COLS,
        |k| actuation_cell(l, triggers.get(k.index as usize)),
        |_| None,
    );
    out.push_str(
        "actuation point in mm; r = rapid trigger, s = rapid trigger with separate press/release\n",
    );
    for (text, keys) in groups(l, triggers) {
        out.push_str(&format!("{text}: {}\n", key_list(&keys)));
    }
    out
}

pub fn actuation_table(l: &TravelLimits, triggers: &PerKey<KeyTrigger>) -> String {
    let mut rows = vec![["key", "legend", "mode", "actuation", "press", "release"]
        .map(String::from)
        .to_vec()];
    for (idx, t) in triggers.iter() {
        let label = layout::by_index(idx).map_or("", |k| k.label);
        let (press, release) = match t.mode {
            TriggerMode::Fixed => ("-".to_string(), "-".to_string()),
            _ => (mm(l, t.press), mm(l, t.normalized().release)),
        };
        rows.push(vec![
            key_name(idx),
            label.into(),
            trigger_mode_name(t.mode).into(),
            mm(l, t.actuation),
            press,
            release,
        ]);
    }
    table(&rows)
}

/// New trigger settings for `t` from `actuation set` options (raw units). None keeps a value.
pub fn edit_trigger(
    t: &KeyTrigger,
    point: Option<u16>,
    rt: Option<(u16, Option<u16>)>,
    mode: Option<TriggerMode>,
) -> Result<KeyTrigger, String> {
    let mut n = *t;
    if let Some(p) = point {
        n.actuation = p;
    }
    if let Some((press, release)) = rt {
        n.press = press;
        n.release = release.unwrap_or(press);
        n.mode = if release.is_some() {
            TriggerMode::RapidSplit
        } else {
            TriggerMode::Rapid
        };
    }
    if let Some(m) = mode {
        if m == TriggerMode::Rapid && matches!(rt, Some((p, Some(r))) if p != r) {
            return Err(
                "rt uses one sensitivity for press and release; use --mode rt-split for two".into(),
            );
        }
        n.mode = m;
    }
    Ok(n.normalized())
}

pub fn actuation<L: Link>(c: &mut Cli<L>, rest: &[&str]) -> Result<(), String> {
    let mut a = Args::new(rest);
    let all_rows = a.flag("--table");
    let point = a.opt("--point")?;
    let rt = a.opt("--rt")?;
    let mode = a.opt("--mode")?;
    let pos = a.positional()?;
    let l = c.kb.travel_limits()?;
    match pos[..] {
        ["show"] | ["show", _] => {
            let sel = pos.get(1).copied();
            let triggers = c.kb.read_triggers(&select(sel)?)?;
            let text = if all_rows || sel.is_some() {
                actuation_table(&l, &triggers)
            } else {
                actuation_view(&l, &triggers)
            };
            put(c.out, &text)
        }
        ["set", sel] => {
            if point.is_none() && rt.is_none() && mode.is_none() {
                return Err("give --point MM, --rt PRESS[/RELEASE] and/or --mode".into());
            }
            let point = point.map(|p| mm_to_raw(&l, parse_mm(p)?)).transpose()?;
            let rt = match rt {
                Some(v) => {
                    let (p, r) = parse_rt(v)?;
                    Some((mm_to_raw(&l, p)?, r.map(|r| mm_to_raw(&l, r)).transpose()?))
                }
                None => None,
            };
            let mode = mode.map(parse_trigger_mode).transpose()?;
            let keys = select(Some(sel))?;
            let now = c.kb.read_triggers(&keys)?;
            let mut changes = PerKey::new();
            for (idx, t) in now.iter() {
                let n = edit_trigger(t, point, rt, mode)?;
                if n != t.normalized() {
                    changes.0.insert(idx as u8, n);
                }
            }
            if changes.is_empty() {
                return say(
                    c.out,
                    "nothing to change: the keys already have these settings",
                );
            }
            c.kb.write_triggers(&changes)?;
            c.report_backup()?;
            for (text, keys) in groups(&l, &changes) {
                say(c.out, &format!("set {text}: {}", key_list(&keys)))?;
            }
            Ok(())
        }
        _ => Err(bad_args(&[&["actuation"], rest].concat())),
    }
}

pub fn dead_band<L: Link>(c: &mut Cli<L>, rest: &[&str]) -> Result<(), String> {
    let mut a = Args::new(rest);
    let top = a.opt("--top")?;
    let bottom = a.opt("--bottom")?;
    let pos = a.positional()?;
    let l = c.kb.travel_limits()?;
    let text = |d: &DeadBand| {
        format!(
            "top {} mm, bottom {} mm",
            mm(&l, d.top.into()),
            mm(&l, d.bottom.into())
        )
    };
    match pos[..] {
        ["show"] | ["show", _] => {
            let keys = select(pos.get(1).copied())?;
            let all = c.kb.read_dead_band()?;
            let mut by: BTreeMap<(u8, u8), Vec<usize>> = BTreeMap::new();
            for idx in keys {
                let d = all.get(idx).copied().unwrap_or_default();
                by.entry((d.top, d.bottom)).or_default().push(idx);
            }
            for ((t, b), keys) in by {
                say(
                    c.out,
                    &format!(
                        "{}: {}",
                        text(&DeadBand { top: t, bottom: b }),
                        key_list(&keys)
                    ),
                )?;
            }
            Ok(())
        }
        ["set", sel] => {
            let raw = |v: Option<&str>| -> Result<Option<u8>, String> {
                let Some(v) = v else { return Ok(None) };
                let r = mm_to_raw(&l, parse_mm(v)?)?;
                match u8::try_from(r) {
                    Ok(r) if r <= DEAD_BAND_MAX => Ok(Some(r)),
                    _ => Err(format!(
                        "dead band '{v}' is outside 0..{} mm",
                        mm(&l, DEAD_BAND_MAX.into())
                    )),
                }
            };
            let (top, bottom) = (raw(top)?, raw(bottom)?);
            if top.is_none() && bottom.is_none() {
                return Err("give --top MM and/or --bottom MM".into());
            }
            let keys = select(Some(sel))?;
            let mut map = c.kb.read_dead_band()?;
            let before = map.clone();
            for idx in &keys {
                let d = map.0.entry(*idx as u8).or_default();
                d.top = top.unwrap_or(d.top);
                d.bottom = bottom.unwrap_or(d.bottom);
            }
            if map == before {
                return say(
                    c.out,
                    "nothing to change: the keys already have this dead band",
                );
            }
            c.kb.write_dead_band(&map)?;
            c.report_backup()?;
            let d = map.get(keys[0]).copied().unwrap_or_default();
            say(c.out, &format!("set {}: {}", text(&d), key_list(&keys)))
        }
        _ => Err(bad_args(&[&["deadband"], rest].concat())),
    }
}

/// A switch type by id (`3`) or name (`CY1`, `TC1`, case-insensitive, the part before any bracket).
pub fn parse_switch(v: &str) -> Result<u8, String> {
    if let Ok(id) = v.parse::<u8>() {
        return Ok(id);
    }
    SWITCHES
        .iter()
        .find(|(_, n, _)| {
            n.split(" (")
                .next()
                .is_some_and(|short| short.eq_ignore_ascii_case(v))
        })
        .map(|(id, _, _)| *id)
        .ok_or_else(|| format!("unknown switch type '{v}'; see 'aulactl switch list'"))
}

fn switch_text(id: u8) -> String {
    match switch_name(id) {
        Some(n) => format!("{n} (id {id})"),
        None => format!("id {id}"),
    }
}

pub fn switch<L: Link>(c: &mut Cli<L>, rest: &[&str]) -> Result<(), String> {
    match rest {
        ["show"] | [] => {
            let map = c.kb.read_switch_types()?;
            let mut by: BTreeMap<u8, Vec<usize>> = BTreeMap::new();
            for (idx, id) in map.iter() {
                by.entry(*id).or_default().push(idx);
            }
            for (id, keys) in by {
                say(c.out, &format!("{}: {}", switch_text(id), key_list(&keys)))?;
            }
            Ok(())
        }
        ["list"] => {
            let offered = c.kb.read_switch_list()?;
            let lines: Vec<String> = offered.iter().map(|id| switch_text(*id)).collect();
            say(c.out, &lines.join("\n"))
        }
        ["set", sel, ty] => {
            let id = parse_switch(ty)?;
            let keys = select(Some(sel))?;
            let mut map = c.kb.read_switch_types()?;
            let before = map.clone();
            for idx in &keys {
                map.0.insert(*idx as u8, id);
            }
            if map == before {
                return say(
                    c.out,
                    "nothing to change: the keys already have this switch type",
                );
            }
            c.kb.write_switch_types(&map)?;
            c.report_backup()?;
            say(
                c.out,
                &format!("set {}: {}", switch_text(id), key_list(&keys)),
            )?;
            say(
                c.out,
                "recalibrate the changed keys: aulactl calibrate --yes",
            )
        }
        _ => Err(bad_args(&[&["switch"], rest].concat())),
    }
}

pub fn poll_rate<L: Link>(c: &mut Cli<L>, rest: &[&str]) -> Result<Outcome, String> {
    let now = c.kb.read_poll_rate()?;
    match rest {
        [] => {
            say(c.out, &format!("{now} kHz"))?;
            Ok(Outcome::Done)
        }
        [v] => {
            let khz = parse_poll_rate(v)?;
            if khz == now {
                say(c.out, &format!("the poll rate is already {now} kHz"))?;
                return Ok(Outcome::Done);
            }
            c.kb.set_poll_rate(khz)?;
            c.report_backup()?;
            say(
                c.out,
                &format!("poll rate set to {khz} kHz; the keyboard restarts"),
            )?;
            Ok(Outcome::Restarting(Some(khz)))
        }
        _ => Err(bad_args(&[&["pollrate"], rest].concat())),
    }
}

/// Live view: the board with each key's depth, pressed keys inverted, and a line naming them.
pub fn travel_view(l: &TravelLimits, depth: &BTreeMap<usize, u16>, color: bool) -> String {
    let d = |k: &layout::Key| depth.get(&(k.index as usize)).copied().unwrap_or(0);
    let mut out = board(
        5,
        |k| mm(l, d(k)),
        |k| (color && d(k) > 0).then(|| "7".to_string()),
    );
    let down: Vec<String> = depth
        .iter()
        .filter(|(_, v)| **v > 0)
        .map(|(i, v)| format!("{} {}", key_name(*i), mm(l, *v)))
        .collect();
    out.push_str(&format!(
        "down: {}",
        if down.is_empty() {
            "-".to_string()
        } else {
            down.join("  ")
        }
    ));
    out
}

pub fn travel<L: Link>(c: &mut Cli<L>, rest: &[&str]) -> Result<(), String> {
    let mut a = Args::new(rest);
    let raw = a.flag("--raw") || !c.tty;
    let seconds = a
        .opt("--seconds")?
        .map(|s| parse_num(s, 1..=86_400, "number of seconds"))
        .transpose()?;
    let pos = a.positional()?;
    let sel = match pos[..] {
        [] => None,
        [s] => Some(s),
        _ => return Err(bad_args(&[&["travel"], rest].concat())),
    };
    let keys = select(sel)?;
    let l = c.kb.travel_limits()?;
    let deadline = seconds.map(|s| Instant::now() + Duration::from_secs(s.into()));
    signal::install()?;
    let Cli { kb, out, tty, .. } = c;
    let color = *tty;
    let mut stream = kb.travel_stream(&keys)?;
    let mut depth: BTreeMap<usize, u16> = keys.iter().map(|k| (*k, 0)).collect();
    let mut failed: Option<String> = None;
    let mut drawn = false;
    let mut dirty = true;
    let mut last_draw = Instant::now();
    if !raw {
        failed = put(*out, "live key travel; Ctrl-C stops\n").err();
    }
    let run = stream.run(Duration::from_millis(30), |s| {
        if failed.is_some()
            || signal::interrupted()
            || deadline.is_some_and(|d| Instant::now() >= d)
        {
            return ControlFlow::Break(());
        }
        if let Some(s) = s {
            depth.insert(s.index, s.depth);
            dirty = true;
            if raw {
                let line = if s.at_rest {
                    format!("{} {} rest", key_name(s.index), mm(&l, 0))
                } else {
                    format!("{} {}", key_name(s.index), mm(&l, s.depth))
                };
                if let Err(e) = say(*out, &line) {
                    failed = Some(e);
                }
            }
        }
        if !raw && dirty && last_draw.elapsed() >= Duration::from_millis(30) {
            let up = if drawn { "\x1b[6A\r" } else { "" };
            let view = travel_view(&l, &depth, color).replace('\n', "\x1b[K\n");
            if let Err(e) = put(*out, &format!("{up}{view}\x1b[K\n")) {
                failed = Some(e);
            }
            drawn = true;
            dirty = false;
            last_draw = Instant::now();
        }
        ControlFlow::Continue(())
    });
    let closed = stream.close();
    run?;
    if let Some(e) = failed {
        return Err(e);
    }
    closed.map_err(|e| format!("closing the travel test: {e}"))?;
    Ok(())
}

pub fn calibrate<L: Link>(c: &mut Cli<L>, rest: &[&str]) -> Result<(), String> {
    match rest {
        ["--yes"] => {}
        [] => {
            return Err("calibration replaces every key's stored travel range; run 'aulactl calibrate --yes', then \
                        press each key fully down and release it"
                .into())
        }
        _ => return Err(bad_args(&[&["calibrate"], rest].concat())),
    }
    signal::install()?;
    let enter = Arc::new(AtomicBool::new(false));
    if std::io::IsTerminal::is_terminal(&std::io::stdin()) {
        let flag = Arc::clone(&enter);
        std::thread::spawn(move || {
            let mut line = String::new();
            if std::io::stdin().read_line(&mut line).is_ok_and(|n| n > 0) {
                flag.store(true, Ordering::SeqCst);
            }
        });
    }
    let total = layout::KEYS.len();
    let Cli { kb, out, .. } = c;
    let mut cal = kb.calibrate(Confirm::user_confirmed())?;
    say(
        *out,
        "calibrating: press every key fully down and release it; Enter or Ctrl-C finishes",
    )?;
    let mut done: Vec<usize> = Vec::new();
    // a final status the board sent on its own; it wins over what stop() gets afterwards
    let mut early: Option<CalibrationEvent> = None;
    loop {
        if signal::interrupted() || enter.load(Ordering::SeqCst) || done.len() == total {
            break;
        }
        match cal.next_event(Duration::from_millis(100))? {
            Some(CalibrationEvent::Progress(keys)) => {
                let keys: Vec<usize> = keys.into_iter().filter(|k| layout::is_key(*k)).collect();
                if keys.len() != done.len() {
                    say(*out, &format!("{} of {total} keys calibrated", keys.len()))?;
                }
                done = keys;
            }
            Some(e @ (CalibrationEvent::Finished | CalibrationEvent::Failed(_))) => {
                early = Some(e);
                break;
            }
            None => {}
        }
    }
    let stopped = cal.stop();
    let result = match early {
        Some(e) => Some(e),
        None => stopped?,
    };
    let missing: Vec<usize> = layout::all_indices()
        .into_iter()
        .filter(|k| !done.contains(k))
        .collect();
    if !missing.is_empty() && missing.len() < total {
        say(*out, &format!("not calibrated: {}", key_list(&missing)))?;
    }
    match result {
        Some(CalibrationEvent::Failed(code)) => Err(format!(
            "the keyboard reports calibration failure (code {code})"
        )),
        Some(_) => say(*out, "calibration finished"),
        None => Err(
            "the keyboard sent no final calibration status; check the keys with 'aulactl travel'"
                .into(),
        ),
    }
}

pub fn reset<L: Link>(c: &mut Cli<L>, rest: &[&str]) -> Result<(), String> {
    match rest {
        ["triggers", "--yes"] | ["--yes", "triggers"] => {
            let acked = c.kb.reset_triggers(Confirm::user_confirmed())?;
            c.report_backup()?;
            if acked {
                say(c.out, "every key's actuation and rapid trigger is back to factory")
            } else {
                say(c.out, "sent, but the keyboard did not acknowledge it; check with 'aulactl actuation show'")
            }
        }
        ["factory", "--yes"] | ["--yes", "factory"] => {
            let missing = c.kb.factory_reset(Confirm::user_confirmed())?;
            c.report_backup()?;
            if missing.is_empty() {
                say(c.out, "the keyboard is back to factory settings")
            } else {
                say(c.out, &format!("sent; not acknowledged: {} (check with 'aulactl info')", missing.join(", ")))
            }
        }
        ["triggers"] => Err("this resets every key's actuation and rapid trigger; run 'aulactl reset triggers --yes'".into()),
        ["factory"] => Err("this resets the whole keyboard (keymap, macros, lighting, triggers); run 'aulactl reset factory --yes'".into()),
        _ => Err(bad_args(&[&["reset"], rest].concat())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limits() -> TravelLimits {
        TravelLimits {
            max: 340,
            min: 8,
            unit_num: 1,
            high_precision: false,
        }
    }

    fn t(mode: TriggerMode, actuation: u16, press: u16, release: u16) -> KeyTrigger {
        KeyTrigger {
            mode,
            actuation,
            press,
            release,
        }
    }

    #[test]
    fn trigger_texts() {
        let l = limits();
        assert_eq!(
            trigger_text(&l, &t(TriggerMode::Fixed, 120, 50, 50)),
            "fixed at 1.20 mm"
        );
        assert_eq!(
            trigger_text(&l, &t(TriggerMode::Rapid, 100, 5, 5)),
            "rt from 1.00 mm, sensitivity 0.05 mm"
        );
        assert_eq!(
            trigger_text(&l, &t(TriggerMode::RapidSplit, 8, 30, 50)),
            "rt-split from 0.08 mm, press 0.30 mm, release 0.50 mm"
        );
    }

    #[test]
    fn editing_triggers() {
        let now = t(TriggerMode::Rapid, 100, 50, 50);
        assert_eq!(
            edit_trigger(&now, Some(120), None, None),
            Ok(t(TriggerMode::Rapid, 120, 50, 50))
        );
        assert_eq!(
            edit_trigger(&now, None, Some((30, None)), None),
            Ok(t(TriggerMode::Rapid, 100, 30, 30))
        );
        assert_eq!(
            edit_trigger(&now, None, Some((30, Some(40))), None),
            Ok(t(TriggerMode::RapidSplit, 100, 30, 40))
        );
        assert_eq!(
            edit_trigger(&now, None, None, Some(TriggerMode::Fixed)),
            Ok(t(TriggerMode::Fixed, 100, 50, 50))
        );
        // a split key switched to rt keeps press for both
        let split = t(TriggerMode::RapidSplit, 100, 30, 40);
        assert_eq!(
            edit_trigger(&split, None, None, Some(TriggerMode::Rapid)),
            Ok(t(TriggerMode::Rapid, 100, 30, 30))
        );
        assert_eq!(
            edit_trigger(&now, None, Some((30, None)), Some(TriggerMode::RapidSplit)),
            Ok(t(TriggerMode::RapidSplit, 100, 30, 30))
        );
        assert!(edit_trigger(&now, None, Some((30, Some(40))), Some(TriggerMode::Rapid)).is_err());
        assert_eq!(edit_trigger(&now, None, None, None), Ok(now));
    }

    #[test]
    fn grouped_and_tabled() {
        let l = limits();
        let mut map = PerKey::new();
        map.0.insert(46, t(TriggerMode::Fixed, 120, 50, 50));
        map.0.insert(68, t(TriggerMode::Rapid, 100, 50, 50));
        map.0.insert(69, t(TriggerMode::Rapid, 100, 50, 50));
        let g = groups(&l, &map);
        assert_eq!(
            g,
            vec![
                (
                    "rt from 1.00 mm, sensitivity 0.50 mm".to_string(),
                    vec![68, 69]
                ),
                ("fixed at 1.20 mm".to_string(), vec![46])
            ]
        );
        assert!(groups(&l, &PerKey::new()).is_empty());
        assert_eq!(
            actuation_table(&l, &map),
            "key   legend  mode   actuation  press  release\n\
             KeyW  W       fixed  1.20       -      -\n\
             KeyA  A       rt     1.00       0.50   0.50\n\
             KeyS  S       rt     1.00       0.50   0.50\n"
        );
        let view = actuation_view(&l, &map);
        assert!(view.lines().nth(1).unwrap().contains("|1.20 |"), "{view}");
        assert!(
            view.lines().nth(2).unwrap().contains("|1.00r|1.00r|"),
            "{view}"
        );
        assert!(view.ends_with("fixed at 1.20 mm: KeyW\n"), "{view}");
    }

    #[test]
    fn switch_names() {
        assert_eq!(parse_switch("3"), Ok(3));
        assert_eq!(parse_switch("cy1"), Ok(3));
        assert_eq!(parse_switch("TC1"), Ok(5));
        assert!(parse_switch("Gateron").is_err() && parse_switch("").is_err());
        assert_eq!(switch_text(1), "HM1 (id 1)");
        assert_eq!(switch_text(200), "id 200");
    }

    #[test]
    fn travel_views() {
        let l = limits();
        let mut depth = BTreeMap::new();
        assert!(travel_view(&l, &depth, true).ends_with("down: -"));
        depth.insert(46, 340);
        depth.insert(68, 0);
        let v = travel_view(&l, &depth, true);
        assert!(v.contains("|\x1b[7m3.40\x1b[0m|"), "{v}");
        assert!(v.ends_with("down: KeyW 3.40"), "{v}");
        let plain = travel_view(&l, &depth, false);
        assert!(
            !plain.contains('\x1b') && plain.contains("|3.40|") && plain.contains("|0.00|"),
            "{plain}"
        );
        assert_eq!(plain.lines().count(), 6);
    }
}

//! light: effect, brightness, speed, colours, direction, per-key custom colours, music rhythm.

use crate::args::{direction_name, parse_direction, parse_num, parse_on_off, parse_slot, Args};
use crate::render::{board, key_list};
use crate::trigger::select;
use crate::{bad_args, put, say, Cli};
use aula_core::hidraw::Link;
use aula_core::lighting::{
    mode_by_name, mode_info, Colors, CustomColors, Effect, ModeList, Rgb, CUSTOM_SLOTS, MODES,
    MODE_CUSTOM,
};
use std::collections::BTreeMap;

fn mode_name(id: u8) -> String {
    mode_info(id).map_or_else(|| format!("mode {id}"), |m| m.name.to_string())
}

/// One line: mode, brightness, speed, colours, direction.
pub fn effect_text(e: &Effect) -> String {
    let info = mode_info(e.mode);
    let mut parts = vec![mode_name(e.mode)];
    if !e.on {
        parts.push("off".into());
    }
    parts.push(format!("brightness {}", e.brightness));
    if info.is_none_or(|m| m.speed) {
        parts.push(format!("speed {}", e.speed));
    }
    match info.map_or(Colors::FgBg, |m| m.colors) {
        Colors::None => {}
        Colors::Fg => parts.push(format!("colour {}", e.fg)),
        Colors::FgBg if e.full_color => parts.push("rainbow".into()),
        Colors::FgBg => parts.push(format!("colour {} on {}", e.fg, e.bg)),
    }
    if info.is_none_or(|m| !m.directions.is_empty()) {
        parts.push(format!("direction {}", direction_name(info, e.direction)));
    }
    parts.join(", ")
}

pub fn effect_lines(e: &Effect, list: &ModeList, slot: u8, music: bool) -> String {
    let info = mode_info(e.mode);
    let mut rows = vec![
        format!(
            "effect:       {}{}",
            mode_name(e.mode),
            if e.on { "" } else { " (lighting off)" }
        ),
        format!("brightness:   {} of {}", e.brightness, list.max_brightness),
        format!("speed:        {} of {}", e.speed, list.max_speed),
    ];
    let colour = match info.map_or(Colors::FgBg, |m| m.colors) {
        Colors::None => "- (per-key colours: aulactl light custom)".to_string(),
        Colors::Fg => e.fg.to_string(),
        Colors::FgBg if e.full_color => "rainbow".to_string(),
        Colors::FgBg => format!("{} on background {}", e.fg, e.bg),
    };
    rows.push(format!("colour:       {colour}"));
    rows.push(format!(
        "direction:    {}",
        direction_name(info, e.direction)
    ));
    rows.push(format!("custom slot:  {slot}"));
    rows.push(format!(
        "music rhythm: {}",
        if music { "on" } else { "off" }
    ));
    rows.join("\n") + "\n"
}

pub fn modes_text(list: &ModeList) -> String {
    let mut out = String::new();
    for id in list.selectable() {
        let Some(m) = mode_info(id) else { continue };
        let mut what = Vec::new();
        if m.speed {
            what.push("speed".to_string());
        }
        match m.colors {
            Colors::None => {}
            Colors::Fg => what.push("colour".into()),
            Colors::FgBg => what.push("colour, background, rainbow".into()),
        }
        if !m.directions.is_empty() {
            let d: Vec<&str> = m.directions.iter().map(|(_, n)| *n).collect();
            what.push(format!("direction {}", d.join("/")));
        }
        if id == MODE_CUSTOM {
            what.push("per-key colours (light custom)".into());
        }
        out.push_str(&format!("{:<14}{}\n", m.name, what.join(", ")));
    }
    out
}

/// Non-black keys grouped by colour.
pub fn custom_text(colors: &CustomColors) -> String {
    let mut by: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    let mut black = 0;
    for (idx, c) in colors.iter() {
        if *c == Rgb::BLACK {
            black += 1;
        } else {
            by.entry(c.to_string()).or_default().push(idx);
        }
    }
    let mut out = String::new();
    for (c, keys) in &by {
        out.push_str(&format!("{c}: {}\n", key_list(keys)));
    }
    if by.is_empty() {
        out.push_str("every key is black (off)\n");
    } else if black > 0 {
        out.push_str(&format!("{black} other keys are black (off)\n"));
    }
    out
}

/// Applies option changes to an effect. Switching modes moves the direction to one the new mode offers.
pub struct EffectEdit<'a> {
    pub mode: Option<&'a str>,
    pub brightness: Option<&'a str>,
    pub speed: Option<&'a str>,
    pub fg: Option<&'a str>,
    pub bg: Option<&'a str>,
    pub direction: Option<&'a str>,
    pub rainbow: Option<&'a str>,
    pub on: Option<bool>,
}

impl EffectEdit<'_> {
    fn none() -> Self {
        EffectEdit {
            mode: None,
            brightness: None,
            speed: None,
            fg: None,
            bg: None,
            direction: None,
            rainbow: None,
            on: None,
        }
    }

    pub fn apply(&self, e: &Effect, list: &ModeList) -> Result<Effect, String> {
        let mut n = *e;
        if let Some(m) = self.mode {
            let info = mode_by_name(m).ok_or_else(|| {
                let names: Vec<&str> = MODES.iter().map(|m| m.name).collect();
                format!("unknown effect '{m}' ({})", names.join(", "))
            })?;
            n.mode = info.id;
            n.on = true;
            if !info.directions.iter().any(|(d, _)| *d == n.direction) {
                n.direction = info.directions.first().map_or(0, |(d, _)| *d);
            }
        }
        let info = mode_info(n.mode).ok_or_else(|| format!("the keyboard runs effect {} which aulactl does not know; pick one with 'aulactl light effect'", n.mode))?;
        if let Some(b) = self.brightness {
            n.brightness = parse_num(b, 0..=u32::from(list.max_brightness), "brightness")? as u8;
        }
        if let Some(s) = self.speed {
            if !info.speed {
                return Err(format!("{} has no speed", info.name));
            }
            n.speed = parse_num(s, 0..=u32::from(list.max_speed), "speed")? as u8;
        }
        if self.fg.is_some() || self.bg.is_some() {
            match info.colors {
                Colors::None => return Err(format!("{} has no colour setting", info.name)),
                Colors::Fg if self.bg.is_some() => {
                    return Err(format!("{} has no background colour", info.name))
                }
                _ => {}
            }
            if let Some(c) = self.fg {
                n.fg = c.parse()?;
            }
            if let Some(c) = self.bg {
                n.bg = c.parse()?;
            }
            n.full_color = false;
        }
        if let Some(r) = self.rainbow {
            let on = parse_on_off(r)?;
            if on && info.colors != Colors::FgBg {
                return Err(format!("{} has no rainbow colours", info.name));
            }
            n.full_color = on;
        }
        if let Some(d) = self.direction {
            n.direction = parse_direction(info, d)?;
        }
        if let Some(on) = self.on {
            n.on = on;
        }
        Ok(n)
    }
}

fn set_effect<L: Link>(c: &mut Cli<L>, edit: EffectEdit) -> Result<(), String> {
    let list = c.kb.read_mode_list()?;
    let now = c.kb.read_effect()?;
    let new = edit.apply(&now, &list)?;
    if new == now {
        return say(c.out, &format!("nothing to change: {}", effect_text(&now)));
    }
    c.kb.set_effect(&new)?;
    c.report_backup()?;
    say(c.out, &format!("lighting: {}", effect_text(&new)))
}

fn custom_show<L: Link>(c: &mut Cli<L>, slot: Option<&str>) -> Result<(), String> {
    let s = match slot {
        Some(v) => parse_slot(v, CUSTOM_SLOTS, "custom")?,
        None => c.kb.read_active_custom_slot()?,
    };
    let colors = c.kb.read_custom_colors(s)?;
    say(c.out, &format!("custom slot {s}:"))?;
    if c.tty {
        let view = board(
            5,
            |k| k.label.to_string(),
            |k| {
                colors
                    .get(k.index as usize)
                    .filter(|c| **c != Rgb::BLACK)
                    .map(|c| format!("30;48;2;{};{};{}", c.0, c.1, c.2))
            },
        );
        put(c.out, &view)?;
    }
    put(c.out, &custom_text(&colors))
}

pub fn light<L: Link>(c: &mut Cli<L>, rest: &[&str]) -> Result<(), String> {
    let mut a = Args::new(rest);
    let mut edit = EffectEdit::none();
    edit.brightness = a.opt("--brightness")?;
    edit.speed = a.opt("--speed")?;
    edit.fg = a.opt("--color")?;
    edit.bg = a.opt("--bg")?;
    edit.direction = a.opt("--direction")?;
    edit.rainbow = a.opt("--rainbow")?;
    let slot = a
        .opt("--slot")?
        .map(|s| parse_slot(s, CUSTOM_SLOTS, "custom"))
        .transpose()?;
    let pos = a.positional()?;
    let effect_opts = edit.brightness.is_some()
        || edit.speed.is_some()
        || edit.fg.is_some()
        || edit.bg.is_some()
        || edit.direction.is_some()
        || edit.rainbow.is_some();
    if effect_opts && !matches!(pos.first(), Some(&"effect")) {
        return Err("effect options go with 'aulactl light effect MODE'".into());
    }
    if slot.is_some() && !matches!(pos[..], ["custom", "set", ..]) {
        return Err("--slot goes with 'aulactl light custom set'".into());
    }
    match pos[..] {
        [] | ["show"] => {
            let list = c.kb.read_mode_list()?;
            let e = c.kb.read_effect()?;
            let s = c.kb.read_active_custom_slot()?;
            let music = c.kb.refresh_capabilities()?.music_rhythm_on;
            put(c.out, &effect_lines(&e, &list, s, music))
        }
        ["modes"] => {
            let list = c.kb.read_mode_list()?;
            put(c.out, &modes_text(&list))?;
            say(
                c.out,
                &format!(
                    "brightness 0..{}, speed 0..{}",
                    list.max_brightness, list.max_speed
                ),
            )
        }
        ["effect", mode] => set_effect(
            c,
            EffectEdit {
                mode: Some(mode),
                ..edit
            },
        ),
        ["brightness", v] => set_effect(
            c,
            EffectEdit {
                brightness: Some(v),
                ..EffectEdit::none()
            },
        ),
        ["speed", v] => set_effect(
            c,
            EffectEdit {
                speed: Some(v),
                ..EffectEdit::none()
            },
        ),
        ["direction", v] => set_effect(
            c,
            EffectEdit {
                direction: Some(v),
                ..EffectEdit::none()
            },
        ),
        ["rainbow", v] => set_effect(
            c,
            EffectEdit {
                rainbow: Some(v),
                ..EffectEdit::none()
            },
        ),
        ["color" | "colour", fg] => set_effect(
            c,
            EffectEdit {
                fg: Some(fg),
                ..EffectEdit::none()
            },
        ),
        ["color" | "colour", fg, bg] => set_effect(
            c,
            EffectEdit {
                fg: Some(fg),
                bg: Some(bg),
                ..EffectEdit::none()
            },
        ),
        ["on" | "off"] => set_effect(
            c,
            EffectEdit {
                on: Some(pos[0] == "on"),
                ..EffectEdit::none()
            },
        ),
        ["custom"] | ["custom", "show"] => custom_show(c, None),
        ["custom", "show", s] => custom_show(c, Some(s)),
        ["custom", s] if s != "set" => custom_show(c, Some(s)),
        ["custom", "set", sel, colour] => {
            let colour: Rgb = colour.parse()?;
            let keys = select(Some(sel))?;
            let s = match slot {
                Some(s) => s,
                None => c.kb.read_active_custom_slot()?,
            };
            let mut colors = c.kb.read_custom_colors(s)?;
            for idx in &keys {
                colors.0.insert(*idx as u8, colour);
            }
            c.kb.write_custom_colors(s, &colors)?;
            c.report_backup()?;
            say(
                c.out,
                &format!("custom slot {s}: {colour} on {}", key_list(&keys)),
            )?;
            let e = c.kb.read_effect()?;
            if e.mode != MODE_CUSTOM {
                say(
                    c.out,
                    "the custom effect shows these colours: aulactl light effect custom",
                )?;
            }
            Ok(())
        }
        ["slot"] => {
            let s = c.kb.read_active_custom_slot()?;
            say(c.out, &format!("custom slot {s}"))
        }
        ["slot", v] => {
            let s = parse_slot(v, CUSTOM_SLOTS, "custom")?;
            if c.kb.read_active_custom_slot()? == s {
                return say(c.out, &format!("custom slot {s} is already selected"));
            }
            // the board shows the slot written last (protocol 5.5): rewrite it unchanged
            let colors = c.kb.read_custom_colors(s)?;
            c.kb.write_custom_colors(s, &colors)?;
            c.report_backup()?;
            let now = c.kb.read_active_custom_slot()?;
            if now != s {
                return Err(format!(
                    "slot {s} was rewritten but the keyboard still reports slot {now}"
                ));
            }
            say(c.out, &format!("custom slot {s} selected"))
        }
        ["music", v] => {
            let on = parse_on_off(v)?;
            c.kb.set_music_rhythm(on)?;
            c.report_backup()?;
            say(c.out, &format!("music rhythm {v}"))
        }
        _ => Err(bad_args(&[&["light"], rest].concat())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list() -> ModeList {
        ModeList {
            modes: vec![0, 1, 2, 3, 4, 6, 7, 8, 9, 11, 12, 14, 15, 16],
            max_speed: 4,
            max_brightness: 4,
        }
    }

    fn effect(mode: u8) -> Effect {
        Effect {
            mode,
            brightness: 1,
            speed: 2,
            fg: Rgb(255, 0, 255),
            bg: Rgb::BLACK,
            direction: 0,
            full_color: false,
            on: true,
        }
    }

    #[test]
    fn effect_texts() {
        assert_eq!(
            effect_text(&effect(0)),
            "static, brightness 1, colour #ff00ff"
        );
        assert_eq!(
            effect_text(&effect(2)),
            "wave, brightness 1, speed 2, colour #ff00ff on #000000, direction right"
        );
        let off = Effect {
            on: false,
            full_color: true,
            ..effect(1)
        };
        assert_eq!(
            effect_text(&off),
            "breath, off, brightness 1, speed 2, rainbow"
        );
        assert_eq!(effect_text(&effect(10)), "custom, brightness 1");
        assert!(effect_text(&effect(99)).starts_with("mode 99, brightness 1, speed 2"));
        let lines = effect_lines(&effect(2), &list(), 1, false);
        assert!(effect_lines(&effect(0), &list(), 0, false).contains("colour:       #ff00ff\n"));
        assert!(
            lines.contains("colour:       #ff00ff on background #000000\n"),
            "{lines}"
        );
        assert!(
            lines.contains("brightness:   1 of 4\n")
                && lines.contains("custom slot:  1\n")
                && lines.contains("direction:    right\n"),
            "{lines}"
        );
    }

    #[test]
    fn effect_edits() {
        let l = list();
        let e = effect(0);
        let mut ed = EffectEdit::none();
        ed.mode = Some("wave");
        ed.direction = Some("left");
        let n = ed.apply(&e, &l).unwrap();
        assert_eq!((n.mode, n.direction), (2, 1));
        // switching to a mode whose directions do not include the current one
        let aurora = EffectEdit {
            mode: Some("aurora"),
            ..EffectEdit::none()
        }
        .apply(&e, &l)
        .unwrap();
        assert_eq!(aurora.direction, 4);
        let off = Effect { on: false, ..e };
        assert!(
            EffectEdit {
                mode: Some("static"),
                ..EffectEdit::none()
            }
            .apply(&off, &l)
            .unwrap()
            .on
        );
        assert_eq!(
            EffectEdit {
                brightness: Some("4"),
                ..EffectEdit::none()
            }
            .apply(&e, &l)
            .unwrap()
            .brightness,
            4
        );
        assert!(EffectEdit {
            brightness: Some("5"),
            ..EffectEdit::none()
        }
        .apply(&e, &l)
        .is_err());
        assert!(EffectEdit {
            speed: Some("1"),
            ..EffectEdit::none()
        }
        .apply(&e, &l)
        .unwrap_err()
        .contains("no speed"));
        assert!(EffectEdit {
            bg: Some("#ffffff"),
            ..EffectEdit::none()
        }
        .apply(&e, &l)
        .unwrap_err()
        .contains("no background"));
        assert!(EffectEdit {
            rainbow: Some("on"),
            ..EffectEdit::none()
        }
        .apply(&e, &l)
        .unwrap_err()
        .contains("no rainbow"));
        assert!(EffectEdit {
            fg: Some("red"),
            ..EffectEdit::none()
        }
        .apply(&e, &l)
        .is_err());
        assert!(EffectEdit {
            mode: Some("disco"),
            ..EffectEdit::none()
        }
        .apply(&e, &l)
        .unwrap_err()
        .contains("static, breath"));
        let rainbow = EffectEdit {
            rainbow: Some("on"),
            ..EffectEdit::none()
        }
        .apply(&effect(1), &l)
        .unwrap();
        assert!(rainbow.full_color);
        // a colour turns rainbow off
        let back = EffectEdit {
            fg: Some("00ff00"),
            bg: Some("#0000ff"),
            ..EffectEdit::none()
        }
        .apply(&rainbow, &l)
        .unwrap();
        assert_eq!(
            (back.fg, back.bg, back.full_color),
            (Rgb(0, 255, 0), Rgb(0, 0, 255), false)
        );
        assert!(
            !EffectEdit {
                on: Some(false),
                ..EffectEdit::none()
            }
            .apply(&e, &l)
            .unwrap()
            .on
        );
        assert!(EffectEdit::none().apply(&effect(99), &l).is_err());
    }

    #[test]
    fn custom_colour_groups() {
        let mut c = CustomColors::from_fn(|_| Rgb::BLACK);
        assert_eq!(custom_text(&c), "every key is black (off)\n");
        for idx in [46u8, 68, 69, 70] {
            c.0.insert(idx, Rgb(255, 0, 0));
        }
        c.0.insert(22, Rgb(0, 0, 255));
        assert_eq!(
            custom_text(&c),
            "#0000ff: Escape\n#ff0000: KeyW KeyA KeyS KeyD\n56 other keys are black (off)\n"
        );
        assert_eq!(
            custom_text(&CustomColors::from_fn(|_| Rgb(1, 2, 3))),
            "#010203: all keys\n"
        );
    }

    #[test]
    fn mode_listing() {
        let text = modes_text(&list());
        assert!(text.starts_with("static        colour\n"), "{text}");
        assert!(text.contains("wave          speed, colour, background, rainbow, direction right/left/up/down/spread/gather\n"));
        assert!(text.contains("custom        per-key colours (light custom)\n"));
        assert_eq!(text.lines().count(), 15);
    }
}

//! keymap, macro, adv, socd, winlock.

use crate::args::{
    parse_dks_key, parse_layer, parse_routes, parse_slot, parse_socd_mode, parse_threshold,
    parse_win_lock, phases_text, routes_text, socd_mode_name, win_lock_text, Args,
};
use crate::macro_text::{self, parse_play_mode, play_mode_name};
use crate::render::{board, table};
use crate::trigger::select;
use crate::{bad_args, say, Cli};
use aula_core::advanced::{self, AdvancedKey, SLOTS as ADV_SLOTS};
use aula_core::hidraw::Link;
use aula_core::keycodes::{Usage, MEDIA, USAGES};
use aula_core::keymap::{KeyAction, Keymap, Layer, NAMED};
use aula_core::layout;
use aula_core::macros::{Macro, PlayMode, SLOTS as MACRO_SLOTS};
use aula_core::perkey::key_name;
use aula_core::socd::{SocdMode, SocdPair, MAX_PAIRS};
use std::io::Write;

/// The action a key has when nothing was remapped: its own key on the base layer, `Default` on the fn layer.
pub fn default_action(layer: Layer, idx: usize) -> KeyAction {
    match (layer, layout::by_index(idx)) {
        (Layer::Base, Some(k)) => KeyAction::from_entry(k.default_entry, Some(idx)),
        _ => KeyAction::Default,
    }
}

/// Keys whose action differs from the default.
pub fn remapped(layer: Layer, map: &Keymap) -> Vec<usize> {
    map.iter()
        .filter(|(idx, a)| **a != default_action(layer, *idx) && **a != KeyAction::Default)
        .map(|(i, _)| i)
        .collect()
}

/// Keys (with their layer) bound to `action` on either layer.
fn bound_to(base: &Keymap, fn_layer: &Keymap, action: &KeyAction) -> Vec<String> {
    let mut v: Vec<String> = base
        .iter()
        .filter(|(_, a)| *a == action)
        .map(|(i, _)| key_name(i))
        .collect();
    v.extend(
        fn_layer
            .iter()
            .filter(|(_, a)| *a == action)
            .map(|(i, _)| format!("Fn+{}", key_name(i))),
    );
    v
}

pub fn keymap_text(layer: Layer, map: &Keymap, fixed: &[usize], all: bool) -> String {
    let changed = remapped(layer, map);
    let mut out = String::new();
    if !changed.is_empty() {
        let cell = |k: &layout::Key| {
            let idx = k.index as usize;
            if changed.contains(&idx) {
                map.get(idx).map(|a| a.to_string()).unwrap_or_default()
            } else {
                String::new()
            }
        };
        out = board(6, cell, |_| None);
    }
    let mut rows = vec![["key", "legend", "action"].map(String::from).to_vec()];
    for (idx, a) in map.iter() {
        if !all && !changed.contains(&idx) {
            continue;
        }
        let label = layout::by_index(idx).map_or("", |k| k.label);
        let mut action = a.to_string();
        if layer == Layer::Fn && fixed.contains(&idx) {
            action.push_str(" (fixed by the firmware)");
        }
        rows.push(vec![key_name(idx), label.to_string(), action]);
    }
    if rows.len() > 1 {
        out.push_str(&table(&rows));
    }
    let what = match layer {
        Layer::Base => "remapped on the base layer",
        Layer::Fn => "set on the fn layer (others keep the firmware's Fn function)",
    };
    out.push_str(&format!(
        "{} of {} keys {what}\n",
        changed.len(),
        layout::KEYS.len()
    ));
    out
}

pub fn keymap<L: Link>(c: &mut Cli<L>, rest: &[&str]) -> Result<(), String> {
    let mut a = Args::new(rest);
    let layer = parse_layer(a.opt("--layer")?)?;
    let all = a.flag("--all");
    let pos = a.positional()?;
    match pos[..] {
        ["show"] => {
            let map = c.kb.read_keymap(layer)?;
            let fixed = if layer == Layer::Fn {
                c.kb.read_fn_fixed()?
            } else {
                Vec::new()
            };
            crate::put(c.out, &keymap_text(layer, &map, &fixed, all))
        }
        ["set", sel, action] => {
            let action: KeyAction = action.parse()?;
            set_keys(c, layer, &select(Some(sel))?, |_| action.clone())
        }
        ["reset", sel] => set_keys(c, layer, &select(Some(sel))?, |idx| {
            default_action(layer, idx)
        }),
        ["codes", ..] => codes(c.out, &pos[1..]),
        _ => Err(bad_args(&[&["keymap"], rest].concat())),
    }
}

/// Sets `keys` on `layer` to `action(key)` and writes the layer when something changed.
fn set_keys<L: Link>(
    c: &mut Cli<L>,
    layer: Layer,
    keys: &[usize],
    action: impl Fn(usize) -> KeyAction,
) -> Result<(), String> {
    let mut map = c.kb.read_keymap(layer)?;
    let before = map.clone();
    for &idx in keys {
        map.0.insert(idx as u8, action(idx));
    }
    if map == before {
        return say(c.out, "nothing to change: the keys already do that");
    }
    c.kb.write_keymap(layer, &map)?;
    c.report_backup()?;
    for idx in map.changed_keys(&before) {
        let a = map.get(idx).map(|a| a.to_string()).unwrap_or_default();
        say(
            c.out,
            &format!("{} layer {}: {a}", layer.name(), key_name(idx)),
        )?;
    }
    Ok(())
}

/// Every action name, optionally filtered (case-insensitive substring).
pub fn codes(out: &mut dyn Write, rest: &[&str]) -> Result<(), String> {
    let filter = match rest {
        [] => None,
        [f] => Some(f.to_ascii_lowercase()),
        _ => return Err(bad_args(&[&["keymap", "codes"], rest].concat())),
    };
    let keep = |n: &str| {
        filter
            .as_ref()
            .is_none_or(|f| n.to_ascii_lowercase().contains(f.as_str()))
    };
    let mut sections = vec![
        (
            "keys",
            USAGES
                .iter()
                .map(|(_, n)| n.to_string())
                .collect::<Vec<_>>(),
        ),
        ("media", MEDIA.iter().map(|(_, n)| n.to_string()).collect()),
        (
            "functions",
            NAMED.iter().map(|(_, n)| n.to_string()).collect(),
        ),
        ("special", vec!["Default".into(), "Fn".into()]),
    ];
    sections.push((
        "macros",
        (0..MACRO_SLOTS).map(|s| format!("Macro{s}")).collect(),
    ));
    sections.push((
        "advanced keys",
        (0..ADV_SLOTS).map(|s| format!("Adv{s}")).collect(),
    ));
    let mut found = false;
    for (title, names) in sections {
        let names: Vec<String> = names.into_iter().filter(|n| keep(n)).collect();
        if names.is_empty() {
            continue;
        }
        found = true;
        say(out, &format!("{title}: {}", names.join(" ")))?;
    }
    if filter.is_none() {
        say(out, "combinations: 2 or 3 keys with a modifier, e.g. Ctrl+Shift+Esc; raw entries: raw:xxxxxxxx")?;
        say(
            out,
            "short names also work: a, 1, Esc, Del, PgUp, Left, LShift, RCtrl, Win, Menu",
        )?;
    } else if !found {
        return Err(format!("no action name contains '{}'", rest[0]));
    }
    Ok(())
}

fn macro_line(m: &Macro) -> String {
    let mode = match m.mode {
        PlayMode::Repeat => format!("repeat {}x", m.repeat),
        other => play_mode_name(other).to_string(),
    };
    format!(
        "{mode}, {} events: {}",
        m.events.len(),
        macro_text::format(&m.events, 0)
    )
}

pub fn macros<L: Link>(c: &mut Cli<L>, rest: &[&str]) -> Result<(), String> {
    let mut a = Args::new(rest);
    let mode = a.opt("--mode")?;
    let repeat = a.opt("--repeat")?;
    let delay = a.opt("--delay")?;
    let bind = a.opt("--bind")?;
    let layer = parse_layer(a.opt("--layer")?)?;
    let pos = a.positional()?;
    match pos[..] {
        ["list"] => {
            let all = c.kb.read_macros()?;
            let (base, fn_layer) = (c.kb.read_keymap(Layer::Base)?, c.kb.read_keymap(Layer::Fn)?);
            for (s, m) in all.iter().enumerate() {
                let keys = bound_to(&base, &fn_layer, &KeyAction::Macro(s as u8));
                let on = if keys.is_empty() {
                    String::new()
                } else {
                    format!("  [on {}]", keys.join(" "))
                };
                let text = m.as_ref().map_or_else(|| "empty".to_string(), macro_line);
                say(c.out, &format!("{s}: {text}{on}"))?;
            }
            Ok(())
        }
        ["show", slot] => {
            let s = parse_slot(slot, MACRO_SLOTS, "macro")?;
            let Some(m) = c.kb.read_macro(s)? else {
                return say(c.out, &format!("macro slot {s} is empty"));
            };
            let mode = match m.mode {
                PlayMode::Repeat => format!(" --mode repeat --repeat {}", m.repeat),
                PlayMode::Once => String::new(),
                other => format!(" --mode {}", play_mode_name(other)),
            };
            say(c.out, &format!("slot {s}: {}", macro_line(&m)))?;
            say(
                c.out,
                &format!(
                    "as a command: aulactl macro set {s} \"{}\"{mode} --delay 0",
                    macro_text::format(&m.events, 0)
                ),
            )
        }
        ["set", slot, steps] => {
            let s = parse_slot(slot, MACRO_SLOTS, "macro")?;
            let delay = match delay {
                Some(d) => d
                    .strip_suffix("ms")
                    .unwrap_or(d)
                    .parse::<u16>()
                    .map_err(|_| format!("'{d}' is not a delay in ms"))?,
                None => 10,
            };
            let events = macro_text::parse(steps, delay)?;
            let mode = mode.map(parse_play_mode).transpose()?;
            let repeat = repeat
                .map(|r| {
                    r.parse::<u16>()
                        .ok()
                        .filter(|n| *n >= 1)
                        .ok_or(format!("'{r}' is not a repeat count (1..65535)"))
                })
                .transpose()?;
            let (mode, repeat) = match (mode, repeat) {
                (None | Some(PlayMode::Repeat), Some(n)) => (PlayMode::Repeat, n),
                (Some(PlayMode::Repeat), None) => {
                    return Err("--mode repeat needs --repeat N".into())
                }
                (Some(m), Some(_)) => {
                    return Err(format!(
                        "--repeat only goes with --mode repeat, not {}",
                        play_mode_name(m)
                    ))
                }
                (m, None) => (m.unwrap_or(PlayMode::Once), 1),
            };
            let bind_keys = bind.map(|b| select(Some(b))).transpose()?;
            let m = Macro {
                id: s,
                mode,
                repeat,
                events,
            };
            m.validate()?;
            c.kb.write_macro(s, Some(&m))?;
            c.report_backup()?;
            say(c.out, &format!("macro slot {s}: {}", macro_line(&m)))?;
            match bind_keys {
                Some(keys) => set_keys(c, layer, &keys, |_| KeyAction::Macro(s)),
                None => say(
                    c.out,
                    &format!("bind it to a key with: aulactl keymap set KEY Macro{s}"),
                ),
            }
        }
        ["clear", slot] => {
            let s = parse_slot(slot, MACRO_SLOTS, "macro")?;
            c.kb.write_macro(s, None)?;
            c.report_backup()?;
            say(c.out, &format!("macro slot {s} cleared"))?;
            let (base, fn_layer) = (c.kb.read_keymap(Layer::Base)?, c.kb.read_keymap(Layer::Fn)?);
            let keys = bound_to(&base, &fn_layer, &KeyAction::Macro(s));
            if !keys.is_empty() {
                say(
                    c.out,
                    &format!(
                        "still bound to Macro{s}: {} (aulactl keymap reset KEY)",
                        keys.join(" ")
                    ),
                )?;
            }
            Ok(())
        }
        _ => Err(bad_args(&[&["macro"], rest].concat())),
    }
}

pub fn advanced_text(a: &AdvancedKey) -> String {
    match a {
        AdvancedKey::Empty => "empty".into(),
        AdvancedKey::Dks { routes, keys } => {
            let ks: Vec<String> = keys
                .iter()
                .map(|k| format!("{}:{}", k.action, phases_text(&k.phases)))
                .collect();
            format!("dks at {} mm: {}", routes_text(routes), ks.join(" "))
        }
        AdvancedKey::ModTap {
            hold,
            tap,
            threshold,
        } => format!(
            "mt hold {hold}, tap {tap}, after {} ms",
            u32::from(*threshold) * 10
        ),
        AdvancedKey::Toggle { action } => format!("tgl {action}"),
        AdvancedKey::Other { .. } => "a type this board does not support (kept as is)".into(),
    }
}

pub fn advanced<L: Link>(c: &mut Cli<L>, rest: &[&str]) -> Result<(), String> {
    let mut a = Args::new(rest);
    let routes = a.opt("--routes")?;
    let point = a.opt("--point")?;
    let threshold = a.opt("--threshold")?;
    let bind = a.opt("--bind")?;
    let layer = parse_layer(a.opt("--layer")?)?;
    let pos = a.positional()?;
    match pos[..] {
        ["list"] => {
            let all = c.kb.read_advanced()?;
            let (base, fn_layer) = (c.kb.read_keymap(Layer::Base)?, c.kb.read_keymap(Layer::Fn)?);
            let mut used = 0;
            for (s, k) in all.iter().enumerate() {
                if *k == AdvancedKey::Empty {
                    continue;
                }
                used += 1;
                let keys = bound_to(&base, &fn_layer, &KeyAction::Advanced(s as u8));
                let on = if keys.is_empty() {
                    "  [not bound]".to_string()
                } else {
                    format!("  [on {}]", keys.join(" "))
                };
                say(c.out, &format!("{s}: {}{on}", advanced_text(k)))?;
            }
            say(
                c.out,
                &format!("{used} of {} advanced-key slots used", all.len()),
            )
        }
        ["set", slot, kind, ref args @ ..] => {
            let s = parse_slot(slot, ADV_SLOTS, "advanced-key")?;
            let key = match (kind, args) {
                ("dks", [_, ..]) if args.len() <= 4 => {
                    let keys = args
                        .iter()
                        .map(|k| parse_dks_key(k))
                        .collect::<Result<Vec<_>, _>>()?;
                    let routes = match (routes, point) {
                        (Some(_), Some(_)) => {
                            return Err("give --routes or --point, not both".into())
                        }
                        (Some(r), None) => parse_routes(r)?,
                        (None, p) => {
                            let l = c.kb.travel_limits()?;
                            let first = match p {
                                Some(p) => parse_routes(&[p; 4].join(","))?[0],
                                None => 15,
                            };
                            let max_tenths =
                                (l.to_mm(l.max) * 10.0).round().clamp(1.0, 255.0) as u8;
                            advanced::dks_default_routes(first, max_tenths)
                        }
                    };
                    AdvancedKey::Dks { routes, keys }
                }
                ("dks", _) => return Err("a DKS needs 1 to 4 output keys (ACTION:PHASES)".into()),
                ("mt", [hold, tap]) => AdvancedKey::ModTap {
                    hold: hold.parse()?,
                    tap: tap.parse()?,
                    threshold: threshold.map(parse_threshold).transpose()?.unwrap_or(15),
                },
                ("tgl", [action]) => AdvancedKey::Toggle {
                    action: action.parse()?,
                },
                _ => return Err(bad_args(&[&["adv"], rest].concat())),
            };
            let bind_keys = bind.map(|b| select(Some(b))).transpose()?;
            advanced::encode(&key)?;
            c.kb.write_advanced(&[(s, key.clone())])?;
            c.report_backup()?;
            say(
                c.out,
                &format!("advanced-key slot {s}: {}", advanced_text(&key)),
            )?;
            match bind_keys {
                Some(keys) => set_keys(c, layer, &keys, |_| KeyAction::Advanced(s)),
                None => say(
                    c.out,
                    &format!("bind it to a key with: aulactl keymap set KEY Adv{s}"),
                ),
            }
        }
        ["clear", slot] => {
            let s = parse_slot(slot, ADV_SLOTS, "advanced-key")?;
            c.kb.write_advanced(&[(s, AdvancedKey::Empty)])?;
            c.report_backup()?;
            say(c.out, &format!("advanced-key slot {s} cleared"))?;
            let (base, fn_layer) = (c.kb.read_keymap(Layer::Base)?, c.kb.read_keymap(Layer::Fn)?);
            let keys = bound_to(&base, &fn_layer, &KeyAction::Advanced(s));
            if !keys.is_empty() {
                say(
                    c.out,
                    &format!(
                        "still bound to Adv{s}: {} (aulactl keymap reset KEY)",
                        keys.join(" ")
                    ),
                )?;
            }
            Ok(())
        }
        _ => Err(bad_args(&[&["adv"], rest].concat())),
    }
}

pub fn socd_text(enabled: bool, pairs: &[SocdPair]) -> String {
    let mut out = format!("SOCD is {}\n", if enabled { "on" } else { "off" });
    if pairs.is_empty() {
        out.push_str("no key pairs; add one with 'aulactl socd add KEY1 KEY2'\n");
        return out;
    }
    let mut rows = vec![["#", "key1", "key2", "mode"].map(String::from).to_vec()];
    for (i, p) in pairs.iter().enumerate() {
        rows.push(vec![
            (i + 1).to_string(),
            p.key1.to_string(),
            p.key2.to_string(),
            socd_mode_name(p.mode).into(),
        ]);
    }
    out.push_str(&table(&rows));
    out
}

fn usage(v: &str) -> Result<Usage, String> {
    v.parse::<Usage>()
        .map_err(|_| format!("unknown key '{v}' (see 'aulactl keymap codes')"))
}

pub fn socd<L: Link>(c: &mut Cli<L>, rest: &[&str]) -> Result<(), String> {
    let mut a = Args::new(rest);
    let mode = a.opt("--mode")?;
    let pos = a.positional()?;
    match pos[..] {
        [] | ["list"] => {
            let on = c.kb.read_socd_power()?.enabled;
            let pairs = c.kb.read_socd_pairs()?;
            crate::put(c.out, &socd_text(on, &pairs))
        }
        ["on" | "off"] => {
            let on = pos[0] == "on";
            c.kb.set_socd_enabled(on)?;
            c.report_backup()?;
            say(c.out, &format!("SOCD is {}", pos[0]))
        }
        ["add", k1, k2] => {
            let (key1, key2) = (usage(k1)?, usage(k2)?);
            if key1 == key2 {
                return Err("a SOCD pair needs two different keys".into());
            }
            let mode = mode
                .map(parse_socd_mode)
                .transpose()?
                .unwrap_or(SocdMode::LastWins);
            let mut pairs = c.kb.read_socd_pairs()?;
            if let Some(i) = pairs
                .iter()
                .position(|p| (p.key1, p.key2) == (key1, key2) || (p.key1, p.key2) == (key2, key1))
            {
                if pairs[i].mode == mode && pairs[i].key1 == key1 {
                    return say(c.out, "nothing to change: the pair exists");
                }
                pairs[i] = SocdPair { mode, key1, key2 };
            } else if pairs.len() >= MAX_PAIRS {
                return Err(format!(
                    "all {MAX_PAIRS} SOCD pairs are used; remove one first"
                ));
            } else {
                pairs.push(SocdPair { mode, key1, key2 });
            }
            c.kb.write_socd_pairs(&pairs)?;
            c.report_backup()?;
            let on = c.kb.read_socd_power()?.enabled;
            crate::put(c.out, &socd_text(on, &pairs))
        }
        ["remove", ref which @ ..] if !which.is_empty() && which.len() <= 2 => {
            let mut pairs = c.kb.read_socd_pairs()?;
            let i = match which {
                [n] => match n.parse::<usize>() {
                    Ok(n) if (1..=pairs.len()).contains(&n) => n - 1,
                    _ => {
                        return Err(format!(
                            "'{n}' is not a pair number (see 'aulactl socd list')"
                        ))
                    }
                },
                [k1, k2] => {
                    let (a, b) = (usage(k1)?, usage(k2)?);
                    pairs
                        .iter()
                        .position(|p| (p.key1, p.key2) == (a, b) || (p.key1, p.key2) == (b, a))
                        .ok_or_else(|| format!("no SOCD pair {k1} {k2}"))?
                }
                _ => unreachable!("one or two arguments"),
            };
            pairs.remove(i);
            c.kb.write_socd_pairs(&pairs)?;
            c.report_backup()?;
            let on = c.kb.read_socd_power()?.enabled;
            crate::put(c.out, &socd_text(on, &pairs))
        }
        _ => Err(bad_args(&[&["socd"], rest].concat())),
    }
}

pub fn win_lock<L: Link>(c: &mut Cli<L>, rest: &[&str]) -> Result<(), String> {
    if rest.is_empty() || rest == ["show"] {
        let w = c.kb.read_win_lock()?;
        return say(c.out, &format!("win-lock blocks: {}", win_lock_text(&w)));
    }
    let w = parse_win_lock(rest)?;
    if c.kb.read_win_lock()? == w {
        return say(
            c.out,
            &format!(
                "nothing to change: win-lock already blocks {}",
                win_lock_text(&w)
            ),
        );
    }
    c.kb.set_win_lock(w)?;
    c.report_backup()?;
    say(c.out, &format!("win-lock blocks: {}", win_lock_text(&w)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use aula_core::advanced::DksKey;
    use aula_core::perkey::PerKey;

    fn base_default() -> Keymap {
        PerKey::from_fn(|k| default_action(Layer::Base, k.index as usize))
    }

    #[test]
    fn remapped_keys() {
        let mut m = base_default();
        assert!(remapped(Layer::Base, &m).is_empty());
        m.0.insert(66, KeyAction::Key(0x29)); // Caps -> Escape
        m.0.insert(46, KeyAction::Default); // all-zero entry is not a remap
        assert_eq!(remapped(Layer::Base, &m), vec![66]);
        let mut f: Keymap = PerKey::from_fn(|_| KeyAction::Default);
        assert!(remapped(Layer::Fn, &f).is_empty());
        f.0.insert(23, KeyAction::Media(0x10));
        assert_eq!(remapped(Layer::Fn, &f), vec![23]);
        assert_eq!(default_action(Layer::Base, 122), KeyAction::Fn);
        assert_eq!(default_action(Layer::Base, 3), KeyAction::Default);
    }

    #[test]
    fn keymap_listing() {
        let mut m = base_default();
        m.0.insert(66, KeyAction::Key(0x29));
        let text = keymap_text(Layer::Base, &m, &[], false);
        assert!(
            text.contains("key       legend  action\nCapsLock  Caps    Escape\n"),
            "{text}"
        );
        assert!(
            text.ends_with("1 of 61 keys remapped on the base layer\n"),
            "{text}"
        );
        assert!(
            text.lines().nth(2).unwrap().starts_with("| Escape  |"),
            "{text}"
        );
        let all = keymap_text(Layer::Base, &m, &[], true);
        assert_eq!(all.lines().count(), 5 + 1 + 61 + 1);
        let mut f: Keymap = PerKey::from_fn(|_| KeyAction::Default);
        f.0.insert(22, KeyAction::Key(0x35));
        let text = keymap_text(Layer::Fn, &f, &[22], false);
        assert!(
            text.contains("Escape  Esc     Backquote (fixed by the firmware)"),
            "{text}"
        );
        let none = keymap_text(
            Layer::Fn,
            &PerKey::from_fn(|_| KeyAction::Default),
            &[],
            false,
        );
        assert!(
            !none.contains("legend")
                && !none.contains('|')
                && none.ends_with(
                    "0 of 61 keys set on the fn layer (others keep the firmware's Fn function)\n"
                )
        );
    }

    #[test]
    fn code_listing() {
        let mut out = Vec::new();
        codes(&mut out, &[]).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("keys: KeyA KeyB"), "{text}");
        assert!(text.contains("media: VolumeUp"));
        assert!(
            text.contains("Macro9") && text.contains("Adv39") && text.contains("Ctrl+Shift+Esc")
        );
        let mut out = Vec::new();
        codes(&mut out, &["volume"]).unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "media: VolumeUp VolumeDown\n"
        );
        let mut out = Vec::new();
        assert!(codes(&mut out, &["zzz"])
            .unwrap_err()
            .contains("no action name contains 'zzz'"));
        assert!(codes(&mut out, &["a", "b"]).is_err());
    }

    #[test]
    fn advanced_texts() {
        assert_eq!(advanced_text(&AdvancedKey::Empty), "empty");
        let dks = AdvancedKey::Dks {
            routes: [15, 30, 30, 15],
            keys: vec![DksKey {
                action: KeyAction::Key(4),
                phases: [2, 0, 0, 3],
            }],
        };
        assert_eq!(advanced_text(&dks), "dks at 1.5/3.0/3.0/1.5 mm: KeyA:p--h");
        let mt = AdvancedKey::ModTap {
            hold: KeyAction::Key(0xe0),
            tap: KeyAction::Key(0x29),
            threshold: 15,
        };
        assert_eq!(
            advanced_text(&mt),
            "mt hold ControlLeft, tap Escape, after 150 ms"
        );
        assert_eq!(
            advanced_text(&AdvancedKey::Toggle {
                action: KeyAction::Key(0xe1)
            }),
            "tgl ShiftLeft"
        );
        assert!(advanced_text(&AdvancedKey::Other {
            payload: String::new()
        })
        .contains("does not support"));
    }

    #[test]
    fn socd_listing() {
        assert_eq!(
            socd_text(false, &[]),
            "SOCD is off\nno key pairs; add one with 'aulactl socd add KEY1 KEY2'\n"
        );
        let pairs = [SocdPair {
            mode: SocdMode::Neutral,
            key1: Usage(0x04),
            key2: Usage(0x07),
        }];
        assert_eq!(
            socd_text(true, &pairs),
            "SOCD is on\n#  key1  key2  mode\n1  KeyA  KeyD  neutral\n"
        );
    }

    #[test]
    fn macro_lines() {
        let m = aula_core::macros::simple_sequence(0, &[Usage(4)], 10);
        assert_eq!(
            macro_line(&m),
            "once, 2 events: down KeyA, wait 10, up KeyA"
        );
        let r = Macro {
            mode: PlayMode::Repeat,
            repeat: 3,
            ..m
        };
        assert!(macro_line(&r).starts_with("repeat 3x, 2 events"));
    }
}

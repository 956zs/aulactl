//! Compact macro text: `down KeyA, wait 20, up KeyA, tap MouseLeft`.
//!
//! Steps are separated by `,` or `;`. `down X` / `up X` press or release X, `tap X` is `down X, up X`, and
//! `wait MS` sets the delay after the previous event (several waits add up). Events not followed by a wait
//! get the default delay. X is a key name (`KeyA`, `a`, `Esc`, `LShift`, `0x04`) or `MouseLeft`,
//! `MouseRight`, `MouseMiddle`, `mouse:N` (button bits).

use aula_core::keycodes::Usage;
use aula_core::macros::{MacroEvent, MacroInput, PlayMode, MAX_EVENTS};

const MOUSE: [(u8, &str); 3] = [(1, "MouseLeft"), (2, "MouseRight"), (4, "MouseMiddle")];

fn parse_input(s: &str) -> Result<MacroInput, String> {
    if let Some((bit, _)) = MOUSE.iter().find(|(_, n)| n.eq_ignore_ascii_case(s)) {
        return Ok(MacroInput::Mouse(*bit));
    }
    if let Some(n) = s.strip_prefix("mouse:") {
        return match n.parse::<u8>() {
            Ok(b) if (1..=0x0f).contains(&b) => Ok(MacroInput::Mouse(b)),
            _ => Err(format!(
                "'{s}': mouse buttons are 1..15 (1 left, 2 right, 4 middle)"
            )),
        };
    }
    s.parse::<Usage>()
        .map(MacroInput::Key)
        .map_err(|_| format!("unknown key '{s}' (see 'aulactl keymap codes')"))
}

fn input_text(i: &MacroInput) -> String {
    match i {
        MacroInput::Key(u) => u.to_string(),
        MacroInput::Mouse(b) => MOUSE
            .iter()
            .find(|(v, _)| v == b)
            .map_or_else(|| format!("mouse:{b}"), |(_, n)| n.to_string()),
    }
}

/// Parses macro text into events; `default_delay` (ms) applies to events without a following `wait`.
pub fn parse(text: &str, default_delay: u16) -> Result<Vec<MacroEvent>, String> {
    let mut events: Vec<MacroEvent> = Vec::new();
    // whether the last event's delay was set by a wait
    let mut waited = false;
    for step in text.split([',', ';']).map(str::trim) {
        if step.is_empty() {
            continue;
        }
        let (word, arg) = step
            .split_once(char::is_whitespace)
            .map_or((step, ""), |(w, a)| (w, a.trim()));
        if arg.is_empty() || arg.contains(char::is_whitespace) {
            return Err(format!(
                "'{step}' is not a step (down X, up X, tap X, wait MS)"
            ));
        }
        match word.to_ascii_lowercase().as_str() {
            "wait" => {
                let ms: u16 = arg
                    .strip_suffix("ms")
                    .unwrap_or(arg)
                    .parse()
                    .map_err(|_| format!("'{arg}' is not a delay in ms (0..65535)"))?;
                let Some(last) = events.last_mut() else {
                    return Err("a macro cannot start with a wait".into());
                };
                let base = if waited { last.delay_ms } else { 0 };
                last.delay_ms = base.checked_add(ms).ok_or("a delay is at most 65535 ms")?;
                waited = true;
            }
            w @ ("down" | "up" | "tap") => {
                let input = parse_input(arg)?;
                if w != "up" {
                    events.push(MacroEvent {
                        input,
                        down: true,
                        delay_ms: default_delay,
                    });
                }
                if w != "down" {
                    events.push(MacroEvent {
                        input,
                        down: false,
                        delay_ms: default_delay,
                    });
                }
                waited = false;
            }
            _ => {
                return Err(format!(
                    "'{step}' is not a step (down X, up X, tap X, wait MS)"
                ))
            }
        }
    }
    if events.is_empty() {
        return Err("a macro needs at least one step".into());
    }
    if events.len() > MAX_EVENTS {
        return Err(format!(
            "a macro holds at most {MAX_EVENTS} events (a tap is 2), got {}",
            events.len()
        ));
    }
    Ok(events)
}

/// Text form of `events`: waits equal to `default_delay` are left out, a down immediately followed by its up
/// becomes `tap`. `parse(&format(e, d), d)` gives `e` back.
pub fn format(events: &[MacroEvent], default_delay: u16) -> String {
    let mut steps = Vec::new();
    let mut i = 0;
    while i < events.len() {
        let e = &events[i];
        let next = events.get(i + 1);
        let last = match next {
            Some(n) if e.down && !n.down && n.input == e.input && e.delay_ms == default_delay => {
                steps.push(format!("tap {}", input_text(&e.input)));
                i += 1;
                n
            }
            _ => {
                steps.push(format!(
                    "{} {}",
                    if e.down { "down" } else { "up" },
                    input_text(&e.input)
                ));
                e
            }
        };
        if last.delay_ms != default_delay {
            steps.push(format!("wait {}", last.delay_ms));
        }
        i += 1;
    }
    steps.join(", ")
}

pub fn parse_play_mode(v: &str) -> Result<PlayMode, String> {
    match v {
        "once" => Ok(PlayMode::Once),
        "repeat" => Ok(PlayMode::Repeat),
        "toggle" => Ok(PlayMode::Toggle),
        "hold" | "while-held" => Ok(PlayMode::WhileHeld),
        _ => Err(format!(
            "'{v}' is not a play mode (once, repeat, toggle or hold)"
        )),
    }
}

pub fn play_mode_name(m: PlayMode) -> &'static str {
    match m {
        PlayMode::Once => "once",
        PlayMode::Repeat => "repeat",
        PlayMode::Toggle => "toggle",
        PlayMode::WhileHeld => "hold",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(u: u8, down: bool, delay_ms: u16) -> MacroEvent {
        MacroEvent {
            input: MacroInput::Key(Usage(u)),
            down,
            delay_ms,
        }
    }

    #[test]
    fn steps() {
        assert_eq!(
            parse("down KeyA, wait 20, up KeyA", 0),
            Ok(vec![key(4, true, 20), key(4, false, 0)])
        );
        assert_eq!(
            parse("tap a; tap b", 10),
            Ok(vec![
                key(4, true, 10),
                key(4, false, 10),
                key(5, true, 10),
                key(5, false, 10)
            ])
        );
        assert_eq!(
            parse("tap a, wait 5, wait 7ms", 10),
            Ok(vec![key(4, true, 10), key(4, false, 12)])
        );
        assert_eq!(
            parse("down LShift, wait 0, up LShift", 10),
            Ok(vec![key(0xe1, true, 0), key(0xe1, false, 10)])
        );
        assert_eq!(
            parse(" DOWN esc ,, UP 0x29 ,", 0),
            Ok(vec![key(0x29, true, 0), key(0x29, false, 0)])
        );
        let mouse = parse("tap MouseLeft, down mouse:8", 0).unwrap();
        assert_eq!(mouse[0].input, MacroInput::Mouse(1));
        assert_eq!(
            mouse[2],
            MacroEvent {
                input: MacroInput::Mouse(8),
                down: true,
                delay_ms: 0
            }
        );
    }

    #[test]
    fn step_errors() {
        for (text, msg) in [
            ("", "at least one step"),
            (" , ", "at least one step"),
            ("wait 10, tap a", "cannot start with a wait"),
            ("press a", "not a step"),
            ("down", "not a step"),
            ("down a b", "not a step"),
            ("tap nokey", "unknown key"),
            ("tap a, wait -1", "not a delay"),
            ("tap a, wait 65536", "not a delay"),
            ("tap a, wait 65535, wait 1", "at most 65535"),
            ("down mouse:16", "mouse buttons"),
            ("down mouse:0", "mouse buttons"),
        ] {
            let err = parse(text, 0).unwrap_err();
            assert!(err.contains(msg), "{text}: {err}");
        }
        let long = vec!["tap a"; MAX_EVENTS / 2].join(",");
        assert_eq!(parse(&long, 0).unwrap().len(), MAX_EVENTS);
        assert!(parse(&format!("{long}, down b"), 0)
            .unwrap_err()
            .contains("at most 62"));
    }

    #[test]
    fn format_round_trips() {
        let ev = vec![
            key(4, true, 20),
            key(4, false, 0),
            key(0xe1, true, 10),
            key(5, true, 10),
            key(5, false, 10),
            key(0xe1, false, 10),
        ];
        assert_eq!(
            format(&ev, 0),
            "down KeyA, wait 20, up KeyA, down ShiftLeft, wait 10, down KeyB, wait 10, up KeyB, wait 10, up ShiftLeft, wait 10"
        );
        assert_eq!(
            format(&ev, 10),
            "down KeyA, wait 20, up KeyA, wait 0, down ShiftLeft, tap KeyB, up ShiftLeft"
        );
        for d in [0, 10, 20] {
            assert_eq!(parse(&format(&ev, d), d).unwrap(), ev, "default {d}");
        }
        let mouse = vec![
            MacroEvent {
                input: MacroInput::Mouse(1),
                down: true,
                delay_ms: 0,
            },
            MacroEvent {
                input: MacroInput::Mouse(9),
                down: false,
                delay_ms: 0,
            },
        ];
        assert_eq!(format(&mouse, 0), "down MouseLeft, up mouse:9");
        assert_eq!(format(&[], 0), "");
    }

    #[test]
    fn play_modes() {
        for m in [
            PlayMode::Once,
            PlayMode::Repeat,
            PlayMode::Toggle,
            PlayMode::WhileHeld,
        ] {
            assert_eq!(parse_play_mode(play_mode_name(m)), Ok(m));
        }
        assert!(parse_play_mode("loop").is_err());
    }
}

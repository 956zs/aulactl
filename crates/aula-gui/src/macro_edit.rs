//! Macro editor helpers: keys recorded in the window become macro events (the real gaps as delays), and
//! the editor's macro is checked against the core rules before it is written.

use aula_core::keycodes::Usage;
use aula_core::macros::{Macro, MacroEvent, MacroInput, MAX_EVENTS};
use serde_json::{json, Value};

/// One key event seen by the recorder: core usage name, press or release, time in ms.
#[derive(Debug, Clone, PartialEq)]
pub struct Recorded {
    pub name: String,
    pub down: bool,
    pub at_ms: f64,
}

/// Parses `[{ name, down, t }]` from the QML recorder.
pub fn parse_recording(json: &str) -> Result<Vec<Recorded>, String> {
    let v: Value = serde_json::from_str(json).map_err(|e| format!("recording: {e}"))?;
    let list = v.as_array().ok_or("recording: not a list")?;
    list.iter()
        .map(|e| {
            let name = e["name"].as_str().ok_or("recording: event without a key name")?.to_string();
            let down = e["down"].as_bool().ok_or("recording: event without down")?;
            let at_ms = e["t"].as_f64().filter(|t| t.is_finite()).ok_or("recording: event without a time")?;
            Ok(Recorded { name, down, at_ms })
        })
        .collect()
}

/// What the recorder produced: the events, and how many were dropped because the slot is full.
#[derive(Debug, Clone, PartialEq)]
pub struct Conversion {
    pub events: Vec<MacroEvent>,
    pub dropped: usize,
}

/// Turns recorded key events into macro events. Each event waits until the next one (clamped to the
/// 16-bit delay); the last waits `last_delay_ms`. A release whose press was not recorded is skipped, and
/// keys still held at the end are released, so a macro never leaves a key down. Unknown names are skipped.
pub fn from_recording(rec: &[Recorded], last_delay_ms: u16) -> Conversion {
    let mut held: Vec<Usage> = Vec::new();
    let mut kept: Vec<(Usage, bool, f64)> = Vec::new();
    for r in rec {
        let Ok(u) = r.name.parse::<Usage>() else { continue };
        if r.down {
            if held.contains(&u) {
                // auto-repeat or a duplicate press
                continue;
            }
            held.push(u);
        } else {
            let Some(at) = held.iter().position(|h| *h == u) else { continue };
            held.remove(at);
        }
        kept.push((u, r.down, r.at_ms));
    }
    let end = kept.last().map_or(0.0, |k| k.2);
    for u in held {
        kept.push((u, false, end));
    }
    let mut events: Vec<MacroEvent> = kept
        .iter()
        .enumerate()
        .map(|(i, (u, down, at))| {
            let delay = match kept.get(i + 1) {
                Some(next) => (next.2 - at).round().clamp(0.0, f64::from(u16::MAX)) as u16,
                None => last_delay_ms,
            };
            MacroEvent { input: MacroInput::Key(*u), down: *down, delay_ms: delay }
        })
        .collect();
    let dropped = events.len().saturating_sub(MAX_EVENTS);
    events.truncate(MAX_EVENTS);
    Conversion { events, dropped }
}

/// `{ events, dropped }` as the QML editor stores events (core JSON form).
pub fn conversion_json(c: &Conversion) -> Value {
    json!({ "events": c.events, "dropped": c.dropped })
}

/// Empty when the macro JSON (core form) can be written, else the reason.
pub fn problem(json: &str) -> String {
    match serde_json::from_str::<Macro>(json) {
        Ok(m) => m.validate().err().map(String::from).unwrap_or_default(),
        Err(e) => format!("macro: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(name: &str, down: bool, t: f64) -> Recorded {
        Recorded { name: name.into(), down, at_ms: t }
    }

    fn key(name: &str) -> MacroInput {
        MacroInput::Key(name.parse().unwrap())
    }

    #[test]
    fn gaps_become_delays() {
        let c = from_recording(&[rec("KeyA", true, 0.0), rec("KeyA", false, 80.0), rec("KeyB", true, 200.4)], 10);
        assert_eq!(c.dropped, 0);
        let d: Vec<(MacroInput, bool, u16)> = c.events.iter().map(|e| (e.input, e.down, e.delay_ms)).collect();
        assert_eq!(
            d,
            vec![(key("KeyA"), true, 80), (key("KeyA"), false, 120), (key("KeyB"), true, 0), (key("KeyB"), false, 10)]
        );
    }

    #[test]
    fn empty_recording() {
        assert_eq!(from_recording(&[], 50), Conversion { events: vec![], dropped: 0 });
        assert_eq!(parse_recording("[]").unwrap(), vec![]);
    }

    #[test]
    fn stray_releases_repeats_and_unknown_keys_are_skipped() {
        let c = from_recording(
            &[
                rec("Enter", false, 0.0),
                rec("KeyQ", true, 10.0),
                rec("KeyQ", true, 40.0),
                rec("NoSuchKey", true, 50.0),
                rec("KeyQ", false, 60.0),
            ],
            0,
        );
        assert_eq!(c.events.len(), 2);
        assert_eq!(c.events[0].delay_ms, 50);
        assert!(!c.events[1].down);
    }

    #[test]
    fn held_keys_are_released_and_delays_clamped() {
        let c = from_recording(&[rec("ShiftLeft", true, 0.0), rec("KeyZ", true, 1e9)], 5);
        assert_eq!(c.events.len(), 4);
        assert_eq!(c.events[0].delay_ms, u16::MAX);
        assert!(c.events[2..].iter().all(|e| !e.down));
        // Time going backwards gives 0, not a wrap.
        let c = from_recording(&[rec("KeyA", true, 100.0), rec("KeyA", false, 50.0)], 0);
        assert_eq!(c.events[0].delay_ms, 0);
    }

    #[test]
    fn too_many_events_are_dropped() {
        let mut r = Vec::new();
        for i in 0..40 {
            r.push(rec("KeyA", true, f64::from(i) * 2.0));
            r.push(rec("KeyA", false, f64::from(i) * 2.0 + 1.0));
        }
        let c = from_recording(&r, 0);
        assert_eq!(c.events.len(), MAX_EVENTS);
        assert_eq!(c.dropped, 80 - MAX_EVENTS);
        assert_eq!(conversion_json(&c)["dropped"], 80 - MAX_EVENTS);
    }

    #[test]
    fn recording_json_errors() {
        assert!(parse_recording("{").is_err());
        assert!(parse_recording("{}").is_err());
        assert!(parse_recording(r#"[{"down": true, "t": 1}]"#).is_err());
        assert!(parse_recording(r#"[{"name": "KeyA", "t": 1}]"#).is_err());
        assert!(parse_recording(r#"[{"name": "KeyA", "down": true}]"#).is_err());
        let ok = parse_recording(r#"[{"name": "KeyA", "down": true, "t": 1.5}]"#).unwrap();
        assert_eq!(ok, vec![rec("KeyA", true, 1.5)]);
    }

    #[test]
    fn problems() {
        let good = r#"{"id": 0, "mode": "once", "repeat": 1, "events": [{"key": "KeyA", "down": true, "delay_ms": 10}]}"#;
        assert_eq!(problem(good), "");
        let empty = r#"{"id": 0, "mode": "once", "repeat": 1, "events": []}"#;
        assert!(problem(empty).contains("at least one"));
        assert!(problem("{").starts_with("macro:"));
        assert!(problem(r#"{"id": 0, "mode": "sometimes", "repeat": 1, "events": []}"#).starts_with("macro:"));
        let mouse = r#"{"id": 0, "mode": "once", "repeat": 1, "events": [{"mouse": 0, "down": true, "delay_ms": 0}]}"#;
        assert!(problem(mouse).contains("mouse button"));
    }

}

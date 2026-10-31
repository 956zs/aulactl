//! `aulactl feed` (`docs/feed.md`): press/release detection on the host from travel-stream depths, per-key depth
//! coalescing, and the NDJSON lines of the feed contract.

use crate::error::{invalid, Result};
use crate::info::Identity;
use crate::layout;
use crate::perkey::{key_name, PerKey};
use crate::trigger::{KeyTrigger, TravelLimits, TriggerMode};
use serde::Serialize;
use std::collections::BTreeMap;

/// A press or release derived from depth.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    Press,
    Release,
}

/// Press/release state of one key, mirroring the firmware's trigger modes. Depths and settings are in trigger
/// units.
///
/// - fixed: pressed while depth >= actuation point.
/// - rt / rt-split: released below the actuation point; above it, press after moving down by the press
///   sensitivity from the lowest depth since the last release, release after moving up by the release
///   sensitivity from the highest depth since the last press.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyState {
    trigger: KeyTrigger,
    pressed: bool,
    /// Lowest depth since the last release while released, highest since the last press while pressed.
    extreme: u16,
}

impl KeyState {
    /// A key at rest.
    pub fn new(trigger: KeyTrigger) -> Self {
        KeyState { trigger, pressed: false, extreme: 0 }
    }

    pub fn pressed(&self) -> bool {
        self.pressed
    }

    /// Feeds one depth (0 for the board's return-to-rest frame) and returns the edge it causes, if any.
    pub fn update(&mut self, depth: u16) -> Option<Edge> {
        let t = self.trigger;
        // a sensitivity of 0 would toggle on every sample
        let press = t.press.max(1);
        let release = if t.mode == TriggerMode::Rapid { press } else { t.release.max(1) };
        let edge = match (t.mode, self.pressed) {
            (TriggerMode::Fixed, false) => depth >= t.actuation,
            (TriggerMode::Fixed, true) => depth < t.actuation,
            (_, false) => {
                self.extreme = self.extreme.min(depth);
                depth >= t.actuation && depth - self.extreme >= press
            }
            (_, true) => {
                self.extreme = self.extreme.max(depth);
                depth < t.actuation || self.extreme - depth >= release
            }
        };
        if !edge {
            return None;
        }
        self.pressed = !self.pressed;
        self.extreme = depth;
        Some(if self.pressed { Edge::Press } else { Edge::Release })
    }
}

/// Limits depth updates to at most `hz` per key. The last depth of a burst is sent once its interval has
/// passed (`due`), and a return to rest (depth 0) is always sent at once.
#[derive(Debug, Clone)]
pub struct DepthCoalescer {
    /// None: depth updates are turned off (`hz` 0).
    interval_ms: Option<f64>,
    keys: BTreeMap<usize, Slot>,
}

#[derive(Debug, Clone, Copy)]
struct Slot {
    sent_at: f64,
    sent: u16,
    pending: Option<u16>,
}

impl DepthCoalescer {
    /// `hz` 0 turns depth updates off.
    pub fn new(hz: u32) -> Self {
        DepthCoalescer { interval_ms: (hz > 0).then(|| 1000.0 / hz as f64), keys: BTreeMap::new() }
    }

    /// A sample of `key` at `t_ms`: the depth to send now, if any.
    pub fn sample(&mut self, key: usize, t_ms: f64, depth: u16) -> Option<u16> {
        let interval = self.interval_ms?;
        let slot = self.keys.entry(key).or_insert(Slot { sent_at: f64::NEG_INFINITY, sent: 0, pending: None });
        if depth == slot.sent {
            slot.pending = None;
            return None;
        }
        if depth == 0 || t_ms - slot.sent_at >= interval {
            *slot = Slot { sent_at: t_ms, sent: depth, pending: None };
            return Some(depth);
        }
        slot.pending = Some(depth);
        None
    }

    /// Held-back depths whose interval has passed by `t_ms`, in key order.
    pub fn due(&mut self, t_ms: f64) -> Vec<(usize, u16)> {
        let Some(interval) = self.interval_ms else { return Vec::new() };
        let mut out = Vec::new();
        for (key, slot) in &mut self.keys {
            if let Some(depth) = slot.pending {
                if t_ms - slot.sent_at >= interval {
                    *slot = Slot { sent_at: t_ms, sent: depth, pending: None };
                    out.push((*key, depth));
                }
            }
        }
        out
    }

    /// Milliseconds from `t_ms` until the next held-back depth is due (0 if one is overdue), None if none waits.
    pub fn next_due(&self, t_ms: f64) -> Option<f64> {
        let interval = self.interval_ms?;
        self.keys
            .values()
            .filter(|s| s.pending.is_some())
            .map(|s| (s.sent_at + interval - t_ms).max(0.0))
            .min_by(f64::total_cmp)
    }
}

/// Why the feed is not streaming.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Waiting: no keyboard found (or it could not be opened).
    NoKeyboard,
    /// Waiting: another aulactl / aula-gui operation holds the device.
    Busy,
    /// The keyboard went away while streaming.
    Lost,
}

impl Status {
    fn state_reason(self) -> (&'static str, &'static str) {
        match self {
            Status::NoKeyboard => ("waiting", "no keyboard"),
            Status::Busy => ("waiting", "busy"),
            Status::Lost => ("lost", "keyboard unplugged"),
        }
    }
}

/// The `hello` line: board identity and the selected keys.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Hello {
    pub product: String,
    pub firmware: String,
    pub max_mm: f64,
    pub keys: Vec<HelloKey>,
}

/// One selected key: layout geometry in key units and its trigger settings in mm.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct HelloKey {
    pub code: &'static str,
    pub label: &'static str,
    pub idx: usize,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub mode: &'static str,
    pub point: f64,
    pub press: f64,
    pub release: f64,
}

/// Name of a trigger mode in the feed (and the CLI).
pub fn mode_name(m: TriggerMode) -> &'static str {
    match m {
        TriggerMode::Fixed => "fixed",
        TriggerMode::Rapid => "rt",
        TriggerMode::RapidSplit => "rt-split",
    }
}

/// Builds the `hello` line content for `keys`. Every key needs an entry in `triggers`.
pub fn hello(identity: &Identity, limits: &TravelLimits, triggers: &PerKey<KeyTrigger>, keys: &[usize]) -> Result<Hello> {
    let mut out = Vec::with_capacity(keys.len());
    for &idx in keys {
        let Some(key) = layout::by_index(idx) else {
            return invalid(format!("{} is not a key", key_name(idx)));
        };
        let Some(t) = u8::try_from(idx).ok().and_then(|i| triggers.0.get(&i)) else {
            return invalid(format!("no trigger settings for {}", key.code));
        };
        let release = if t.mode == TriggerMode::Rapid { t.press } else { t.release };
        out.push(HelloKey {
            code: key.code,
            label: key.label,
            idx,
            x: round(key.x() as f64, 100.0),
            y: round(key.y() as f64, 100.0),
            w: round(key.width() as f64, 100.0),
            mode: mode_name(t.mode),
            point: mm(limits, t.actuation),
            press: mm(limits, t.press),
            release: mm(limits, release),
        });
    }
    Ok(Hello {
        product: identity.product_code.clone(),
        firmware: identity.firmware.clone(),
        max_mm: mm(limits, limits.max),
        keys: out,
    })
}

/// Trigger units to mm, 2 decimals.
pub fn mm(limits: &TravelLimits, raw: u16) -> f64 {
    round(limits.to_mm(raw), 100.0)
}

fn round(v: f64, scale: f64) -> f64 {
    (v * scale).round() / scale
}

/// One NDJSON line of the feed. `t` is milliseconds since the feed started.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Line<'a> {
    Status { state: &'static str, reason: &'static str },
    Hello(&'a Hello),
    Depth { t: f64, key: &'a str, mm: f64 },
    Press { t: f64, key: &'a str },
    Release { t: f64, key: &'a str },
    Error { msg: &'a str },
}

impl<'a> Line<'a> {
    pub fn status(s: Status) -> Self {
        let (state, reason) = s.state_reason();
        Line::Status { state, reason }
    }

    /// Depth in mm, rounded to 2 decimals; `t` to 0.1 ms.
    pub fn depth(t: f64, key: &'a str, mm: f64) -> Self {
        Line::Depth { t: round(t, 10.0), key, mm: round(mm, 100.0) }
    }

    pub fn edge(t: f64, key: &'a str, edge: Edge) -> Self {
        let t = round(t, 10.0);
        match edge {
            Edge::Press => Line::Press { t, key },
            Edge::Release => Line::Release { t, key },
        }
    }

    /// The line without the trailing newline. Non-finite numbers become `null`.
    pub fn to_json(&self) -> String {
        // serializing these plain structs cannot fail
        serde_json::to_string(self).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trig(mode: TriggerMode, actuation: u16, press: u16, release: u16) -> KeyTrigger {
        KeyTrigger { mode, actuation, press, release }
    }

    /// Edges for a depth sequence, as (sample index, edge).
    fn run(t: KeyTrigger, depths: &[u16]) -> Vec<(usize, Edge)> {
        let mut s = KeyState::new(t);
        depths.iter().enumerate().filter_map(|(i, d)| s.update(*d).map(|e| (i, e))).collect()
    }

    use Edge::{Press as P, Release as R};

    #[test]
    fn fixed_presses_at_the_point_and_releases_below_it() {
        let t = trig(TriggerMode::Fixed, 100, 30, 30);
        assert_eq!(run(t, &[0, 50, 99, 100, 200, 340, 150, 100, 99, 0]), vec![(3, P), (8, R)]);
        assert_eq!(run(t, &[]), vec![]);
        assert_eq!(run(t, &[0, 0, 99]), vec![], "never reaches the point");
    }

    #[test]
    fn fixed_follows_jitter_across_the_point_like_the_firmware() {
        let t = trig(TriggerMode::Fixed, 100, 30, 30);
        assert_eq!(run(t, &[101, 99, 100, 99]), vec![(0, P), (1, R), (2, P), (3, R)]);
        assert_eq!(run(t, &[100, 101, 100, 340, 100]), vec![(0, P)], "jitter above the point");
    }

    #[test]
    fn rt_presses_and_releases_on_movement() {
        let t = trig(TriggerMode::Rapid, 50, 20, 99);
        // down to 50 (press at the point: moved 50 >= 20 from 0), up 20 from the peak (release uses press in rt)
        assert_eq!(run(t, &[0, 30, 50, 200, 181, 180, 170, 190, 200, 0]), vec![(2, P), (5, R), (7, P), (9, R)]);
    }

    #[test]
    fn rt_split_uses_separate_sensitivities() {
        let t = trig(TriggerMode::RapidSplit, 50, 10, 40);
        let d = [0, 60, 100, 70, 61, 60, 69, 70, 300, 50, 49];
        // press at 60; peak 100, release at 60 (100-40); low 60, press at 70; peak 300, release at 260 or below
        assert_eq!(run(t, &d), vec![(1, P), (5, R), (7, P), (9, R)]);
    }

    #[test]
    fn rt_jitter_smaller_than_the_sensitivity_does_not_toggle() {
        let t = trig(TriggerMode::RapidSplit, 50, 10, 10);
        assert_eq!(run(t, &[200, 195, 199, 191, 200, 192]), vec![(0, P)]);
        let t = trig(TriggerMode::Rapid, 100, 30, 30);
        assert_eq!(run(t, &[99, 101, 99, 101]), vec![(1, P), (2, R)], "released below the point, then 2 < 30 from the low");
    }

    #[test]
    fn rt_boundaries_are_inclusive() {
        let t = trig(TriggerMode::Rapid, 50, 20, 20);
        assert_eq!(run(t, &[49, 50]), vec![(1, P)], "exactly at the point");
        assert_eq!(run(t, &[60, 41, 40]), vec![(0, P), (1, R)], "41 is below the point");
        assert_eq!(run(t, &[200, 181, 180]), vec![(0, P), (2, R)], "exactly the release sensitivity");
        assert_eq!(run(t, &[200, 180, 199, 200]), vec![(0, P), (1, R), (3, P)], "exactly the press sensitivity");
    }

    #[test]
    fn point_below_the_sensitivity_waits_for_the_sensitivity() {
        let t = trig(TriggerMode::Rapid, 10, 50, 50);
        assert_eq!(run(t, &[10, 49, 50]), vec![(2, P)]);
    }

    #[test]
    fn return_to_rest_releases_in_every_mode() {
        for mode in [TriggerMode::Fixed, TriggerMode::Rapid, TriggerMode::RapidSplit] {
            let t = trig(mode, 100, 30, 30);
            let mut s = KeyState::new(t);
            assert_eq!(s.update(250), Some(P));
            assert!(s.pressed());
            assert_eq!(s.update(0), Some(R), "{mode:?}");
            assert_eq!(s.update(0), None, "repeated rest frames");
            assert!(!s.pressed());
        }
    }

    #[test]
    fn zero_sensitivity_does_not_chatter() {
        let t = trig(TriggerMode::RapidSplit, 50, 0, 0);
        assert_eq!(run(t, &[60, 60, 61, 60, 59]), vec![(0, P), (3, R)]);
    }

    #[test]
    fn coalescer_limits_the_rate_and_keeps_the_last_depth() {
        let mut c = DepthCoalescer::new(100); // 10 ms
        assert_eq!(c.sample(5, 0.0, 50), Some(50));
        assert_eq!(c.sample(5, 2.0, 80), None);
        assert_eq!(c.sample(5, 4.0, 120), None);
        assert_eq!(c.next_due(4.0), Some(6.0));
        assert_eq!(c.due(9.9), vec![]);
        assert_eq!(c.due(10.0), vec![(5, 120)]);
        assert_eq!(c.next_due(10.0), None);
        assert_eq!(c.due(30.0), vec![], "nothing pending");
        assert_eq!(c.sample(5, 25.0, 130), Some(130), "interval passed");
    }

    #[test]
    fn coalescer_always_sends_the_return_to_rest() {
        let mut c = DepthCoalescer::new(1);
        assert_eq!(c.sample(1, 0.0, 300), Some(300));
        assert_eq!(c.sample(1, 1.0, 100), None);
        assert_eq!(c.sample(1, 2.0, 0), Some(0), "final 0 at once");
        assert_eq!(c.due(5000.0), vec![], "the held-back 100 is dropped");
        assert_eq!(c.sample(1, 3.0, 0), None, "repeated rest frames");
        assert_eq!(c.sample(2, 3.0, 0), None, "a key that never moved");
    }

    #[test]
    fn coalescer_keys_are_independent_and_duplicates_dropped() {
        let mut c = DepthCoalescer::new(1000);
        assert_eq!(c.sample(1, 0.0, 10), Some(10));
        assert_eq!(c.sample(2, 0.0, 20), Some(20));
        assert_eq!(c.sample(1, 0.5, 15), None);
        assert_eq!(c.sample(1, 0.7, 10), None, "back to the sent value: nothing pending");
        assert_eq!(c.due(2.0), vec![]);
        assert_eq!(c.sample(1, 3.0, 10), None, "unchanged");
    }

    #[test]
    fn coalescer_off() {
        let mut c = DepthCoalescer::new(0);
        assert_eq!(c.sample(1, 0.0, 10), None);
        assert_eq!(c.sample(1, 100.0, 0), None);
        assert_eq!(c.due(1e9), vec![]);
        assert_eq!(c.next_due(0.0), None);
    }

    fn limits() -> TravelLimits {
        TravelLimits { max: 340, min: 8, unit_num: 1, high_precision: false }
    }

    #[test]
    fn hello_line() {
        let id = Identity {
            raw: String::new(),
            product_code: "SI2825KZHEARGB".into(),
            firmware: "3.17.07".into(),
            build_date: String::new(),
        };
        let z = layout::by_code("KeyZ").unwrap().index as usize;
        let mut t = PerKey::new();
        t.0.insert(z as u8, trig(TriggerMode::Rapid, 100, 50, 7));
        let h = hello(&id, &limits(), &t, &[z]).unwrap();
        let json = Line::Hello(&h).to_json();
        assert!(json.starts_with(r#"{"type":"hello","product":"SI2825KZHEARGB","firmware":"3.17.07","max_mm":3.4,"keys":[{"code":"KeyZ","label":"Z","idx":"#), "{json}");
        assert!(json.ends_with(r#""mode":"rt","point":1.0,"press":0.5,"release":0.5}]}"#), "rt reports press as release: {json}");
        let k = &h.keys[0];
        assert_eq!((k.x, k.y, k.w), (round(k.x, 100.0), 3.0, 1.0));
        let empty = hello(&id, &limits(), &PerKey::new(), &[]).unwrap();
        assert!(Line::Hello(&empty).to_json().ends_with(r#""keys":[]}"#));
        assert!(hello(&id, &limits(), &PerKey::new(), &[z]).is_err(), "missing trigger");
        assert!(hello(&id, &limits(), &t, &[300]).is_err(), "not a key");
    }

    #[test]
    fn event_lines() {
        assert_eq!(Line::status(Status::NoKeyboard).to_json(), r#"{"type":"status","state":"waiting","reason":"no keyboard"}"#);
        assert_eq!(Line::status(Status::Busy).to_json(), r#"{"type":"status","state":"waiting","reason":"busy"}"#);
        assert_eq!(Line::status(Status::Lost).to_json(), r#"{"type":"status","state":"lost","reason":"keyboard unplugged"}"#);
        assert_eq!(Line::depth(1234.5432, "KeyZ", 1.2345).to_json(), r#"{"type":"depth","t":1234.5,"key":"KeyZ","mm":1.23}"#);
        assert_eq!(Line::depth(0.0, "KeyZ", 0.0).to_json(), r#"{"type":"depth","t":0.0,"key":"KeyZ","mm":0.0}"#);
        assert_eq!(Line::edge(1234.56, "KeyZ", P).to_json(), r#"{"type":"press","t":1234.6,"key":"KeyZ"}"#);
        assert_eq!(Line::edge(1301.0, "KeyX", R).to_json(), r#"{"type":"release","t":1301.0,"key":"KeyX"}"#);
        assert_eq!(Line::Error { msg: "a \"b\"\nc" }.to_json(), r#"{"type":"error","msg":"a \"b\"\nc"}"#);
        assert_eq!(Line::depth(f64::NAN, "KeyZ", f64::INFINITY).to_json(), r#"{"type":"depth","t":null,"key":"KeyZ","mm":null}"#);
    }

    #[test]
    fn mm_rounding() {
        assert_eq!(mm(&limits(), 340), 3.4);
        assert_eq!(mm(&limits(), 0), 0.0);
        assert_eq!(mm(&limits(), 123), 1.23);
        let hp = TravelLimits { high_precision: true, ..limits() };
        assert_eq!(mm(&hp, 1234), 1.23);
    }
}

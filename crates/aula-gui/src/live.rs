//! Live travel view state: the latest depth of every key, a peak that holds for a while, and how often each
//! key went down. Fed with travel-stream samples on the worker thread; times are passed in (ms) so the
//! rules are testable.

use aula_core::layout;
use aula_core::travel::TravelSample;
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

/// How long a peak stays before it follows the key back up.
pub const PEAK_HOLD_MS: u64 = 1500;

#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct KeyLive {
    depth: u16,
    peak: u16,
    peak_at_ms: u64,
    presses: u32,
}

#[derive(Debug, Default)]
pub struct LiveState {
    keys: BTreeMap<usize, KeyLive>,
    dirty: bool,
}

impl LiveState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Takes one sample. Samples for non-keys are ignored.
    pub fn apply(&mut self, s: &TravelSample, now_ms: u64) {
        if !layout::is_key(s.index) {
            return;
        }
        let k = self.keys.entry(s.index).or_default();
        let depth = if s.at_rest { 0 } else { s.depth };
        if k.depth == 0 && depth > 0 {
            k.presses = k.presses.saturating_add(1);
        }
        k.depth = depth;
        if depth >= k.peak {
            k.peak = depth;
            k.peak_at_ms = now_ms;
        }
        self.dirty = true;
    }

    /// Lets held peaks expire. Returns true when something changed.
    pub fn tick(&mut self, now_ms: u64) -> bool {
        for k in self.keys.values_mut() {
            if k.peak > k.depth && now_ms.saturating_sub(k.peak_at_ms) >= PEAK_HOLD_MS {
                k.peak = k.depth;
                k.peak_at_ms = now_ms;
                self.dirty = true;
            }
        }
        self.dirty
    }

    pub fn reset(&mut self) {
        self.keys.clear();
        self.dirty = true;
    }

    /// `{ code: { d, p, n } }` (depth, peak, presses) and clears the dirty flag.
    pub fn take_json(&mut self) -> Value {
        self.dirty = false;
        let mut m = Map::new();
        for (idx, k) in &self.keys {
            if let Some(key) = layout::by_index(*idx) {
                m.insert(key.code.to_string(), json!({ "d": k.depth, "p": k.peak, "n": k.presses }));
            }
        }
        Value::Object(m)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(code: &str, depth: u16, at_rest: bool) -> TravelSample {
        TravelSample { index: layout::by_code(code).unwrap().index as usize, depth, at_rest }
    }

    #[test]
    fn depth_peak_and_presses() {
        let mut l = LiveState::new();
        l.apply(&s("KeyA", 120, false), 0);
        l.apply(&s("KeyA", 300, false), 10);
        l.apply(&s("KeyA", 50, false), 20);
        let v = l.take_json();
        assert_eq!(v["KeyA"], json!({ "d": 50, "p": 300, "n": 1 }));
        // Back at rest, then a second press.
        l.apply(&s("KeyA", 0, true), 30);
        l.apply(&s("KeyA", 10, false), 40);
        assert_eq!(l.take_json()["KeyA"]["n"], 2);
    }

    #[test]
    fn peak_holds_then_follows() {
        let mut l = LiveState::new();
        l.apply(&s("KeyW", 340, false), 1000);
        l.apply(&s("KeyW", 0, true), 1100);
        l.take_json();
        assert!(!l.tick(1000 + PEAK_HOLD_MS - 1));
        assert_eq!(l.take_json()["KeyW"]["p"], 340);
        assert!(l.tick(1000 + PEAK_HOLD_MS));
        assert_eq!(l.take_json()["KeyW"]["p"], 0);
        // Nothing left to expire.
        assert!(!l.tick(1_000_000));
    }

    #[test]
    fn empty_and_reset() {
        let mut l = LiveState::new();
        assert_eq!(l.take_json(), json!({}));
        assert!(!l.tick(0));
        l.apply(&s("Space", 5, false), 0);
        l.reset();
        assert!(l.tick(0));
        assert_eq!(l.take_json(), json!({}));
    }

    #[test]
    fn non_keys_are_ignored() {
        let mut l = LiveState::new();
        l.apply(&TravelSample { index: 0, depth: 100, at_rest: false }, 0);
        l.apply(&TravelSample { index: 131, depth: 100, at_rest: false }, 0);
        assert!(!l.tick(0));
        assert_eq!(l.take_json(), json!({}));
    }

    #[test]
    fn at_rest_frame_zeroes_depth_and_extremes() {
        let mut l = LiveState::new();
        l.apply(&s("KeyD", u16::MAX, false), 0);
        assert_eq!(l.take_json()["KeyD"]["d"], u16::MAX);
        // An at-rest frame carries no depth even if a value is present.
        l.apply(&s("KeyD", 77, true), 1);
        assert_eq!(l.take_json()["KeyD"]["d"], 0);
    }
}

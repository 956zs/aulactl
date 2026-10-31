//! Advanced keys (protocol section 8): DKS, mod-tap and toggle in 40 slots. A key uses slot `s` through its
//! keymap entry `Adv<s>` (`17 s s 00`).
//!
//! The payload layout is from the official driver source only; no advanced key was ever seen on the wire.

use crate::error::{invalid, Result};
use crate::frame::{self, Body, CMD_ADVANCED};
use crate::keymap::KeyAction;
use serde::{Deserialize, Serialize};

pub const SLOTS: u8 = 40;
/// Payload bytes of one slot in a read reply (`b[4] = 0x28`).
pub const READ_LEN: usize = 40;
/// `b[4]` of a write frame.
const WRITE_B4: u8 = 0x32;

const TYPE_DKS: u8 = 1;
/// Bytes a DKS payload uses: type, 4 routes, 4 x (entry + 4 phases).
const DKS_LEN: usize = 5 + 4 * 8;
const TYPE_MT: u8 = 2;
const TYPE_TGL: u8 = 3;

/// Mod-tap hold threshold range in 10 ms units (official UI: 150..1000 ms).
pub const MT_THRESHOLD: std::ops::RangeInclusive<u8> = 15..=100;
/// DKS phase actions.
pub const PHASE_NONE: u8 = 0;
pub const PHASE_TAP: u8 = 1;
pub const PHASE_PRESS: u8 = 2;
pub const PHASE_HOLD: u8 = 3;

/// One output key of a DKS.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DksKey {
    pub action: KeyAction,
    /// Action at each phase: passing route1 down, reaching route2 (bottom), leaving route3 up, passing route4 up.
    /// 0 nothing, 1 tap, 2 press and keep holding, 3 still held.
    pub phases: [u8; 4],
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum AdvancedKey {
    Empty,
    /// Dynamic keystroke: up to 4 output keys acting at 4 travel points.
    Dks {
        /// route1..route4 in 0.1 mm.
        routes: [u8; 4],
        keys: Vec<DksKey>,
    },
    /// Mod-tap: `hold` when held past `threshold` x 10 ms, else `tap`.
    ModTap { hold: KeyAction, tap: KeyAction, threshold: u8 },
    /// Tap latches `action` down, tap again releases it.
    Toggle { action: KeyAction },
    /// RS / RKRT or unknown types (not supported on this board), kept byte for byte (hex payload).
    Other { payload: String },
}

fn entry(p: &[u8], at: usize) -> KeyAction {
    KeyAction::from_entry(p[at..at + 4].try_into().expect("4 bytes"), None)
}

/// Decodes a slot payload (`b[5..]` of a `22 80` reply page).
pub fn decode(p: &[u8]) -> Result<AdvancedKey> {
    if p.len() < READ_LEN {
        return invalid(format!("advanced-key payload is {} bytes, expected {READ_LEN}", p.len()));
    }
    Ok(match p[0] {
        0 => AdvancedKey::Empty,
        // bytes past the 4 keys are not modelled: keep such a slot byte for byte
        TYPE_DKS if p[DKS_LEN..READ_LEN].iter().any(|b| *b != 0) => other(p),
        TYPE_DKS => {
            let mut keys: Vec<DksKey> = (0..4)
                .map(|j| DksKey { action: entry(p, 5 + 8 * j), phases: p[9 + 8 * j..13 + 8 * j].try_into().expect("4") })
                .collect();
            // only all-zero trailing positions are dropped (encode writes them as zeros), so gaps keep their place
            while keys.last().is_some_and(|k| k.action == KeyAction::Default && k.phases == [0; 4]) {
                keys.pop();
            }
            AdvancedKey::Dks { routes: [p[1], p[2], p[3], p[4]], keys }
        }
        TYPE_MT => AdvancedKey::ModTap { hold: entry(p, 1), tap: entry(p, 5), threshold: p[9] },
        TYPE_TGL => AdvancedKey::Toggle { action: entry(p, 1) },
        _ => other(p),
    })
}

fn other(p: &[u8]) -> AdvancedKey {
    AdvancedKey::Other { payload: frame::hex(&p[..READ_LEN]).replace(' ', "") }
}

/// Encodes a slot payload (`READ_LEN` bytes; the write frame pads with zeros).
pub fn encode(a: &AdvancedKey) -> Result<Vec<u8>> {
    let mut p = vec![0u8; READ_LEN];
    let put = |p: &mut Vec<u8>, at: usize, k: &KeyAction| -> Result<()> {
        if matches!(k, KeyAction::Macro(_) | KeyAction::Advanced(_)) {
            return invalid(format!("{k} cannot be used inside an advanced key"));
        }
        p[at..at + 4].copy_from_slice(&k.to_entry(None)?);
        Ok(())
    };
    match a {
        AdvancedKey::Empty => {}
        AdvancedKey::Dks { routes, keys } => {
            if keys.is_empty() || keys.len() > 4 {
                return invalid(format!("a DKS has 1 to 4 output keys, got {}", keys.len()));
            }
            if routes.contains(&0) {
                return invalid("DKS routes must be at least 0.1 mm");
            }
            p[0] = TYPE_DKS;
            p[1..5].copy_from_slice(routes);
            for (j, k) in keys.iter().enumerate() {
                if k.phases.iter().any(|v| *v > PHASE_HOLD) {
                    return invalid(format!("DKS key {j}: phase values are 0..=3"));
                }
                put(&mut p, 5 + 8 * j, &k.action)?;
                p[9 + 8 * j..13 + 8 * j].copy_from_slice(&k.phases);
            }
        }
        AdvancedKey::ModTap { hold, tap, threshold } => {
            if !MT_THRESHOLD.contains(threshold) {
                return invalid(format!("mod-tap threshold {threshold} is outside 15..=100 (x 10 ms)"));
            }
            p[0] = TYPE_MT;
            put(&mut p, 1, hold)?;
            put(&mut p, 5, tap)?;
            p[9] = *threshold;
        }
        AdvancedKey::Toggle { action } => {
            p[0] = TYPE_TGL;
            put(&mut p, 1, action)?;
        }
        AdvancedKey::Other { payload } => {
            if payload.len() != READ_LEN * 2 || !payload.bytes().all(|b| b.is_ascii_hexdigit()) {
                return invalid(format!("advanced-key payload must be {} hex digits", READ_LEN * 2));
            }
            for (i, b) in p.iter_mut().enumerate() {
                *b = u8::from_str_radix(&payload[2 * i..2 * i + 2], 16).expect("checked hex");
            }
        }
    }
    Ok(p)
}

impl AdvancedKey {
    /// Whether the slot carries bytes aulactl does not model (`Other`, or a `Raw` key action), which may only
    /// be kept, never written (`keymap::RAW_NOT_KEPT`).
    pub fn has_raw(&self) -> bool {
        let raw = |a: &KeyAction| matches!(a, KeyAction::Raw(_));
        match self {
            AdvancedKey::Empty => false,
            AdvancedKey::Other { .. } => true,
            AdvancedKey::Dks { keys, .. } => keys.iter().any(|k| raw(&k.action)),
            AdvancedKey::ModTap { hold, tap, .. } => raw(hold) || raw(tap),
            AdvancedKey::Toggle { action } => raw(action),
        }
    }

    /// Checks DKS routes (0.1 mm) against the board's full travel (`max_travel_01mm`, in 0.1 mm).
    pub fn check_travel(&self, max_travel_01mm: u8) -> Result<()> {
        if let AdvancedKey::Dks { routes, .. } = self {
            if let Some(r) = routes.iter().find(|r| **r > max_travel_01mm) {
                return invalid(format!(
                    "DKS route {r} (x 0.1 mm) is beyond the full travel of {max_travel_01mm} (x 0.1 mm)"
                ));
            }
        }
        Ok(())
    }
}

/// Slots with raw bytes must be exactly what `current` (the board's slots) holds.
pub fn check_raw_kept(slots: &[(u8, AdvancedKey)], current: &[AdvancedKey]) -> Result<()> {
    for (s, a) in slots {
        if a.has_raw() && current.get(*s as usize) != Some(a) {
            return invalid(format!("advanced-key slot {s}: {}", crate::keymap::RAW_NOT_KEPT));
        }
    }
    Ok(())
}

/// Full travel in 0.1 mm (the unit of DKS routes).
pub fn max_travel_01mm(l: &crate::trigger::TravelLimits) -> u8 {
    (l.to_mm(l.max) * 10.0).round().clamp(1.0, u8::MAX as f64) as u8
}

/// DKS with the official defaults: route1 = route4 = `actuation` (0.1 mm), route2 = route3 = max travel
/// - 0.4 mm (`max_travel_01mm` is in 0.1 mm).
pub fn dks_default_routes(actuation: u8, max_travel_01mm: u8) -> [u8; 4] {
    let bottom = max_travel_01mm.saturating_sub(4).max(1);
    [actuation, bottom, bottom, actuation]
}

/// Write frame for slot `slot`: `22 s 00 00 32 payload..`.
pub fn write_frame(slot: u8, a: &AdvancedKey) -> Result<Body> {
    check_slot(slot)?;
    let p = encode(a)?;
    let mut b = frame::request(CMD_ADVANCED, slot);
    b[4] = WRITE_B4;
    b[5..5 + p.len()].copy_from_slice(&p);
    Ok(b)
}

pub fn check_slot(slot: u8) -> Result<()> {
    if slot >= SLOTS {
        return invalid(format!("advanced-key slot {slot} is outside 0..{SLOTS}"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::body;

    fn key(s: &str) -> KeyAction {
        s.parse().unwrap()
    }

    #[test]
    fn owner_board_is_all_empty() {
        let page = body("22 80 00 05 28");
        assert_eq!(decode(&page[5..]).unwrap(), AdvancedKey::Empty);
        assert_eq!(encode(&AdvancedKey::Empty).unwrap(), vec![0; 40]);
    }

    #[test]
    fn dks_layout() {
        let a = AdvancedKey::Dks {
            routes: dks_default_routes(15, 34),
            keys: vec![
                DksKey { action: key("KeyA"), phases: [PHASE_TAP, 0, 0, 0] },
                DksKey { action: key("ShiftLeft"), phases: [PHASE_PRESS, PHASE_HOLD, PHASE_HOLD, 0] },
            ],
        };
        let p = encode(&a).unwrap();
        assert_eq!(&p[..5], &[1, 15, 30, 30, 15]);
        assert_eq!(&p[5..13], &[0, 4, 0, 0, 1, 0, 0, 0]);
        assert_eq!(&p[13..21], &[0, 0xe1, 0, 0, 2, 3, 3, 0]);
        assert_eq!(&p[21..], &[0; 19]);
        assert_eq!(decode(&p).unwrap(), a);
    }

    #[test]
    fn dks_gaps_and_phases_round_trip() {
        let mut p = vec![0u8; READ_LEN];
        p[..5].copy_from_slice(&[1, 15, 30, 30, 15]);
        p[5..13].copy_from_slice(&[0, 4, 0, 0, 1, 0, 0, 0]); // KeyA
        p[13..21].copy_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0]); // gap
        p[21..29].copy_from_slice(&[0, 5, 0, 0, 2, 3, 3, 0]); // KeyB in position 3
        let a = decode(&p).unwrap();
        let AdvancedKey::Dks { keys, .. } = &a else { panic!("{a:?}") };
        assert_eq!(keys.len(), 3, "gap kept, trailing empty position dropped");
        assert_eq!(keys[1], DksKey { action: KeyAction::Default, phases: [0; 4] });
        assert_eq!(encode(&a).unwrap(), p);
        // phases on an empty action are kept too
        p[29..37].copy_from_slice(&[0, 0, 0, 0, 0, 0, 1, 0]);
        let a = decode(&p).unwrap();
        assert!(matches!(&a, AdvancedKey::Dks { keys, .. } if keys.len() == 4));
        assert_eq!(encode(&a).unwrap(), p);
        // unknown bytes past the keys: kept raw
        p[38] = 7;
        let a = decode(&p).unwrap();
        assert!(matches!(a, AdvancedKey::Other { .. }));
        assert_eq!(encode(&a).unwrap(), p);
    }

    #[test]
    fn mod_tap_and_toggle() {
        let mt = AdvancedKey::ModTap { hold: key("ControlLeft"), tap: key("Escape"), threshold: 20 };
        let p = encode(&mt).unwrap();
        assert_eq!(&p[..10], &[2, 0, 0xe0, 0, 0, 0, 0x29, 0, 0, 20]);
        assert_eq!(decode(&p).unwrap(), mt);
        let t = AdvancedKey::Toggle { action: key("ShiftLeft") };
        let p = encode(&t).unwrap();
        assert_eq!(&p[..5], &[3, 0, 0xe1, 0, 0]);
        assert_eq!(decode(&p).unwrap(), t);
    }

    #[test]
    fn unknown_types_round_trip() {
        let mut p = vec![0u8; 40];
        p[0] = 4;
        p[10] = 2;
        let a = decode(&p).unwrap();
        assert!(matches!(a, AdvancedKey::Other { .. }));
        assert_eq!(encode(&a).unwrap(), p);
    }

    #[test]
    fn validation() {
        assert!(decode(&[0; 39]).is_err());
        let dks = |keys: Vec<DksKey>, routes| AdvancedKey::Dks { routes, keys };
        let k = DksKey { action: key("KeyA"), phases: [1, 0, 0, 0] };
        assert!(encode(&dks(vec![], [15, 30, 30, 15])).is_err());
        assert!(encode(&dks(vec![k.clone(); 5], [15, 30, 30, 15])).is_err());
        assert!(encode(&dks(vec![k.clone()], [0, 30, 30, 15])).is_err());
        assert!(encode(&dks(vec![DksKey { phases: [4, 0, 0, 0], ..k.clone() }], [15, 30, 30, 15])).is_err());
        assert!(encode(&dks(vec![k.clone(); 4], [1, 1, 1, 1])).is_ok());
        let mt = |threshold| AdvancedKey::ModTap { hold: key("ControlLeft"), tap: key("KeyA"), threshold };
        assert!(encode(&mt(14)).is_err() && encode(&mt(101)).is_err());
        assert!(encode(&mt(15)).is_ok() && encode(&mt(100)).is_ok());
        assert!(encode(&AdvancedKey::Toggle { action: key("Macro1") }).is_err());
        assert!(encode(&AdvancedKey::Toggle { action: key("Adv1") }).is_err());
        assert!(encode(&AdvancedKey::Other { payload: "00".into() }).is_err());
    }

    #[test]
    fn write_frames() {
        let f = write_frame(39, &AdvancedKey::Toggle { action: key("KeyA") }).unwrap();
        assert_eq!(&f[..10], &[0x22, 39, 0, 0, 0x32, 3, 0, 4, 0, 0]);
        assert!(frame::check_allowed(&f).is_ok());
        assert!(write_frame(40, &AdvancedKey::Empty).is_err());
        assert_eq!(dks_default_routes(15, 2), [15, 1, 1, 15]);
    }

    #[test]
    fn serde_shape() {
        let a = AdvancedKey::ModTap { hold: key("ControlLeft"), tap: key("Escape"), threshold: 20 };
        let j = serde_json::to_string(&a).unwrap();
        assert_eq!(j, r#"{"type":"mod-tap","hold":"ControlLeft","tap":"Escape","threshold":20}"#);
        assert_eq!(serde_json::from_str::<AdvancedKey>(&j).unwrap(), a);
        assert_eq!(serde_json::to_string(&AdvancedKey::Empty).unwrap(), r#"{"type":"empty"}"#);
    }

    #[test]
    fn raw_detection_and_kept_check() {
        let raw = KeyAction::Raw([0x99, 0, 0, 0]);
        let other = AdvancedKey::Other { payload: "09".repeat(READ_LEN) };
        assert!(!AdvancedKey::Empty.has_raw());
        assert!(other.has_raw());
        assert!(AdvancedKey::Toggle { action: raw.clone() }.has_raw());
        assert!(!AdvancedKey::Toggle { action: KeyAction::Key(4) }.has_raw());
        assert!(AdvancedKey::ModTap { hold: KeyAction::Key(4), tap: raw.clone(), threshold: 20 }.has_raw());
        assert!(AdvancedKey::Dks { routes: [1; 4], keys: vec![DksKey { action: raw, phases: [0; 4] }] }.has_raw());
        let current = vec![AdvancedKey::Empty, other.clone()];
        assert!(check_raw_kept(&[(1, other.clone())], &current).is_ok());
        assert!(check_raw_kept(&[(0, AdvancedKey::Empty)], &current).is_ok());
        assert!(check_raw_kept(&[(0, other.clone())], &current).is_err());
        assert!(check_raw_kept(&[(39, other)], &current).is_err(), "slot beyond the board's list");
        assert!(check_raw_kept(&[], &[]).is_ok());
    }

    #[test]
    fn dks_routes_within_travel() {
        let dks = |r: u8| AdvancedKey::Dks { routes: [15, r, r, 15], keys: vec![] };
        assert!(dks(34).check_travel(34).is_ok());
        assert!(dks(35).check_travel(34).is_err());
        assert!(dks(255).check_travel(34).is_err());
        assert!(AdvancedKey::Toggle { action: KeyAction::Key(4) }.check_travel(0).is_ok());
        let l = crate::trigger::TravelLimits { max: 340, min: 8, unit_num: 1, high_precision: false };
        assert_eq!(max_travel_01mm(&l), 34);
        assert_eq!(max_travel_01mm(&crate::trigger::TravelLimits { max: 4000, high_precision: true, ..l }), 40);
        assert_eq!(max_travel_01mm(&crate::trigger::TravelLimits { max: 0, ..l }), 1);
    }
}

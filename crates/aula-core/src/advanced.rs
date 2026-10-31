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
        TYPE_DKS => AdvancedKey::Dks {
            routes: [p[1], p[2], p[3], p[4]],
            keys: (0..4)
                .map(|j| DksKey { action: entry(p, 5 + 8 * j), phases: p[9 + 8 * j..13 + 8 * j].try_into().expect("4") })
                .filter(|k| k.action != KeyAction::Default)
                .collect(),
        },
        TYPE_MT => AdvancedKey::ModTap { hold: entry(p, 1), tap: entry(p, 5), threshold: p[9] },
        TYPE_TGL => AdvancedKey::Toggle { action: entry(p, 1) },
        _ => AdvancedKey::Other { payload: frame::hex(&p[..READ_LEN]).replace(' ', "") },
    })
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
}

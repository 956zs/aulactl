//! Keymap (protocol section 6): base and Fn layer, 4-byte entries, text names for every action.

use crate::error::{invalid, Error, Result};
use crate::frame::{TableSpec, MATRIX_KEYS};
use crate::keycodes::{self, is_modifier, name_of_usage, usage_by_name};
use crate::layout;
use crate::perkey::{key_name, PerKey};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

/// 132 entries x 4 bytes, 56-byte pages (protocol 6.1).
pub const TABLE: TableSpec = TableSpec { len: MATRIX_KEYS * 4, page: 56 };
/// Macro slots (protocol 7).
pub const MACRO_SLOTS: u8 = 10;
/// Advanced-key slots (protocol 8).
pub const ADVANCED_SLOTS: u8 = 40;

const T_KEY: u8 = 0x00;
const T_FN: u8 = 0x01;
const FN_CODE: u8 = 0xfa;
const T_MEDIA: u8 = 0x07;
const T_MACRO: u8 = 0x10;
const T_COMBO: u8 = 0x13;
const T_ADVANCED: u8 = 0x17;

/// Keymap layer and its selector in `b[1]` (read adds 0x80).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Layer {
    Base,
    Fn,
}

impl Layer {
    pub fn selector(self) -> u8 {
        match self {
            Layer::Base => 0,
            Layer::Fn => 2,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Layer::Base => "base",
            Layer::Fn => "fn",
        }
    }
}

/// Fixed-function entries with names: lighting, lock, mouse and the official predefined combinations
/// (protocol 6.3 / 6.4, wire order).
pub static NAMED: &[([u8; 4], &str)] = &[
    ([0x02, 0x02, 0, 0], "LightNextMode"),
    ([0x03, 0x00, 0, 0], "BrightnessUp"),
    ([0x03, 0x01, 0, 0], "BrightnessDown"),
    ([0x04, 0x00, 0, 0], "LightSpeedUp"),
    ([0x04, 0x01, 0, 0], "LightSpeedDown"),
    ([0x05, 0x02, 0, 0], "LightNextDirection"),
    ([0x06, 0x00, 0, 0], "LightFullColor"),
    ([0x06, 0x01, 0, 0], "LightNextColor"),
    ([0x06, 0x02, 0, 0], "LightNextBgColor"),
    ([0x11, 0x00, 0, 0], "LockKeyboard"),
    ([0x11, 0x02, 0, 0], "WinLock"),
    ([0x11, 0x04, 0, 0], "LightsOff"),
    ([0x12, 0x00, 0x01, 0], "MouseLeft"),
    ([0x12, 0x00, 0x02, 0], "MouseRight"),
    ([0x12, 0x00, 0x04, 0], "MouseMiddle"),
    ([0x12, 0x00, 0x08, 0], "MouseForward"),
    ([0x12, 0x00, 0x10, 0], "MouseBack"),
    ([0x12, 0x01, 0x01, 0], "WheelUp"),
    ([0x12, 0x01, 0xff, 0], "WheelDown"),
    ([0x13, 0xe3, 0x07, 0], "ShowDesktop"),
    ([0x13, 0xe2, 0x3d, 0], "CloseWindow"),
    ([0x13, 0xe3, 0x0f, 0], "LockPc"),
    ([0x13, 0xe0, 0x1d, 0], "Undo"),
    ([0x13, 0xe0, 0x06, 0], "Copy"),
    ([0x13, 0xe0, 0x19, 0], "Paste"),
    ([0x13, 0xe0, 0x1b, 0], "Cut"),
    ([0x13, 0xe0, 0x16, 0], "Save"),
    ([0x13, 0xe0, 0x11, 0], "New"),
    ([0x13, 0xe0, 0x13, 0], "Print"),
];

/// What a key does. Text form (Display / FromStr, also used in snapshots):
///
/// | variant | text |
/// |---|---|
/// | `Default` | `Default` (all-zero entry: firmware default; in the Fn layer the key's firmware Fn function) |
/// | `Key` | keycode name, `KeyA`, `Escape`, `ShiftLeft` (aliases like `a`, `Esc` parse too) |
/// | `Fn` | `Fn` |
/// | `Named` | a name from [`NAMED`], e.g. `BrightnessUp`, `MouseLeft`, `Copy` |
/// | `Media` | `VolumeUp`, `PlayPause`, ... ([`keycodes::MEDIA`]) |
/// | `Combo` | `ControlLeft+KeyC`, 2 or 3 keys, at least one modifier and one normal key |
/// | `Macro` | `Macro0` .. `Macro9` (slot) |
/// | `Advanced` | `Adv0` .. `Adv39` (slot) |
/// | `Raw` | `raw:10040300` (anything else, kept byte for byte) |
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyAction {
    Default,
    Key(u8),
    Fn,
    Named([u8; 4]),
    Media(u8),
    Combo(Vec<u8>),
    Macro(u8),
    Advanced(u8),
    Raw([u8; 4]),
}

impl KeyAction {
    /// Decodes an entry. `key` is the physical key the entry belongs to: a macro binding carries that key's
    /// default usage (protocol 7.2) and only decodes as `Macro` when it does.
    pub fn from_entry(e: [u8; 4], key: Option<usize>) -> KeyAction {
        if e == [0; 4] {
            return KeyAction::Default;
        }
        if let Some((n, _)) = NAMED.iter().find(|(n, _)| *n == e) {
            return KeyAction::Named(*n);
        }
        match e {
            [T_KEY, u, 0, 0] if name_of_usage(u).is_some() => KeyAction::Key(u),
            [T_FN, FN_CODE | 0, 0, 0] => KeyAction::Fn,
            [T_MEDIA, c, 0, 0] if keycodes::name_of_media(c).is_some() => KeyAction::Media(c),
            [T_MACRO, u, slot, 0] if slot < MACRO_SLOTS && key.and_then(macro_usage) == Some(u) => {
                KeyAction::Macro(slot)
            }
            [T_ADVANCED, s, s2, 0] if s == s2 && s < ADVANCED_SLOTS => KeyAction::Advanced(s),
            [T_COMBO, a, b, c] => {
                let keys: Vec<u8> = [a, b, c].into_iter().filter(|u| *u != 0).collect();
                if c != 0 && b == 0 || validate_combo(&keys).is_err() {
                    KeyAction::Raw(e)
                } else {
                    KeyAction::Combo(keys)
                }
            }
            _ => KeyAction::Raw(e),
        }
    }

    /// Encodes for the physical key `key` (needed only by `Macro`). Validates every field.
    pub fn to_entry(&self, key: Option<usize>) -> Result<[u8; 4]> {
        Ok(match self {
            KeyAction::Default => [0; 4],
            KeyAction::Key(u) => {
                if name_of_usage(*u).is_none() {
                    return invalid(format!("usage 0x{u:02x} is not in the keycode table"));
                }
                [T_KEY, *u, 0, 0]
            }
            KeyAction::Fn => [T_FN, FN_CODE, 0, 0],
            KeyAction::Named(e) => {
                if !NAMED.iter().any(|(n, _)| n == e) {
                    return invalid(format!("entry {e:02x?} is not a named function"));
                }
                *e
            }
            KeyAction::Media(c) => {
                if keycodes::name_of_media(*c).is_none() {
                    return invalid(format!("media code 0x{c:02x} is unknown"));
                }
                [T_MEDIA, *c, 0, 0]
            }
            KeyAction::Combo(keys) => {
                validate_combo(keys)?;
                [T_COMBO, keys[0], keys[1], keys.get(2).copied().unwrap_or(0)]
            }
            KeyAction::Macro(slot) => {
                if *slot >= MACRO_SLOTS {
                    return invalid(format!("macro slot {slot} is outside 0..{MACRO_SLOTS}"));
                }
                let Some(u) = key.and_then(macro_usage) else {
                    return invalid("a macro can only be bound to a physical key with a normal default key code");
                };
                [T_MACRO, u, *slot, 0]
            }
            KeyAction::Advanced(slot) => {
                if *slot >= ADVANCED_SLOTS {
                    return invalid(format!("advanced-key slot {slot} is outside 0..{ADVANCED_SLOTS}"));
                }
                [T_ADVANCED, *slot, *slot, 0]
            }
            KeyAction::Raw(e) => *e,
        })
    }
}

/// Usage a macro binding on `key` carries: the key's default HID usage.
fn macro_usage(key: usize) -> Option<u8> {
    layout::by_index(key).map(|k| k.default_usage()).filter(|u| *u != 0)
}

/// 2 or 3 named usages, at least one modifier and one normal key (the official UI's rule).
fn validate_combo(keys: &[u8]) -> Result<()> {
    if !(2..=3).contains(&keys.len()) {
        return invalid(format!("a key combination needs 2 or 3 keys, got {}", keys.len()));
    }
    if let Some(u) = keys.iter().find(|u| name_of_usage(**u).is_none()) {
        return invalid(format!("usage 0x{u:02x} is not in the keycode table"));
    }
    if !keys.iter().any(|u| is_modifier(*u)) || keys.iter().all(|u| is_modifier(*u)) {
        return invalid("a key combination needs at least one modifier and one normal key");
    }
    Ok(())
}

impl fmt::Display for KeyAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            KeyAction::Default => f.write_str("Default"),
            KeyAction::Key(u) => match name_of_usage(*u) {
                Some(n) => f.write_str(n),
                None => write!(f, "raw:00{u:02x}0000"),
            },
            KeyAction::Fn => f.write_str("Fn"),
            KeyAction::Named(e) => match NAMED.iter().find(|(n, _)| n == e) {
                Some((_, name)) => f.write_str(name),
                None => write!(f, "raw:{}", hex4(e)),
            },
            KeyAction::Media(c) => match keycodes::name_of_media(*c) {
                Some(n) => f.write_str(n),
                None => write!(f, "raw:07{c:02x}0000"),
            },
            KeyAction::Combo(keys) => {
                let names: Vec<String> = keys.iter().map(|u| keycodes::Usage(*u).to_string()).collect();
                f.write_str(&names.join("+"))
            }
            KeyAction::Macro(s) => write!(f, "Macro{s}"),
            KeyAction::Advanced(s) => write!(f, "Adv{s}"),
            KeyAction::Raw(e) => write!(f, "raw:{}", hex4(e)),
        }
    }
}

fn hex4(e: &[u8; 4]) -> String {
    e.iter().map(|b| format!("{b:02x}")).collect()
}

impl std::str::FromStr for KeyAction {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        let s = s.trim();
        let lower = s.to_ascii_lowercase();
        if lower == "default" {
            return Ok(KeyAction::Default);
        }
        if lower == "fn" {
            return Ok(KeyAction::Fn);
        }
        if let Some((e, _)) = NAMED.iter().find(|(_, n)| n.eq_ignore_ascii_case(s)) {
            return Ok(KeyAction::Named(*e));
        }
        if let Some(c) = keycodes::media_by_name(s) {
            return Ok(KeyAction::Media(c));
        }
        if let Some(n) = lower.strip_prefix("macro") {
            let slot: u8 = n.parse().map_err(|_| Error::Invalid(format!("bad macro slot in {s:?}")))?;
            if slot >= MACRO_SLOTS {
                return invalid(format!("macro slot {slot} is outside 0..{MACRO_SLOTS}"));
            }
            return Ok(KeyAction::Macro(slot));
        }
        if let Some(n) = lower.strip_prefix("adv") {
            let slot: u8 = n.parse().map_err(|_| Error::Invalid(format!("bad advanced-key slot in {s:?}")))?;
            if slot >= ADVANCED_SLOTS {
                return invalid(format!("advanced-key slot {slot} is outside 0..{ADVANCED_SLOTS}"));
            }
            return Ok(KeyAction::Advanced(slot));
        }
        if let Some(h) = lower.strip_prefix("raw:") {
            if h.len() != 8 || !h.bytes().all(|b| b.is_ascii_hexdigit()) {
                return invalid(format!("{s:?}: raw entries are 8 hex digits, e.g. raw:00040000"));
            }
            let mut e = [0u8; 4];
            for (i, b) in e.iter_mut().enumerate() {
                *b = u8::from_str_radix(&h[2 * i..2 * i + 2], 16).expect("checked hex");
            }
            return Ok(KeyAction::Raw(e));
        }
        if s.contains('+') {
            let keys = s
                .split('+')
                .map(|p| usage_by_name(p.trim()).ok_or_else(|| Error::Invalid(format!("unknown key {p:?} in {s:?}"))))
                .collect::<Result<Vec<u8>>>()?;
            let combo = KeyAction::Combo(keys);
            // canonical form: `Ctrl+C` is the predefined `Copy`
            return Ok(KeyAction::from_entry(combo.to_entry(None)?, None));
        }
        if let Some(u) = usage_by_name(s) {
            return Ok(KeyAction::Key(u));
        }
        invalid(format!("unknown key action {s:?}"))
    }
}

impl Serialize for KeyAction {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for KeyAction {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

/// One layer: an action per physical key.
///
/// In the base layer an all-zero entry (never-written board) decodes as the key's default action, so a
/// fresh board and one written by the official driver compare equal. In the Fn layer it stays `Default`.
pub type Keymap = PerKey<KeyAction>;

/// Decodes a 528-byte layer table. Non-key slots are ignored.
pub fn decode_layer(layer: Layer, table: &[u8]) -> Result<Keymap> {
    if table.len() != TABLE.len {
        return invalid(format!("keymap table is {} bytes, expected {}", table.len(), TABLE.len));
    }
    Ok(PerKey::from_fn(|k| {
        let i = k.index as usize;
        let e: [u8; 4] = table[i * 4..i * 4 + 4].try_into().expect("4 bytes");
        if layer == Layer::Base && e == [0; 4] {
            KeyAction::from_entry(k.default_entry, Some(i))
        } else {
            KeyAction::from_entry(e, Some(i))
        }
    }))
}

/// Encodes a layer for writing. Keys missing from `map` get their default (base: the layout's default entry,
/// Fn: zero). In the base layer `Default` is written as the explicit default entry, like the official driver
/// (protocol 6.1). Non-key slots stay zero.
pub fn encode_layer(layer: Layer, map: &Keymap) -> Result<Vec<u8>> {
    let mut table = vec![0u8; TABLE.len];
    for (idx, _) in map.iter() {
        if !layout::is_key(idx) {
            return invalid(format!("index {idx} is not a key"));
        }
    }
    for k in &layout::KEYS {
        let i = k.index as usize;
        let e = match map.get(i) {
            Some(KeyAction::Default) | None if layer == Layer::Base => k.default_entry,
            Some(a) => a.to_entry(Some(i)).map_err(|e| Error::Invalid(format!("{}: {e}", k.code)))?,
            None => [0; 4],
        };
        table[i * 4..i * 4 + 4].copy_from_slice(&e);
    }
    Ok(table)
}

/// Checks a Fn layer against the board's Fn-fixed mask (`0x0E`): fixed keys must stay `Default`.
pub fn check_fn_fixed(map: &Keymap, fixed: &[usize]) -> Result<()> {
    for (idx, a) in map.iter() {
        if fixed.contains(&idx) && *a != KeyAction::Default {
            return invalid(format!("{} has a fixed Fn function and cannot be remapped in the Fn layer", key_name(idx)));
        }
    }
    Ok(())
}

/// Error for a raw entry or payload that is not what the board already holds.
pub const RAW_NOT_KEPT: &str = "raw entries can only be kept, not written";

/// Raw entries are only ever written back unchanged: each `Raw` in `map` must be what `current` (the board's
/// layer) holds for that key.
pub fn check_raw_kept(map: &Keymap, current: &Keymap) -> Result<()> {
    for (idx, a) in map.iter() {
        if matches!(a, KeyAction::Raw(_)) && current.get(idx) != Some(a) {
            return invalid(format!("{}: {a}: {RAW_NOT_KEPT}", key_name(idx)));
        }
    }
    Ok(())
}

pub fn has_raw(map: &Keymap) -> bool {
    map.iter().any(|(_, a)| matches!(a, KeyAction::Raw(_)))
}

/// Read request selector for a layer.
pub fn read_selector(layer: Layer) -> u8 {
    0x80 | layer.selector()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::body;

    fn parse(s: &str) -> KeyAction {
        s.parse().unwrap()
    }

    #[test]
    fn entries_decode_by_type() {
        assert_eq!(KeyAction::from_entry([0, 0x29, 0, 0], None), KeyAction::Key(0x29));
        assert_eq!(KeyAction::from_entry([1, 0xfa, 0, 0], None), KeyAction::Fn);
        assert_eq!(KeyAction::from_entry([1, 0, 0, 0], None), KeyAction::Fn, "never-written Fn key");
        assert_eq!(KeyAction::from_entry([7, 0x10, 0, 0], None), KeyAction::Media(0x10));
        assert_eq!(KeyAction::from_entry([0x17, 5, 5, 0], None), KeyAction::Advanced(5));
        assert_eq!(KeyAction::from_entry([0x13, 0xe0, 0x06, 0], None), KeyAction::Named([0x13, 0xe0, 6, 0]));
        assert_eq!(KeyAction::from_entry([0x13, 0xe1, 0x04, 0], None), KeyAction::Combo(vec![0xe1, 4]));
        assert_eq!(KeyAction::from_entry([0x12, 0, 0x08, 0], None).to_string(), "MouseForward");
        assert_eq!(KeyAction::from_entry([0, 0, 0, 0], None), KeyAction::Default);
    }

    #[test]
    fn macro_binding_carries_the_key_default_usage() {
        // capture: macro slot 0 bound to KeyA (idx 68)
        assert_eq!(KeyAction::from_entry([0x10, 0x04, 0, 0], Some(68)), KeyAction::Macro(0));
        assert_eq!(KeyAction::Macro(3).to_entry(Some(68)).unwrap(), [0x10, 0x04, 3, 0]);
        // wrong usage for the key, unknown key, slot out of range: kept raw
        assert_eq!(KeyAction::from_entry([0x10, 0x05, 0, 0], Some(68)), KeyAction::Raw([0x10, 5, 0, 0]));
        assert_eq!(KeyAction::from_entry([0x10, 0x04, 0, 0], None), KeyAction::Raw([0x10, 4, 0, 0]));
        assert_eq!(KeyAction::from_entry([0x10, 0x04, 10, 0], Some(68)), KeyAction::Raw([0x10, 4, 10, 0]));
        assert!(KeyAction::Macro(0).to_entry(None).is_err());
        assert!(KeyAction::Macro(0).to_entry(Some(122)).is_err(), "Fn has no default usage");
        assert!(KeyAction::Macro(10).to_entry(Some(68)).is_err());
    }

    #[test]
    fn odd_entries_stay_raw() {
        for e in [[0, 0x01, 0, 0], [0, 4, 1, 0], [7, 0x99, 0, 0], [0x17, 1, 2, 0], [0x17, 40, 40, 0], [0x18, 1, 0, 0],
            [0x13, 4, 5, 0], [0x13, 0xe0, 0, 4], [0x12, 0, 3, 0], [0x13, 0xe0, 0xe1, 0]]
        {
            let a = KeyAction::from_entry(e, Some(68));
            assert_eq!(a, KeyAction::Raw(e), "{e:02x?}");
            assert_eq!(a.to_entry(Some(68)).unwrap(), e);
            assert_eq!(a.to_string().parse::<KeyAction>().unwrap(), a);
        }
    }

    #[test]
    fn every_decodable_entry_round_trips_through_text() {
        let mut entries: Vec<[u8; 4]> = NAMED.iter().map(|(e, _)| *e).collect();
        entries.extend(keycodes::USAGES.iter().map(|(u, _)| [0, *u, 0, 0]));
        entries.extend(keycodes::MEDIA.iter().map(|(c, _)| [7, *c, 0, 0]));
        entries.extend([[1, 0xfa, 0, 0], [0x10, 4, 9, 0], [0x17, 39, 39, 0], [0x13, 0xe0, 0xe1, 0x04], [0, 0, 0, 0]]);
        for e in entries {
            let a = KeyAction::from_entry(e, Some(68));
            let text = a.to_string();
            let back: KeyAction = text.parse().unwrap();
            assert_eq!(back, a, "{text}");
            assert_eq!(back.to_entry(Some(68)).unwrap(), e, "{text}");
        }
    }

    #[test]
    fn text_parsing() {
        assert_eq!(parse("KeyA"), KeyAction::Key(4));
        assert_eq!(parse("esc"), KeyAction::Key(0x29));
        assert_eq!(parse(" volumeup "), KeyAction::Media(0x10));
        assert_eq!(parse("Macro3"), KeyAction::Macro(3));
        assert_eq!(parse("adv39"), KeyAction::Advanced(39));
        assert_eq!(parse("Ctrl+Shift+Esc"), KeyAction::Combo(vec![0xe0, 0xe1, 0x29]));
        assert_eq!(parse("Ctrl+c"), parse("Copy"), "canonical predefined name");
        assert_eq!(parse("Copy").to_entry(None).unwrap(), [0x13, 0xe0, 0x06, 0]);
        assert_eq!(parse("WheelDown").to_entry(None).unwrap(), [0x12, 0x01, 0xff, 0]);
        assert_eq!(parse("raw:180100ff"), KeyAction::Raw([0x18, 1, 0, 0xff]));
        assert_eq!(parse("Default"), KeyAction::Default);
        assert_eq!(parse("FN"), KeyAction::Fn);
    }

    #[test]
    fn text_errors() {
        for bad in ["", "Macro10", "Macro", "Adv40", "advx", "raw:123", "raw:zzzzzzzz", "KeyA+KeyB", "Ctrl+Shift",
            "Ctrl+A+B+C", "Ctrl+Nope", "Nope", "+"]
        {
            assert!(bad.parse::<KeyAction>().is_err(), "{bad:?}");
        }
    }

    #[test]
    fn names_do_not_collide() {
        for (_, n) in NAMED {
            assert!(usage_by_name(n).is_none() && keycodes::media_by_name(n).is_none(), "{n}");
            assert_eq!(NAMED.iter().filter(|(_, m)| m.eq_ignore_ascii_case(n)).count(), 1);
        }
    }

    #[test]
    fn encode_validates() {
        assert!(KeyAction::Key(0x01).to_entry(None).is_err());
        assert!(KeyAction::Media(0x99).to_entry(None).is_err());
        assert!(KeyAction::Named([9, 9, 9, 9]).to_entry(None).is_err());
        assert!(KeyAction::Combo(vec![4]).to_entry(None).is_err());
        assert!(KeyAction::Advanced(40).to_entry(None).is_err());
    }

    /// Owner's never-written board: all zero except Fn = `01 00 00 00` (Appendix A, page 8 offset 40).
    fn owner_base_table() -> Vec<u8> {
        let mut pages = Vec::new();
        for p in 0..10u8 {
            let len = if p == 9 { 0x18 } else { 0x38 };
            let mut b = body(&format!("18 80 00 {p:02x} {len:02x}"));
            if p == 8 {
                b[5 + 40] = 0x01;
            }
            pages.push(b);
        }
        TABLE.assemble(&pages).unwrap()
    }

    #[test]
    fn fresh_board_decodes_to_defaults() {
        let base = decode_layer(Layer::Base, &owner_base_table()).unwrap();
        assert_eq!(base.len(), 61);
        assert_eq!(base.get(22), Some(&KeyAction::Key(0x29)));
        assert_eq!(base.get(122), Some(&KeyAction::Fn));
        let fn_layer = decode_layer(Layer::Fn, &vec![0; 528]).unwrap();
        assert!(fn_layer.iter().all(|(_, a)| *a == KeyAction::Default));
    }

    #[test]
    fn base_layer_encodes_explicit_defaults() {
        let base = decode_layer(Layer::Base, &owner_base_table()).unwrap();
        let t = encode_layer(Layer::Base, &base).unwrap();
        assert_eq!(&t[22 * 4..22 * 4 + 4], &[0, 0x29, 0, 0]);
        assert_eq!(&t[122 * 4..122 * 4 + 4], &[1, 0xfa, 0, 0]);
        assert_eq!(&t[0..4], &[0; 4], "non-key slot");
        assert_eq!(decode_layer(Layer::Base, &t).unwrap(), base);
        // empty map = every default
        assert_eq!(encode_layer(Layer::Base, &PerKey::new()).unwrap(), t);
        assert_eq!(encode_layer(Layer::Fn, &PerKey::new()).unwrap(), vec![0; 528]);
    }

    #[test]
    fn layer_remap_round_trip() {
        let mut m = PerKey::new();
        m.0.insert(66, KeyAction::Key(0x29)); // Caps -> Esc
        m.0.insert(68, KeyAction::Macro(2));
        let t = encode_layer(Layer::Base, &m).unwrap();
        assert_eq!(&t[66 * 4..66 * 4 + 4], &[0, 0x29, 0, 0]);
        assert_eq!(&t[68 * 4..68 * 4 + 4], &[0x10, 0x04, 2, 0]);
        let back = decode_layer(Layer::Base, &t).unwrap();
        assert_eq!(back.get(66), Some(&KeyAction::Key(0x29)));
        assert_eq!(back.get(68), Some(&KeyAction::Macro(2)));
    }

    #[test]
    fn layer_errors() {
        assert!(decode_layer(Layer::Base, &[]).is_err());
        assert!(decode_layer(Layer::Base, &[0; 527]).is_err());
        let mut m = PerKey::new();
        m.0.insert(0, KeyAction::Key(4));
        assert!(encode_layer(Layer::Base, &m).is_err(), "non-key index");
        let mut m = PerKey::new();
        m.0.insert(122, KeyAction::Macro(0));
        assert!(encode_layer(Layer::Base, &m).is_err());
    }

    #[test]
    fn fn_fixed_check() {
        let mut m = PerKey::new();
        m.0.insert(46, KeyAction::Default);
        m.0.insert(45, KeyAction::Key(4));
        assert!(check_fn_fixed(&m, &[46]).is_ok());
        assert!(check_fn_fixed(&m, &[]).is_ok());
        assert!(check_fn_fixed(&m, &[45]).is_err());
    }

    #[test]
    fn serde_as_text() {
        let mut m = PerKey::new();
        m.0.insert(66, KeyAction::Combo(vec![0xe0, 0x05]));
        let j = serde_json::to_string(&m).unwrap();
        assert_eq!(j, r#"{"CapsLock":"ControlLeft+KeyB"}"#);
        assert_eq!(serde_json::from_str::<Keymap>(&j).unwrap(), m);
        assert!(serde_json::from_str::<Keymap>(r#"{"CapsLock":"Bogus"}"#).is_err());
    }

    #[test]
    fn selectors() {
        assert_eq!(read_selector(Layer::Base), 0x80);
        assert_eq!(read_selector(Layer::Fn), 0x82);
        assert_eq!(Layer::Fn.selector(), 2);
    }

    #[test]
    fn raw_entries_must_be_kept() {
        let mut board = Keymap::new();
        board.0.insert(68, KeyAction::Raw([0x10, 5, 0, 0]));
        let mut m = board.clone();
        assert!(has_raw(&m));
        assert!(check_raw_kept(&m, &board).is_ok());
        m.0.insert(66, KeyAction::Key(0x29));
        assert!(check_raw_kept(&m, &board).is_ok(), "other keys may change");
        m.0.insert(69, KeyAction::Raw([1, 2, 3, 4]));
        assert!(check_raw_kept(&m, &board).unwrap_err().to_string().contains(RAW_NOT_KEPT));
        m.0.remove(&69);
        m.0.insert(68, KeyAction::Raw([0x10, 6, 0, 0]));
        assert!(check_raw_kept(&m, &board).is_err());
        assert!(check_raw_kept(&Keymap::new(), &Keymap::new()).is_ok());
        assert!(!has_raw(&Keymap::new()));
    }
}

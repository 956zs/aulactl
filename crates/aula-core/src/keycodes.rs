//! Keycode names (protocol 6.4, official `config/keys.json`).
//!
//! Keyboard usages are named like `KeyboardEvent.code` (`KeyA`, `Digit1`, `Escape`, `ShiftLeft`), which is
//! also the `code` column of the layout table. Parsing is case-insensitive and accepts a few short aliases.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

/// HID keyboard usages (page 7) offered by the official driver, with their canonical names.
pub static USAGES: &[(u8, &str)] = &[
    (0x04, "KeyA"), (0x05, "KeyB"), (0x06, "KeyC"), (0x07, "KeyD"), (0x08, "KeyE"), (0x09, "KeyF"),
    (0x0a, "KeyG"), (0x0b, "KeyH"), (0x0c, "KeyI"), (0x0d, "KeyJ"), (0x0e, "KeyK"), (0x0f, "KeyL"),
    (0x10, "KeyM"), (0x11, "KeyN"), (0x12, "KeyO"), (0x13, "KeyP"), (0x14, "KeyQ"), (0x15, "KeyR"),
    (0x16, "KeyS"), (0x17, "KeyT"), (0x18, "KeyU"), (0x19, "KeyV"), (0x1a, "KeyW"), (0x1b, "KeyX"),
    (0x1c, "KeyY"), (0x1d, "KeyZ"),
    (0x1e, "Digit1"), (0x1f, "Digit2"), (0x20, "Digit3"), (0x21, "Digit4"), (0x22, "Digit5"),
    (0x23, "Digit6"), (0x24, "Digit7"), (0x25, "Digit8"), (0x26, "Digit9"), (0x27, "Digit0"),
    (0x28, "Enter"), (0x29, "Escape"), (0x2a, "Backspace"), (0x2b, "Tab"), (0x2c, "Space"),
    (0x2d, "Minus"), (0x2e, "Equal"), (0x2f, "BracketLeft"), (0x30, "BracketRight"), (0x31, "Backslash"),
    (0x33, "Semicolon"), (0x34, "Quote"), (0x35, "Backquote"), (0x36, "Comma"), (0x37, "Period"),
    (0x38, "Slash"), (0x39, "CapsLock"),
    (0x3a, "F1"), (0x3b, "F2"), (0x3c, "F3"), (0x3d, "F4"), (0x3e, "F5"), (0x3f, "F6"),
    (0x40, "F7"), (0x41, "F8"), (0x42, "F9"), (0x43, "F10"), (0x44, "F11"), (0x45, "F12"),
    (0x46, "PrintScreen"), (0x47, "ScrollLock"), (0x48, "Pause"), (0x49, "Insert"), (0x4a, "Home"),
    (0x4b, "PageUp"), (0x4c, "Delete"), (0x4d, "End"), (0x4e, "PageDown"),
    (0x4f, "ArrowRight"), (0x50, "ArrowLeft"), (0x51, "ArrowDown"), (0x52, "ArrowUp"),
    (0x53, "NumLock"), (0x54, "NumpadDivide"), (0x55, "NumpadMultiply"), (0x56, "NumpadSubtract"),
    (0x57, "NumpadAdd"), (0x58, "NumpadEnter"),
    (0x59, "Numpad1"), (0x5a, "Numpad2"), (0x5b, "Numpad3"), (0x5c, "Numpad4"), (0x5d, "Numpad5"),
    (0x5e, "Numpad6"), (0x5f, "Numpad7"), (0x60, "Numpad8"), (0x61, "Numpad9"), (0x62, "Numpad0"),
    (0x63, "NumpadDecimal"), (0x64, "IntlBackslash"), (0x65, "ContextMenu"),
    (0x68, "F13"), (0x69, "F14"), (0x6a, "F15"), (0x6b, "F16"), (0x6c, "F17"), (0x6d, "F18"),
    (0x6e, "F19"), (0x6f, "F20"), (0x70, "F21"), (0x71, "F22"), (0x72, "F23"), (0x73, "F24"),
    (0x87, "IntlRo"), (0x88, "KanaMode"), (0x89, "IntlYen"), (0x8a, "Convert"), (0x8b, "NonConvert"),
    (0xe0, "ControlLeft"), (0xe1, "ShiftLeft"), (0xe2, "AltLeft"), (0xe3, "MetaLeft"),
    (0xe4, "ControlRight"), (0xe5, "ShiftRight"), (0xe6, "AltRight"), (0xe7, "MetaRight"),
];

/// Short aliases accepted when parsing (single letters and digits are handled separately).
static ALIASES: &[(&str, u8)] = &[
    ("Esc", 0x29), ("Return", 0x28), ("Bksp", 0x2a), ("Caps", 0x39), ("Grave", 0x35),
    ("Ins", 0x49), ("Del", 0x4c), ("PgUp", 0x4b), ("PgDn", 0x4e), ("PrtSc", 0x46),
    ("Right", 0x4f), ("Left", 0x50), ("Down", 0x51), ("Up", 0x52), ("Menu", 0x65), ("App", 0x65),
    ("LCtrl", 0xe0), ("LShift", 0xe1), ("LAlt", 0xe2), ("LWin", 0xe3), ("Win", 0xe3),
    ("RCtrl", 0xe4), ("RShift", 0xe5), ("RAlt", 0xe6), ("RWin", 0xe7),
    ("Ctrl", 0xe0), ("Shift", 0xe1), ("Alt", 0xe2),
];

/// Media / system functions (`t = 0x07`, vendor index, not consumer usages).
pub static MEDIA: &[(u8, &str)] = &[
    (0x10, "VolumeUp"), (0x11, "VolumeDown"), (0x12, "Mute"), (0x13, "PlayPause"), (0x14, "NextTrack"),
    (0x15, "PrevTrack"), (0x16, "MediaStop"), (0x20, "Mail"), (0x21, "BrowserSearch"),
    (0x22, "BrowserFavorites"), (0x23, "BrowserHome"), (0x24, "BrowserForward"), (0x25, "BrowserBack"),
    (0x26, "BrowserStop"), (0x27, "BrowserRefresh"), (0x30, "MediaPlayer"), (0x32, "Calculator"),
    (0x34, "MyComputer"),
];

/// Canonical name of a keyboard usage.
pub fn name_of_usage(usage: u8) -> Option<&'static str> {
    USAGES.iter().find(|(u, _)| *u == usage).map(|(_, n)| *n)
}

/// Keyboard usage for a name: canonical (`KeyA`), alias (`Esc`), single letter or digit (`a`, `1`).
/// Case-insensitive.
pub fn usage_by_name(name: &str) -> Option<u8> {
    if let Some((u, _)) = USAGES.iter().find(|(_, n)| n.eq_ignore_ascii_case(name)) {
        return Some(*u);
    }
    if let Some((_, u)) = ALIASES.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)) {
        return Some(*u);
    }
    match name.as_bytes() {
        [c] if c.is_ascii_alphabetic() => Some(0x04 + (c.to_ascii_lowercase() - b'a')),
        [b'0'] => Some(0x27),
        [c @ b'1'..=b'9'] => Some(0x1e + (c - b'1')),
        _ => None,
    }
}

/// Whether a usage is a modifier (`0xE0..=0xE7`).
pub fn is_modifier(usage: u8) -> bool {
    (0xe0..=0xe7).contains(&usage)
}

pub fn name_of_media(code: u8) -> Option<&'static str> {
    MEDIA.iter().find(|(c, _)| *c == code).map(|(_, n)| *n)
}

pub fn media_by_name(name: &str) -> Option<u8> {
    MEDIA.iter().find(|(_, n)| n.eq_ignore_ascii_case(name)).map(|(c, _)| *c)
}

/// A keyboard usage that serialises as its name (`"KeyA"`), or `"0xNN"` when it has none.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Usage(pub u8);

impl fmt::Display for Usage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match name_of_usage(self.0) {
            Some(n) => f.write_str(n),
            None => write!(f, "0x{:02x}", self.0),
        }
    }
}

impl std::str::FromStr for Usage {
    type Err = crate::Error;

    fn from_str(s: &str) -> crate::Result<Self> {
        if let Some(h) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
            if let Ok(v) = u8::from_str_radix(h, 16) {
                return Ok(Usage(v));
            }
        }
        usage_by_name(s).map(Usage).ok_or_else(|| crate::Error::Invalid(format!("unknown key name {s:?}")))
    }
}

impl Serialize for Usage {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Usage {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_unique_and_round_trip() {
        for (u, n) in USAGES {
            assert_eq!(usage_by_name(n), Some(*u), "{n}");
            assert_eq!(name_of_usage(*u), Some(*n));
            assert_eq!(USAGES.iter().filter(|(_, m)| m.eq_ignore_ascii_case(n)).count(), 1);
        }
        for (c, n) in MEDIA {
            assert_eq!(media_by_name(n), Some(*c));
            assert_eq!(name_of_media(*c), Some(*n));
            assert!(usage_by_name(n).is_none(), "media name {n} collides with a key name");
        }
        assert_eq!(USAGES.len(), 26 + 10 + 17 + 12 + 9 + 4 + 6 + 10 + 3 + 12 + 5 + 8);
    }

    #[test]
    fn aliases_letters_digits() {
        assert_eq!(usage_by_name("a"), Some(0x04));
        assert_eq!(usage_by_name("Z"), Some(0x1d));
        assert_eq!(usage_by_name("1"), Some(0x1e));
        assert_eq!(usage_by_name("9"), Some(0x26));
        assert_eq!(usage_by_name("0"), Some(0x27));
        assert_eq!(usage_by_name("esc"), Some(0x29));
        assert_eq!(usage_by_name("WIN"), Some(0xe3));
        assert_eq!(usage_by_name("escape"), Some(0x29));
        assert_eq!(usage_by_name(""), None);
        assert_eq!(usage_by_name("Key"), None);
        assert_eq!(usage_by_name("F25"), None);
        assert_eq!(usage_by_name("ä"), None);
        for (n, u) in ALIASES {
            assert!(name_of_usage(*u).is_some(), "alias {n}");
        }
    }

    #[test]
    fn modifiers() {
        assert!(is_modifier(0xe0) && is_modifier(0xe7));
        assert!(!is_modifier(0xdf) && !is_modifier(0xe8) && !is_modifier(0x04));
    }

    #[test]
    fn usage_text_and_serde() {
        assert_eq!(Usage(0x04).to_string(), "KeyA");
        assert_eq!(Usage(0x01).to_string(), "0x01");
        assert_eq!("0x01".parse::<Usage>().unwrap(), Usage(1));
        assert_eq!("keya".parse::<Usage>().unwrap(), Usage(4));
        assert!("0xzz".parse::<Usage>().is_err());
        assert!("".parse::<Usage>().is_err());
        assert_eq!(serde_json::to_string(&Usage(0x07)).unwrap(), "\"KeyD\"");
        assert_eq!(serde_json::from_str::<Usage>("\"0xff\"").unwrap(), Usage(0xff));
        assert!(serde_json::from_str::<Usage>("\"nope\"").is_err());
    }
}

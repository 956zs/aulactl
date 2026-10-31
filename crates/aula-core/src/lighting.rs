//! Lighting (protocol section 5): effect, mode list, per-key RGB slots, active slot, music rhythm.

use crate::error::{invalid, protocol, Error, Result};
use crate::frame::{self, payload, payload_min, Body, TableSpec, CMD_CUSTOM_RGB, CMD_EFFECT, CMD_MUSIC, MATRIX_KEYS};
use crate::layout;
use crate::perkey::{key_name, PerKey};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

/// Per-key RGB table: 132 x RGB, 54-byte pages (protocol 5.5).
pub const RGB_TABLE: TableSpec = TableSpec { len: MATRIX_KEYS * 3, page: 54 };
/// Custom (per-key) slots.
pub const CUSTOM_SLOTS: u8 = 2;
/// Mode id of Custom (per-key) lighting.
pub const MODE_CUSTOM: u8 = 10;
/// Mode id of music rhythm (host plugin; not offered by aulactl).
pub const MODE_MUSIC: u8 = 100;
/// Marker between mode ids and the ranges in a mode-list reply.
const MODE_LIST_MARKER: [u8; 2] = [0xa5, 0x5a];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    pub const BLACK: Rgb = Rgb(0, 0, 0);
}

impl fmt::Display for Rgb {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{:02x}{:02x}{:02x}", self.0, self.1, self.2)
    }
}

impl std::str::FromStr for Rgb {
    type Err = Error;

    /// `#rrggbb` or `rrggbb`.
    fn from_str(s: &str) -> Result<Self> {
        let h = s.trim().trim_start_matches('#');
        if h.len() != 6 || !h.bytes().all(|b| b.is_ascii_hexdigit()) {
            return invalid(format!("bad colour {s:?} (expected #rrggbb)"));
        }
        let c = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).expect("checked hex");
        Ok(Rgb(c(0), c(2), c(4)))
    }
}

impl Serialize for Rgb {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Rgb {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        String::deserialize(d)?.parse().map_err(serde::de::Error::custom)
    }
}

/// Which colour controls a mode offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Colors {
    None,
    /// Foreground only, no background, no full-colour.
    Fg,
    /// Foreground, background and full-colour.
    FgBg,
}

/// A lighting mode and its controls (protocol 5.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModeInfo {
    pub id: u8,
    pub name: &'static str,
    pub speed: bool,
    pub colors: Colors,
    /// Direction values offered, with names. Empty: the mode sends direction 0.
    pub directions: &'static [(u8, &'static str)],
}

const DIR_WAVE: &[(u8, &str)] = &[(0, "right"), (1, "left"), (2, "up"), (3, "down"), (4, "spread"), (5, "gather")];
const DIR_RADAR: &[(u8, &str)] = &[(0, "clockwise"), (1, "anticlockwise")];
const DIR_AURORA: &[(u8, &str)] = &[(4, "spread"), (5, "gather")];
const DIR_SPEED: &[(u8, &str)] = &[(2, "up"), (3, "down")];
const DIR_STRIATION: &[(u8, &str)] = &[(0, "right"), (1, "left")];

const fn mode(id: u8, name: &'static str, speed: bool, colors: Colors, directions: &'static [(u8, &'static str)]) -> ModeInfo {
    ModeInfo { id, name, speed, colors, directions }
}

/// Every mode the official driver knows, except music rhythm (100), which needs a vendor desktop plugin.
pub static MODES: &[ModeInfo] = &[
    mode(0, "static", false, Colors::Fg, &[]),
    mode(1, "breath", true, Colors::FgBg, &[]),
    mode(2, "wave", true, Colors::FgBg, DIR_WAVE),
    mode(3, "neon", true, Colors::FgBg, &[]),
    mode(4, "radar", true, Colors::FgBg, DIR_RADAR),
    mode(6, "reactive", true, Colors::FgBg, &[]),
    mode(7, "aurora", true, Colors::FgBg, DIR_AURORA),
    mode(8, "ripple", true, Colors::FgBg, &[]),
    mode(9, "twinkle", true, Colors::FgBg, &[]),
    mode(MODE_CUSTOM, "custom", false, Colors::None, &[]),
    mode(11, "cross", true, Colors::FgBg, &[]),
    mode(12, "speed-respond", false, Colors::FgBg, DIR_SPEED),
    mode(14, "auto-ripple", true, Colors::FgBg, &[]),
    mode(15, "striation", true, Colors::FgBg, DIR_STRIATION),
    mode(16, "fireworks", true, Colors::FgBg, &[]),
];

pub fn mode_info(id: u8) -> Option<&'static ModeInfo> {
    MODES.iter().find(|m| m.id == id)
}

/// Mode by name (case-insensitive) or numeric id.
pub fn mode_by_name(name: &str) -> Option<&'static ModeInfo> {
    let n = name.trim();
    MODES
        .iter()
        .find(|m| m.name.eq_ignore_ascii_case(n))
        .or_else(|| n.parse::<u8>().ok().and_then(mode_info))
}

/// The `0x0A` (keyboard) or `0x02` (side light) mode list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModeList {
    /// Mode ids the board advertises (Custom 10 is not listed but always available).
    pub modes: Vec<u8>,
    pub max_speed: u8,
    pub max_brightness: u8,
}

impl ModeList {
    /// Advertised modes plus Custom, as the official UI offers them (without music rhythm).
    pub fn selectable(&self) -> Vec<u8> {
        let mut v = self.modes.clone();
        if !v.contains(&MODE_CUSTOM) {
            v.push(MODE_CUSTOM);
        }
        v
    }
}

/// Payload `ids.., A5 5A, max_speed, max_brightness`.
pub fn parse_mode_list(b: &Body) -> Result<ModeList> {
    let p = payload(b)?;
    let Some(at) = p.windows(2).position(|w| w == MODE_LIST_MARKER) else {
        return protocol(format!("mode list {} has no A5 5A marker", frame::hex(p)));
    };
    let Some(&[max_speed, max_brightness]) = p.get(at + 2..at + 4) else {
        return protocol("mode list ends after the marker");
    };
    Ok(ModeList { modes: p[..at].to_vec(), max_speed, max_brightness })
}

/// The lighting effect (`0x07`, protocol 5.3 / 5.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Effect {
    pub mode: u8,
    pub brightness: u8,
    pub speed: u8,
    pub fg: Rgb,
    pub bg: Rgb,
    pub direction: u8,
    /// Rainbow / full-colour instead of fg/bg.
    pub full_color: bool,
    /// Lighting on (wire: 0 = on, 1 = off).
    pub on: bool,
}

/// Decodes a `07 01` (or `08 01`) reply.
pub fn parse_effect(b: &Body) -> Result<Effect> {
    let p = payload_min(b, 12)?;
    Ok(Effect {
        mode: p[0],
        brightness: p[1],
        speed: p[2],
        fg: Rgb(p[3], p[4], p[5]),
        bg: Rgb(p[6], p[7], p[8]),
        direction: p[9],
        full_color: p[10] == 1,
        on: p[11] == 0,
    })
}

impl Effect {
    /// Checks the mode and ranges against the board's mode list. Music rhythm is refused.
    pub fn validate(&self, list: &ModeList) -> Result<()> {
        if self.mode == MODE_MUSIC {
            return invalid("music rhythm needs the vendor desktop plugin and is not supported");
        }
        let Some(info) = mode_info(self.mode) else {
            return invalid(format!("unknown lighting mode {}", self.mode));
        };
        if !list.selectable().contains(&self.mode) {
            return invalid(format!("this board does not offer mode {}", info.name));
        }
        if self.brightness > list.max_brightness {
            return invalid(format!("brightness {} is outside 0..={}", self.brightness, list.max_brightness));
        }
        if self.speed > list.max_speed {
            return invalid(format!("speed {} is outside 0..={}", self.speed, list.max_speed));
        }
        // 0 is always accepted: the official driver maps unknown directions to 0
        if self.direction != 0
            && !info.directions.is_empty()
            && !info.directions.iter().any(|(d, _)| *d == self.direction)
        {
            let ok: Vec<String> = info.directions.iter().map(|(d, n)| format!("{d} {n}")).collect();
            return invalid(format!("direction {} is not offered by {} ({})", self.direction, info.name, ok.join(", ")));
        }
        if info.directions.is_empty() && self.direction != 0 {
            return invalid(format!("{} has no direction (use 0)", info.name));
        }
        Ok(())
    }
}

/// `07 00 00 00 0E mode bri speed fg bg dir full power`. No ACK.
pub fn effect_frame(e: &Effect) -> Body {
    let mut b = frame::request(CMD_EFFECT, 0);
    b[4] = 0x0e;
    b[5..17].copy_from_slice(&[
        e.mode,
        e.brightness,
        e.speed,
        e.fg.0,
        e.fg.1,
        e.fg.2,
        e.bg.0,
        e.bg.1,
        e.bg.2,
        e.direction,
        e.full_color as u8,
        (!e.on) as u8,
    ]);
    b
}

/// `17 00 00 00 02 v v`: enter (true) or leave music-rhythm mode. ACK `17 00 ..`.
pub fn music_frame(on: bool) -> Body {
    let mut b = frame::request(CMD_MUSIC, 0);
    b[4] = 0x02;
    b[5] = on as u8;
    b[6] = on as u8;
    b
}

/// Active custom slot from the `09 20` reply (`b[1]`).
pub fn parse_active_slot(b: &Body) -> Result<u8> {
    if b[0] != CMD_CUSTOM_RGB || b[1] >= CUSTOM_SLOTS {
        return protocol(format!("unexpected 09 20 reply {}", frame::hex(&b[..6])));
    }
    Ok(b[1])
}

/// Per-key colours of one custom slot. Keys missing from the map are black.
pub type CustomColors = PerKey<Rgb>;

pub fn decode_custom(table: &[u8]) -> Result<CustomColors> {
    if table.len() != RGB_TABLE.len {
        return invalid(format!("RGB table is {} bytes, expected {}", table.len(), RGB_TABLE.len));
    }
    Ok(PerKey::from_fn(|k| {
        let i = k.index as usize * 3;
        Rgb(table[i], table[i + 1], table[i + 2])
    }))
}

pub fn encode_custom(colors: &CustomColors) -> Result<Vec<u8>> {
    let mut t = vec![0u8; RGB_TABLE.len];
    for (idx, c) in colors.iter() {
        if !layout::is_key(idx) {
            return invalid(format!("{} is not a key", key_name(idx)));
        }
        t[idx * 3..idx * 3 + 3].copy_from_slice(&[c.0, c.1, c.2]);
    }
    Ok(t)
}

pub fn check_slot(slot: u8) -> Result<()> {
    if slot >= CUSTOM_SLOTS {
        return invalid(format!("custom slot {slot} is outside 0..{CUSTOM_SLOTS}"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::body;

    fn owner_list() -> ModeList {
        parse_mode_list(&body("0a 00 00 00 12 02 01 03 09 04 00 0e 0f 06 07 0b 0c 10 08 a5 5a 04 04")).unwrap()
    }

    #[test]
    fn owner_mode_lists() {
        let l = owner_list();
        assert_eq!(l.modes, vec![2, 1, 3, 9, 4, 0, 14, 15, 6, 7, 11, 12, 16, 8]);
        assert_eq!((l.max_speed, l.max_brightness), (4, 4));
        assert_eq!(l.selectable().len(), 15);
        assert!(l.modes.iter().all(|m| mode_info(*m).is_some()), "every advertised mode is known");
        let side = parse_mode_list(&body("02 00 00 00 04 a5 5a 04 04")).unwrap();
        assert!(side.modes.is_empty());
    }

    #[test]
    fn mode_list_errors() {
        assert!(parse_mode_list(&body("0a 00 00 00 00")).is_err());
        assert!(parse_mode_list(&body("0a 00 00 00 03 01 02 03")).is_err());
        assert!(parse_mode_list(&body("0a 00 00 00 03 a5 5a 04")).is_err(), "truncated ranges");
    }

    #[test]
    fn owner_effect() {
        let e = parse_effect(&body("07 00 00 00 0c 00 01 02 ff 00 ff")).unwrap();
        assert_eq!(e.mode, 0);
        assert_eq!((e.brightness, e.speed), (1, 2));
        assert_eq!(e.fg, Rgb(0xff, 0, 0xff));
        assert!(e.on && !e.full_color);
        let side = parse_effect(&body("08 00 00 00 0c 02 04 03 ff 00 00 00 00 00 00 00 01")).unwrap();
        assert!(!side.on);
        assert!(parse_effect(&body("07 00 00 00 0b")).is_err());
    }

    #[test]
    fn effect_frame_matches_the_capture() {
        let e = Effect {
            mode: 2,
            brightness: 4,
            speed: 4,
            fg: Rgb(0xff, 0, 0),
            bg: Rgb::BLACK,
            direction: 0,
            full_color: false,
            on: true,
        };
        assert_eq!(effect_frame(&e), body("07 00 00 00 0e 02 04 04 ff 00 00 00 00 00 00 00 00"));
        // the read reply decodes back to the same effect
        let mut reply = effect_frame(&e);
        reply[4] = 0x0c;
        assert_eq!(parse_effect(&reply).unwrap(), e);
        let off = Effect { on: false, ..e };
        assert_eq!(effect_frame(&off)[16], 1);
    }

    #[test]
    fn effect_validation() {
        let l = owner_list();
        let ok = Effect { mode: 2, brightness: 4, speed: 0, fg: Rgb::BLACK, bg: Rgb::BLACK, direction: 5, full_color: true, on: true };
        assert!(ok.validate(&l).is_ok());
        assert!(Effect { mode: 10, direction: 0, ..ok }.validate(&l).is_ok(), "custom is always offered");
        assert!(Effect { brightness: 5, ..ok }.validate(&l).is_err());
        assert!(Effect { speed: 5, ..ok }.validate(&l).is_err());
        assert!(Effect { direction: 6, ..ok }.validate(&l).is_err());
        assert!(Effect { mode: 7, direction: 1, ..ok }.validate(&l).is_err(), "aurora: spread or gather");
        assert!(Effect { mode: 7, direction: 0, ..ok }.validate(&l).is_ok(), "0 is always accepted");
        assert!(Effect { mode: 0, direction: 1, ..ok }.validate(&l).is_err(), "static has no direction");
        assert!(Effect { mode: 5, direction: 0, ..ok }.validate(&l).is_err(), "unused id");
        assert!(Effect { mode: 100, ..ok }.validate(&l).is_err(), "music rhythm");
        let small = ModeList { modes: vec![0], max_speed: 4, max_brightness: 4 };
        assert!(ok.validate(&small).is_err(), "not advertised");
    }

    #[test]
    fn mode_names() {
        assert_eq!(mode_by_name("Wave").unwrap().id, 2);
        assert_eq!(mode_by_name("16").unwrap().name, "fireworks");
        assert!(mode_by_name("5").is_none());
        assert!(mode_by_name("").is_none());
        assert!(mode_by_name("music").is_none());
    }

    #[test]
    fn music_frames() {
        assert_eq!(music_frame(false), body("17 00 00 00 02 00 00"));
        assert_eq!(music_frame(true), body("17 00 00 00 02 01 01"));
    }

    #[test]
    fn active_slot() {
        assert_eq!(parse_active_slot(&body("09")).unwrap(), 0);
        assert_eq!(parse_active_slot(&body("09 01")).unwrap(), 1);
        assert!(parse_active_slot(&body("09 02")).is_err());
        assert!(parse_active_slot(&body("0a")).is_err());
    }

    /// Owner's slot 0: W A S D red (idx 46, 68, 69, 70), Appendix A.
    #[test]
    fn owner_custom_slot() {
        let mut pages = Vec::new();
        for p in 0..8usize {
            let len = RGB_TABLE.page_len(p);
            let mut b = body(&format!("09 80 00 {p:02x} {len:02x}"));
            for idx in [46usize, 68, 69, 70] {
                let off = idx * 3;
                if off / 54 == p {
                    b[5 + off % 54] = 0xff;
                }
            }
            pages.push(b);
        }
        // page 2 ends with ff (idx 46 R at offset 138 = page 2 byte 30)
        let t = RGB_TABLE.assemble(&pages).unwrap();
        let c = decode_custom(&t).unwrap();
        let red: Vec<usize> = c.iter().filter(|(_, c)| **c != Rgb::BLACK).map(|(i, _)| i).collect();
        assert_eq!(red, vec![46, 68, 69, 70]);
        assert_eq!(c.get(46), Some(&Rgb(0xff, 0, 0)));
        assert_eq!(encode_custom(&c).unwrap(), t);
    }

    #[test]
    fn custom_table_errors_and_empty() {
        assert!(decode_custom(&[]).is_err());
        assert_eq!(encode_custom(&PerKey::new()).unwrap(), vec![0; 396]);
        let mut bad = PerKey::new();
        bad.0.insert(0, Rgb(1, 2, 3));
        assert!(encode_custom(&bad).is_err());
        assert!(check_slot(1).is_ok());
        assert!(check_slot(2).is_err());
    }

    #[test]
    fn colour_text() {
        assert_eq!("#FF0080".parse::<Rgb>().unwrap(), Rgb(255, 0, 128));
        assert_eq!("00ff00".parse::<Rgb>().unwrap(), Rgb(0, 255, 0));
        assert_eq!(Rgb(1, 2, 3).to_string(), "#010203");
        for bad in ["", "#fff", "#gg0000", "#0000000"] {
            assert!(bad.parse::<Rgb>().is_err(), "{bad}");
        }
        assert_eq!(serde_json::to_string(&Rgb(255, 0, 0)).unwrap(), "\"#ff0000\"");
    }
}

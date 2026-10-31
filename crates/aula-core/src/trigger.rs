//! Actuation, rapid trigger, dead band, switch type and poll rate (protocol section 9).
//!
//! Travel values are raw trigger units: 0.01 mm on this board (`TravelLimits::unit_mm`).

use crate::error::{invalid, protocol, Result};
use crate::frame::{
    self, encode_mask, payload, payload_min, Body, TableSpec, MATRIX_KEYS, SUB_KEY_TRIGGER, SUB_LIMITS,
    SUB_MODE_FIXED, SUB_MODE_RAPID, SUB_MODE_RAPID_SPLIT, SUB_POLL_RATE, TRIGGER_B4,
};
use crate::layout;
use crate::perkey::{key_name, PerKey};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Dead band: 132 x `[top, bottom]`, 58-byte pages (protocol 9.5).
pub const DEAD_BAND_TABLE: TableSpec = TableSpec { len: MATRIX_KEYS * 2, page: 58 };
/// Switch type per key: 132 bytes, 58-byte pages (protocol 9.6).
pub const SWITCH_TABLE: TableSpec = TableSpec { len: MATRIX_KEYS, page: 58 };
/// Dead-band values are 0..=100 trigger units (0..1.00 mm).
pub const DEAD_BAND_MAX: u8 = 100;

/// `21 .. 18 04` reply.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TravelLimits {
    /// Full travel in trigger units (340 = 3.40 mm).
    pub max: u16,
    /// Smallest actuation point in trigger units.
    pub min: u16,
    /// Unit numerator (`b[7]`, 0 means 10).
    pub unit_num: u8,
    /// Divisor 1000 instead of 100 (`0x01` flags2 bit 0).
    pub high_precision: bool,
}

impl TravelLimits {
    /// Size of one trigger unit in mm.
    pub fn unit_mm(&self) -> f64 {
        self.unit_num as f64 / if self.high_precision { 1000.0 } else { 100.0 }
    }

    /// Rapid-trigger sensitivity range: 1 ..= ceil(max / 2).
    pub fn max_sensitivity(&self) -> u16 {
        self.max.div_ceil(2)
    }

    pub fn to_mm(&self, raw: u16) -> f64 {
        raw as f64 * self.unit_mm()
    }

    /// Nearest raw value for `mm`.
    pub fn from_mm(&self, mm: f64) -> Result<u16> {
        let raw = (mm / self.unit_mm()).round();
        if !raw.is_finite() || !(0.0..=u16::MAX as f64).contains(&raw) {
            return invalid(format!("{mm} mm is out of range"));
        }
        Ok(raw as u16)
    }
}

pub fn limits_request() -> Body {
    frame::trigger_request(TRIGGER_B4, SUB_LIMITS, &[])
}

pub fn parse_limits(b: &Body, high_precision: bool) -> Result<TravelLimits> {
    let p = payload_min(b, 6)?;
    if p[0] != SUB_LIMITS {
        return protocol(format!("expected a travel-limits reply, got sub 0x{:02x}", p[0]));
    }
    let max = u16::from_be_bytes([b[9], b[6]]);
    let unit_num = if b[7] == 0 { 10 } else { b[7] };
    let mut limits = TravelLimits { max, min: b[10] as u16, unit_num, high_precision };
    if limits.min == 0 {
        let unit = limits.unit_mm();
        limits.min = if (unit - 0.04).abs() < 1e-9 { 2 } else { 1 };
    }
    if limits.max == 0 || limits.min > limits.max {
        return protocol(format!("travel limits {}..{} make no sense", limits.min, limits.max));
    }
    Ok(limits)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TriggerMode {
    /// Fixed actuation point (0).
    Fixed,
    /// Rapid trigger, one sensitivity for press and release (12).
    Rapid,
    /// Rapid trigger with separate press / release sensitivity (13).
    RapidSplit,
}

impl TriggerMode {
    pub fn byte(self) -> u8 {
        match self {
            TriggerMode::Fixed => SUB_MODE_FIXED,
            TriggerMode::Rapid => SUB_MODE_RAPID,
            TriggerMode::RapidSplit => SUB_MODE_RAPID_SPLIT,
        }
    }

    pub fn from_byte(b: u8) -> Option<Self> {
        Some(match b {
            SUB_MODE_FIXED => TriggerMode::Fixed,
            SUB_MODE_RAPID => TriggerMode::Rapid,
            SUB_MODE_RAPID_SPLIT => TriggerMode::RapidSplit,
            _ => return None,
        })
    }
}

/// One key's trigger configuration, in trigger units.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct KeyTrigger {
    pub mode: TriggerMode,
    /// Actuation point.
    pub actuation: u16,
    /// Rapid-trigger press sensitivity (interval1). Stored also in fixed mode.
    pub press: u16,
    /// Rapid-trigger release sensitivity (interval2). Equals `press` in `Rapid` mode.
    pub release: u16,
}

impl KeyTrigger {
    pub fn validate(&self, l: &TravelLimits) -> Result<()> {
        if !(l.min..=l.max).contains(&self.actuation) {
            return invalid(format!("actuation {} is outside {}..={}", self.actuation, l.min, l.max));
        }
        let s = 1..=l.max_sensitivity();
        if !s.contains(&self.press) || !s.contains(&self.release) {
            return invalid(format!(
                "sensitivity {}/{} is outside 1..={}",
                self.press,
                self.release,
                l.max_sensitivity()
            ));
        }
        Ok(())
    }

    /// The values the board stores: in `Rapid` mode release follows press.
    pub fn normalized(&self) -> KeyTrigger {
        match self.mode {
            TriggerMode::Rapid => KeyTrigger { release: self.press, ..*self },
            _ => *self,
        }
    }
}

/// Per-key read request `21 00 00 00 18 05 row col`.
pub fn key_request(idx: usize) -> Body {
    let (row, col) = frame::row_col(idx);
    frame::trigger_request(TRIGGER_B4, SUB_KEY_TRIGGER, &[row, col])
}

/// Decodes a per-key reply. Returns the key index it echoes and the config.
pub fn parse_key(b: &Body) -> Result<(usize, KeyTrigger)> {
    let p = payload_min(b, 0x0c)?;
    if p[0] != SUB_KEY_TRIGGER {
        return protocol(format!("expected a key-trigger reply, got sub 0x{:02x}", p[0]));
    }
    let Some(mode) = TriggerMode::from_byte(b[6]) else {
        return protocol(format!("unknown trigger mode {}", b[6]));
    };
    let Some(idx) = frame::index_of(b[15], b[16]) else {
        return protocol(format!("key-trigger reply for row {} col {}", b[15], b[16]));
    };
    let w = |lo: usize| u16::from_le_bytes([b[lo], b[lo + 4]]);
    Ok((idx, KeyTrigger { mode, actuation: w(7), press: w(9), release: w(10) }))
}

/// One write frame setting `t` on every key in `keys` (protocol 9.4).
pub fn write_frame(keys: &[usize], t: &KeyTrigger) -> Result<Body> {
    if keys.is_empty() {
        return invalid("no keys selected");
    }
    let t = t.normalized();
    let mut b = frame::trigger_request(TRIGGER_B4, t.mode.byte(), &encode_mask(keys)?);
    for (at, v) in [(28, t.actuation), (29, t.actuation), (30, t.press), (31, t.release)] {
        let [lo, hi] = v.to_le_bytes();
        b[at] = lo;
        b[at + 4] = hi;
    }
    Ok(b)
}

/// Write frames for a set of per-key changes, one per group of keys with identical settings, like the
/// official driver.
pub fn write_frames(changes: &PerKey<KeyTrigger>) -> Result<Vec<Body>> {
    let mut groups: BTreeMap<KeyTrigger, Vec<usize>> = BTreeMap::new();
    for (idx, t) in changes.iter() {
        if !layout::is_key(idx) {
            return invalid(format!("{} is not a key", key_name(idx)));
        }
        groups.entry(t.normalized()).or_default().push(idx);
    }
    groups.iter().map(|(t, keys)| write_frame(keys, t)).collect()
}

/// `[top, bottom]` dead band of one key, in trigger units.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct DeadBand {
    pub top: u8,
    pub bottom: u8,
}

pub fn decode_dead_band(table: &[u8]) -> Result<PerKey<DeadBand>> {
    if table.len() != DEAD_BAND_TABLE.len {
        return invalid(format!("dead-band table is {} bytes, expected {}", table.len(), DEAD_BAND_TABLE.len));
    }
    Ok(PerKey::from_fn(|k| {
        let i = k.index as usize * 2;
        DeadBand { top: table[i], bottom: table[i + 1] }
    }))
}

/// Missing keys are written as 0/0 (the never-written state).
pub fn encode_dead_band(map: &PerKey<DeadBand>) -> Result<Vec<u8>> {
    let mut t = vec![0u8; DEAD_BAND_TABLE.len];
    for (idx, d) in map.iter() {
        if !layout::is_key(idx) {
            return invalid(format!("{} is not a key", key_name(idx)));
        }
        if d.top > DEAD_BAND_MAX || d.bottom > DEAD_BAND_MAX {
            return invalid(format!("{}: dead band {}/{} is outside 0..={DEAD_BAND_MAX}", key_name(idx), d.top, d.bottom));
        }
        t[idx * 2] = d.top;
        t[idx * 2 + 1] = d.bottom;
    }
    Ok(t)
}

/// Known switch ids (official `switchDefault`): id, name, travel cap in 0.1 mm.
pub static SWITCHES: &[(u8, &str, u8)] = &[
    (1, "HM1", 34), (2, "HH1", 34), (3, "CY1", 34), (4, "TW1", 34), (5, "TC1 (TTC Kom)", 34), (6, "SL1", 34),
    (7, "FZ1", 39), (8, "GJP (Gateron Jade Pro)", 34), (9, "HMZ", 34), (10, "GW1 (Gateron K2-20 white)", 39),
    (11, "VA1 (VARO Very Peri)", 34), (12, "OTK", 34), (13, "CN1", 34), (14, "RY1", 34), (15, "WCG", 34),
    (16, "JY1", 36), (17, "CS1", 34), (18, "VA2 (Contactless Very Peri)", 34), (19, "CD", 34),
    (21, "YH1 (Jade King)", 34), (22, "EM1 (electro-magnetic)", 35),
];

pub fn switch_name(id: u8) -> Option<&'static str> {
    SWITCHES.iter().find(|(i, _, _)| *i == id).map(|(_, n, _)| *n)
}

/// `25 00` reply: `n_hi n_lo id..` = switch ids the board offers.
pub fn parse_switch_list(b: &Body) -> Result<Vec<u8>> {
    let p = payload_min(b, 2)?;
    let n = u16::from_be_bytes([p[0], p[1]]) as usize;
    let Some(ids) = p.get(2..2 + n) else {
        return protocol(format!("switch list claims {n} ids but carries {}", p.len() - 2));
    };
    Ok(ids.to_vec())
}

pub fn decode_switch_table(table: &[u8]) -> Result<PerKey<u8>> {
    if table.len() != SWITCH_TABLE.len {
        return invalid(format!("switch table is {} bytes, expected {}", table.len(), SWITCH_TABLE.len));
    }
    Ok(PerKey::from_fn(|k| table[k.index as usize]))
}

/// Every key must be present and use an id from `offered`.
pub fn encode_switch_table(map: &PerKey<u8>, offered: &[u8]) -> Result<Vec<u8>> {
    let mut t = vec![0u8; SWITCH_TABLE.len];
    for k in &layout::KEYS {
        let Some(id) = map.get(k.index as usize) else {
            return invalid(format!("{}: no switch type given", k.code));
        };
        if !offered.contains(id) {
            return invalid(format!("{}: switch type {id} is not offered by this board ({offered:?})", k.code));
        }
        t[k.index as usize] = *id;
    }
    if let Some((idx, _)) = map.iter().find(|(i, _)| !layout::is_key(*i)) {
        return invalid(format!("{} is not a key", key_name(idx)));
    }
    Ok(t)
}

/// Poll rates in kHz (`r` byte).
pub const POLL_RATES: [u8; 4] = [1, 2, 4, 8];

pub fn poll_rate_request() -> Body {
    frame::trigger_request(0x01, SUB_POLL_RATE, &[])
}

/// `21 00 00 00 02 09 r` -> kHz.
pub fn parse_poll_rate(b: &Body) -> Result<u8> {
    let p = payload_min(b, 2)?;
    if p[0] != SUB_POLL_RATE || !POLL_RATES.contains(&p[1]) {
        return protocol(format!("unexpected poll-rate reply {}", frame::hex(payload(b)?)));
    }
    Ok(p[1])
}

/// `21 00 00 00 00 09 01 r`. No ACK; the keyboard restarts.
pub fn poll_rate_frame(khz: u8) -> Result<Body> {
    if !POLL_RATES.contains(&khz) {
        return invalid(format!("poll rate {khz} kHz is not one of 1, 2, 4, 8"));
    }
    Ok(frame::trigger_request(0x00, SUB_POLL_RATE, &[0x01, khz]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::{body, check_allowed};

    fn owner_limits() -> TravelLimits {
        parse_limits(&body("21 00 00 00 06 04 54 01 01 01 08"), false).unwrap()
    }

    #[test]
    fn owner_travel_limits() {
        let l = owner_limits();
        assert_eq!((l.max, l.min, l.unit_num), (340, 8, 1));
        assert!((l.unit_mm() - 0.01).abs() < 1e-12);
        assert_eq!(l.max_sensitivity(), 170);
        assert_eq!(l.from_mm(1.0).unwrap(), 100);
        assert!((l.to_mm(340) - 3.4).abs() < 1e-9);
        assert!(l.from_mm(-1.0).is_err() && l.from_mm(f64::NAN).is_err() && l.from_mm(1e9).is_err());
        assert_eq!(limits_request(), body("21 00 00 00 18 04"));
    }

    #[test]
    fn limits_derived_minimum_and_errors() {
        // unit 4/100 = 0.04 mm, no min -> 2
        let l = parse_limits(&body("21 00 00 00 06 04 64 04 01 00 00"), false).unwrap();
        assert_eq!(l.min, 2);
        // unit numerator 0 means 10 -> 0.1 mm -> min 1
        let l = parse_limits(&body("21 00 00 00 06 04 28 00 01 00 00"), false).unwrap();
        assert_eq!((l.unit_num, l.min), (10, 1));
        assert!((parse_limits(&body("21 00 00 00 06 04 54 01 01 01 08"), true).unwrap().unit_mm() - 0.001).abs() < 1e-12);
        assert!(parse_limits(&body("21 00 00 00 05 04"), false).is_err(), "short");
        assert!(parse_limits(&body("21 00 00 00 06 05 54 01 01 01 08"), false).is_err(), "wrong sub");
        assert!(parse_limits(&body("21 00 00 00 06 04 00 01 01 00 08"), false).is_err(), "max 0");
    }

    #[test]
    fn owner_key_trigger() {
        let (idx, t) = parse_key(&body("21 00 00 00 0c 05 0c 64 64 32 32 00 00 00 00 05 0c")).unwrap();
        assert_eq!(idx, 122);
        assert_eq!(t, KeyTrigger { mode: TriggerMode::Rapid, actuation: 100, press: 50, release: 50 });
        assert!(t.validate(&owner_limits()).is_ok());
        assert_eq!(key_request(22), body("21 00 00 00 18 05 01 00"));
        assert_eq!(key_request(122), body("21 00 00 00 18 05 05 0c"));
    }

    #[test]
    fn key_trigger_high_bytes() {
        let (idx, t) = parse_key(&body("21 00 00 00 0c 05 0d 54 54 aa 01 01 01 00 00 01 00")).unwrap();
        assert_eq!(idx, 22);
        assert_eq!(t, KeyTrigger { mode: TriggerMode::RapidSplit, actuation: 340, press: 170, release: 1 });
    }

    #[test]
    fn key_trigger_errors() {
        assert!(parse_key(&body("21 00 00 00 0b 05 0c")).is_err(), "short");
        assert!(parse_key(&body("21 00 00 00 0c 04 0c")).is_err(), "wrong sub");
        assert!(parse_key(&body("21 00 00 00 0c 05 01 64 64 32 32 00 00 00 00 01 00")).is_err(), "mode 1");
        assert!(parse_key(&body("21 00 00 00 0c 05 00 64 64 32 32 00 00 00 00 06 00")).is_err(), "row 6");
    }

    #[test]
    fn write_frame_matches_the_capture() {
        let t = KeyTrigger { mode: TriggerMode::Fixed, actuation: 100, press: 1, release: 1 };
        let f = write_frame(&layout::all_indices(), &t).unwrap();
        assert_eq!(
            f,
            body("21 00 00 00 18 00 3e 26 3e 1e 1e 1e 3e 1e 1e 3e 3e 3e 3e 00 0e 00 00 00 00 00 00 00 64 64 01 01 00 00 00 00")
        );
        assert!(check_allowed(&f).is_ok());
    }

    #[test]
    fn write_frame_rapid_modes() {
        let t = KeyTrigger { mode: TriggerMode::Rapid, actuation: 340, press: 170, release: 3 };
        let f = write_frame(&[46], &t).unwrap();
        assert_eq!(f[5], 0x0c);
        assert_eq!(&f[28..36], &[0x54, 0x54, 0xaa, 0xaa, 1, 1, 0, 0], "release follows press in mode 12");
        let t = KeyTrigger { mode: TriggerMode::RapidSplit, actuation: 8, press: 1, release: 170 };
        let f = write_frame(&[46], &t).unwrap();
        assert_eq!(f[5], 0x0d);
        assert_eq!(&f[28..36], &[8, 8, 1, 0xaa, 0, 0, 0, 0]);
        assert!(write_frame(&[], &t).is_err());
        assert!(write_frame(&[132], &t).is_err());
    }

    #[test]
    fn grouped_frames() {
        let a = KeyTrigger { mode: TriggerMode::Fixed, actuation: 100, press: 50, release: 50 };
        let b = KeyTrigger { actuation: 50, ..a };
        let mut m = PerKey::new();
        for (i, t) in [(46, a), (68, b), (69, a), (70, a)] {
            m.0.insert(i, t);
        }
        let frames = write_frames(&m).unwrap();
        assert_eq!(frames.len(), 2);
        let masks: Vec<Vec<usize>> = frames.iter().map(|f| frame::decode_mask(&f[6..28])).collect();
        assert!(masks.contains(&vec![68]) && masks.contains(&vec![46, 69, 70]));
        assert!(write_frames(&PerKey::new()).unwrap().is_empty());
        m.0.insert(0, a);
        assert!(write_frames(&m).is_err());
    }

    #[test]
    fn trigger_validation_boundaries() {
        let l = owner_limits();
        let t = KeyTrigger { mode: TriggerMode::Fixed, actuation: 8, press: 1, release: 170 };
        assert!(t.validate(&l).is_ok());
        assert!(KeyTrigger { actuation: 340, ..t }.validate(&l).is_ok());
        assert!(KeyTrigger { actuation: 7, ..t }.validate(&l).is_err());
        assert!(KeyTrigger { actuation: 341, ..t }.validate(&l).is_err());
        assert!(KeyTrigger { press: 0, ..t }.validate(&l).is_err());
        assert!(KeyTrigger { release: 171, ..t }.validate(&l).is_err());
    }

    #[test]
    fn dead_band_tables() {
        let zero = decode_dead_band(&[0; 264]).unwrap();
        assert!(zero.iter().all(|(_, d)| *d == DeadBand::default()));
        assert_eq!(encode_dead_band(&zero).unwrap(), vec![0; 264]);
        let mut m = PerKey::new();
        m.0.insert(122, DeadBand { top: 100, bottom: 2 });
        let t = encode_dead_band(&m).unwrap();
        assert_eq!(&t[244..246], &[100, 2]);
        assert_eq!(decode_dead_band(&t).unwrap().get(122), Some(&DeadBand { top: 100, bottom: 2 }));
        m.0.insert(46, DeadBand { top: 101, bottom: 0 });
        assert!(encode_dead_band(&m).is_err());
        assert!(decode_dead_band(&[0; 263]).is_err());
        let frames = DEAD_BAND_TABLE.write_frames(0x26, 1, &t).unwrap();
        assert_eq!(frames.len(), 5);
        assert_eq!(frames[4][4], 0x20);
    }

    #[test]
    fn owner_switch_list_and_table() {
        assert_eq!(parse_switch_list(&body("25 00 00 00 06 00 04 01 02 03 05")).unwrap(), vec![1, 2, 3, 5]);
        assert_eq!(parse_switch_list(&body("25 00 00 00 02 00 00")).unwrap(), Vec::<u8>::new());
        assert!(parse_switch_list(&body("25 00 00 00 03 00 04 01")).is_err());
        assert!(parse_switch_list(&body("25 00 00 00 01 00")).is_err());
        let pages = [body(&format!("25 02 00 00 3a {}", "01 ".repeat(58))), body(&format!("25 02 00 01 3a {}", "01 ".repeat(58))),
            body(&format!("25 02 00 02 10 {}", "01 ".repeat(20)))];
        let t = SWITCH_TABLE.assemble(&pages).unwrap();
        let m = decode_switch_table(&t).unwrap();
        assert!(m.iter().all(|(_, id)| *id == 1));
        let enc = encode_switch_table(&m, &[1, 2, 3, 5]).unwrap();
        assert_eq!(enc[22], 1);
        assert_eq!(enc[0], 0, "non-key slots are written as zero");
        assert_eq!(switch_name(5), Some("TC1 (TTC Kom)"));
        assert_eq!(switch_name(20), None);
    }

    #[test]
    fn switch_table_errors() {
        let m = PerKey::from_fn(|_| 1u8);
        assert!(encode_switch_table(&m, &[2]).is_err(), "not offered");
        let mut missing = m.clone();
        missing.0.remove(&22);
        assert!(encode_switch_table(&missing, &[1]).is_err());
        let mut extra = m.clone();
        extra.0.insert(0, 1);
        assert!(encode_switch_table(&extra, &[1]).is_err());
        assert!(decode_switch_table(&[]).is_err());
    }

    #[test]
    fn poll_rate() {
        assert_eq!(poll_rate_request(), body("21 00 00 00 01 09"));
        assert_eq!(parse_poll_rate(&body("21 00 00 00 02 09 08")).unwrap(), 8);
        assert!(parse_poll_rate(&body("21 00 00 00 02 09 03")).is_err());
        assert!(parse_poll_rate(&body("21 00 00 00 01 09")).is_err());
        assert_eq!(poll_rate_frame(1).unwrap(), body("21 00 00 00 00 09 01 01"));
        assert!(check_allowed(&poll_rate_frame(8).unwrap()).is_ok());
        assert!(poll_rate_frame(0).is_err() && poll_rate_frame(16).is_err());
    }
}

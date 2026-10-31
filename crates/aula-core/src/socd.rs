//! SOCD / "PRCS" (protocol section 12): on/off and up to 20 key pairs.

use crate::error::{invalid, protocol, Result};
use crate::frame::{self, payload_min, Body, TableSpec, CMD_SOCD};
use crate::keycodes::{name_of_usage, Usage};
use serde::{Deserialize, Serialize};

/// 20 entries x 4 bytes, 40-byte pages.
pub const TABLE: TableSpec = TableSpec { len: MAX_PAIRS * 4, page: 40 };
pub const MAX_PAIRS: usize = 20;

/// What happens when both keys of a pair are down (wire value 0..3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SocdMode {
    /// The key pressed first is released ("last input wins").
    LastWins,
    /// Key 1 overrides key 2.
    Key1Wins,
    /// Key 2 overrides key 1.
    Key2Wins,
    /// The later key is ignored and only releases the first.
    Neutral,
}

impl SocdMode {
    fn from_byte(b: u8) -> Option<Self> {
        Some(match b {
            0 => SocdMode::LastWins,
            1 => SocdMode::Key1Wins,
            2 => SocdMode::Key2Wins,
            3 => SocdMode::Neutral,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SocdPair {
    pub mode: SocdMode,
    /// HID usages, not key indices.
    pub key1: Usage,
    pub key2: Usage,
}

/// Power state from `24 02`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SocdPower {
    pub enabled: bool,
    /// The on-board toggle hotkey (two HID-style codes; `fa e5` = Fn + R-Shift). Display only.
    pub hotkey: [u8; 2],
}

/// `24 02 00 00 06 on x x x hk1 hk2`.
pub fn parse_power(b: &Body) -> Result<SocdPower> {
    let p = payload_min(b, 6)?;
    Ok(SocdPower { enabled: p[0] == 1, hotkey: [p[4], p[5]] })
}

/// `24 03 00 00 01 on`. ACK `24 00 ..`.
pub fn power_frame(enabled: bool) -> Body {
    let mut b = frame::request(CMD_SOCD, 3);
    b[4] = 1;
    b[5] = enabled as u8;
    b
}

/// Decodes the 80-byte pair table. An entry is used when its keys are set; byte 0 is ignored on read
/// because the capture read back `00` for an entry written with `01` (protocol 12.2). `07 ff ff ff` is empty.
pub fn decode_pairs(table: &[u8]) -> Result<Vec<SocdPair>> {
    if table.len() != TABLE.len {
        return invalid(format!("SOCD table is {} bytes, expected {}", table.len(), TABLE.len));
    }
    let mut out = Vec::new();
    for e in table.chunks(4) {
        if e == [0x07, 0xff, 0xff, 0xff] || (e[2] == 0 && e[3] == 0) {
            continue;
        }
        let Some(mode) = SocdMode::from_byte(e[1]) else {
            return protocol(format!("SOCD entry {} has mode {}", frame::hex(e), e[1]));
        };
        out.push(SocdPair { mode, key1: Usage(e[2]), key2: Usage(e[3]) });
    }
    Ok(out)
}

/// Entries packed from the start, `[01, mode, key1, key2]`, unused entries zero.
pub fn encode_pairs(pairs: &[SocdPair]) -> Result<Vec<u8>> {
    if pairs.len() > MAX_PAIRS {
        return invalid(format!("at most {MAX_PAIRS} SOCD pairs, got {}", pairs.len()));
    }
    let mut t = vec![0u8; TABLE.len];
    for (i, p) in pairs.iter().enumerate() {
        for k in [p.key1, p.key2] {
            if name_of_usage(k.0).is_none() {
                return invalid(format!("SOCD pair {i}: {k} is not a key code"));
            }
        }
        if p.key1 == p.key2 {
            return invalid(format!("SOCD pair {i}: both keys are {}", p.key1));
        }
        t[i * 4..i * 4 + 4].copy_from_slice(&[1, p.mode as u8, p.key1.0, p.key2.0]);
    }
    Ok(t)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::body;

    #[test]
    fn owner_power() {
        let p = parse_power(&body("24 02 00 00 06 00 00 00 00 fa e5")).unwrap();
        assert!(!p.enabled);
        assert_eq!(p.hotkey, [0xfa, 0xe5]);
        assert!(parse_power(&body("24 02 00 00 06 01")).unwrap().enabled);
        assert!(parse_power(&body("24 02 00 00 05")).is_err());
    }

    #[test]
    fn power_frames() {
        assert_eq!(power_frame(true), body("24 03 00 00 01 01"));
        assert_eq!(power_frame(false), body("24 03 00 00 01 00"));
        assert!(frame::check_allowed(&power_frame(true)).is_ok());
    }

    #[test]
    fn capture_pair() {
        // A/D, model 2 (0-based 1 = key 1 wins) -> 01 01 04 07
        let pair = SocdPair { mode: SocdMode::Key1Wins, key1: Usage(0x04), key2: Usage(0x07) };
        let t = encode_pairs(&[pair]).unwrap();
        assert_eq!(&t[..8], &[1, 1, 4, 7, 0, 0, 0, 0]);
        assert_eq!(decode_pairs(&t).unwrap(), vec![pair]);
        // the capture read it back with byte 0 = 00
        let mut read = t.clone();
        read[0] = 0;
        assert_eq!(decode_pairs(&read).unwrap(), vec![pair]);
    }

    #[test]
    fn empty_and_full() {
        assert!(decode_pairs(&[0; 80]).unwrap().is_empty());
        let mut ff = vec![0u8; 80];
        ff[..4].copy_from_slice(&[7, 0xff, 0xff, 0xff]);
        assert!(decode_pairs(&ff).unwrap().is_empty());
        assert_eq!(encode_pairs(&[]).unwrap(), vec![0; 80]);
        let pair = SocdPair { mode: SocdMode::Neutral, key1: Usage(0x04), key2: Usage(0x07) };
        let t = encode_pairs(&[pair; 20]).unwrap();
        assert_eq!(decode_pairs(&t).unwrap().len(), 20);
        let frames = TABLE.write_frames(CMD_SOCD, 0, &t).unwrap();
        assert_eq!(frames.len(), 2);
        assert_eq!(&frames[1][..5], &[0x24, 0, 0, 1, 0x28]);
    }

    #[test]
    fn errors() {
        let pair = SocdPair { mode: SocdMode::LastWins, key1: Usage(0x04), key2: Usage(0x07) };
        assert!(encode_pairs(&[pair; 21]).is_err());
        assert!(encode_pairs(&[SocdPair { key2: Usage(0x04), ..pair }]).is_err());
        assert!(encode_pairs(&[SocdPair { key2: Usage(0x01), ..pair }]).is_err());
        let mut bad = vec![0u8; 80];
        bad[..4].copy_from_slice(&[1, 4, 4, 7]);
        assert!(decode_pairs(&bad).is_err());
        assert!(decode_pairs(&[0; 79]).is_err());
    }
}

//! Identification (`0x0D`), capabilities (`0x01`) and the performance / win-lock write (`0x14`)
//! (protocol sections 1.1, 4 and 11).

use crate::error::{protocol, Result};
use crate::frame::{self, payload, payload_min, Body, CMD_PERF};
use serde::{Deserialize, Serialize};

/// Product codes of the WIN 60 HE (protocol 1.1). Writes are refused for anything else.
pub const SUPPORTED_PRODUCT_CODES: [&str; 2] = ["SI2825HEARGB", "SI2825KZHEARGB"];

/// Parsed `0x0D` reply.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Identity {
    /// Page 0 string, e.g. `W669,34,KB,SI,SI2825KZHEARGB,V3.17.07`.
    pub raw: String,
    /// Field 4, e.g. `SI2825KZHEARGB`.
    pub product_code: String,
    /// Field 5 without `V`, `_` turned into `.`, e.g. `3.17.07`.
    pub firmware: String,
    /// Page 1, e.g. `Apr 15 2026,11:20:35`.
    pub build_date: String,
}

impl Identity {
    /// Whether this is a WIN 60 HE (`SI2825HEARGB` / `SI2825KZHEARGB`).
    pub fn is_supported(&self) -> bool {
        SUPPORTED_PRODUCT_CODES.contains(&self.product_code.as_str())
    }

    /// Behind a 2.4 GHz dongle (never true for the wired WIN 60 HE).
    pub fn is_dongle(&self) -> bool {
        self.raw.contains("dongle")
    }
}

/// ASCII payload up to the first NUL.
fn ascii(b: &Body) -> Result<String> {
    let p = payload(b)?;
    let end = p.iter().position(|c| *c == 0).unwrap_or(p.len());
    Ok(p[..end].iter().map(|c| if c.is_ascii_graphic() || *c == b' ' { *c as char } else { '?' }).collect())
}

/// Parses the two `0x0D` reply pages.
pub fn parse_identity(page0: &Body, page1: &Body) -> Result<Identity> {
    if frame::page_of(page0) != 0 || frame::page_of(page1) != 1 {
        return protocol("0x0D reply pages out of order");
    }
    let raw = ascii(page0)?;
    let fields: Vec<&str> = raw.split(',').collect();
    if fields.len() < 6 {
        return protocol(format!("0x0D string {raw:?} has {} fields, expected 6", fields.len()));
    }
    let firmware: String = fields[5]
        .trim_start_matches(['V', 'v'])
        .chars()
        .filter(|c| !c.is_control())
        .map(|c| if c == '_' { '.' } else { c })
        .collect();
    Ok(Identity { product_code: fields[4].trim().to_string(), firmware, build_date: ascii(page1)?, raw })
}

/// Parsed `0x01` reply.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capabilities {
    /// `b[1]` connection-mode token.
    pub connection: u8,
    pub charging: bool,
    pub battery: u8,
    /// `b[10]`: 0 = USB, 1 = 2.4 GHz.
    pub wireless: bool,
    /// `b[14] == 1`: SOCD ("PRCS") supported.
    pub socd: bool,
    /// `b[17]`.
    pub flags1: u8,
    /// `b[18]`.
    pub flags2: u8,
    /// `b[20]`: current win-lock option bits.
    pub win_lock_bits: u8,
    /// `b[21] == 1`.
    pub music_rhythm_on: bool,
    /// `b[22]` bit 0: 6-key rollover active (else NKRO).
    pub six_key_rollover: bool,
}

impl Capabilities {
    pub fn any_key_calibration(&self) -> bool {
        self.flags1 & 0x01 != 0
    }
    pub fn custom_light(&self) -> bool {
        self.flags1 & 0x02 != 0
    }
    pub fn switch_types(&self) -> bool {
        self.flags1 & 0x04 != 0
    }
    pub fn poll_rate(&self) -> bool {
        self.flags1 & 0x08 != 0
    }
    pub fn dead_band(&self) -> bool {
        self.flags1 & 0x10 != 0
    }
    pub fn music_rhythm(&self) -> bool {
        self.flags1 & 0x20 != 0
    }
    pub fn rollover_selector(&self) -> bool {
        self.flags1 & 0x80 != 0
    }
    /// Trigger unit divisor 1000 instead of 100.
    pub fn high_precision(&self) -> bool {
        self.flags2 & 0x01 != 0
    }
    pub fn gamepad(&self) -> bool {
        self.flags2 & 0x04 != 0
    }
    pub fn rs(&self) -> bool {
        self.flags2 & 0x08 != 0
    }
    pub fn rkrt(&self) -> bool {
        self.flags2 & 0x10 != 0
    }

    pub fn win_lock(&self) -> WinLock {
        WinLock::from_bits(self.win_lock_bits)
    }

    /// Feature names and whether the board has them, for display.
    pub fn features(&self) -> Vec<(&'static str, bool)> {
        vec![
            ("socd", self.socd),
            ("any-key calibration", self.any_key_calibration()),
            ("per-key rgb", self.custom_light()),
            ("switch types", self.switch_types()),
            ("poll rate", self.poll_rate()),
            ("dead band", self.dead_band()),
            ("music rhythm", self.music_rhythm()),
            ("nkro/6kro selector", self.rollover_selector()),
            ("high precision", self.high_precision()),
            ("gamepad", self.gamepad()),
            ("rs", self.rs()),
            ("rkrt", self.rkrt()),
        ]
    }
}

pub fn parse_capabilities(b: &Body) -> Result<Capabilities> {
    if b[0] != frame::CMD_INFO {
        return protocol(format!("expected a 0x01 reply, got 0x{:02x}", b[0]));
    }
    // b[4] = 0x11: 17 payload bytes, b[5..22]; b[22] is read as well (protocol 4.2 lists it).
    payload_min(b, 0x11)?;
    Ok(Capabilities {
        connection: b[1],
        charging: b[8] == 1,
        battery: b[9],
        wireless: b[10] == 1,
        socd: b[14] == 1,
        flags1: b[17],
        flags2: b[18],
        win_lock_bits: b[20],
        music_rhythm_on: b[21] == 1,
        six_key_rollover: b[22] & 1 != 0,
    })
}

/// What the on-keyboard win-lock blocks (protocol 11.1). Win-lock itself is toggled on the keyboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct WinLock {
    pub win: bool,
    pub shift_tab: bool,
    pub alt_f4: bool,
    pub alt_tab: bool,
}

impl WinLock {
    /// From `0x01` reply `b[20]`: bit 0 Win, 1 Shift+Tab, 2 Alt+F4, 3 Alt+Tab.
    pub fn from_bits(bits: u8) -> Self {
        WinLock { win: bits & 1 != 0, shift_tab: bits & 2 != 0, alt_f4: bits & 4 != 0, alt_tab: bits & 8 != 0 }
    }

    pub fn bits(&self) -> u8 {
        self.win as u8 | (self.shift_tab as u8) << 1 | (self.alt_f4 as u8) << 2 | (self.alt_tab as u8) << 3
    }
}

/// `14 00 00 00 01`, `b[12..15]` = win-lock options, `b[18]` = 6KRO (always the current value: this board
/// has no rollover selector). ACK `14 00 ..`.
pub fn win_lock_frame(w: WinLock, six_key_rollover: bool) -> Body {
    let mut b = frame::request(CMD_PERF, 0);
    b[4] = 0x01;
    b[12] = w.win as u8;
    b[13] = w.shift_tab as u8;
    b[14] = w.alt_f4 as u8;
    b[15] = w.alt_tab as u8;
    b[18] = six_key_rollover as u8;
    b
}

/// Unsolicited `14 .. b[5]=01`: the board reports a reset (protocol 4.3). Never answer it with a reset.
pub fn is_reset_notice(b: &Body) -> bool {
    b[0] == CMD_PERF && b[5] == 0x01
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::body;

    fn owner_identity() -> (Body, Body) {
        (
            body("0d 00 00 00 32 57 36 36 39 2c 33 34 2c 4b 42 2c 53 49 2c 53 49 32 38 32 35 4b 5a 48 45 41 52 47 42 2c 56 33 2e 31 37 2e 30 37"),
            body("0d 00 00 01 32 41 70 72 20 31 35 20 32 30 32 36 2c 31 31 3a 32 30 3a 33 35 00 00 00 00 41 70 72 20"),
        )
    }

    #[test]
    fn owner_board_identity() {
        let (p0, p1) = owner_identity();
        let id = parse_identity(&p0, &p1).unwrap();
        assert_eq!(id.raw, "W669,34,KB,SI,SI2825KZHEARGB,V3.17.07");
        assert_eq!(id.product_code, "SI2825KZHEARGB");
        assert_eq!(id.firmware, "3.17.07");
        assert_eq!(id.build_date, "Apr 15 2026,11:20:35", "stale bytes after the NUL are dropped");
        assert!(id.is_supported());
        assert!(!id.is_dongle());
    }

    #[test]
    fn other_boards_are_not_supported() {
        let mut p0 = body("0d 00 00 00 00");
        let s = b"W669,34,KB,SI,SI2828HEARGB,V1_02";
        p0[4] = s.len() as u8;
        p0[5..5 + s.len()].copy_from_slice(s);
        let (_, p1) = owner_identity();
        let id = parse_identity(&p0, &p1).unwrap();
        assert_eq!(id.product_code, "SI2828HEARGB");
        assert_eq!(id.firmware, "1.02");
        assert!(!id.is_supported());
    }

    #[test]
    fn identity_errors() {
        let (p0, p1) = owner_identity();
        assert!(parse_identity(&p1, &p0).is_err(), "pages swapped");
        let empty = body("0d 00 00 00 00");
        assert!(parse_identity(&empty, &p1).is_err());
        assert!(parse_identity(&body("0d 00 00 00 05 61 2c 62 2c 63"), &p1).is_err());
    }

    #[test]
    fn owner_board_capabilities() {
        let c = parse_capabilities(&body("01 00 00 00 11 00 00 00 00 00 00 00 00 00 01 00 00 7f 00 01 01")).unwrap();
        assert!(c.socd && c.any_key_calibration() && c.custom_light() && c.switch_types());
        assert!(c.poll_rate() && c.dead_band() && c.music_rhythm());
        assert!(!c.rollover_selector() && !c.high_precision() && !c.gamepad() && !c.rs() && !c.rkrt());
        assert!(!c.wireless && !c.charging && !c.music_rhythm_on && !c.six_key_rollover);
        assert_eq!(c.win_lock(), WinLock { win: true, ..Default::default() });
        assert_eq!(c.features().len(), 12);
    }

    #[test]
    fn capability_errors_and_boundaries() {
        assert!(parse_capabilities(&body("01 00 00 00 10")).is_err(), "short payload");
        assert!(parse_capabilities(&body("02 00 00 00 11")).is_err(), "wrong command");
        let all = parse_capabilities(&body("01 07 00 00 11 00 00 00 01 64 01 00 00 00 01 00 00 ff ff 00 0f 01 01")).unwrap();
        assert_eq!(all.connection, 7);
        assert!(all.charging && all.wireless && all.music_rhythm_on && all.six_key_rollover);
        assert_eq!(all.battery, 100);
        assert_eq!(all.win_lock().bits(), 0x0f);
    }

    #[test]
    fn win_lock_bits_round_trip() {
        for bits in 0..16u8 {
            assert_eq!(WinLock::from_bits(bits).bits(), bits);
        }
        assert_eq!(WinLock::from_bits(0xf0).bits(), 0, "upper bits ignored");
        let w = WinLock { alt_f4: true, ..Default::default() };
        assert_eq!(w.bits(), 0x04, "bit 2 = Alt+F4 (protocol 15 #6)");
    }

    #[test]
    fn win_lock_write_frame() {
        let f = win_lock_frame(WinLock { win: true, alt_tab: true, ..Default::default() }, false);
        assert_eq!(f, body("14 00 00 00 01 00 00 00 00 00 00 00 01 00 00 01"));
        let f = win_lock_frame(WinLock::default(), true);
        assert_eq!(f[18], 1);
        assert!(frame::check_allowed(&f).is_ok());
    }

    #[test]
    fn reset_notice() {
        assert!(is_reset_notice(&body("14 00 00 00 01 01")));
        assert!(!is_reset_notice(&body("14 00 00 00 00 00")));
        assert!(!is_reset_notice(&body("21 00 00 00 00 01")));
    }
}

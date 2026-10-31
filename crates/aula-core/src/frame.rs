//! Framing shared by every command (protocol section 2): 63-byte bodies, payload length, page index,
//! key-select masks, paged tables, ACK frames and the command allowlist.
//!
//! All offsets are body offsets `b[n]` (report ID excluded), as in the protocol document.

use crate::error::{protocol, Error, Result};

/// Report ID of every vendor Input and Output report.
pub const REPORT_ID: u8 = 0x01;
/// Data bytes of one report, report ID excluded.
pub const BODY_LEN: usize = 63;
/// Payload starts at `b[5]`.
pub const PAYLOAD_OFFSET: usize = 5;
/// Largest payload of one frame (`b[5..63]`).
pub const MAX_PAYLOAD: usize = BODY_LEN - PAYLOAD_OFFSET;

/// One report body, report ID excluded.
pub type Body = [u8; BODY_LEN];

/// Key index range of the virtual 6 x 22 matrix (protocol 2.5).
pub const MATRIX_ROWS: usize = 6;
pub const MATRIX_COLS: usize = 22;
pub const MATRIX_KEYS: usize = MATRIX_ROWS * MATRIX_COLS;
/// Length of a transposed key-select mask.
pub const MASK_LEN: usize = MATRIX_COLS;

pub const CMD_INFO: u8 = 0x01;
pub const CMD_SIDE_MODES: u8 = 0x02;
pub const CMD_EFFECT: u8 = 0x07;
pub const CMD_SIDE_EFFECT: u8 = 0x08;
pub const CMD_CUSTOM_RGB: u8 = 0x09;
pub const CMD_MODES: u8 = 0x0a;
pub const CMD_IDENT: u8 = 0x0d;
pub const CMD_FN_FIXED: u8 = 0x0e;
pub const CMD_PERF: u8 = 0x14;
pub const CMD_MUSIC: u8 = 0x17;
pub const CMD_KEYMAP: u8 = 0x18;
pub const CMD_MACRO: u8 = 0x19;
pub const CMD_TRIGGER: u8 = 0x21;
pub const CMD_ADVANCED: u8 = 0x22;
pub const CMD_SOCD: u8 = 0x24;
pub const CMD_SWITCH: u8 = 0x25;
pub const CMD_DEAD_BAND: u8 = 0x26;

/// Sub-commands of `0x21` (in `b[5]`).
pub const SUB_MODE_FIXED: u8 = 0x00;
pub const SUB_TRAVEL_SAMPLE: u8 = 0x01;
pub const SUB_TRAVEL_OPEN: u8 = 0x02;
pub const SUB_TRAVEL_CLOSE: u8 = 0x03;
pub const SUB_LIMITS: u8 = 0x04;
pub const SUB_KEY_TRIGGER: u8 = 0x05;
pub const SUB_TRIGGER_RESET: u8 = 0x06;
pub const SUB_CALIB_PROGRESS_OLD: u8 = 0x08;
pub const SUB_POLL_RATE: u8 = 0x09;
pub const SUB_MODE_RAPID: u8 = 0x0c;
pub const SUB_MODE_RAPID_SPLIT: u8 = 0x0d;
pub const SUB_CALIB_START: u8 = 0x0f;
pub const SUB_CALIB_STOP: u8 = 0x10;
/// `b[4]` of most `0x21` requests: a constant, not a length (protocol 9.1).
pub const TRIGGER_B4: u8 = 0x18;

/// A request with `b[0] = cmd`, `b[1] = sel` and every other byte zero.
pub fn request(cmd: u8, sel: u8) -> Body {
    let mut b = [0u8; BODY_LEN];
    b[0] = cmd;
    b[1] = sel;
    b
}

/// A `0x21` request: `21 00 00 00 b4 sub args..`.
pub fn trigger_request(b4: u8, sub: u8, args: &[u8]) -> Body {
    let mut b = request(CMD_TRIGGER, 0);
    b[4] = b4;
    b[5] = sub;
    b[6..6 + args.len()].copy_from_slice(args);
    b
}

/// Page index `b[2..3]`, big-endian.
pub fn page_of(b: &Body) -> u16 {
    u16::from_be_bytes([b[2], b[3]])
}

/// Reply payload `b[5 .. 5 + b[4]]`. Bytes past it are stale firmware buffer contents (protocol 2.2).
pub fn payload(b: &Body) -> Result<&[u8]> {
    let len = b[4] as usize;
    if len > MAX_PAYLOAD {
        return protocol(format!("frame {:02x}: payload length {len} exceeds {MAX_PAYLOAD}", b[0]));
    }
    Ok(&b[PAYLOAD_OFFSET..PAYLOAD_OFFSET + len])
}

/// Like `payload`, but at least `min` bytes long.
pub fn payload_min(b: &Body, min: usize) -> Result<&[u8]> {
    let p = payload(b)?;
    if p.len() < min {
        return protocol(format!("frame {:02x}: payload is {} bytes, expected at least {min}", b[0], p.len()));
    }
    Ok(p)
}

/// Row and column of a key index (`idx = row * 22 + col`).
pub fn row_col(idx: usize) -> (u8, u8) {
    ((idx / MATRIX_COLS) as u8, (idx % MATRIX_COLS) as u8)
}

/// Key index of a row/column pair, or None outside the 6 x 22 matrix.
pub fn index_of(row: u8, col: u8) -> Option<usize> {
    let (row, col) = (row as usize, col as usize);
    (row < MATRIX_ROWS && col < MATRIX_COLS).then_some(row * MATRIX_COLS + col)
}

/// Transposed key-select mask: `mask[idx % 22] |= 1 << (idx / 22)`. Indices outside the matrix are an error.
pub fn encode_mask(indices: &[usize]) -> Result<[u8; MASK_LEN]> {
    let mut mask = [0u8; MASK_LEN];
    for &idx in indices {
        if idx >= MATRIX_KEYS {
            return Err(Error::Invalid(format!("key index {idx} is outside 0..{MATRIX_KEYS}")));
        }
        mask[idx % MATRIX_COLS] |= 1 << (idx / MATRIX_COLS);
    }
    Ok(mask)
}

/// Key indices set in a transposed mask, ascending. Bits 6 and 7 (rows past the matrix) are ignored.
pub fn decode_mask(mask: &[u8]) -> Vec<usize> {
    let mut out = Vec::new();
    for row in 0..MATRIX_ROWS {
        for (col, byte) in mask.iter().take(MASK_LEN).enumerate() {
            if byte & (1 << row) != 0 {
                out.push(row * MATRIX_COLS + col);
            }
        }
    }
    out
}

/// A table that is read and written in fixed-size pages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TableSpec {
    pub len: usize,
    pub page: usize,
}

impl TableSpec {
    pub const fn pages(&self) -> usize {
        self.len.div_ceil(self.page)
    }

    /// Payload length of page `n`.
    pub fn page_len(&self, n: usize) -> usize {
        self.page.min(self.len.saturating_sub(n * self.page))
    }

    /// Write frames `cmd sel 00 pp len data..` for the whole table.
    pub fn write_frames(&self, cmd: u8, sel: u8, table: &[u8]) -> Result<Vec<Body>> {
        if table.len() != self.len {
            return Err(Error::Invalid(format!("table is {} bytes, expected {}", table.len(), self.len)));
        }
        Ok(table
            .chunks(self.page)
            .enumerate()
            .map(|(n, chunk)| {
                let mut b = request(cmd, sel);
                b[2..4].copy_from_slice(&(n as u16).to_be_bytes());
                b[4] = chunk.len() as u8;
                b[PAYLOAD_OFFSET..PAYLOAD_OFFSET + chunk.len()].copy_from_slice(chunk);
                b
            })
            .collect())
    }

    /// Joins read pages in order. Each page must carry its index and exactly its expected length.
    pub fn assemble(&self, pages: &[Body]) -> Result<Vec<u8>> {
        if pages.len() != self.pages() {
            return protocol(format!("got {} pages, expected {}", pages.len(), self.pages()));
        }
        let mut out = Vec::with_capacity(self.len);
        for (n, b) in pages.iter().enumerate() {
            if page_of(b) as usize != n {
                return protocol(format!("frame {:02x}: page {} where {n} was expected", b[0], page_of(b)));
            }
            let p = payload(b)?;
            if p.len() != self.page_len(n) {
                return protocol(format!(
                    "frame {:02x} page {n}: {} bytes, expected {}",
                    b[0],
                    p.len(),
                    self.page_len(n)
                ));
            }
            out.extend_from_slice(p);
        }
        Ok(out)
    }
}

/// How a reply is recognised (replies carry no request ID, protocol 2.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reply {
    /// `b[0] == cmd`.
    Cmd(u8),
    /// `b[0] == cmd && b[1] == sel`.
    CmdSel(u8, u8),
    /// `0x21` reply whose `b[5]` echoes the sub-command.
    TriggerSub(u8),
    /// `0x21` per-key trigger reply, echoing row and column in `b[15..16]`.
    KeyTrigger(u8, u8),
    /// ACK frame: the first six bytes must match exactly (protocol 2.4).
    Ack([u8; 6]),
}

impl Reply {
    pub fn matches(&self, b: &Body) -> bool {
        match *self {
            Reply::Cmd(c) => b[0] == c,
            Reply::CmdSel(c, s) => b[0] == c && b[1] == s,
            Reply::TriggerSub(sub) => b[0] == CMD_TRIGGER && b[4] != 0x01 && b[5] == sub,
            Reply::KeyTrigger(row, col) => {
                b[0] == CMD_TRIGGER && b[5] == SUB_KEY_TRIGGER && b[4] >= 0x0c && b[15] == row && b[16] == col
            }
            Reply::Ack(a) => b[..6] == a,
        }
    }
}

/// ACK `cmd sel 00 00 00 00`.
pub const fn ack(cmd: u8, sel: u8) -> Reply {
    Reply::Ack([cmd, sel, 0, 0, 0, 0])
}

/// ACK of every `0x21` write and of travel-test open/close: `21 00 00 00 01 00`.
pub const TRIGGER_ACK: Reply = Reply::Ack([CMD_TRIGGER, 0, 0, 0, 1, 0]);

/// Whether a request may be sent to a board that has not passed the product-code check (protocol 1.1):
/// only `0x0D` and `0x01`.
pub fn is_identification(b: &Body) -> bool {
    matches!(b[0], CMD_IDENT | CMD_INFO)
}

/// Allowlist of protocol 2.6. Everything else, and everything marked DO NOT USE (side-light write
/// `08 00`, gamepad `14 03`, sleep timers `14 01/02`, onboard profiles `21 sub 11/12`, single-key
/// calibration `21 sub 07`, old-style calibration `21 sub 08`), is refused.
pub fn check_allowed(b: &Body) -> Result<()> {
    let (cmd, sel) = (b[0], b[1]);
    let ok = match cmd {
        CMD_INFO | CMD_SIDE_MODES | CMD_MODES | CMD_IDENT | CMD_FN_FIXED => sel == 0,
        CMD_EFFECT => sel <= 1,
        CMD_SIDE_EFFECT => sel == 1,
        CMD_CUSTOM_RGB => matches!(sel, 0 | 1 | 0x20 | 0x80 | 0x81),
        CMD_PERF => sel == 0,
        CMD_MUSIC => sel == 0,
        CMD_KEYMAP => matches!(sel, 0 | 2 | 0x80 | 0x82),
        CMD_MACRO => matches!(sel, 0..=9 | 0x80..=0x89),
        CMD_TRIGGER => {
            sel == 0
                && matches!(
                    (b[4], b[5]),
                    (TRIGGER_B4, SUB_MODE_FIXED | SUB_MODE_RAPID | SUB_MODE_RAPID_SPLIT)
                        | (TRIGGER_B4, SUB_TRAVEL_OPEN | SUB_TRAVEL_CLOSE | SUB_LIMITS | SUB_KEY_TRIGGER)
                        | (TRIGGER_B4, SUB_TRIGGER_RESET | SUB_CALIB_START | SUB_CALIB_STOP)
                        | (0x00 | 0x01, SUB_POLL_RATE)
                )
        }
        CMD_ADVANCED => matches!(sel, 0..=39 | 0x80),
        CMD_SOCD => sel <= 3,
        CMD_SWITCH => sel <= 2,
        CMD_DEAD_BAND => sel <= 1,
        _ => false,
    };
    if ok {
        Ok(())
    } else {
        Err(Error::Refused(format!("frame {} is not an allowed command", hex(&b[..8]))))
    }
}

/// Space-separated lowercase hex.
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" ")
}

/// Parses space-separated hex (`"01 0d 00"`) into a zero-padded body; for tests and fixtures.
/// A leading report ID is not stripped.
pub fn parse_hex_body(s: &str) -> Result<Body> {
    let mut b = [0u8; BODY_LEN];
    let bytes: Vec<&str> = s.split_whitespace().collect();
    if bytes.len() > BODY_LEN {
        return Err(Error::Invalid(format!("{} bytes do not fit a {BODY_LEN}-byte body", bytes.len())));
    }
    for (i, t) in bytes.iter().enumerate() {
        b[i] = u8::from_str_radix(t, 16).map_err(|_| Error::Invalid(format!("bad hex byte {t:?}")))?;
    }
    Ok(b)
}

#[cfg(test)]
pub(crate) fn body(s: &str) -> Body {
    parse_hex_body(s).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// All-keys mask of the WIN 60 HE (protocol 2.5).
    const ALL_KEYS_MASK: [u8; 22] = [
        0x3e, 0x26, 0x3e, 0x1e, 0x1e, 0x1e, 0x3e, 0x1e, 0x1e, 0x3e, 0x3e, 0x3e, 0x3e, 0x00, 0x0e, 0, 0, 0, 0, 0, 0, 0,
    ];

    #[test]
    fn mask_of_all_layout_keys_matches_the_capture() {
        let idx: Vec<usize> = crate::layout::KEYS.iter().map(|k| k.index as usize).collect();
        assert_eq!(encode_mask(&idx).unwrap(), ALL_KEYS_MASK);
        assert_eq!(decode_mask(&ALL_KEYS_MASK), idx);
    }

    #[test]
    fn mask_empty_boundary_and_error() {
        assert_eq!(encode_mask(&[]).unwrap(), [0; 22]);
        assert!(decode_mask(&[]).is_empty());
        assert!(decode_mask(&[0; 22]).is_empty());
        let m = encode_mask(&[0, 131]).unwrap();
        assert_eq!(m[0], 1);
        assert_eq!(m[21], 1 << 5);
        assert_eq!(decode_mask(&m), vec![0, 131]);
        assert!(matches!(encode_mask(&[132]), Err(Error::Invalid(_))));
        // bits for rows 6 and 7 do not exist
        assert!(decode_mask(&[0xc0; 22]).is_empty());
    }

    #[test]
    fn fn_fixed_mask_from_the_owner_board() {
        let m = body("02 22 0e 0a 02 02 22 12 1e 3e 0e 2e 3e 20 20 20 00 00 00 00 00 00");
        let keys = decode_mask(&m[..22]);
        let mut expect: Vec<usize> = (22..=34).collect();
        expect.extend([46, 52, 53, 54, 55, 56, 68, 69, 74, 75, 76, 77, 78, 95, 96, 97, 100, 111, 116, 119, 121, 122]);
        expect.extend([123, 124, 125]);
        expect.sort();
        assert_eq!(keys, expect);
    }

    #[test]
    fn row_col_round_trip() {
        assert_eq!(row_col(110), (5, 0));
        assert_eq!(row_col(46), (2, 2));
        assert_eq!(index_of(5, 12), Some(122));
        assert_eq!(index_of(0, 0), Some(0));
        assert_eq!(index_of(6, 0), None);
        assert_eq!(index_of(0, 22), None);
    }

    #[test]
    fn payload_honours_length_byte() {
        let b = body("0d 00 00 01 03 41 42 43 99 99");
        assert_eq!(payload(&b).unwrap(), b"ABC");
        assert_eq!(payload(&body("0d")).unwrap(), b"");
        assert_eq!(payload(&body("0d 00 00 00 3a")).unwrap().len(), 58);
        assert!(payload(&body("0d 00 00 00 3b")).is_err());
        assert!(payload_min(&b, 4).is_err());
        assert!(payload_min(&b, 3).is_ok());
    }

    #[test]
    fn table_paging_round_trip() {
        let spec = TableSpec { len: 396, page: 54 };
        assert_eq!(spec.pages(), 8);
        assert_eq!(spec.page_len(7), 18);
        let table: Vec<u8> = (0..396).map(|i| i as u8).collect();
        let frames = spec.write_frames(0x09, 1, &table).unwrap();
        assert_eq!(frames.len(), 8);
        assert_eq!(&frames[7][..5], &[0x09, 0x01, 0x00, 0x07, 0x12]);
        assert_eq!(&frames[0][..6], &[0x09, 0x01, 0x00, 0x00, 0x36, 0x00]);
        assert_eq!(spec.assemble(&frames).unwrap(), table);
    }

    #[test]
    fn table_paging_errors() {
        let spec = TableSpec { len: 132, page: 58 };
        assert!(spec.write_frames(0x25, 1, &[0; 131]).is_err());
        assert!(spec.write_frames(0x25, 1, &[]).is_err());
        let frames = spec.write_frames(0x25, 1, &[1; 132]).unwrap();
        assert!(spec.assemble(&frames[..2]).is_err(), "missing page");
        let mut swapped = frames.clone();
        swapped.swap(0, 1);
        assert!(spec.assemble(&swapped).is_err(), "out of order");
        let mut short = frames.clone();
        short[2][4] = 15;
        assert!(spec.assemble(&short).is_err(), "wrong length");
    }

    #[test]
    fn reply_matching() {
        let a = body("21 00 00 00 01 00");
        let sample = body("21 00 00 00 05 01 05 00 6e");
        assert!(TRIGGER_ACK.matches(&a));
        assert!(!TRIGGER_ACK.matches(&sample));
        assert!(ack(0x25, 1).matches(&body("25 01 00 00 00 00 ff ff")));
        assert!(!ack(0x25, 1).matches(&body("25 00 00 00 00 00")));
        let key = body("21 00 00 00 0c 05 0c 64 64 32 32 00 00 00 00 05 0c");
        assert!(Reply::KeyTrigger(5, 12).matches(&key));
        assert!(!Reply::KeyTrigger(1, 0).matches(&key));
        assert!(Reply::TriggerSub(4).matches(&body("21 00 00 00 06 04 54 01 01 01 08")));
        assert!(!Reply::TriggerSub(1).matches(&body("21 00 00 00 01 01")), "an ACK is not a reply");
        assert!(Reply::CmdSel(0x18, 0x80).matches(&body("18 80 00 00 38")));
        assert!(!Reply::CmdSel(0x18, 0x80).matches(&body("18 82 00 00 38")));
    }

    #[test]
    fn allowlist() {
        for ok in [
            "0d", "01", "0e", "0a", "02", "07 01", "07 00", "08 01", "09 20", "09 80", "09 01", "14 00 00 00 01",
            "17 00", "18 82", "18 02", "19 89", "19 09", "22 80", "22 27", "24 03", "25 02", "26 01",
            "21 00 00 00 18 00", "21 00 00 00 18 0c", "21 00 00 00 18 0d", "21 00 00 00 18 02", "21 00 00 00 18 03",
            "21 00 00 00 18 04", "21 00 00 00 18 05 01", "21 00 00 00 18 06", "21 00 00 00 01 09",
            "21 00 00 00 00 09 01 08", "21 00 00 00 18 0f", "21 00 00 00 18 10",
        ] {
            assert!(check_allowed(&body(ok)).is_ok(), "{ok}");
        }
        for bad in [
            "00", "03", "0b", "ff", "08 00", "14 01", "14 02", "14 03", "19 0a", "19 8a", "22 28", "22 81", "24 04",
            "25 03", "26 02", "09 02", "18 01", "21 00 00 00 00 11", "21 00 00 00 00 12 01", "21 00 00 00 18 07",
            "21 00 00 00 18 08", "21 01 00 00 18 04", "21 00 00 00 18 09", "21 00 00 00 00 05",
        ] {
            assert!(matches!(check_allowed(&body(bad)), Err(Error::Refused(_))), "{bad}");
        }
    }

    #[test]
    fn identification_commands() {
        assert!(is_identification(&body("0d")));
        assert!(is_identification(&body("01")));
        assert!(!is_identification(&body("18 80")));
    }

    #[test]
    fn hex_parsing() {
        assert_eq!(parse_hex_body("").unwrap(), [0; 63]);
        assert!(parse_hex_body("zz").is_err());
        assert!(parse_hex_body(&"00 ".repeat(64)).is_err());
        assert_eq!(hex(&[1, 0xab]), "01 ab");
    }
}

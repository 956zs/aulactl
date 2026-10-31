//! Physical layout of the WIN 60 HE (protocol section 14) and key selectors for per-key commands.
//!
//! Geometry is the official GUI's: 1 u = 38 px pitch, keys drawn 3 px narrower than their pitch,
//! origin (10, 15) px.

use crate::error::{invalid, Result};
use crate::frame::MATRIX_COLS;
use crate::keycodes;

/// Pixel pitch of 1 u in the official layout.
pub const PX_PER_U: f32 = 38.0;
const ORIGIN_X_PX: f32 = 10.0;
const ORIGIN_Y_PX: f32 = 15.0;
/// Keys are drawn this much narrower than their pitch.
const GAP_PX: f32 = 3.0;
/// Number of visual rows (0 = number row, 4 = bottom row).
pub const VISUAL_ROWS: u8 = 5;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Key {
    /// Key index `row * 22 + col` used by every per-key table (protocol 2.5).
    pub index: u8,
    /// Short legend, e.g. `Esc`, `Q`, `L-Shift`.
    pub label: &'static str,
    /// `KeyboardEvent.code`-style name, unique per key, e.g. `Escape`, `KeyQ`, `KeyFn`.
    pub code: &'static str,
    /// Base-layer keymap entry the official driver writes for this key.
    pub default_entry: [u8; 4],
    /// Visual row, 0 (number row) .. 4 (bottom row).
    pub visual_row: u8,
    pub x_px: u16,
    pub y_px: u16,
    pub w_px: u16,
    /// In the `0x0E` Fn-fixed mask of the owner's board. The board's own mask is authoritative.
    pub fn_fixed: bool,
}

impl Key {
    pub fn row(&self) -> u8 {
        self.index / MATRIX_COLS as u8
    }

    pub fn col(&self) -> u8 {
        self.index % MATRIX_COLS as u8
    }

    /// Left edge in u.
    pub fn x(&self) -> f32 {
        (self.x_px as f32 - ORIGIN_X_PX) / PX_PER_U
    }

    /// Top edge in u (equals the visual row).
    pub fn y(&self) -> f32 {
        (self.y_px as f32 - ORIGIN_Y_PX) / PX_PER_U
    }

    /// Width in u, including the gap.
    pub fn width(&self) -> f32 {
        (self.w_px as f32 + GAP_PX) / PX_PER_U
    }

    /// HID usage of the default entry (0 for Fn).
    pub fn default_usage(&self) -> u8 {
        if self.default_entry[0] == 0 {
            self.default_entry[1]
        } else {
            0
        }
    }
}

#[allow(clippy::too_many_arguments)]
const fn key(
    index: u8,
    label: &'static str,
    code: &'static str,
    default_entry: [u8; 4],
    visual_row: u8,
    x_px: u16,
    y_px: u16,
    w_px: u16,
    fn_fixed: bool,
) -> Key {
    Key { index, label, code, default_entry, visual_row, x_px, y_px, w_px, fn_fixed }
}

/// The 61 keys, in index order.
pub static KEYS: [Key; 61] = [
    key(22, "Esc", "Escape", [0x00, 0x29, 0, 0], 0, 10, 15, 35, true),
    key(23, "1!", "Digit1", [0x00, 0x1e, 0, 0], 0, 48, 15, 35, true),
    key(24, "2@", "Digit2", [0x00, 0x1f, 0, 0], 0, 86, 15, 35, true),
    key(25, "3#", "Digit3", [0x00, 0x20, 0, 0], 0, 124, 15, 35, true),
    key(26, "4$", "Digit4", [0x00, 0x21, 0, 0], 0, 162, 15, 35, true),
    key(27, "5%", "Digit5", [0x00, 0x22, 0, 0], 0, 200, 15, 35, true),
    key(28, "6^", "Digit6", [0x00, 0x23, 0, 0], 0, 238, 15, 35, true),
    key(29, "7&", "Digit7", [0x00, 0x24, 0, 0], 0, 276, 15, 35, true),
    key(30, "8*", "Digit8", [0x00, 0x25, 0, 0], 0, 314, 15, 35, true),
    key(31, "9(", "Digit9", [0x00, 0x26, 0, 0], 0, 352, 15, 35, true),
    key(32, "0)", "Digit0", [0x00, 0x27, 0, 0], 0, 390, 15, 35, true),
    key(33, "-_", "Minus", [0x00, 0x2d, 0, 0], 0, 428, 15, 35, true),
    key(34, "=+", "Equal", [0x00, 0x2e, 0, 0], 0, 466, 15, 35, true),
    key(36, "Back", "Backspace", [0x00, 0x2a, 0, 0], 0, 504, 15, 73, false),
    key(44, "Tab", "Tab", [0x00, 0x2b, 0, 0], 1, 10, 53, 54, false),
    key(45, "Q", "KeyQ", [0x00, 0x14, 0, 0], 1, 67, 53, 35, false),
    key(46, "W", "KeyW", [0x00, 0x1a, 0, 0], 1, 105, 53, 35, true),
    key(47, "E", "KeyE", [0x00, 0x08, 0, 0], 1, 143, 53, 35, false),
    key(48, "R", "KeyR", [0x00, 0x15, 0, 0], 1, 181, 53, 35, false),
    key(49, "T", "KeyT", [0x00, 0x17, 0, 0], 1, 219, 53, 35, false),
    key(50, "Y", "KeyY", [0x00, 0x1c, 0, 0], 1, 257, 53, 35, false),
    key(51, "U", "KeyU", [0x00, 0x18, 0, 0], 1, 295, 53, 35, false),
    key(52, "I", "KeyI", [0x00, 0x0c, 0, 0], 1, 333, 53, 35, true),
    key(53, "O", "KeyO", [0x00, 0x12, 0, 0], 1, 371, 53, 35, true),
    key(54, "P", "KeyP", [0x00, 0x13, 0, 0], 1, 409, 53, 35, true),
    key(55, "[{", "BracketLeft", [0x00, 0x2f, 0, 0], 1, 447, 53, 35, true),
    key(56, "]}", "BracketRight", [0x00, 0x30, 0, 0], 1, 485, 53, 35, true),
    key(58, "\\|", "Backslash", [0x00, 0x31, 0, 0], 1, 523, 53, 54, false),
    key(66, "Caps", "CapsLock", [0x00, 0x39, 0, 0], 2, 10, 91, 63, false),
    key(68, "A", "KeyA", [0x00, 0x04, 0, 0], 2, 76, 91, 35, true),
    key(69, "S", "KeyS", [0x00, 0x16, 0, 0], 2, 114, 91, 35, true),
    key(70, "D", "KeyD", [0x00, 0x07, 0, 0], 2, 152, 91, 35, false),
    key(71, "F", "KeyF", [0x00, 0x09, 0, 0], 2, 190, 91, 35, false),
    key(72, "G", "KeyG", [0x00, 0x0a, 0, 0], 2, 228, 91, 35, false),
    key(73, "H", "KeyH", [0x00, 0x0b, 0, 0], 2, 266, 91, 35, false),
    key(74, "J", "KeyJ", [0x00, 0x0d, 0, 0], 2, 304, 91, 35, true),
    key(75, "K", "KeyK", [0x00, 0x0e, 0, 0], 2, 342, 91, 35, true),
    key(76, "L", "KeyL", [0x00, 0x0f, 0, 0], 2, 380, 91, 35, true),
    key(77, ";:", "Semicolon", [0x00, 0x33, 0, 0], 2, 418, 91, 35, true),
    key(78, "'\"", "Quote", [0x00, 0x34, 0, 0], 2, 456, 91, 35, true),
    key(80, "Enter", "Enter", [0x00, 0x28, 0, 0], 2, 494, 91, 83, false),
    key(88, "L-Shift", "ShiftLeft", [0x00, 0xe1, 0, 0], 3, 10, 129, 83, false),
    key(90, "Z", "KeyZ", [0x00, 0x1d, 0, 0], 3, 96, 129, 35, false),
    key(91, "X", "KeyX", [0x00, 0x1b, 0, 0], 3, 134, 129, 35, false),
    key(92, "C", "KeyC", [0x00, 0x06, 0, 0], 3, 172, 129, 35, false),
    key(93, "V", "KeyV", [0x00, 0x19, 0, 0], 3, 210, 129, 35, false),
    key(94, "B", "KeyB", [0x00, 0x05, 0, 0], 3, 248, 129, 35, false),
    key(95, "N", "KeyN", [0x00, 0x11, 0, 0], 3, 286, 129, 35, true),
    key(96, "M", "KeyM", [0x00, 0x10, 0, 0], 3, 324, 129, 35, true),
    key(97, ",<", "Comma", [0x00, 0x36, 0, 0], 3, 362, 129, 35, true),
    key(98, ".>", "Period", [0x00, 0x37, 0, 0], 3, 400, 129, 35, false),
    key(99, "/?", "Slash", [0x00, 0x38, 0, 0], 3, 438, 129, 35, false),
    key(100, "R-Shift", "ShiftRight", [0x00, 0xe5, 0, 0], 3, 476, 129, 101, true),
    key(110, "L-Ctrl", "ControlLeft", [0x00, 0xe0, 0, 0], 4, 10, 167, 47, false),
    key(111, "L-Win", "MetaLeft", [0x00, 0xe3, 0, 0], 4, 60, 167, 47, true),
    key(112, "L-Alt", "AltLeft", [0x00, 0xe2, 0, 0], 4, 110, 167, 47, false),
    key(116, "Space", "Space", [0x00, 0x2c, 0, 0], 4, 160, 167, 217, true),
    key(119, "R-Alt", "AltRight", [0x00, 0xe6, 0, 0], 4, 380, 167, 47, true),
    key(120, "Menu", "ContextMenu", [0x00, 0x65, 0, 0], 4, 430, 167, 47, false),
    key(121, "R-Ctrl", "ControlRight", [0x00, 0xe4, 0, 0], 4, 480, 167, 47, true),
    key(122, "Fn", "KeyFn", [0x01, 0xfa, 0, 0], 4, 530, 167, 47, true),
];

/// The key at matrix index `idx`, if it is a physical key.
pub fn by_index(idx: usize) -> Option<&'static Key> {
    KEYS.iter().find(|k| k.index as usize == idx)
}

/// The key with this `code` (exact, case-insensitive).
pub fn by_code(code: &str) -> Option<&'static Key> {
    KEYS.iter().find(|k| k.code.eq_ignore_ascii_case(code))
}

/// Whether `idx` is one of the 61 physical keys.
pub fn is_key(idx: usize) -> bool {
    by_index(idx).is_some()
}

/// All physical key indices, ascending.
pub fn all_indices() -> Vec<usize> {
    KEYS.iter().map(|k| k.index as usize).collect()
}

const GROUP_WASD: [&str; 4] = ["KeyW", "KeyA", "KeyS", "KeyD"];
const GROUP_MODS: [&str; 9] = [
    "ShiftLeft", "ShiftRight", "ControlLeft", "ControlRight", "AltLeft", "AltRight", "MetaLeft", "ContextMenu", "KeyFn",
];

/// Resolves a key selector to key indices (ascending, without duplicates).
///
/// A selector is a comma-separated list of terms, case-insensitive:
/// `all`; groups `wasd`, `letters`, `digits`, `mods`; `row:N` (visual row, 0 = number row .. 4 = bottom row);
/// `idx:N` (matrix index); a key code (`KeyQ`, `Escape`, `ShiftLeft`); a legend (`Q`, `Esc`, `Caps`,
/// `L-Shift`); or any keycode name that is the default of a key (`a`, `Esc`, `LShift`, `Win`).
/// `,` cannot be used as a legend; use `Comma`.
pub fn select(selector: &str) -> Result<Vec<usize>> {
    let mut out = Vec::new();
    let terms: Vec<&str> = selector.split(',').map(str::trim).collect();
    if terms.iter().all(|t| t.is_empty()) {
        return invalid("empty key selector");
    }
    for term in terms {
        if term.is_empty() {
            return invalid(format!("empty term in key selector {selector:?}"));
        }
        out.extend(select_term(term)?);
    }
    out.sort_unstable();
    out.dedup();
    Ok(out)
}

fn select_term(term: &str) -> Result<Vec<usize>> {
    let lower = term.to_ascii_lowercase();
    let codes = |codes: &[&str]| codes.iter().filter_map(|c| by_code(c)).map(|k| k.index as usize).collect();
    match lower.as_str() {
        "all" => return Ok(all_indices()),
        "wasd" => return Ok(codes(&GROUP_WASD)),
        "mods" => return Ok(codes(&GROUP_MODS)),
        "letters" => return Ok(KEYS.iter().filter(|k| k.code.len() == 4 && k.code.starts_with("Key")).map(|k| k.index as usize).collect()),
        "digits" => return Ok(KEYS.iter().filter(|k| k.code.starts_with("Digit")).map(|k| k.index as usize).collect()),
        _ => {}
    }
    if let Some(n) = lower.strip_prefix("row:") {
        let row: u8 = n.parse().map_err(|_| crate::Error::Invalid(format!("bad row in {term:?}")))?;
        if row >= VISUAL_ROWS {
            return invalid(format!("row {row} does not exist (0..{})", VISUAL_ROWS - 1));
        }
        return Ok(KEYS.iter().filter(|k| k.visual_row == row).map(|k| k.index as usize).collect());
    }
    if let Some(n) = lower.strip_prefix("idx:") {
        let idx: usize = n.parse().map_err(|_| crate::Error::Invalid(format!("bad index in {term:?}")))?;
        if !is_key(idx) {
            return invalid(format!("index {idx} is not a key of this board"));
        }
        return Ok(vec![idx]);
    }
    if let Some(k) = by_code(term).or_else(|| KEYS.iter().find(|k| k.label.eq_ignore_ascii_case(term))) {
        return Ok(vec![k.index as usize]);
    }
    if let Some(usage) = keycodes::usage_by_name(term) {
        if let Some(k) = KEYS.iter().find(|k| k.default_entry == [0, usage, 0, 0]) {
            return Ok(vec![k.index as usize]);
        }
    }
    if lower == "fn" {
        return Ok(codes(&["KeyFn"]));
    }
    invalid(format!("unknown key {term:?} (use a code like KeyQ, a legend like Esc, row:N, idx:N, wasd or all)"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_is_consistent() {
        assert_eq!(KEYS.len(), 61);
        for w in KEYS.windows(2) {
            assert!(w[0].index < w[1].index, "sorted and unique");
        }
        for k in &KEYS {
            assert!(by_code(k.code).unwrap().index == k.index, "unique code {}", k.code);
            assert_eq!(k.y_px, 15 + 38 * k.visual_row as u16);
            assert!((k.y() - k.visual_row as f32).abs() < 1e-6);
            if k.code == "KeyFn" {
                assert_eq!(k.default_entry, [1, 0xfa, 0, 0]);
            } else {
                assert_eq!(keycodes::name_of_usage(k.default_usage()), Some(k.code), "keycode name of {}", k.code);
            }
        }
    }

    #[test]
    fn geometry_matches_the_protocol_table() {
        let caps = by_code("CapsLock").unwrap();
        assert!((caps.width() - 1.74).abs() < 0.01);
        let space = by_code("Space").unwrap();
        assert!((space.x() - 3.95).abs() < 0.01);
        assert!((space.width() - 5.79).abs() < 0.01);
        let a = by_code("KeyA").unwrap();
        assert_eq!((a.row(), a.col()), (3, 2));
        assert!((a.x() - 1.74).abs() < 0.01);
        // every visual row is 15 u wide
        for row in 0..VISUAL_ROWS {
            let last = KEYS.iter().filter(|k| k.visual_row == row).next_back().unwrap();
            assert!((last.x() + last.width() - 15.0).abs() < 0.02, "row {row}");
        }
    }

    #[test]
    fn lookup_by_index() {
        assert_eq!(by_index(110).unwrap().code, "ControlLeft");
        assert!(by_index(0).is_none());
        assert!(by_index(131).is_none());
        assert!(by_index(1000).is_none());
        assert!(is_key(122));
        assert_eq!(all_indices().len(), 61);
    }

    #[test]
    fn selectors() {
        assert_eq!(select("wasd").unwrap(), vec![46, 68, 69, 70]);
        assert_eq!(select("W, a ,S,KeyD").unwrap(), vec![46, 68, 69, 70]);
        assert_eq!(select("all").unwrap().len(), 61);
        assert_eq!(select("row:0").unwrap().len(), 14);
        assert_eq!(select("row:4").unwrap().len(), 8);
        assert_eq!(select("Esc").unwrap(), vec![22]);
        assert_eq!(select("escape").unwrap(), vec![22]);
        assert_eq!(select("L-Shift").unwrap(), vec![88]);
        assert_eq!(select("LShift").unwrap(), vec![88]);
        assert_eq!(select("fn").unwrap(), vec![122]);
        assert_eq!(select("idx:46").unwrap(), vec![46]);
        assert_eq!(select("Q,Q,q").unwrap(), vec![45]);
        assert_eq!(select("letters").unwrap().len(), 26);
        assert_eq!(select("digits").unwrap().len(), 10);
        assert_eq!(select("mods").unwrap().len(), 9);
        assert_eq!(select("Comma").unwrap(), vec![97]);
    }

    #[test]
    fn selector_errors() {
        for bad in ["", " , ", "a,,b", "row:5", "row:x", "idx:0", "idx:-1", "F1", "nosuchkey", "row:"] {
            assert!(select(bad).is_err(), "{bad:?}");
        }
    }
}

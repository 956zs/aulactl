//! Keys pressed in the window: Qt's `nativeScanCode` (X11 / Wayland keycode = evdev code + 8) to the HID
//! keyboard usage and its core name, for macro recording and the key tester.

use aula_core::keycodes;
use aula_core::layout;

/// `(HID usage, evdev code)`. Usages from the HID usage tables (keyboard page), evdev codes from
/// linux/input-event-codes.h.
const KEYS: &[(u8, u16)] = &[
    (0x04, 30), (0x05, 48), (0x06, 46), (0x07, 32), (0x08, 18), (0x09, 33), (0x0a, 34), (0x0b, 35),
    (0x0c, 23), (0x0d, 36), (0x0e, 37), (0x0f, 38), (0x10, 50), (0x11, 49), (0x12, 24), (0x13, 25),
    (0x14, 16), (0x15, 19), (0x16, 31), (0x17, 20), (0x18, 22), (0x19, 47), (0x1a, 17), (0x1b, 45),
    (0x1c, 21), (0x1d, 44),
    (0x1e, 2), (0x1f, 3), (0x20, 4), (0x21, 5), (0x22, 6), (0x23, 7), (0x24, 8), (0x25, 9), (0x26, 10), (0x27, 11),
    (0x28, 28), (0x29, 1), (0x2a, 14), (0x2b, 15), (0x2c, 57), (0x2d, 12), (0x2e, 13), (0x2f, 26), (0x30, 27),
    (0x31, 43), (0x33, 39), (0x34, 40), (0x35, 41), (0x36, 51), (0x37, 52), (0x38, 53), (0x39, 58),
    (0x3a, 59), (0x3b, 60), (0x3c, 61), (0x3d, 62), (0x3e, 63), (0x3f, 64), (0x40, 65), (0x41, 66),
    (0x42, 67), (0x43, 68), (0x44, 87), (0x45, 88),
    (0x46, 99), (0x47, 70), (0x48, 119), (0x49, 110), (0x4a, 102), (0x4b, 104), (0x4c, 111), (0x4d, 107),
    (0x4e, 109), (0x4f, 106), (0x50, 105), (0x51, 108), (0x52, 103),
    (0x53, 69), (0x54, 98), (0x55, 55), (0x56, 74), (0x57, 78), (0x58, 96), (0x59, 79), (0x5a, 80),
    (0x5b, 81), (0x5c, 75), (0x5d, 76), (0x5e, 77), (0x5f, 71), (0x60, 72), (0x61, 73), (0x62, 82),
    (0x63, 83), (0x64, 86), (0x65, 127),
    (0x68, 183), (0x69, 184), (0x6a, 185), (0x6b, 186), (0x6c, 187), (0x6d, 188), (0x6e, 189), (0x6f, 190),
    (0x70, 191), (0x71, 192), (0x72, 193), (0x73, 194),
    (0x87, 89), (0x88, 93), (0x89, 124), (0x8a, 92), (0x8b, 94),
    (0xe0, 29), (0xe1, 42), (0xe2, 56), (0xe3, 125), (0xe4, 97), (0xe5, 54), (0xe6, 100), (0xe7, 126),
];

/// X11 and Wayland keycodes are the evdev code plus 8.
const XKB_OFFSET: i32 = 8;

/// HID usage of a `KeyEvent.nativeScanCode`.
pub fn usage_of_scan_code(scan_code: i32) -> Option<u8> {
    let evdev = u16::try_from(scan_code.checked_sub(XKB_OFFSET)?).ok()?;
    KEYS.iter().find(|(_, e)| *e == evdev).map(|(u, _)| *u)
}

/// Core keycode name (`KeyA`, `ShiftLeft`) of a scan code, for macro steps.
pub fn usage_name(scan_code: i32) -> Option<&'static str> {
    usage_of_scan_code(scan_code).and_then(keycodes::name_of_usage)
}

/// Layout code of the physical key that sends this scan code with the default keymap (`KeyQ`, `Space`).
pub fn layout_code(scan_code: i32) -> Option<&'static str> {
    let usage = usage_of_scan_code(scan_code)?;
    layout::KEYS.iter().find(|k| k.default_entry == [0, usage, 0, 0]).map(|k| k.code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn common_keys() {
        // A = evdev 30, Esc = 1, LShift = 42.
        assert_eq!(usage_name(30 + 8), Some("KeyA"));
        assert_eq!(usage_name(1 + 8), Some("Escape"));
        assert_eq!(usage_name(42 + 8), Some("ShiftLeft"));
        assert_eq!(usage_name(125 + 8), Some("MetaLeft"));
        assert_eq!(usage_name(88 + 8), Some("F12"));
    }

    #[test]
    fn unknown_and_out_of_range() {
        assert_eq!(usage_name(0), None);
        assert_eq!(usage_name(7), None);
        assert_eq!(usage_name(-1), None);
        assert_eq!(usage_name(i32::MIN), None);
        assert_eq!(usage_name(i32::MAX), None);
        // KEY_MUTE (113) is a media key, not a keyboard usage.
        assert_eq!(usage_name(113 + 8), None);
    }

    #[test]
    fn layout_codes_for_the_tester() {
        assert_eq!(layout_code(16 + 8), Some("KeyQ"));
        assert_eq!(layout_code(57 + 8), Some("Space"));
        assert_eq!(layout_code(127 + 8), Some("ContextMenu"));
        // F1 is not a key on a 60 % board.
        assert_eq!(layout_code(59 + 8), None);
    }

    #[test]
    fn every_entry_is_named_and_unique() {
        for (u, e) in KEYS {
            assert!(keycodes::name_of_usage(*u).is_some(), "usage {u:#x}");
            assert_eq!(KEYS.iter().filter(|(_, e2)| e2 == e).count(), 1, "evdev {e}");
        }
        // Every default key of the board can be recorded (Fn never reaches the host).
        for k in layout::KEYS.iter().filter(|k| k.code != "KeyFn") {
            assert!(KEYS.iter().any(|(u, _)| k.default_entry == [0, *u, 0, 0]), "{}", k.code);
        }
    }
}

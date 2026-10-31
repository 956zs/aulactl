//! Macros (protocol section 7): 10 slots x 256 bytes.

use crate::error::{invalid, protocol, Result};
use crate::frame::TableSpec;
use crate::keycodes::{name_of_usage, Usage};
use serde::{Deserialize, Serialize};

/// One slot image, 58-byte pages.
pub const TABLE: TableSpec = TableSpec { len: SLOT_LEN, page: 58 };
pub const SLOT_LEN: usize = 256;
pub const SLOTS: u8 = 10;
const HEADER_LEN: usize = 8;
/// Events that fit a slot: (256 - 8) / 4.
pub const MAX_EVENTS: usize = (SLOT_LEN - HEADER_LEN) / 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PlayMode {
    /// Play once.
    Once,
    /// Repeat `repeat` times.
    Repeat,
    /// Press to start, press again to stop.
    Toggle,
    /// Repeat while held.
    WhileHeld,
}

impl PlayMode {
    fn from_byte(b: u8) -> Option<Self> {
        Some(match b {
            0 => PlayMode::Once,
            1 => PlayMode::Repeat,
            2 => PlayMode::Toggle,
            3 => PlayMode::WhileHeld,
            _ => return None,
        })
    }

    fn byte(self) -> u8 {
        self as u8
    }
}

/// What an event presses or releases.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MacroInput {
    /// Keyboard usage.
    Key(Usage),
    /// Mouse button bit: 1 left, 2 right, 4 middle (the official write path).
    Mouse(u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MacroEvent {
    #[serde(flatten)]
    pub input: MacroInput,
    /// true = press, false = release.
    pub down: bool,
    /// Wait after this event, in ms.
    pub delay_ms: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Macro {
    /// Header byte 0: the macro number of the official UI; bit 7 = "same macro already stored in an earlier
    /// slot". Kept as read.
    pub id: u8,
    pub mode: PlayMode,
    /// Repeat count for `Repeat` (the official driver writes 1 for the other modes).
    pub repeat: u16,
    pub events: Vec<MacroEvent>,
}

impl Macro {
    pub fn validate(&self) -> Result<()> {
        if self.events.is_empty() {
            return invalid("a macro needs at least one event");
        }
        if self.events.len() > MAX_EVENTS {
            return invalid(format!("a macro holds at most {MAX_EVENTS} events, got {}", self.events.len()));
        }
        for (i, e) in self.events.iter().enumerate() {
            match e.input {
                MacroInput::Key(u) if name_of_usage(u.0).is_none() => {
                    return invalid(format!("event {i}: {u} is not in the keycode table"))
                }
                MacroInput::Mouse(m) if m == 0 || m > 0x0f => {
                    return invalid(format!("event {i}: mouse button {m} (use 1 left, 2 right, 4 middle)"))
                }
                _ => {}
            }
        }
        Ok(())
    }
}

/// Decodes a slot image. All `0xFF` (never written), all zero (written empty) or an event length of 0
/// is an empty slot.
pub fn decode(image: &[u8]) -> Result<Option<Macro>> {
    if image.len() != SLOT_LEN {
        return invalid(format!("macro image is {} bytes, expected {SLOT_LEN}", image.len()));
    }
    if image.iter().all(|b| *b == 0xff) || image.iter().all(|b| *b == 0) {
        return Ok(None);
    }
    let len = u16::from_be_bytes([image[2], image[3]]) as usize;
    if len == 0 {
        return Ok(None);
    }
    if !len.is_multiple_of(4) || len > MAX_EVENTS * 4 {
        return protocol(format!("macro event block length {len} is not a multiple of 4 up to {}", MAX_EVENTS * 4));
    }
    let Some(mode) = PlayMode::from_byte(image[1]) else {
        return protocol(format!("macro play mode {} is unknown", image[1]));
    };
    let events = image[HEADER_LEN..HEADER_LEN + len]
        .chunks(4)
        .enumerate()
        .map(|(i, e)| {
            let down = match e[1] >> 4 {
                1 => true,
                0 => false,
                s => return protocol(format!("macro event {i}: state {s}")),
            };
            let input = match e[1] & 0x0f {
                0 => MacroInput::Key(Usage(e[0])),
                1 => MacroInput::Mouse(e[0]),
                t => return protocol(format!("macro event {i}: type {t}")),
            };
            Ok(MacroEvent { input, down, delay_ms: u16::from_be_bytes([e[2], e[3]]) })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Some(Macro { id: image[0], mode, repeat: u16::from_be_bytes([image[4], image[5]]), events }))
}

/// Encodes a slot image; `None` is the all-zero image the official driver writes for unused slots.
pub fn encode(m: Option<&Macro>) -> Result<Vec<u8>> {
    let mut img = vec![0u8; SLOT_LEN];
    let Some(m) = m else { return Ok(img) };
    m.validate()?;
    img[0] = m.id;
    img[1] = m.mode.byte();
    img[2..4].copy_from_slice(&((m.events.len() * 4) as u16).to_be_bytes());
    img[4..6].copy_from_slice(&m.repeat.to_be_bytes());
    for (i, e) in m.events.iter().enumerate() {
        let (code, ty) = match e.input {
            MacroInput::Key(u) => (u.0, 0),
            MacroInput::Mouse(b) => (b, 1),
        };
        let at = HEADER_LEN + 4 * i;
        img[at] = code;
        img[at + 1] = (e.down as u8) << 4 | ty;
        img[at + 2..at + 4].copy_from_slice(&e.delay_ms.to_be_bytes());
    }
    Ok(img)
}

pub fn check_slot(slot: u8) -> Result<()> {
    if slot >= SLOTS {
        return invalid(format!("macro slot {slot} is outside 0..{SLOTS}"));
    }
    Ok(())
}

/// A macro that taps `keys` in order with `delay_ms` after each press and release (last delay 0).
pub fn simple_sequence(id: u8, keys: &[Usage], delay_ms: u16) -> Macro {
    let mut events = Vec::new();
    for k in keys {
        events.push(MacroEvent { input: MacroInput::Key(*k), down: true, delay_ms });
        events.push(MacroEvent { input: MacroInput::Key(*k), down: false, delay_ms });
    }
    if let Some(last) = events.last_mut() {
        last.delay_ms = 0;
    }
    Macro { id, mode: PlayMode::Once, repeat: 1, events }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::body;

    /// Captured slot 0 "A B C" play once (protocol 7.1).
    fn capture_image() -> Vec<u8> {
        let mut pages = vec![body(
            "19 00 00 00 3a 00 00 00 18 00 01 00 00 04 10 00 7e 04 00 00 ca 05 10 00 7a 05 00 00 cb 06 10 00 7a 06 00 00 00",
        )];
        for p in 1..5u8 {
            pages.push(body(&format!("19 00 00 {p:02x} {:02x}", TABLE.page_len(p as usize))));
        }
        TABLE.assemble(&pages).unwrap()
    }

    #[test]
    fn decodes_the_capture() {
        let m = decode(&capture_image()).unwrap().unwrap();
        assert_eq!(m.id, 0);
        assert_eq!(m.mode, PlayMode::Once);
        assert_eq!(m.repeat, 1);
        assert_eq!(m.events.len(), 6);
        assert_eq!(m.events[0], MacroEvent { input: MacroInput::Key(Usage(4)), down: true, delay_ms: 0x7e });
        assert_eq!(m.events[1], MacroEvent { input: MacroInput::Key(Usage(4)), down: false, delay_ms: 0xca });
        assert_eq!(m.events[5].delay_ms, 0);
        assert_eq!(encode(Some(&m)).unwrap(), capture_image(), "byte-exact round trip");
    }

    #[test]
    fn empty_slots() {
        assert_eq!(decode(&[0xff; 256]).unwrap(), None);
        assert_eq!(decode(&[0; 256]).unwrap(), None);
        let mut header_only = vec![0u8; 256];
        header_only[0] = 3;
        assert_eq!(decode(&header_only).unwrap(), None);
        assert_eq!(encode(None).unwrap(), vec![0; 256]);
    }

    #[test]
    fn decode_errors() {
        assert!(decode(&[]).is_err());
        assert!(decode(&[0; 255]).is_err());
        let mut img = capture_image();
        img[3] = 0x17; // not a multiple of 4
        assert!(decode(&img).is_err());
        let mut img = capture_image();
        img[2] = 0x01; // 0x118 > 248
        assert!(decode(&img).is_err());
        let mut img = capture_image();
        img[1] = 4;
        assert!(decode(&img).is_err());
        let mut img = capture_image();
        img[9] = 0x20; // state 2
        assert!(decode(&img).is_err());
        let mut img = capture_image();
        img[9] = 0x12; // type 2
        assert!(decode(&img).is_err());
    }

    #[test]
    fn mouse_events_and_full_slot() {
        let ev = |i: usize| MacroEvent { input: MacroInput::Mouse(1), down: i.is_multiple_of(2), delay_ms: u16::MAX };
        let m = Macro { id: 0x81, mode: PlayMode::Repeat, repeat: 65535, events: (0..MAX_EVENTS).map(ev).collect() };
        let img = encode(Some(&m)).unwrap();
        assert_eq!(&img[..6], &[0x81, 1, 0, 248, 0xff, 0xff]);
        assert_eq!(&img[8..12], &[1, 0x11, 0xff, 0xff]);
        assert_eq!(&img[12..16], &[1, 0x01, 0xff, 0xff]);
        assert_eq!(decode(&img).unwrap().unwrap(), m);
    }

    #[test]
    fn encode_validation() {
        let mut m = simple_sequence(0, &[Usage(4)], 10);
        assert!(encode(Some(&m)).is_ok());
        m.events.clear();
        assert!(encode(Some(&m)).is_err(), "no events");
        let ev = MacroEvent { input: MacroInput::Key(Usage(4)), down: true, delay_ms: 0 };
        m.events = vec![ev; MAX_EVENTS + 1];
        assert!(encode(Some(&m)).is_err(), "too many");
        m.events = vec![MacroEvent { input: MacroInput::Key(Usage(0)), ..ev }];
        assert!(encode(Some(&m)).is_err());
        m.events = vec![MacroEvent { input: MacroInput::Mouse(0x10), ..ev }];
        assert!(encode(Some(&m)).is_err());
    }

    #[test]
    fn sequence_helper_and_slots() {
        let m = simple_sequence(2, &[Usage(4), Usage(5)], 30);
        assert_eq!(m.events.len(), 4);
        assert_eq!(m.events[3].delay_ms, 0);
        assert_eq!(m.events[0].delay_ms, 30);
        assert!(simple_sequence(0, &[], 5).events.is_empty());
        assert!(check_slot(9).is_ok() && check_slot(10).is_err());
    }

    #[test]
    fn serde_shape() {
        let m = simple_sequence(0, &[Usage(4)], 5);
        let j = serde_json::to_string(&m).unwrap();
        assert!(j.contains(r#"{"key":"KeyA","down":true,"delay_ms":5}"#), "{j}");
        assert_eq!(serde_json::from_str::<Macro>(&j).unwrap(), m);
        let mouse: MacroEvent = serde_json::from_str(r#"{"mouse":2,"down":false,"delay_ms":0}"#).unwrap();
        assert_eq!(mouse.input, MacroInput::Mouse(2));
    }
}

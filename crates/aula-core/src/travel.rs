//! Live key travel and calibration frames (protocol section 10), and the confirmation token for destructive
//! commands.

use crate::error::{invalid, Result};
use crate::frame::{
    self, encode_mask, Body, CMD_TRIGGER, MASK_LEN, SUB_CALIB_PROGRESS_OLD, SUB_CALIB_START, SUB_CALIB_STOP,
    SUB_TRAVEL_CLOSE, SUB_TRAVEL_OPEN, SUB_TRAVEL_SAMPLE, TRIGGER_B4,
};

/// Proof that the user confirmed a destructive operation (factory reset, trigger reset, calibration).
/// Construct it only right after an explicit confirmation (CLI `--yes`, GUI dialog).
#[derive(Debug, Clone, Copy)]
pub struct Confirm(());

impl Confirm {
    /// The user explicitly confirmed this destructive operation.
    pub fn user_confirmed() -> Self {
        Confirm(())
    }
}

/// `21 00 00 00 18 02 <mask>`: start streaming depth of the selected keys. ACK `21 00 00 00 01 00`.
pub fn open_frame(keys: &[usize]) -> Result<Body> {
    if keys.is_empty() {
        return invalid("no keys selected for the travel test");
    }
    Ok(frame::trigger_request(TRIGGER_B4, SUB_TRAVEL_OPEN, &encode_mask(keys)?))
}

/// `21 00 00 00 18 03`: stop streaming. ACK `21 00 00 00 01 00`.
pub fn close_frame() -> Body {
    frame::trigger_request(TRIGGER_B4, SUB_TRAVEL_CLOSE, &[])
}

/// One travel-stream frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TravelSample {
    pub index: usize,
    /// Depth in trigger units (0.01 mm), 0 at rest.
    pub depth: u16,
    /// The `b[4] = 3` frame the board sends when the key is back at rest.
    pub at_rest: bool,
}

/// Decodes `21 00 00 00 05 01 row col lo hi` (depth) or `21 00 00 00 03 01 row col` (at rest).
/// Returns None for any other frame.
pub fn parse_sample(b: &Body) -> Option<TravelSample> {
    if b[0] != CMD_TRIGGER || b[5] != SUB_TRAVEL_SAMPLE {
        return None;
    }
    let index = frame::index_of(b[6], b[7])?;
    match b[4] {
        5 => Some(TravelSample { index, depth: u16::from_le_bytes([b[8], b[9]]), at_rest: false }),
        3 => Some(TravelSample { index, depth: 0, at_rest: true }),
        _ => None,
    }
}

/// `21 00 00 00 18 0F 00`: start any-key calibration.
pub fn calibration_start_frame() -> Body {
    frame::trigger_request(TRIGGER_B4, SUB_CALIB_START, &[0])
}

/// `21 00 00 00 18 10 00`: stop any-key calibration.
pub fn calibration_stop_frame() -> Body {
    frame::trigger_request(TRIGGER_B4, SUB_CALIB_STOP, &[0])
}

/// `21 .. 18 06`: reset every key's trigger config. DESTRUCTIVE.
pub fn trigger_reset_frame(_: Confirm) -> Body {
    frame::trigger_request(TRIGGER_B4, frame::SUB_TRIGGER_RESET, &[])
}

/// `14 00 00 00 01 01`: factory reset. DESTRUCTIVE.
pub fn factory_reset_frame(_: Confirm) -> Body {
    let mut b = frame::request(frame::CMD_PERF, 0);
    b[4] = 1;
    b[5] = 1;
    b
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CalibrationEvent {
    /// Keys calibrated so far.
    Progress(Vec<usize>),
    /// Calibration finished or stopped successfully.
    Finished,
    /// The board reported failure (code 2 or 4).
    Failed(u8),
}

/// Decodes a calibration notification (`21 .. b[5] = 08 or 0F`, protocol 10.3). None for other frames.
pub fn parse_calibration(b: &Body) -> Option<CalibrationEvent> {
    if b[0] != CMD_TRIGGER || !matches!(b[5], SUB_CALIB_PROGRESS_OLD | SUB_CALIB_START) {
        return None;
    }
    Some(match b[6] {
        0 => CalibrationEvent::Finished,
        1 => CalibrationEvent::Progress(frame::decode_mask(&b[7..7 + MASK_LEN])),
        code => CalibrationEvent::Failed(code),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::{body, check_allowed};
    use crate::layout;

    #[test]
    fn open_and_close_match_the_probe() {
        let open = open_frame(&layout::all_indices()).unwrap();
        assert_eq!(open, body("21 00 00 00 18 02 3e 26 3e 1e 1e 1e 3e 1e 1e 3e 3e 3e 3e 00 0e"));
        assert_eq!(close_frame(), body("21 00 00 00 18 03"));
        assert!(check_allowed(&open).is_ok() && check_allowed(&close_frame()).is_ok());
        assert!(open_frame(&[]).is_err());
        assert!(open_frame(&[200]).is_err());
    }

    #[test]
    fn samples_from_the_probe() {
        let s = parse_sample(&body("21 00 00 00 05 01 05 00 6e")).unwrap();
        assert_eq!(s, TravelSample { index: 110, depth: 110, at_rest: false });
        assert_eq!(parse_sample(&body("21 00 00 00 05 01 05 00 54 01")).unwrap().depth, 340);
        let rest = parse_sample(&body("21 00 00 00 03 01 02 02 00 00 ff ff 27 0a ff 09")).unwrap();
        assert_eq!(rest, TravelSample { index: 46, depth: 0, at_rest: true });
        let rest = parse_sample(&body("21 00 00 00 03 01 02 05 08 00 ff ff")).unwrap();
        assert_eq!((rest.index, rest.depth), (49, 0), "stale 08 00 is not a depth");
        assert_eq!(parse_sample(&body("21 00 00 00 05 01 02 05 00 00")).unwrap().depth, 0);
    }

    #[test]
    fn non_samples() {
        assert_eq!(parse_sample(&body("21 00 00 00 01 00")), None, "ACK");
        assert_eq!(parse_sample(&body("21 00 00 00 04 01 02 02")), None, "unknown length");
        assert_eq!(parse_sample(&body("21 00 00 00 05 01 06 00")), None, "row 6");
        assert_eq!(parse_sample(&body("18 00 00 00 05 01 02 02")), None);
        assert_eq!(parse_sample(&body("")), None);
    }

    #[test]
    fn calibration_frames() {
        assert_eq!(calibration_start_frame(), body("21 00 00 00 18 0f 00"));
        assert_eq!(calibration_stop_frame(), body("21 00 00 00 18 10 00"));
        assert!(check_allowed(&calibration_start_frame()).is_ok());
        let c = Confirm::user_confirmed();
        assert_eq!(trigger_reset_frame(c), body("21 00 00 00 18 06"));
        assert_eq!(factory_reset_frame(c), body("14 00 00 00 01 01"));
        assert!(check_allowed(&factory_reset_frame(c)).is_ok());
    }

    #[test]
    fn calibration_events() {
        let mut b = body("21 00 00 00 18 0f 01");
        b[7..29].copy_from_slice(&encode_mask(&[22, 46]).unwrap());
        assert_eq!(parse_calibration(&b), Some(CalibrationEvent::Progress(vec![22, 46])));
        assert_eq!(parse_calibration(&body("21 00 00 00 18 0f 01")), Some(CalibrationEvent::Progress(vec![])));
        assert_eq!(parse_calibration(&body("21 00 00 00 18 08 00")), Some(CalibrationEvent::Finished));
        assert_eq!(parse_calibration(&body("21 00 00 00 18 0f 02")), Some(CalibrationEvent::Failed(2)));
        assert_eq!(parse_calibration(&body("21 00 00 00 18 0f 04")), Some(CalibrationEvent::Failed(4)));
        assert_eq!(parse_calibration(&body("21 00 00 00 01 00")), None);
        assert_eq!(parse_calibration(&body("21 00 00 00 05 01 05 00 6e")), None);
    }
}

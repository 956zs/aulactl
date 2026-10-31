//! The transport behind the GUI's `Keyboard`: the real hidraw node, or (`--sim`) the core simulator of the
//! owner's board, shared so it survives a reopen and animated so the live view and calibration have
//! something to show.

use aula_core::frame::{self, Body, CMD_TRIGGER, MASK_LEN, SUB_CALIB_START, SUB_TRAVEL_SAMPLE};
use aula_core::hidraw::{Device, Link};
use aula_core::layout;
use aula_core::sim::SimKeyboard;
use aula_core::{Error, Result};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

pub struct GuiLink {
    kind: LinkKind,
    /// A send/receive/lock failed with an I/O error: the handle is dead (unplugged, or re-enumerated under the
    /// same hidrawN) and must be opened again.
    io_failed: bool,
}

enum LinkKind {
    Hid(Device),
    Sim(SimLink),
}

impl GuiLink {
    pub fn hid(d: Device) -> Self {
        GuiLink { kind: LinkKind::Hid(d), io_failed: false }
    }

    pub fn sim(s: SimLink) -> Self {
        GuiLink { kind: LinkKind::Sim(s), io_failed: false }
    }

    pub fn io_failed(&self) -> bool {
        self.io_failed
    }

    fn note<T>(&mut self, r: Result<T>) -> Result<T> {
        if matches!(r, Err(Error::Io(_))) {
            self.io_failed = true;
        }
        r
    }
}

impl Link for GuiLink {
    fn send(&mut self, body: &Body) -> Result<()> {
        let r = match &mut self.kind {
            LinkKind::Hid(d) => d.send(body),
            LinkKind::Sim(s) => s.send(body),
        };
        self.note(r)
    }

    fn recv(&mut self, timeout: Duration) -> Result<Option<Body>> {
        let r = match &mut self.kind {
            LinkKind::Hid(d) => d.recv(timeout),
            LinkKind::Sim(s) => s.recv(timeout),
        };
        self.note(r)
    }

    fn drain(&mut self) -> Result<()> {
        let r = match &mut self.kind {
            LinkKind::Hid(d) => d.drain(),
            LinkKind::Sim(s) => s.drain(),
        };
        self.note(r)
    }

    fn lock(&mut self) -> Result<()> {
        let r = match &mut self.kind {
            LinkKind::Hid(d) => d.lock(),
            LinkKind::Sim(_) => Ok(()),
        };
        self.note(r)
    }

    fn unlock(&mut self) {
        if let LinkKind::Hid(d) = &mut self.kind {
            d.unlock();
        }
    }
}

/// Time between generated travel samples.
const SAMPLE_EVERY: Duration = Duration::from_millis(6);
/// Time between generated calibration progress frames.
const CALIBRATION_STEP: Duration = Duration::from_millis(120);
/// Keys the simulated hand presses in the live view.
const SIM_KEYS: [&str; 6] = ["KeyW", "KeyA", "KeyS", "KeyD", "Space", "ShiftLeft"];

/// The simulator plus the animation state for travel and calibration.
pub struct SimLink {
    board: Arc<Mutex<SimKeyboard>>,
    started: Instant,
    next_sample: Instant,
    turn: usize,
    calibrated: Vec<usize>,
    next_calibration: Instant,
}

impl SimLink {
    pub fn new(board: Arc<Mutex<SimKeyboard>>) -> Self {
        let now = Instant::now();
        Self { board, started: now, next_sample: now, turn: 0, calibrated: Vec::new(), next_calibration: now }
    }

    fn board(&self) -> MutexGuard<'_, SimKeyboard> {
        self.board.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn send(&mut self, body: &Body) -> Result<()> {
        let mut b = self.board();
        let was_calibrating = b.calibrating;
        let r = b.send(body);
        if b.calibrating && !was_calibrating {
            drop(b);
            self.calibrated.clear();
            self.next_calibration = Instant::now() + CALIBRATION_STEP;
        }
        r
    }

    fn drain(&mut self) -> Result<()> {
        self.board().drain()
    }

    fn recv(&mut self, timeout: Duration) -> Result<Option<Body>> {
        let (queued, travel, calibrating) = {
            let mut b = self.board();
            (b.input.pop_front(), b.travel_open, b.calibrating)
        };
        if queued.is_some() {
            return Ok(queued);
        }
        let now = Instant::now();
        let due = if travel {
            Some(self.next_sample)
        } else if calibrating && self.calibrated.len() < layout::KEYS.len() {
            Some(self.next_calibration)
        } else {
            None
        };
        match due {
            Some(at) if at <= now + timeout => {
                std::thread::sleep(at.saturating_duration_since(now));
                Ok(Some(if travel { self.travel_frame() } else { self.calibration_frame() }))
            }
            _ => {
                // Like a quiet device: nothing within the timeout (capped so stop flags are seen soon).
                std::thread::sleep(timeout.min(Duration::from_millis(50)));
                Ok(None)
            }
        }
    }

    /// One depth sample: the simulated keys take turns, each pressing in a slow wave.
    fn travel_frame(&mut self) -> Body {
        self.next_sample = Instant::now() + SAMPLE_EVERY;
        let code = SIM_KEYS[self.turn % SIM_KEYS.len()];
        self.turn = self.turn.wrapping_add(1);
        let k = layout::by_code(code).expect("simulated keys exist");
        let t = self.started.elapsed().as_secs_f64();
        let phase = t * 1.3 + self.turn as f64 % SIM_KEYS.len() as f64 * 0.9;
        let depth = ((phase.sin() * 1.15).clamp(0.0, 1.0) * 340.0) as u16;
        let (row, col) = frame::row_col(k.index as usize);
        let mut b = frame::request(CMD_TRIGGER, 0);
        b[5] = SUB_TRAVEL_SAMPLE;
        b[6] = row;
        b[7] = col;
        if depth == 0 {
            b[4] = 3;
        } else {
            b[4] = 5;
            b[8..10].copy_from_slice(&depth.to_le_bytes());
        }
        b
    }

    /// Calibration progress: one more key done per step, in layout order.
    fn calibration_frame(&mut self) -> Body {
        self.next_calibration = Instant::now() + CALIBRATION_STEP;
        if let Some(k) = layout::KEYS.get(self.calibrated.len()) {
            self.calibrated.push(k.index as usize);
        }
        let mut b = frame::request(CMD_TRIGGER, 0);
        b[4] = 2 + MASK_LEN as u8;
        b[5] = SUB_CALIB_START;
        b[6] = 1;
        let mask = frame::encode_mask(&self.calibrated).expect("layout keys are in the matrix");
        b[7..7 + MASK_LEN].copy_from_slice(&mask);
        b
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aula_core::travel::{self, CalibrationEvent};

    fn sim() -> SimLink {
        SimLink::new(Arc::new(Mutex::new(SimKeyboard::owner_board())))
    }

    #[test]
    fn generated_frames_decode() {
        let mut s = sim();
        for _ in 0..12 {
            let f = s.travel_frame();
            let sample = travel::parse_sample(&f).expect("travel sample");
            assert!(SIM_KEYS.iter().any(|c| layout::by_code(c).unwrap().index as usize == sample.index));
            assert!(sample.depth <= 340);
        }
        let first = s.calibration_frame();
        assert_eq!(travel::parse_calibration(&first), Some(CalibrationEvent::Progress(vec![22])));
        for _ in 1..layout::KEYS.len() + 3 {
            s.calibration_frame();
        }
        let all = s.calibration_frame();
        let Some(CalibrationEvent::Progress(keys)) = travel::parse_calibration(&all) else { panic!() };
        assert_eq!(keys.len(), layout::KEYS.len());
    }

    #[test]
    fn io_errors_mark_the_link_dead() {
        let board = Arc::new(Mutex::new(SimKeyboard::owner_board()));
        let mut l = GuiLink::sim(SimLink::new(board.clone()));
        l.send(&frame::request(0x01, 0)).unwrap();
        assert!(!l.io_failed());
        board.lock().unwrap().io_error = true;
        assert!(matches!(l.send(&frame::request(0x01, 0)), Err(Error::Io(_))));
        assert!(l.io_failed());
        board.lock().unwrap().io_error = false;
        l.send(&frame::request(0x01, 0)).unwrap();
        assert!(l.io_failed(), "stays dead until reopened");
    }

    #[test]
    fn quiet_without_stream() {
        let mut s = sim();
        assert_eq!(s.recv(Duration::from_millis(1)).unwrap(), None);
    }
}

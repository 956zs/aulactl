//! In-memory WIN 60 HE for tests (`sim` feature): answers every allowed command like the owner's board
//! (protocol Appendix A) and records what was sent.

use crate::error::{Error, Result};
use crate::frame::{self, Body, TableSpec, BODY_LEN, MATRIX_KEYS};
use crate::hidraw::Link;
use crate::{keymap, layout, lighting, macros, socd, trigger};
use std::collections::VecDeque;
use std::time::Duration;

/// Board state and a log of every frame sent to it.
#[derive(Debug, Clone)]
pub struct SimKeyboard {
    pub ident: String,
    pub caps: Body,
    pub effect: [u8; 12],
    pub modes: Vec<u8>,
    pub custom: [Vec<u8>; 2],
    pub active_slot: u8,
    pub keymap: [Vec<u8>; 2],
    pub fn_fixed: [u8; 22],
    pub macros: Vec<Vec<u8>>,
    pub advanced: Vec<[u8; 40]>,
    pub socd_on: bool,
    pub socd: Vec<u8>,
    pub switch_list: Vec<u8>,
    pub switches: Vec<u8>,
    pub dead_band: Vec<u8>,
    /// Per key index: mode, actuation, press, release.
    pub triggers: Vec<(u8, u16, u16, u16)>,
    pub poll_khz: u8,
    pub music: bool,
    pub travel_open: bool,
    pub calibrating: bool,
    /// Every frame received, in order.
    pub sent: Vec<Body>,
    /// Frames waiting to be read.
    pub input: VecDeque<Body>,
    /// When set, replies to frames with this command byte are dropped (to test timeouts).
    pub mute_cmd: Option<u8>,
    /// When set, the next write to this table is stored corrupted (to test read-back verification).
    pub corrupt_next_write: Option<u8>,
}

impl Default for SimKeyboard {
    fn default() -> Self {
        Self::owner_board()
    }
}

fn b(s: &str) -> Body {
    frame::parse_hex_body(s).expect("valid fixture")
}

impl SimKeyboard {
    /// The owner's board as probed on 2026-11-01.
    pub fn owner_board() -> Self {
        let mut custom0 = vec![0u8; lighting::RGB_TABLE.len];
        for idx in [46usize, 68, 69, 70] {
            custom0[idx * 3] = 0xff;
        }
        let mut base = vec![0u8; keymap::TABLE.len];
        base[122 * 4] = 1;
        let mut triggers = vec![(0u8, 0u16, 0u16, 0u16); MATRIX_KEYS];
        for k in &layout::KEYS {
            triggers[k.index as usize] = (12, 100, 50, 50);
        }
        let mut switches = vec![0u8; MATRIX_KEYS];
        for k in &layout::KEYS {
            switches[k.index as usize] = 1;
        }
        let mut fn_fixed = [0u8; 22];
        fn_fixed.copy_from_slice(&b("02 22 0e 0a 02 02 22 12 1e 3e 0e 2e 3e 20 20 20 00 00 00 00 00 00")[..22]);
        SimKeyboard {
            ident: "W669,34,KB,SI,SI2825KZHEARGB,V3.17.07".into(),
            caps: b("01 00 00 00 11 00 00 00 00 00 00 00 00 00 01 00 00 7f 00 01 01"),
            effect: [0, 1, 2, 0xff, 0, 0xff, 0, 0, 0, 0, 0, 0],
            modes: vec![2, 1, 3, 9, 4, 0, 14, 15, 6, 7, 11, 12, 16, 8],
            custom: [custom0, vec![0u8; lighting::RGB_TABLE.len]],
            active_slot: 0,
            keymap: [base, vec![0u8; keymap::TABLE.len]],
            fn_fixed,
            macros: vec![vec![0u8; macros::SLOT_LEN]; macros::SLOTS as usize],
            advanced: vec![[0u8; 40]; 40],
            socd_on: false,
            socd: vec![0u8; socd::TABLE.len],
            switch_list: vec![1, 2, 3, 5],
            switches,
            dead_band: vec![0u8; trigger::DEAD_BAND_TABLE.len],
            triggers,
            poll_khz: 8,
            music: false,
            travel_open: false,
            calibrating: false,
            sent: Vec::new(),
            input: VecDeque::new(),
            mute_cmd: None,
            corrupt_next_write: None,
        }
    }

    /// Another `2e3c:c365` board.
    pub fn other_board() -> Self {
        SimKeyboard { ident: "W669,34,KB,SI,SI2828HEARGB,V1.00.00".into(), ..Self::owner_board() }
    }

    /// Queues an unsolicited input frame (travel samples, calibration progress).
    pub fn push_input(&mut self, body: Body) {
        self.input.push_back(body);
    }

    fn reply(&mut self, body: Body) {
        if self.mute_cmd != Some(body[0]) {
            self.input.push_back(body);
        }
    }

    fn reply_ack(&mut self, a: [u8; 6]) {
        let mut body = [0u8; BODY_LEN];
        body[..6].copy_from_slice(&a);
        self.reply(body);
    }

    fn reply_pages(&mut self, cmd: u8, sel: u8, spec: TableSpec, table: &[u8]) {
        for f in spec.write_frames(cmd, sel, table).expect("table size") {
            self.reply(f);
        }
    }

    /// Stores page `b` into `table` (a write frame of `spec`).
    fn store_page(&mut self, cmd: u8, table: &mut [u8], spec: TableSpec, body: &Body) {
        let page = frame::page_of(body) as usize;
        let len = body[4] as usize;
        let at = page * spec.page;
        table[at..at + len].copy_from_slice(&body[5..5 + len]);
        if self.corrupt_next_write == Some(cmd) {
            table[at] ^= 0xff;
            self.corrupt_next_write = None;
        }
    }

    fn handle(&mut self, req: &Body) -> Result<()> {
        frame::check_allowed(req).map_err(|e| Error::Protocol(format!("sim received a refused frame: {e}")))?;
        let (cmd, sel) = (req[0], req[1]);
        match cmd {
            0x0d => {
                let mut p0 = b("0d 00 00 00 32");
                p0[5..5 + self.ident.len()].copy_from_slice(self.ident.as_bytes());
                let mut p1 = b("0d 00 00 01 32");
                let date = b"Apr 15 2026,11:20:35\0\0\0\0Apr ";
                p1[5..5 + date.len()].copy_from_slice(date);
                self.reply(p0);
                self.reply(p1);
            }
            0x01 => {
                let mut c = self.caps;
                c[21] = self.music as u8;
                self.reply(c);
            }
            0x0e => {
                let mut r = b("0e 00 00 00 16");
                r[5..27].copy_from_slice(&self.fn_fixed);
                self.reply(r);
            }
            0x0a => {
                let mut r = b("0a");
                let mut p = self.modes.clone();
                p.extend([0xa5, 0x5a, 4, 4]);
                r[4] = p.len() as u8;
                r[5..5 + p.len()].copy_from_slice(&p);
                self.reply(r);
            }
            0x02 => self.reply(b("02 00 00 00 04 a5 5a 04 04")),
            0x07 if sel == 1 => {
                let mut r = b("07 00 00 00 0c");
                r[5..17].copy_from_slice(&self.effect);
                self.reply(r);
            }
            0x07 => self.effect.copy_from_slice(&req[5..17]),
            0x08 => self.reply(b("08 00 00 00 0c 02 04 03 ff 00 00 00 00 00 00 00 01")),
            0x09 if sel == 0x20 => {
                let mut r = b("09");
                r[1] = self.active_slot;
                self.reply(r);
            }
            0x09 if sel & 0x80 != 0 => {
                let t = self.custom[(sel & 1) as usize].clone();
                self.reply_pages(0x09, sel, lighting::RGB_TABLE, &t);
            }
            0x09 => {
                let mut t = std::mem::take(&mut self.custom[sel as usize]);
                self.store_page(0x09, &mut t, lighting::RGB_TABLE, req);
                self.custom[sel as usize] = t;
                self.active_slot = sel;
                self.reply_ack([0x09, sel, 0, 0, 0, 0]);
            }
            0x17 => {
                self.music = req[5] == 1;
                self.reply_ack([0x17, 0, 0, 0, 0, 0]);
            }
            0x18 if sel & 0x80 != 0 => {
                let t = self.keymap[((sel & 2) / 2) as usize].clone();
                self.reply_pages(0x18, sel, keymap::TABLE, &t);
            }
            0x18 => {
                let l = (sel / 2) as usize;
                let mut t = std::mem::take(&mut self.keymap[l]);
                self.store_page(0x18, &mut t, keymap::TABLE, req);
                self.keymap[l] = t;
                self.reply_ack([0x18, 0, 0, 0, 0, 0]);
            }
            0x19 if sel & 0x80 != 0 => {
                let t = self.macros[(sel & 0x7f) as usize].clone();
                self.reply_pages(0x19, sel, macros::TABLE, &t);
            }
            0x19 => {
                let mut t = std::mem::take(&mut self.macros[sel as usize]);
                self.store_page(0x19, &mut t, macros::TABLE, req);
                self.macros[sel as usize] = t;
                self.reply_ack([0x19, 0, 0, 0, 0, 0]);
            }
            0x22 if sel == 0x80 => {
                for (s, p) in self.advanced.clone().iter().enumerate() {
                    let mut r = b("22 80 00 00 28");
                    r[3] = s as u8;
                    r[5..45].copy_from_slice(p);
                    self.reply(r);
                }
            }
            0x22 => {
                self.advanced[sel as usize].copy_from_slice(&req[5..45]);
                if self.corrupt_next_write == Some(0x22) {
                    self.advanced[sel as usize][1] ^= 0xff;
                    self.corrupt_next_write = None;
                }
                self.reply_ack([0x22, 0, 0, 0, 0, 0]);
            }
            0x24 => match sel {
                0 => {
                    let mut t = std::mem::take(&mut self.socd);
                    self.store_page(0x24, &mut t, socd::TABLE, req);
                    // the capture read byte 0 back as 00
                    for e in t.chunks_mut(4) {
                        e[0] = 0;
                    }
                    self.socd = t;
                    self.reply_ack([0x24, 0, 0, 0, 0, 0]);
                }
                1 => {
                    let t = self.socd.clone();
                    self.reply_pages(0x24, 1, socd::TABLE, &t);
                }
                2 => {
                    let mut r = b("24 02 00 00 06 00 00 00 00 fa e5");
                    r[5] = self.socd_on as u8;
                    self.reply(r);
                }
                _ => {
                    self.socd_on = req[5] == 1;
                    self.reply_ack([0x24, 0, 0, 0, 0, 0]);
                }
            },
            0x25 => match sel {
                0 => {
                    let mut r = b("25");
                    r[4] = 2 + self.switch_list.len() as u8;
                    r[6] = self.switch_list.len() as u8;
                    r[7..7 + self.switch_list.len()].copy_from_slice(&self.switch_list);
                    self.reply(r);
                }
                1 => {
                    let mut t = std::mem::take(&mut self.switches);
                    self.store_page(0x25, &mut t, trigger::SWITCH_TABLE, req);
                    self.switches = t;
                    self.reply_ack([0x25, 1, 0, 0, 0, 0]);
                }
                _ => {
                    let t = self.switches.clone();
                    self.reply_pages(0x25, 2, trigger::SWITCH_TABLE, &t);
                }
            },
            0x26 if sel == 0 => {
                let t = self.dead_band.clone();
                self.reply_pages(0x26, 0, trigger::DEAD_BAND_TABLE, &t);
            }
            0x26 => {
                let mut t = std::mem::take(&mut self.dead_band);
                self.store_page(0x26, &mut t, trigger::DEAD_BAND_TABLE, req);
                self.dead_band = t;
                self.reply_ack([0x26, 1, 0, 0, 0, 0]);
            }
            0x14 => {
                if req[5] == 1 {
                    *self = SimKeyboard { sent: std::mem::take(&mut self.sent), ..Self::owner_board() };
                } else {
                    self.caps[20] = req[12] | req[13] << 1 | req[14] << 2 | req[15] << 3;
                }
                self.reply_ack([0x14, 0, 0, 0, 0, 0]);
            }
            0x21 => self.handle_trigger(req),
            _ => unreachable!("allowlist"),
        }
        Ok(())
    }

    fn handle_trigger(&mut self, req: &Body) {
        let ack = [0x21, 0, 0, 0, 1, 0];
        match (req[4], req[5]) {
            (0x18, m @ (0 | 12 | 13)) => {
                let w = |lo: usize| u16::from_le_bytes([req[lo], req[lo + 4]]);
                for idx in frame::decode_mask(&req[6..28]) {
                    self.triggers[idx] = (m, w(28), w(30), w(31));
                }
                self.reply_ack(ack);
            }
            (0x18, 2) => {
                self.travel_open = true;
                self.reply_ack(ack);
            }
            (0x18, 3) => {
                self.travel_open = false;
                self.reply_ack(ack);
            }
            (0x18, 4) => self.reply(b("21 00 00 00 06 04 54 01 01 01 08")),
            (0x18, 5) => {
                let Some(idx) = frame::index_of(req[6], req[7]) else { return };
                let (m, a, p, r) = self.triggers[idx];
                let mut out = b("21 00 00 00 0c 05");
                out[6] = m;
                for (at, v) in [(7, a), (8, a), (9, p), (10, r)] {
                    let [lo, hi] = v.to_le_bytes();
                    out[at] = lo;
                    out[at + 4] = hi;
                }
                out[15] = req[6];
                out[16] = req[7];
                self.reply(out);
            }
            (0x18, 6) => {
                for k in &layout::KEYS {
                    self.triggers[k.index as usize] = (0, 100, 1, 1);
                }
                self.reply_ack(ack);
            }
            (0x18, 0x0f) => self.calibrating = true,
            (0x18, 0x10) => {
                self.calibrating = false;
                self.reply(b("21 00 00 00 02 0f 00"));
            }
            (0x01, 9) => {
                let mut r = b("21 00 00 00 02 09");
                r[6] = self.poll_khz;
                self.reply(r);
            }
            (0x00, 9) => self.poll_khz = req[7],
            _ => unreachable!("allowlist"),
        }
    }
}

impl Link for SimKeyboard {
    fn send(&mut self, body: &Body) -> Result<()> {
        self.sent.push(*body);
        self.handle(body)
    }

    fn recv(&mut self, _timeout: Duration) -> Result<Option<Body>> {
        Ok(self.input.pop_front())
    }

    fn drain(&mut self) -> Result<()> {
        self.input.clear();
        Ok(())
    }
}

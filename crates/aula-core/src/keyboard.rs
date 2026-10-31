//! Keyboard operations built on the transport: identification guard, request/reply discipline, every feature's
//! read and write with read-back verification, automatic backup before the first write.

use crate::advanced::{self, AdvancedKey};
use crate::error::{protocol, Error, Result};
use crate::frame::{
    self, ack, check_allowed, is_identification, Body, Reply, TableSpec, CMD_ADVANCED, CMD_CUSTOM_RGB, CMD_DEAD_BAND,
    CMD_EFFECT, CMD_FN_FIXED, CMD_IDENT, CMD_INFO, CMD_KEYMAP, CMD_MACRO, CMD_MODES, CMD_MUSIC, CMD_PERF,
    CMD_SIDE_MODES, CMD_SOCD, CMD_SWITCH, SUB_LIMITS, SUB_POLL_RATE, TRIGGER_ACK,
};
use crate::hidraw::{self, Device, DeviceInfo, Link};
use crate::info::{self, Capabilities, Identity, WinLock};
use crate::keymap::{self, Keymap, Layer};
use crate::layout;
use crate::lighting::{self, CustomColors, Effect, ModeList};
use crate::macros::{self, Macro};
use crate::perkey::{key_name, PerKey};
use crate::snapshot::Snapshot;
use crate::socd::{self, SocdPair, SocdPower};
use crate::store;
use crate::travel::{self, CalibrationEvent, Confirm, TravelSample};
use crate::trigger::{self, DeadBand, KeyTrigger, TravelLimits};
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Per-frame timeout (protocol 2.3).
pub const FRAME_TIMEOUT: Duration = Duration::from_millis(200);
/// Per-table timeout (protocol 2.3).
pub const TABLE_TIMEOUT: Duration = Duration::from_secs(1);
/// The official driver sleeps 5 ms after each per-key RGB page.
const RGB_PAGE_PAUSE: Duration = Duration::from_millis(5);
/// How long closing the travel test or stopping calibration waits for the board.
const CLOSE_TIMEOUT: Duration = Duration::from_millis(500);
const CALIBRATION_STOP_TIMEOUT: Duration = Duration::from_secs(2);

/// A WIN 60 HE (or another `2e3c:c365` board, which only gets identification commands).
pub struct Keyboard<L: Link = Device> {
    link: L,
    identity: Identity,
    caps: Option<Capabilities>,
    limits: Option<TravelLimits>,
    /// Where the automatic backup goes: `Ok(None)` = turned off explicitly (simulator, tests), `Err` = no
    /// directory could be resolved, so every write is refused.
    backup_dir: std::result::Result<Option<PathBuf>, String>,
    backed_up: bool,
    last_backup: Option<PathBuf>,
    /// Set after a poll-rate write: the board re-enumerates and this handle is dead.
    gone: bool,
}

impl Keyboard<Device> {
    /// Opens the first vendor interface (`hidraw::find_devices`) and identifies it.
    pub fn open_first() -> Result<Self> {
        let info = hidraw::find_devices().into_iter().next().ok_or(Error::NotFound)?;
        Self::open(&info)
    }

    /// Opens and identifies the board. Automatic backups go to `store::backups_dir()`; when that cannot be
    /// resolved, writes are refused.
    pub fn open(info: &DeviceInfo) -> Result<Self> {
        Self::open_path(&info.node)
    }

    pub fn open_path(node: &Path) -> Result<Self> {
        let mut kb = Self::with_link(Device::open(node)?)?;
        kb.backup_dir = store::backups_dir().map(Some).map_err(|e| e.to_string());
        Ok(kb)
    }
}

impl<L: Link> Keyboard<L> {
    /// Identifies the board behind `link` (`0x0D`, then `0x01` when it is a WIN 60 HE).
    /// Writes are refused until `set_auto_backup` says where the automatic backup goes (or turns it off).
    pub fn with_link(link: L) -> Result<Self> {
        let mut kb = Keyboard {
            link,
            identity: Identity { raw: String::new(), product_code: String::new(), firmware: String::new(), build_date: String::new() },
            caps: None,
            limits: None,
            backup_dir: Err("no automatic backup directory was set".into()),
            backed_up: false,
            last_backup: None,
            gone: false,
        };
        let pages = kb.exchange(&frame::request(CMD_IDENT, 0), Reply::Cmd(CMD_IDENT), 2)?;
        kb.identity = info::parse_identity(&pages[0], &pages[1])?;
        if kb.identity.is_supported() {
            kb.refresh_capabilities()?;
        }
        Ok(kb)
    }

    pub fn link(&self) -> &L {
        &self.link
    }

    #[cfg(test)]
    pub(crate) fn link_mut(&mut self) -> &mut L {
        &mut self.link
    }

    pub fn identity(&self) -> &Identity {
        &self.identity
    }

    pub fn is_supported(&self) -> bool {
        self.identity.is_supported()
    }

    /// Capabilities read at open (or by `refresh_capabilities`). Unsupported boards have none.
    pub fn capabilities(&self) -> Result<&Capabilities> {
        self.caps.as_ref().ok_or_else(|| self.unsupported())
    }

    /// Re-reads `0x01` (win-lock bits and music-rhythm state change). Only `0x0D` is ever sent to other boards.
    pub fn refresh_capabilities(&mut self) -> Result<Capabilities> {
        if !self.identity.is_supported() {
            return Err(self.unsupported());
        }
        let b = self.query(&frame::request(CMD_INFO, 0), Reply::Cmd(CMD_INFO))?;
        let caps = info::parse_capabilities(&b)?;
        self.caps = Some(caps);
        Ok(caps)
    }

    /// Directory for the automatic backup before the first write, or None to turn it off (simulator, tests).
    pub fn set_auto_backup(&mut self, dir: Option<PathBuf>) {
        self.backup_dir = Ok(dir);
    }

    /// The automatic backup this handle wrote, if any.
    pub fn last_backup(&self) -> Option<&Path> {
        self.last_backup.as_deref()
    }

    fn unsupported(&self) -> Error {
        Error::UnsupportedProduct { product_code: self.identity.product_code.clone() }
    }

    fn require(&self, what: &str, has: impl Fn(&Capabilities) -> bool) -> Result<()> {
        if has(self.capabilities()?) {
            Ok(())
        } else {
            Err(Error::Unsupported(what.to_string()))
        }
    }

    // ---- transport -------------------------------------------------------------------------------------------

    /// Guards every frame: allowlist, identification-only for unsupported boards, dead handle.
    fn check(&self, req: &Body) -> Result<()> {
        if self.gone {
            return Err(Error::Io("the keyboard is restarting after a poll-rate change; open it again".into()));
        }
        check_allowed(req)?;
        if !is_identification(req) && !self.identity.is_supported() {
            return Err(self.unsupported());
        }
        Ok(())
    }

    /// Sends `req` and collects `n` frames matching `reply` (200 ms per frame, 1 s in total; more for long
    /// tables). Other frames are skipped. One request in flight, stale input drained first.
    fn exchange(&mut self, req: &Body, reply: Reply, n: usize) -> Result<Vec<Body>> {
        self.check(req)?;
        self.link.lock()?;
        let r = self.exchange_locked(req, reply, n);
        self.link.unlock();
        r
    }

    fn exchange_locked(&mut self, req: &Body, reply: Reply, n: usize) -> Result<Vec<Body>> {
        self.link.drain()?;
        self.link.send(req)?;
        let total = TABLE_TIMEOUT.max(FRAME_TIMEOUT * n as u32 / 4);
        let deadline = Instant::now() + total;
        let mut out = Vec::with_capacity(n);
        let mut frame_deadline = Instant::now() + FRAME_TIMEOUT;
        while out.len() < n {
            let now = Instant::now();
            let until = frame_deadline.min(deadline);
            if now >= until {
                return Err(Error::Timeout(format!(
                    "{} of {n} replies to {} arrived",
                    out.len(),
                    frame::hex(&req[..6])
                )));
            }
            match self.link.recv(until - now)? {
                Some(b) if reply.matches(&b) => {
                    out.push(b);
                    frame_deadline = Instant::now() + FRAME_TIMEOUT;
                }
                Some(_) | None => {}
            }
        }
        Ok(out)
    }

    fn query(&mut self, req: &Body, reply: Reply) -> Result<Body> {
        Ok(self.exchange(req, reply, 1)?.remove(0))
    }

    /// Reads a paged table: `cmd sel`, replies `cmd reply_sel 00 pp len data..`.
    fn read_table(&mut self, cmd: u8, sel: u8, spec: TableSpec) -> Result<Vec<u8>> {
        let pages = self.exchange(&frame::request(cmd, sel), Reply::CmdSel(cmd, sel), spec.pages())?;
        spec.assemble(&pages).map_err(|e| Error::Protocol(format!("{cmd:02x} {sel:02x}: {e}")))
    }

    /// Writes a paged table, waiting for each page's ACK before the next.
    fn write_table(&mut self, cmd: u8, sel: u8, spec: TableSpec, table: &[u8], ack: Reply, pause: Duration) -> Result<()> {
        let frames = spec.write_frames(cmd, sel, table)?;
        self.with_lock(|kb| {
            for f in &frames {
                kb.exchange(f, ack, 1)?;
                if !pause.is_zero() {
                    std::thread::sleep(pause);
                }
            }
            Ok(())
        })
    }

    /// Sends a frame that gets no reply.
    fn send_only(&mut self, req: &Body) -> Result<()> {
        self.check(req)?;
        self.link.lock()?;
        let r = self.link.drain().and_then(|_| self.link.send(req));
        self.link.unlock();
        r
    }

    /// Runs `f` as one locked operation (other processes wait).
    pub fn with_lock<T>(&mut self, f: impl FnOnce(&mut Self) -> Result<T>) -> Result<T> {
        self.link.lock()?;
        let r = f(self);
        self.link.unlock();
        r
    }

    /// Every write goes through here: supported board, then the automatic backup once per handle.
    fn before_write(&mut self) -> Result<()> {
        if !self.identity.is_supported() {
            return Err(self.unsupported());
        }
        if self.backed_up {
            return Ok(());
        }
        let dir = match &self.backup_dir {
            Ok(dir) => dir.clone(),
            Err(e) => return Err(Error::Refused(format!("no automatic backup is possible ({e}); nothing was written"))),
        };
        if let Some(dir) = dir {
            let snap = self.read_snapshot()?;
            let path = store::save_auto_backup(&dir, &snap)?;
            self.last_backup = Some(path);
        }
        self.backed_up = true;
        Ok(())
    }

    // ---- lighting --------------------------------------------------------------------------------------------

    pub fn read_mode_list(&mut self) -> Result<ModeList> {
        let b = self.query(&frame::request(CMD_MODES, 0), Reply::Cmd(CMD_MODES))?;
        lighting::parse_mode_list(&b)
    }

    /// Side-light mode list (empty on this board).
    pub fn read_side_mode_list(&mut self) -> Result<ModeList> {
        let b = self.query(&frame::request(CMD_SIDE_MODES, 0), Reply::Cmd(CMD_SIDE_MODES))?;
        lighting::parse_mode_list(&b)
    }

    pub fn read_effect(&mut self) -> Result<Effect> {
        let b = self.query(&frame::request(CMD_EFFECT, 1), Reply::Cmd(CMD_EFFECT))?;
        lighting::parse_effect(&b)
    }

    /// Sets the lighting effect: music rhythm off first (like the official driver), effect, read back.
    pub fn set_effect(&mut self, e: &Effect) -> Result<()> {
        let list = self.read_mode_list()?;
        e.validate(&list)?;
        self.before_write()?;
        self.with_lock(|kb| {
            kb.exchange(&lighting::music_frame(false), ack(CMD_MUSIC, 0), 1)?;
            kb.send_only(&lighting::effect_frame(e))?;
            let back = kb.read_effect()?;
            if back != *e {
                return Err(Error::Verify(format!("effect reads back as {back:?}")));
            }
            Ok(())
        })
    }

    /// Enters or leaves music-rhythm mode (`0x17`). Mode 100 needs a vendor plugin; aulactl only turns it off.
    pub fn set_music_rhythm(&mut self, on: bool) -> Result<()> {
        self.require("music rhythm", Capabilities::music_rhythm)?;
        self.before_write()?;
        self.exchange(&lighting::music_frame(on), ack(CMD_MUSIC, 0), 1)?;
        Ok(())
    }

    /// Active custom slot (`09 20`).
    pub fn read_active_custom_slot(&mut self) -> Result<u8> {
        let b = self.query(&frame::request(CMD_CUSTOM_RGB, 0x20), Reply::Cmd(CMD_CUSTOM_RGB))?;
        lighting::parse_active_slot(&b)
    }

    pub fn read_custom_colors(&mut self, slot: u8) -> Result<CustomColors> {
        lighting::check_slot(slot)?;
        self.require("per-key RGB", Capabilities::custom_light)?;
        let t = self.read_table(CMD_CUSTOM_RGB, 0x80 | slot, lighting::RGB_TABLE)?;
        lighting::decode_custom(&t)
    }

    /// Writes one custom slot (keys missing from `colors` are black) and reads it back.
    pub fn write_custom_colors(&mut self, slot: u8, colors: &CustomColors) -> Result<()> {
        lighting::check_slot(slot)?;
        self.require("per-key RGB", Capabilities::custom_light)?;
        let table = lighting::encode_custom(colors)?;
        self.before_write()?;
        self.with_lock(|kb| {
            kb.write_table(CMD_CUSTOM_RGB, slot, lighting::RGB_TABLE, &table, ack(CMD_CUSTOM_RGB, slot), RGB_PAGE_PAUSE)?;
            let back = kb.read_table(CMD_CUSTOM_RGB, 0x80 | slot, lighting::RGB_TABLE)?;
            verify_table(&format!("custom colours slot {slot}"), &table, &back, 3)
        })
    }

    // ---- keymap and macros -----------------------------------------------------------------------------------

    /// Physical keys whose Fn-layer function is fixed (`0x0E`).
    pub fn read_fn_fixed(&mut self) -> Result<Vec<usize>> {
        let b = self.query(&frame::request(CMD_FN_FIXED, 0), Reply::Cmd(CMD_FN_FIXED))?;
        let p = frame::payload_min(&b, frame::MASK_LEN)?;
        Ok(frame::decode_mask(p).into_iter().filter(|i| layout::is_key(*i)).collect())
    }

    pub fn read_keymap(&mut self, layer: Layer) -> Result<Keymap> {
        let t = self.read_table(CMD_KEYMAP, keymap::read_selector(layer), keymap::TABLE)?;
        keymap::decode_layer(layer, &t)
    }

    /// Writes a whole layer (keys missing from `map` get their default) and reads it back.
    /// Fn-layer keys in the board's Fn-fixed mask must stay `Default`.
    pub fn write_keymap(&mut self, layer: Layer, map: &Keymap) -> Result<()> {
        if layer == Layer::Fn {
            let fixed = self.read_fn_fixed()?;
            keymap::check_fn_fixed(map, &fixed)?;
        }
        let table = keymap::encode_layer(layer, map)?;
        self.before_write()?;
        self.with_lock(|kb| {
            if keymap::has_raw(map) {
                keymap::check_raw_kept(map, &kb.read_keymap(layer)?)?;
            }
            kb.write_table(CMD_KEYMAP, layer.selector(), keymap::TABLE, &table, ack(CMD_KEYMAP, 0), Duration::ZERO)?;
            let back = kb.read_table(CMD_KEYMAP, keymap::read_selector(layer), keymap::TABLE)?;
            verify_table(&format!("{} layer", layer.name()), &table, &back, 4)
        })
    }

    pub fn read_macro(&mut self, slot: u8) -> Result<Option<Macro>> {
        macros::check_slot(slot)?;
        let t = self.read_table(CMD_MACRO, 0x80 | slot, macros::TABLE)?;
        macros::decode(&t).map_err(|e| Error::Protocol(format!("macro slot {slot}: {e}")))
    }

    pub fn read_macros(&mut self) -> Result<Vec<Option<Macro>>> {
        self.with_lock(|kb| (0..macros::SLOTS).map(|s| kb.read_macro(s)).collect())
    }

    /// Writes one macro slot (`None` = empty) and reads it back.
    pub fn write_macro(&mut self, slot: u8, m: Option<&Macro>) -> Result<()> {
        macros::check_slot(slot)?;
        let image = macros::encode(m)?;
        self.before_write()?;
        self.with_lock(|kb| {
            kb.write_table(CMD_MACRO, slot, macros::TABLE, &image, ack(CMD_MACRO, 0), Duration::ZERO)?;
            let back = kb.read_table(CMD_MACRO, 0x80 | slot, macros::TABLE)?;
            if back != image {
                return Err(Error::Verify(format!("macro slot {slot}")));
            }
            Ok(())
        })
    }

    // ---- advanced keys ---------------------------------------------------------------------------------------

    /// All 40 advanced-key slots (`22 80`, one page per slot).
    pub fn read_advanced(&mut self) -> Result<Vec<AdvancedKey>> {
        let pages = self.exchange(
            &frame::request(CMD_ADVANCED, 0x80),
            Reply::CmdSel(CMD_ADVANCED, 0x80),
            advanced::SLOTS as usize,
        )?;
        pages
            .iter()
            .enumerate()
            .map(|(n, b)| {
                if frame::page_of(b) as usize != n {
                    return protocol(format!("advanced-key page {} where {n} was expected", frame::page_of(b)));
                }
                advanced::decode(frame::payload_min(b, advanced::READ_LEN)?)
            })
            .collect()
    }

    /// Writes the given slots and reads all slots back. Slots with raw bytes (`AdvancedKey::has_raw`) must
    /// be what the board already holds.
    pub fn write_advanced(&mut self, slots: &[(u8, AdvancedKey)]) -> Result<()> {
        let frames = slots.iter().map(|(s, a)| advanced::write_frame(*s, a)).collect::<Result<Vec<_>>>()?;
        let max = advanced::max_travel_01mm(&self.travel_limits()?);
        for (s, a) in slots {
            a.check_travel(max).map_err(|e| Error::Invalid(format!("advanced-key slot {s}: {e}")))?;
        }
        self.before_write()?;
        self.with_lock(|kb| {
            if slots.iter().any(|(_, a)| a.has_raw()) {
                let current = kb.read_advanced()?;
                advanced::check_raw_kept(slots, &current)?;
            }
            for f in &frames {
                kb.exchange(f, ack(CMD_ADVANCED, 0), 1)?;
            }
            let back = kb.read_advanced()?;
            for (s, a) in slots {
                let want = advanced::decode(&advanced::encode(a)?)?;
                if back[*s as usize] != want {
                    return Err(Error::Verify(format!("advanced-key slot {s} reads back as {:?}", back[*s as usize])));
                }
            }
            Ok(())
        })
    }

    // ---- trigger ---------------------------------------------------------------------------------------------

    /// Travel limits and unit (`21 sub 04`), cached.
    pub fn travel_limits(&mut self) -> Result<TravelLimits> {
        if let Some(l) = self.limits {
            return Ok(l);
        }
        let hp = self.capabilities()?.high_precision();
        let b = self.query(&trigger::limits_request(), Reply::TriggerSub(SUB_LIMITS))?;
        let l = trigger::parse_limits(&b, hp)?;
        self.limits = Some(l);
        Ok(l)
    }

    pub fn read_trigger(&mut self, idx: usize) -> Result<KeyTrigger> {
        if !layout::is_key(idx) {
            return Err(Error::Invalid(format!("{} is not a key", key_name(idx))));
        }
        let (row, col) = frame::row_col(idx);
        let b = self.query(&trigger::key_request(idx), Reply::KeyTrigger(row, col))?;
        let (got, t) = trigger::parse_key(&b)?;
        if got != idx {
            return protocol(format!("trigger reply for {} instead of {}", key_name(got), key_name(idx)));
        }
        Ok(t)
    }

    /// Trigger config of the given keys (all keys: `layout::all_indices()`).
    pub fn read_triggers(&mut self, keys: &[usize]) -> Result<PerKey<KeyTrigger>> {
        self.with_lock(|kb| {
            let mut out = PerKey::new();
            for &idx in keys {
                out.0.insert(idx as u8, kb.read_trigger(idx)?);
            }
            Ok(out)
        })
    }

    /// Writes per-key trigger settings (one frame per group of identical settings) and reads every key back.
    pub fn write_triggers(&mut self, changes: &PerKey<KeyTrigger>) -> Result<()> {
        let limits = self.travel_limits()?;
        for (idx, t) in changes.iter() {
            t.validate(&limits).map_err(|e| Error::Invalid(format!("{}: {e}", key_name(idx))))?;
        }
        let frames = trigger::write_frames(changes)?;
        if frames.is_empty() {
            return Ok(());
        }
        self.before_write()?;
        self.with_lock(|kb| {
            for f in &frames {
                kb.exchange(f, TRIGGER_ACK, 1)?;
            }
            for (idx, t) in changes.iter() {
                let back = kb.read_trigger(idx)?;
                if back != t.normalized() {
                    return Err(Error::Verify(format!("{} trigger reads back as {back:?}", key_name(idx))));
                }
            }
            Ok(())
        })
    }

    /// Resets every key's actuation / rapid-trigger config to factory (`21 .. 18 06`). DESTRUCTIVE.
    /// Returns false when the board sent no ACK (the command was never seen on the wire).
    pub fn reset_triggers(&mut self, confirm: Confirm) -> Result<bool> {
        self.before_write()?;
        let acked = self.ack_or_timeout(&travel::trigger_reset_frame(confirm), TRIGGER_ACK)?;
        self.limits = None;
        Ok(acked)
    }

    /// Factory reset like the official "Reset" button: `14 00 00 00 01 01`, trigger reset, SOCD off.
    /// DESTRUCTIVE. Returns the steps the board did not acknowledge.
    pub fn factory_reset(&mut self, confirm: Confirm) -> Result<Vec<&'static str>> {
        self.before_write()?;
        let socd = self.capabilities()?.socd;
        self.with_lock(|kb| {
            let mut missing = Vec::new();
            if !kb.ack_or_timeout(&travel::factory_reset_frame(confirm), ack(CMD_PERF, 0))? {
                missing.push("factory reset");
            }
            if !kb.ack_or_timeout(&travel::trigger_reset_frame(confirm), TRIGGER_ACK)? {
                missing.push("trigger reset");
            }
            if socd && !kb.ack_or_timeout(&socd::power_frame(false), ack(CMD_SOCD, 0))? {
                missing.push("SOCD off");
            }
            kb.limits = None;
            Ok(missing)
        })
    }

    /// Sends a JS-only command whose ACK is expected but unproven: a timeout is reported, not an error.
    fn ack_or_timeout(&mut self, req: &Body, reply: Reply) -> Result<bool> {
        match self.exchange(req, reply, 1) {
            Ok(_) => Ok(true),
            Err(Error::Timeout(_)) => Ok(false),
            Err(e) => Err(e),
        }
    }

    pub fn read_dead_band(&mut self) -> Result<PerKey<DeadBand>> {
        self.require("dead band", Capabilities::dead_band)?;
        let t = self.read_table(CMD_DEAD_BAND, 0, trigger::DEAD_BAND_TABLE)?;
        trigger::decode_dead_band(&t)
    }

    /// Writes the whole dead-band table (missing keys 0/0) and reads it back.
    pub fn write_dead_band(&mut self, map: &PerKey<DeadBand>) -> Result<()> {
        self.require("dead band", Capabilities::dead_band)?;
        let table = trigger::encode_dead_band(map)?;
        trigger::validate_dead_band(map, &self.travel_limits()?)?;
        self.before_write()?;
        self.with_lock(|kb| {
            kb.write_table(CMD_DEAD_BAND, 1, trigger::DEAD_BAND_TABLE, &table, ack(CMD_DEAD_BAND, 1), Duration::ZERO)?;
            let back = kb.read_table(CMD_DEAD_BAND, 0, trigger::DEAD_BAND_TABLE)?;
            verify_table("dead band", &table, &back, 2)
        })
    }

    /// Switch ids the board offers (`25 00`).
    pub fn read_switch_list(&mut self) -> Result<Vec<u8>> {
        self.require("switch types", Capabilities::switch_types)?;
        let b = self.query(&frame::request(CMD_SWITCH, 0), Reply::CmdSel(CMD_SWITCH, 0))?;
        trigger::parse_switch_list(&b)
    }

    pub fn read_switch_types(&mut self) -> Result<PerKey<u8>> {
        self.require("switch types", Capabilities::switch_types)?;
        let t = self.read_table(CMD_SWITCH, 2, trigger::SWITCH_TABLE)?;
        trigger::decode_switch_table(&t)
    }

    /// Writes the switch type of every key and reads it back. Recalibrate changed keys afterwards.
    pub fn write_switch_types(&mut self, map: &PerKey<u8>) -> Result<()> {
        let offered = self.read_switch_list()?;
        let table = trigger::encode_switch_table(map, &offered)?;
        self.before_write()?;
        self.with_lock(|kb| {
            kb.write_table(CMD_SWITCH, 1, trigger::SWITCH_TABLE, &table, ack(CMD_SWITCH, 1), Duration::ZERO)?;
            let back = kb.read_table(CMD_SWITCH, 2, trigger::SWITCH_TABLE)?;
            verify_table("switch types", &table, &back, 1)
        })
    }

    /// Poll rate in kHz.
    pub fn read_poll_rate(&mut self) -> Result<u8> {
        self.require("poll rate", Capabilities::poll_rate)?;
        let b = self.query(&trigger::poll_rate_request(), Reply::TriggerSub(SUB_POLL_RATE))?;
        trigger::parse_poll_rate(&b)
    }

    /// Sets the poll rate. The keyboard restarts (USB re-enumeration): this handle is dead afterwards; wait with
    /// `hidraw::wait_for_device` and open it again. Cannot be read back here.
    pub fn set_poll_rate(&mut self, khz: u8) -> Result<()> {
        self.require("poll rate", Capabilities::poll_rate)?;
        let f = trigger::poll_rate_frame(khz)?;
        self.before_write()?;
        self.send_only(&f)?;
        self.gone = true;
        Ok(())
    }

    // ---- SOCD, win-lock --------------------------------------------------------------------------------------

    pub fn read_socd_power(&mut self) -> Result<SocdPower> {
        self.require("SOCD", |c| c.socd)?;
        let b = self.query(&frame::request(CMD_SOCD, 2), Reply::CmdSel(CMD_SOCD, 2))?;
        socd::parse_power(&b)
    }

    pub fn set_socd_enabled(&mut self, on: bool) -> Result<()> {
        self.require("SOCD", |c| c.socd)?;
        self.before_write()?;
        self.with_lock(|kb| {
            kb.exchange(&socd::power_frame(on), ack(CMD_SOCD, 0), 1)?;
            if kb.read_socd_power()?.enabled != on {
                return Err(Error::Verify("SOCD power".into()));
            }
            Ok(())
        })
    }

    pub fn read_socd_pairs(&mut self) -> Result<Vec<SocdPair>> {
        self.require("SOCD", |c| c.socd)?;
        let t = self.read_table(CMD_SOCD, 1, socd::TABLE)?;
        socd::decode_pairs(&t)
    }

    pub fn write_socd_pairs(&mut self, pairs: &[SocdPair]) -> Result<()> {
        self.require("SOCD", |c| c.socd)?;
        let table = socd::encode_pairs(pairs)?;
        self.before_write()?;
        self.with_lock(|kb| {
            kb.write_table(CMD_SOCD, 0, socd::TABLE, &table, ack(CMD_SOCD, 0), Duration::ZERO)?;
            let back = kb.read_socd_pairs()?;
            if back != pairs {
                return Err(Error::Verify(format!("SOCD pairs read back as {back:?}")));
            }
            Ok(())
        })
    }

    pub fn read_win_lock(&mut self) -> Result<WinLock> {
        Ok(self.refresh_capabilities()?.win_lock())
    }

    /// Sets what win-lock blocks. Keeps the current NKRO/6KRO state.
    pub fn set_win_lock(&mut self, w: WinLock) -> Result<()> {
        let six = self.refresh_capabilities()?.six_key_rollover;
        self.before_write()?;
        self.with_lock(|kb| {
            kb.exchange(&info::win_lock_frame(w, six), ack(CMD_PERF, 0), 1)?;
            let back = kb.read_win_lock()?;
            if back != w {
                return Err(Error::Verify(format!("win-lock reads back as {back:?}")));
            }
            Ok(())
        })
    }

    // ---- travel test and calibration -------------------------------------------------------------------------

    /// Opens the live travel test for `keys`. The stream is closed when the returned guard is dropped.
    /// Holds the device lock until then.
    pub fn travel_stream(&mut self, keys: &[usize]) -> Result<TravelStream<'_, L>> {
        let open = travel::open_frame(keys)?;
        self.check(&open)?;
        self.link.lock()?;
        if let Err(e) = self.exchange_locked(&open, TRIGGER_ACK, 1) {
            // it may have opened even though the ACK was lost
            let _ = self.close_travel_locked();
            self.link.unlock();
            return Err(e);
        }
        Ok(TravelStream { kb: self, closed: false })
    }

    fn close_travel_locked(&mut self) -> Result<()> {
        let close = travel::close_frame();
        self.link.send(&close)?;
        let deadline = Instant::now() + CLOSE_TIMEOUT;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(Error::Timeout("travel test close was not acknowledged".into()));
            }
            match self.link.recv(left)? {
                Some(b) if TRIGGER_ACK.matches(&b) => return Ok(()),
                _ => {}
            }
        }
    }

    /// Starts any-key calibration: press every key to calibrate fully down and release it, then `stop`.
    /// Calibration is stopped when the returned guard is dropped. Holds the device lock until then.
    pub fn calibrate(&mut self, confirm: Confirm) -> Result<Calibration<'_, L>> {
        self.require("any-key calibration", Capabilities::any_key_calibration)?;
        self.before_write()?;
        let start = travel::calibration_start_frame(confirm);
        self.check(&start)?;
        self.link.lock()?;
        if let Err(e) = self.link.drain().and_then(|_| self.link.send(&start)) {
            self.link.unlock();
            return Err(e);
        }
        Ok(Calibration { kb: self, stopped: false })
    }

    // ---- snapshot --------------------------------------------------------------------------------------------

    /// Reads everything readable into a snapshot.
    pub fn read_snapshot(&mut self) -> Result<Snapshot> {
        crate::snapshot::read(self)
    }

    /// Writes only the tables where `new` differs from `old` (normally a fresh `read_snapshot`), each with
    /// read-back. The poll rate goes last because the board restarts.
    pub fn apply_snapshot(&mut self, new: &Snapshot, old: &Snapshot) -> Result<crate::snapshot::ApplyReport> {
        crate::snapshot::apply(self, new, old)
    }
}

/// Compares a written table with its read-back and names the first differing key.
fn verify_table(what: &str, wrote: &[u8], back: &[u8], entry: usize) -> Result<()> {
    if wrote == back {
        return Ok(());
    }
    let at = wrote.iter().zip(back).position(|(a, b)| a != b).unwrap_or(0);
    Err(Error::Verify(format!("{what}: {} differs", key_name(at / entry))))
}

/// Live travel test. Dropping it closes the test (`21 .. 18 03`) and releases the device lock.
pub struct TravelStream<'a, L: Link> {
    kb: &'a mut Keyboard<L>,
    closed: bool,
}

impl<L: Link> TravelStream<'_, L> {
    /// Next sample, or None when no key moved within `timeout`.
    pub fn next_sample(&mut self, timeout: Duration) -> Result<Option<TravelSample>> {
        let deadline = Instant::now() + timeout;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Ok(None);
            }
            match self.kb.link.recv(left)? {
                Some(b) => {
                    if let Some(s) = travel::parse_sample(&b) {
                        return Ok(Some(s));
                    }
                }
                None => return Ok(None),
            }
        }
    }

    /// Calls `f` with each sample, or with None every `tick` without samples (to check a stop flag or redraw),
    /// until `f` breaks.
    pub fn run(&mut self, tick: Duration, mut f: impl FnMut(Option<TravelSample>) -> ControlFlow<()>) -> Result<()> {
        loop {
            if f(self.next_sample(tick)?).is_break() {
                return Ok(());
            }
        }
    }

    /// Closes the test and reports whether the board acknowledged it.
    pub fn close(mut self) -> Result<()> {
        self.closed = true;
        let r = self.kb.close_travel_locked();
        self.kb.link.unlock();
        r
    }
}

impl<L: Link> Drop for TravelStream<'_, L> {
    fn drop(&mut self) {
        if !self.closed {
            if let Err(e) = self.kb.close_travel_locked() {
                eprintln!("aula-core: closing the travel test: {e}");
            }
            self.kb.link.unlock();
        }
    }
}

/// Running calibration. Dropping it sends stop (`21 .. 18 10 00`) and releases the device lock.
pub struct Calibration<'a, L: Link> {
    kb: &'a mut Keyboard<L>,
    stopped: bool,
}

impl<L: Link> Calibration<'_, L> {
    /// Next progress notification, or None within `timeout`.
    pub fn next_event(&mut self, timeout: Duration) -> Result<Option<CalibrationEvent>> {
        let deadline = Instant::now() + timeout;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Ok(None);
            }
            match self.kb.link.recv(left)? {
                Some(b) => {
                    if let Some(e) = travel::parse_calibration(&b) {
                        return Ok(Some(e));
                    }
                }
                None => return Ok(None),
            }
        }
    }

    /// Sends stop and waits for the board's final status: `Finished`, `Failed(code)`, or None when it sent
    /// nothing within 2 s.
    pub fn stop(mut self) -> Result<Option<CalibrationEvent>> {
        self.stopped = true;
        let r = self.stop_inner();
        self.kb.link.unlock();
        r
    }

    fn stop_inner(&mut self) -> Result<Option<CalibrationEvent>> {
        self.kb.link.send(&travel::calibration_stop_frame())?;
        let deadline = Instant::now() + CALIBRATION_STOP_TIMEOUT;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.next_event(left)? {
                Some(e @ (CalibrationEvent::Finished | CalibrationEvent::Failed(_))) => return Ok(Some(e)),
                Some(CalibrationEvent::Progress(_)) => {}
                None => return Ok(None),
            }
        }
    }
}

impl<L: Link> Drop for Calibration<'_, L> {
    fn drop(&mut self) {
        if !self.stopped {
            if let Err(e) = self.stop_inner() {
                eprintln!("aula-core: stopping calibration: {e}");
            }
            self.kb.link.unlock();
        }
    }
}


#[cfg(test)]
impl<L: Link> TravelStream<'_, L> {
    pub(crate) fn link_for_test(&mut self) -> &mut L {
        &mut self.kb.link
    }
}

#[cfg(test)]
impl<L: Link> Calibration<'_, L> {
    pub(crate) fn link_for_test(&mut self) -> &mut L {
        &mut self.kb.link
    }
}

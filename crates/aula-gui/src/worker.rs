//! Device worker thread: owns the `Keyboard`, watches for hot-plug, and runs the commands the UI sends
//! (writes, profiles, backups, the live travel stream and calibration). Nothing here touches Qt; results go
//! out through a `publish` callback. The UI thread never waits for the keyboard.

use crate::files::{self, Backup, Dirs};
use crate::link::{GuiLink, SimLink};
use crate::live::LiveState;
use aula_core::hidraw::{self, Device};
use aula_core::layout;
use aula_core::lighting::ModeList;
use aula_core::sim::SimKeyboard;
use aula_core::store::{self, Profiles};
use aula_core::travel::{CalibrationEvent, Confirm};
use aula_core::{Error, Keyboard, Snapshot};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Set by SIGINT / SIGTERM / SIGHUP: the worker closes any stream and asks the UI to quit.
pub static SIGNALLED: AtomicBool = AtomicBool::new(false);

/// How often the main loop looks at the quit flag while idle.
const IDLE_TICK: Duration = Duration::from_millis(200);
/// How long one wait for a travel sample or calibration event lasts before commands are checked.
const STREAM_TICK: Duration = Duration::from_millis(15);
/// After a poll-rate change: how long the old node may take to go away, and the board to come back.
const REENUMERATE_GONE: Duration = Duration::from_secs(3);
const REENUMERATE_BACK: Duration = Duration::from_secs(10);
const REOPEN_TRIES: u32 = 15;
const REOPEN_PAUSE: Duration = Duration::from_millis(200);

const NOT_CONNECTED: &str = "no AULA WIN 60 HE is connected";
const NOT_LOADED: &str = "the settings have not been read yet";

pub enum Command {
    Refresh,
    /// A whole snapshot (JSON) with the page's edits; only the tables that differ are written.
    Apply(String),
    SetMusicRhythm(bool),
    SaveProfile { name: String, overwrite: bool },
    ApplyProfile(String),
    DeleteProfile(String),
    BackupNow,
    Restore(PathBuf),
    StartLive,
    StopLive,
    ResetLive,
    StartCalibration,
    StopCalibration,
    ResetTriggers,
    FactoryReset,
    Quit,
}

/// Board details that are not part of a snapshot.
#[derive(Debug, Clone, PartialEq)]
pub struct Info {
    pub features: Vec<(&'static str, bool)>,
    pub music_rhythm: bool,
    pub music_rhythm_on: bool,
    pub calibration: bool,
    pub modes: ModeList,
    pub switches: Vec<u8>,
    pub fn_fixed: Vec<usize>,
    pub socd_hotkey: Option<[u8; 2]>,
}

/// What the UI shows.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct State {
    /// A `2e3c:c365` vendor interface exists (or the simulator runs).
    pub present: bool,
    /// Opened and identified.
    pub connected: bool,
    /// Identified as a WIN 60 HE; other boards only get identification commands.
    pub supported: bool,
    pub sim: bool,
    pub node: String,
    pub product_code: String,
    pub firmware: String,
    pub build_date: String,
    pub info: Option<Info>,
    /// None until the first full read on this connection, and again after a failed one.
    pub snapshot: Option<Snapshot>,
}

pub enum Event {
    State(Box<State>),
    Busy(bool),
    Error(String),
    /// A message for the snackbar: i18n key and the `%1` argument.
    Notice(&'static str, String),
    Profiles(Vec<String>),
    Backups(Vec<Backup>),
    LiveRunning(bool),
    /// `LiveState::take_json`.
    LiveFrame(String),
    CalibrationRunning(bool),
    CalibrationKeys(Vec<usize>),
    /// The board's final word: finished (true) or failed with a code; None when it said nothing.
    CalibrationResult(Option<(bool, u8)>),
    QuitRequested,
}

/// The running worker, so `shutdown` can stop it after the event loop has ended.
struct Running {
    commands: Sender<Command>,
    /// Disconnects when the worker thread ends.
    done: Receiver<()>,
}

static RUNNING: Mutex<Option<Running>> = Mutex::new(None);

/// Starts the worker thread `f`; `commands` is kept for `shutdown`.
pub fn spawn(commands: Sender<Command>, f: impl FnOnce() + Send + 'static) -> std::io::Result<()> {
    let (done_tx, done) = mpsc::channel::<()>();
    std::thread::Builder::new().name("aula-device".into()).spawn(move || {
        let _done = done_tx;
        f();
    })?;
    *RUNNING.lock().unwrap_or_else(|p| p.into_inner()) = Some(Running { commands, done });
    Ok(())
}

/// Stops the worker (closing a travel stream or ending calibration first) and waits for it up to `timeout`.
/// Called by main after the window has closed.
pub fn shutdown(timeout: Duration) {
    let running = RUNNING.lock().unwrap_or_else(|p| p.into_inner()).take();
    if let Some(r) = running {
        let _ = r.commands.send(Command::Quit);
        let _ = r.done.recv_timeout(timeout);
    }
}

pub struct Settings {
    pub sim: bool,
    pub poll: Duration,
    pub live_publish: Duration,
    /// Backup and profile directories; None = `Dirs::for_run(sim)`.
    pub dirs: Option<Dirs>,
    /// The simulated board (with `sim`); None = a fresh copy of the owner's board.
    pub sim_board: Option<Arc<Mutex<SimKeyboard>>>,
}

/// Runs until `Quit`, the command channel closes, or `publish` returns false (the UI object is gone).
pub fn run(commands: Receiver<Command>, settings: Settings, publish: impl FnMut(Event) -> bool) {
    let dirs = settings.dirs.map_or_else(|| Dirs::for_run(settings.sim), Ok);
    let sim_board = settings.sim_board;
    let mut w = Worker {
        publish,
        alive: true,
        commands,
        sim: settings.sim.then(|| sim_board.unwrap_or_else(|| Arc::new(Mutex::new(SimKeyboard::owner_board())))),
        dirs,
        kb: None,
        node: None,
        state: State { sim: settings.sim, ..State::default() },
        live_publish: settings.live_publish,
        poll_error: None,
        announced_backup: None,
    };
    w.poll();
    w.list_files();
    let mut next_poll = Instant::now() + settings.poll;
    while w.alive {
        if SIGNALLED.load(Ordering::Relaxed) {
            w.emit(Event::QuitRequested);
            break;
        }
        match w.commands.recv_timeout(IDLE_TICK) {
            Ok(Command::Quit) | Err(RecvTimeoutError::Disconnected) => break,
            Ok(c) => w.handle(c),
            Err(RecvTimeoutError::Timeout) => {}
        }
        if Instant::now() >= next_poll {
            w.poll();
            next_poll = Instant::now() + settings.poll;
        }
    }
}

struct Worker<P> {
    publish: P,
    alive: bool,
    commands: Receiver<Command>,
    /// The simulated board, kept across reopen.
    sim: Option<Arc<Mutex<SimKeyboard>>>,
    dirs: Result<Dirs, String>,
    kb: Option<Keyboard<GuiLink>>,
    node: Option<PathBuf>,
    state: State,
    live_publish: Duration,
    /// Last polling error: shown once, not on every poll.
    poll_error: Option<String>,
    /// The automatic backup already reported.
    announced_backup: Option<PathBuf>,
}

/// What the Overview page shows as the device node.
fn node_label(node: Option<&Path>) -> String {
    node.map_or_else(|| "simulator".into(), |n| n.display().to_string())
}

fn ms_since(start: Instant) -> u64 {
    u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX)
}

/// Reads everything the pages need in one locked operation.
fn read_all(kb: &mut Keyboard<GuiLink>) -> Result<(Info, Snapshot), Error> {
    kb.with_lock(|kb| {
        let caps = kb.refresh_capabilities()?;
        let modes = kb.read_mode_list()?;
        let switches = if caps.switch_types() { kb.read_switch_list()? } else { Vec::new() };
        let fn_fixed = kb.read_fn_fixed()?;
        let socd_hotkey = if caps.socd { Some(kb.read_socd_power()?.hotkey) } else { None };
        let snapshot = kb.read_snapshot()?;
        let info = Info {
            features: caps.features(),
            music_rhythm: caps.music_rhythm(),
            music_rhythm_on: caps.music_rhythm_on,
            calibration: caps.any_key_calibration(),
            modes,
            switches,
            fn_fixed,
            socd_hotkey,
        };
        Ok((info, snapshot))
    })
}

/// Errors after which the handle is dropped and the board opened again on the next poll.
fn is_link_error(e: &Error) -> bool {
    matches!(e, Error::Io(_) | Error::NotFound)
}

/// Whether two snapshots hold the same settings (the read time aside).
const CHANGED_ELSEWHERE: &str = "the keyboard's settings changed since they were read (another program?); nothing was written and the pages now show the current values";

pub fn same_settings(a: &Snapshot, b: &Snapshot) -> bool {
    let mut b = b.clone();
    b.created_unix = a.created_unix;
    *a == b
}

impl<P: FnMut(Event) -> bool> Worker<P> {
    fn emit(&mut self, event: Event) {
        if !(self.publish)(event) {
            self.alive = false;
        }
    }

    fn report(&mut self, error: String) {
        eprintln!("aula-gui: {error}");
        self.emit(Event::Error(error));
    }

    fn publish_state(&mut self, next: State) {
        if next != self.state {
            self.state = next.clone();
            self.emit(Event::State(Box::new(next)));
        }
    }

    fn disconnect(&mut self) {
        self.kb = None;
        self.node = None;
        let next = State { sim: self.state.sim, ..State::default() };
        self.publish_state(next);
    }

    /// Hot-plug check: no HID traffic while connected, only the node list.
    fn poll(&mut self) {
        match self.try_poll() {
            Ok(()) => self.poll_error = None,
            Err(e) => {
                if self.poll_error.as_ref() != Some(&e) {
                    self.poll_error = Some(e.clone());
                    self.report(e);
                }
            }
        }
    }

    fn try_poll(&mut self) -> Result<(), String> {
        if self.sim.is_some() {
            if self.kb.is_none() {
                self.open()?;
            }
            return Ok(());
        }
        let found = hidraw::find_devices();
        if let Some(node) = &self.node {
            if found.iter().any(|d| &d.node == node) {
                return Ok(());
            }
            self.disconnect();
        }
        if found.is_empty() {
            if self.state.present {
                self.disconnect();
            }
            return Ok(());
        }
        self.open()
    }

    fn open_keyboard(&self) -> Result<(Keyboard<GuiLink>, Option<PathBuf>), Error> {
        if let Some(board) = &self.sim {
            let mut kb = Keyboard::with_link(GuiLink::sim(SimLink::new(board.clone())))?;
            kb.set_auto_backup(None);
            return Ok((kb, None));
        }
        let info = hidraw::find_devices().into_iter().next().ok_or(Error::NotFound)?;
        let mut kb = Keyboard::with_link(GuiLink::hid(Device::open(&info.node)?))?;
        // Before the first write of this handle the whole configuration is saved here. Without a backup
        // directory the core refuses every write.
        if let Ok(d) = &self.dirs {
            kb.set_auto_backup(Some(d.backups.clone()));
        }
        Ok((kb, Some(info.node)))
    }

    /// Opens and identifies the first board, then reads everything.
    fn open(&mut self) -> Result<(), String> {
        let mut next = State { sim: self.state.sim, present: true, ..State::default() };
        let (kb, node) = match self.open_keyboard() {
            Ok(v) => v,
            Err(e) => {
                self.publish_state(next);
                return Err(e.into());
            }
        };
        let id = kb.identity().clone();
        next.connected = true;
        next.supported = kb.is_supported();
        next.node = node_label(node.as_deref());
        next.product_code = id.product_code;
        next.firmware = id.firmware;
        next.build_date = id.build_date;
        self.kb = Some(kb);
        self.node = node;
        self.publish_state(next);
        if self.state.supported {
            self.reload()?;
        }
        Ok(())
    }

    /// Reads everything again and publishes it. A failed read leaves the pages without settings.
    fn reload(&mut self) -> Result<(), String> {
        let kb = self.kb.as_mut().ok_or(NOT_CONNECTED)?;
        match read_all(kb) {
            Ok((info, snapshot)) => {
                let next = State { info: Some(info), snapshot: Some(snapshot), ..self.state.clone() };
                self.publish_state(next);
                Ok(())
            }
            Err(e) => {
                if is_link_error(&e) {
                    self.disconnect();
                } else {
                    let next = State { info: None, snapshot: None, ..self.state.clone() };
                    self.publish_state(next);
                }
                Err(e.into())
            }
        }
    }

    fn handle(&mut self, c: Command) {
        match c {
            Command::Refresh => self.command(|w| {
                if w.kb.is_none() {
                    w.try_poll()
                } else {
                    w.reload()
                }
            }),
            Command::Apply(json) => self.command(|w| w.apply_json(&json)),
            Command::SetMusicRhythm(on) => self.command(|w| w.set_music_rhythm(on)),
            Command::SaveProfile { name, overwrite } => self.command(|w| w.save_profile(&name, overwrite)),
            Command::ApplyProfile(name) => self.command(|w| w.apply_profile(&name)),
            Command::DeleteProfile(name) => self.command(|w| w.delete_profile(&name)),
            Command::BackupNow => self.command(Worker::backup_now),
            Command::Restore(path) => self.command(|w| w.restore(&path)),
            Command::StartLive => self.command(Worker::live),
            Command::StartCalibration => self.command(Worker::calibrate),
            Command::ResetTriggers => self.command(Worker::reset_triggers),
            Command::FactoryReset => self.command(Worker::factory_reset),
            // Only meaningful while a stream runs; the stream loops take these themselves.
            Command::StopLive | Command::ResetLive | Command::StopCalibration => {}
            Command::Quit => self.alive = false,
        }
    }

    /// A UI command: busy while it runs, every failure reported. After an I/O error the handle is dropped so
    /// the next poll opens the board again (also when it came back under the same hidrawN).
    fn command(&mut self, f: impl FnOnce(&mut Self) -> Result<(), String>) {
        self.emit(Event::Busy(true));
        if let Err(e) = f(self) {
            if self.kb.as_ref().is_some_and(|kb| kb.link().io_failed()) {
                self.disconnect();
            }
            self.report(e);
        }
        self.announce_backup();
        self.emit(Event::Busy(false));
    }

    /// Tells the user once where the automatic backup before the first write went.
    fn announce_backup(&mut self) {
        let path = self.kb.as_ref().and_then(|kb| kb.last_backup()).map(Path::to_path_buf);
        if let Some(p) = path {
            if self.announced_backup.as_ref() != Some(&p) {
                self.announced_backup = Some(p.clone());
                self.emit(Event::Notice("notice.auto_backup", p.display().to_string()));
                self.list_backups();
            }
        }
    }

    fn supported_kb(&mut self) -> Result<&mut Keyboard<GuiLink>, String> {
        let kb = self.kb.as_mut().ok_or(NOT_CONNECTED)?;
        if !kb.is_supported() {
            return Err(Error::UnsupportedProduct { product_code: kb.identity().product_code.clone() }.into());
        }
        Ok(kb)
    }

    /// Reads the board and checks it still holds what the pages show, so an edit made on an old view
    /// (another program changed the board) is never written over it.
    fn fresh_snapshot(&mut self) -> Result<Snapshot, String> {
        let shown = self.state.snapshot.clone().ok_or(NOT_LOADED)?;
        let fresh = self.supported_kb()?.read_snapshot().map_err(String::from)?;
        if !same_settings(&shown, &fresh) {
            let _ = self.reload();
            return Err(CHANGED_ELSEWHERE.into());
        }
        Ok(fresh)
    }

    /// Writes the tables where `new` differs from the board, then reads everything back. The check that the
    /// board still holds what the pages show and the write happen under one device lock.
    fn write_snapshot(&mut self, new: &Snapshot) -> Result<(), String> {
        let shown = self.state.snapshot.clone().ok_or(NOT_LOADED)?;
        let result = self.supported_kb()?.with_lock(|kb| {
            let fresh = kb.read_snapshot()?;
            if !same_settings(&shown, &fresh) {
                return Ok(None);
            }
            kb.apply_snapshot(new, &fresh).map(Some)
        });
        let report = match result {
            Ok(Some(r)) => r,
            Ok(None) => {
                let _ = self.reload();
                return Err(CHANGED_ELSEWHERE.into());
            }
            Err(e) => {
                // Part may have been written: show what the board holds now.
                let _ = self.reload();
                return Err(e.into());
            }
        };
        if report.reconnect_needed {
            self.reconnect()?;
        }
        self.reload()?;
        let n = report.written.len();
        self.emit(Event::Notice(if n == 0 { "notice.nothing_changed" } else { "notice.applied" }, n.to_string()));
        Ok(())
    }

    fn apply_json(&mut self, json: &str) -> Result<(), String> {
        let new = Snapshot::from_json(json)?;
        self.write_snapshot(&new)
    }

    /// After a poll-rate change the board restarts: wait for the old node to go and a new one to come.
    fn reconnect(&mut self) -> Result<(), String> {
        let old = self.node.clone();
        self.kb = None;
        self.node = None;
        if self.sim.is_none() {
            let deadline = Instant::now() + REENUMERATE_GONE;
            while Instant::now() < deadline && hidraw::find_devices().iter().any(|d| Some(&d.node) == old.as_ref()) {
                std::thread::sleep(REOPEN_PAUSE);
            }
            hidraw::wait_for_device(REENUMERATE_BACK).map_err(String::from)?;
        }
        let mut last = String::new();
        for _ in 0..REOPEN_TRIES {
            match self.open_keyboard() {
                Ok((kb, node)) => {
                    let next = State { node: node_label(node.as_deref()), ..self.state.clone() };
                    self.kb = Some(kb);
                    self.node = node;
                    self.publish_state(next);
                    self.emit(Event::Notice("notice.reconnected", String::new()));
                    return Ok(());
                }
                // udev may not have applied the access rule yet.
                Err(e) => last = e.to_string(),
            }
            std::thread::sleep(REOPEN_PAUSE);
        }
        self.disconnect();
        Err(format!("the keyboard did not come back after the poll-rate change: {last}"))
    }

    fn set_music_rhythm(&mut self, on: bool) -> Result<(), String> {
        self.supported_kb()?.set_music_rhythm(on).map_err(String::from)?;
        self.reload()
    }

    fn profiles(&self) -> Result<Profiles, String> {
        Ok(Profiles::in_dir(self.dirs.as_ref().map_err(Clone::clone)?.profiles.clone()))
    }

    fn list_files(&mut self) {
        self.list_profiles();
        self.list_backups();
    }

    fn list_profiles(&mut self) {
        match self.profiles().and_then(|p| p.list().map_err(String::from)) {
            Ok(list) => self.emit(Event::Profiles(list)),
            Err(e) => self.report(e),
        }
    }

    fn list_backups(&mut self) {
        let list = self.dirs.as_ref().map_err(Clone::clone).and_then(|d| files::list_backups(&d.backups));
        match list {
            Ok(list) => self.emit(Event::Backups(list)),
            Err(e) => self.report(e),
        }
    }

    fn save_profile(&mut self, name: &str, overwrite: bool) -> Result<(), String> {
        store::validate_profile_name(name)?;
        let snap = self.fresh_snapshot()?;
        let path = self.profiles()?.save(name, &snap, overwrite)?;
        self.list_profiles();
        self.emit(Event::Notice("notice.profile_saved", path.display().to_string()));
        Ok(())
    }

    fn apply_profile(&mut self, name: &str) -> Result<(), String> {
        let snap = self.profiles()?.load(name)?;
        self.write_snapshot(&snap)
    }

    fn delete_profile(&mut self, name: &str) -> Result<(), String> {
        self.profiles()?.delete(name)?;
        self.list_profiles();
        Ok(())
    }

    fn backup_now(&mut self) -> Result<(), String> {
        let snap = self.fresh_snapshot()?;
        let dir = self.dirs.as_ref().map_err(Clone::clone)?.backups.clone();
        store::private_dir(&dir)?;
        let path = files::manual_backup_path(&dir, snap.created_unix);
        store::save_snapshot_new(&path, &snap)?;
        self.list_backups();
        self.emit(Event::Notice("notice.backup_saved", path.display().to_string()));
        Ok(())
    }

    fn restore(&mut self, path: &Path) -> Result<(), String> {
        let snap = store::load_snapshot(path)?;
        self.write_snapshot(&snap)
    }

    /// Streams the depth of every key until `StopLive` (or quit). The stream is closed on every way out.
    fn live(&mut self) -> Result<(), String> {
        self.supported_kb()?;
        let Worker { kb, commands, publish, alive, live_publish, .. } = self;
        let kb = kb.as_mut().ok_or(NOT_CONNECTED)?;
        let mut emit = |e: Event, alive: &mut bool| {
            if !publish(e) {
                *alive = false;
            }
        };
        let mut stream = kb.travel_stream(&layout::all_indices())?;
        emit(Event::LiveRunning(true), alive);
        let start = Instant::now();
        let mut state = LiveState::new();
        let mut last_frame = Instant::now();
        let result = loop {
            if SIGNALLED.load(Ordering::Relaxed) || !*alive {
                break Ok(());
            }
            match commands.try_recv() {
                Ok(Command::StopLive) => break Ok(()),
                Ok(Command::Quit) | Err(TryRecvError::Disconnected) => {
                    *alive = false;
                    break Ok(());
                }
                Ok(Command::ResetLive) => state.reset(),
                Ok(_) => emit(Event::Error("stop the live view first".into()), alive),
                Err(TryRecvError::Empty) => {}
            }
            match stream.next_sample(STREAM_TICK) {
                Ok(Some(s)) => state.apply(&s, ms_since(start)),
                Ok(None) => {}
                Err(e) => break Err(e),
            }
            if last_frame.elapsed() >= *live_publish {
                last_frame = Instant::now();
                if state.tick(ms_since(start)) {
                    emit(Event::LiveFrame(state.take_json().to_string()), alive);
                }
            }
        };
        let closed = stream.close();
        emit(Event::LiveRunning(false), alive);
        result?;
        closed?;
        Ok(())
    }

    /// Any-key calibration until `StopCalibration`; stopping makes the board store the result.
    fn calibrate(&mut self) -> Result<(), String> {
        self.supported_kb()?;
        let result = {
            let Worker { kb, commands, publish, alive, .. } = self;
            let kb = kb.as_mut().ok_or(NOT_CONNECTED)?;
            let mut emit = |e: Event, alive: &mut bool| {
                if !publish(e) {
                    *alive = false;
                }
            };
            // The user confirmed in the dialog that sent this command.
            let mut cal = kb.calibrate(Confirm::user_confirmed())?;
            emit(Event::CalibrationRunning(true), alive);
            let mut early: Option<CalibrationEvent> = None;
            let run = loop {
                if SIGNALLED.load(Ordering::Relaxed) || !*alive {
                    break Ok(());
                }
                match commands.try_recv() {
                    Ok(Command::StopCalibration) => break Ok(()),
                    Ok(Command::Quit) | Err(TryRecvError::Disconnected) => {
                        *alive = false;
                        break Ok(());
                    }
                    Ok(_) => emit(Event::Error("finish the calibration first".into()), alive),
                    Err(TryRecvError::Empty) => {}
                }
                match cal.next_event(STREAM_TICK) {
                    Ok(Some(CalibrationEvent::Progress(keys))) => {
                        let keys = keys.into_iter().filter(|k| layout::is_key(*k)).collect();
                        emit(Event::CalibrationKeys(keys), alive);
                    }
                    Ok(Some(end)) => {
                        early = Some(end);
                        break Ok(());
                    }
                    Ok(None) => {}
                    Err(e) => break Err(e),
                }
            };
            let stopped = cal.stop();
            emit(Event::CalibrationRunning(false), alive);
            let end = match (early, &stopped) {
                (Some(e), _) => Some(e),
                (None, Ok(e)) => e.clone(),
                (None, Err(_)) => None,
            };
            let outcome = match end {
                Some(CalibrationEvent::Finished) => Some((true, 0)),
                Some(CalibrationEvent::Failed(code)) => Some((false, code)),
                _ => None,
            };
            emit(Event::CalibrationResult(outcome), alive);
            run.and(stopped.map(|_| ()))
        };
        result.map_err(String::from)
    }

    fn reset_triggers(&mut self) -> Result<(), String> {
        // The confirmation dialog sent this command.
        let acked = self.supported_kb()?.reset_triggers(Confirm::user_confirmed()).map_err(String::from)?;
        self.reload()?;
        self.emit(Event::Notice(if acked { "notice.triggers_reset" } else { "notice.no_ack" }, String::new()));
        Ok(())
    }

    fn factory_reset(&mut self) -> Result<(), String> {
        // The confirmation dialog sent this command.
        let missing = self.supported_kb()?.factory_reset(Confirm::user_confirmed()).map_err(String::from)?;
        self.reload()?;
        if missing.is_empty() {
            self.emit(Event::Notice("notice.factory_reset", String::new()));
        } else {
            self.emit(Event::Notice("notice.factory_reset_partial", missing.join(", ")));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Per-test backup/profile directory, removed when the test ends.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new() -> Self {
            static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            let n = N.fetch_add(1, Ordering::Relaxed);
            let d = std::env::temp_dir().join(format!("aula-gui-worker-test-{}-{n}", std::process::id()));
            let _ = std::fs::remove_dir_all(&d);
            std::fs::create_dir_all(&d).unwrap();
            Scratch(d)
        }

        fn dirs(&self) -> Dirs {
            Dirs { backups: self.0.join("backups"), profiles: self.0.join("profiles") }
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    struct Sim {
        tx: Sender<Command>,
        rx: Receiver<Event>,
        h: std::thread::JoinHandle<()>,
        board: Arc<Mutex<SimKeyboard>>,
        _scratch: Scratch,
    }

    /// Runs a simulator worker on a thread with its own scratch directory.
    fn start_sim() -> Sim {
        let (tx, rx) = mpsc::channel();
        let (etx, erx) = mpsc::channel();
        let scratch = Scratch::new();
        let board = Arc::new(Mutex::new(SimKeyboard::owner_board()));
        let settings = Settings {
            sim: true,
            poll: Duration::from_millis(100),
            live_publish: Duration::from_millis(20),
            dirs: Some(scratch.dirs()),
            sim_board: Some(board.clone()),
        };
        let h = std::thread::spawn(move || {
            run(rx, settings, move |e| etx.send(e).is_ok());
        });
        Sim { tx, rx: erx, h, board, _scratch: scratch }
    }

    /// Like `start_sim`; the scratch directory goes away when the returned guard drops.
    fn spawn_sim() -> (Sender<Command>, Receiver<Event>, JoinOnDrop) {
        let s = start_sim();
        (s.tx, s.rx, JoinOnDrop { h: Some(s.h), _scratch: s._scratch })
    }

    /// Join handle that also owns the scratch directory (removed after the worker ended).
    struct JoinOnDrop {
        h: Option<std::thread::JoinHandle<()>>,
        _scratch: Scratch,
    }

    impl JoinOnDrop {
        fn join(mut self) -> std::thread::Result<()> {
            self.h.take().expect("joined once").join()
        }
    }

    #[test]
    fn io_error_in_a_command_reopens_the_board() {
        let s = start_sim();
        loaded(&s.rx);
        s.board.lock().unwrap().io_error = true;
        s.tx.send(Command::BackupNow).unwrap();
        let gone = wait_for(&s.rx, |e| match e {
            Event::State(st) => Some(*st),
            _ => None,
        });
        assert!(!gone.connected, "handle dropped");
        let e = wait_for(&s.rx, |e| match e {
            Event::Error(m) => Some(m),
            _ => None,
        });
        assert!(e.contains("No such device"), "{e}");
        s.board.lock().unwrap().io_error = false;
        let back = loaded(&s.rx);
        assert!(back.connected && back.snapshot.is_some(), "the next poll opened it again");
        s.tx.send(Command::Quit).unwrap();
        s.h.join().unwrap();
    }

    #[test]
    fn other_errors_keep_the_handle() {
        let s = start_sim();
        loaded(&s.rx);
        s.tx.send(Command::ApplyProfile("missing".into())).unwrap();
        wait_for(&s.rx, |e| matches!(e, Event::Error(_)).then_some(()));
        s.tx.send(Command::Quit).unwrap();
        s.h.join().unwrap();
        while let Ok(e) = s.rx.try_recv() {
            assert!(!matches!(e, Event::State(st) if !st.connected), "not disconnected");
        }
    }

    #[test]
    fn poll_rate_change_reconnects_and_keeps_the_node() {
        let s = start_sim();
        let st = loaded(&s.rx);
        let mut snap = st.snapshot.unwrap();
        snap.poll_rate_khz = Some(if snap.poll_rate_khz == Some(8) { 1 } else { 8 });
        s.tx.send(Command::Apply(snap.to_json().unwrap())).unwrap();
        wait_for(&s.rx, |e| matches!(e, Event::Notice("notice.reconnected", _)).then_some(()));
        let after = loaded(&s.rx);
        assert!(after.connected);
        assert_eq!(after.node, "simulator");
        assert_eq!(after.snapshot.unwrap().poll_rate_khz, snap.poll_rate_khz);
        s.tx.send(Command::Quit).unwrap();
        s.h.join().unwrap();
    }

    #[test]
    fn node_labels() {
        assert_eq!(node_label(None), "simulator");
        assert_eq!(node_label(Some(Path::new("/dev/hidraw7"))), "/dev/hidraw7");
    }

    /// Waits for the first event `f` accepts.
    fn wait_for<T>(rx: &Receiver<Event>, mut f: impl FnMut(Event) -> Option<T>) -> T {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let e = rx.recv_timeout(left).expect("event in time");
            if let Some(v) = f(e) {
                return v;
            }
        }
    }

    fn loaded(rx: &Receiver<Event>) -> State {
        wait_for(rx, |e| match e {
            Event::State(s) if s.snapshot.is_some() => Some(*s),
            _ => None,
        })
    }

    #[test]
    fn sim_loads_and_applies_a_trigger_change() {
        let (tx, rx, h) = spawn_sim();
        let s = loaded(&rx);
        assert!(s.connected && s.supported && s.sim);
        assert_eq!(s.product_code, "SI2825KZHEARGB");
        let info = s.info.clone().unwrap();
        assert!(info.fn_fixed.contains(&22));
        let mut snap = s.snapshot.clone().unwrap();
        let q = layout::by_code("KeyQ").unwrap().index;
        snap.trigger.0.get_mut(&q).unwrap().actuation = 150;
        tx.send(Command::Apply(snap.to_json().unwrap())).unwrap();
        let after = loaded(&rx);
        assert_eq!(after.snapshot.unwrap().trigger.0[&q].actuation, 150);
        let n = wait_for(&rx, |e| match e {
            Event::Notice("notice.applied", n) => Some(n),
            _ => None,
        });
        assert_eq!(n, "1");
        tx.send(Command::Quit).unwrap();
        h.join().unwrap();
    }

    #[test]
    fn sim_rejects_an_invalid_snapshot() {
        let (tx, rx, h) = spawn_sim();
        loaded(&rx);
        tx.send(Command::Apply("{".into())).unwrap();
        let e = wait_for(&rx, |e| match e {
            Event::Error(m) => Some(m),
            _ => None,
        });
        assert!(e.contains("JSON"), "{e}");
        tx.send(Command::Quit).unwrap();
        h.join().unwrap();
    }

    #[test]
    fn sim_refuses_raw_entries_the_board_does_not_hold() {
        let (tx, rx, h) = spawn_sim();
        let s = loaded(&rx);
        let mut snap = s.snapshot.clone().unwrap();
        snap.keymap.base.0.insert(66, aula_core::keymap::KeyAction::Raw([0x99, 0, 0, 0]));
        tx.send(Command::Apply(snap.to_json().unwrap())).unwrap();
        let e = wait_for(&rx, |e| match e {
            Event::Error(m) => Some(m),
            _ => None,
        });
        assert!(e.contains(aula_core::keymap::RAW_NOT_KEPT), "{e}");
        let mut snap = s.snapshot.unwrap();
        snap.advanced[0] = aula_core::advanced::AdvancedKey::Other { payload: "05".repeat(40) };
        tx.send(Command::Apply(snap.to_json().unwrap())).unwrap();
        let e = wait_for(&rx, |e| match e {
            Event::Error(m) => Some(m),
            _ => None,
        });
        assert!(e.contains(aula_core::keymap::RAW_NOT_KEPT), "{e}");
        tx.send(Command::Quit).unwrap();
        h.join().unwrap();
    }

    #[test]
    fn sim_live_stream_runs_and_closes() {
        let (tx, rx, h) = spawn_sim();
        loaded(&rx);
        tx.send(Command::StartLive).unwrap();
        wait_for(&rx, |e| matches!(e, Event::LiveRunning(true)).then_some(()));
        let frame = wait_for(&rx, |e| match e {
            Event::LiveFrame(j) => Some(j),
            _ => None,
        });
        assert!(frame.starts_with('{'));
        tx.send(Command::StopLive).unwrap();
        wait_for(&rx, |e| matches!(e, Event::LiveRunning(false)).then_some(()));
        tx.send(Command::Quit).unwrap();
        h.join().unwrap();
    }

    #[test]
    fn sim_calibration_reports_progress_and_finishes() {
        let (tx, rx, h) = spawn_sim();
        loaded(&rx);
        tx.send(Command::StartCalibration).unwrap();
        let keys = wait_for(&rx, |e| match e {
            Event::CalibrationKeys(k) => Some(k),
            _ => None,
        });
        assert!(!keys.is_empty());
        tx.send(Command::StopCalibration).unwrap();
        let r = wait_for(&rx, |e| match e {
            Event::CalibrationResult(r) => Some(r),
            _ => None,
        });
        assert_eq!(r, Some((true, 0)));
        tx.send(Command::Quit).unwrap();
        h.join().unwrap();
    }

    #[test]
    fn quit_while_streaming_closes_the_stream() {
        let (tx, rx, h) = spawn_sim();
        loaded(&rx);
        tx.send(Command::StartLive).unwrap();
        wait_for(&rx, |e| matches!(e, Event::LiveRunning(true)).then_some(()));
        tx.send(Command::Quit).unwrap();
        wait_for(&rx, |e| matches!(e, Event::LiveRunning(false)).then_some(()));
        h.join().unwrap();
    }

    #[test]
    fn same_settings_ignores_read_time() {
        let a: Snapshot = Snapshot::from_json(include_str!("../../aula-core/tests/fixtures/owner-2026-11-01.json")).unwrap();
        let mut b = a.clone();
        b.created_unix += 100;
        assert!(same_settings(&a, &b));
        b.poll_rate_khz = Some(1);
        assert!(!same_settings(&a, &b));
    }
}

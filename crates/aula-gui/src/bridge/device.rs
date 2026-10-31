//! `Device` QML singleton: keyboard state for the UI. All HID I/O runs on the worker thread in
//! `crate::worker`; this object only sends it commands and applies the events it queues back.

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    #[auto_cxx_name]
    extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qml_singleton]
        #[qproperty(bool, present, READ, NOTIFY)]
        #[qproperty(bool, connected, READ, NOTIFY)]
        #[qproperty(bool, supported, READ, NOTIFY)]
        #[qproperty(bool, sim, READ, CONSTANT)]
        #[qproperty(QString, node, READ, NOTIFY)]
        #[qproperty(QString, product_code, READ, NOTIFY)]
        #[qproperty(QString, firmware, READ, NOTIFY)]
        #[qproperty(QString, build_date, READ, NOTIFY)]
        #[qproperty(bool, loaded, READ, NOTIFY)]
        #[qproperty(bool, busy, READ, NOTIFY)]
        #[qproperty(QString, snapshot_json, READ, NOTIFY)]
        #[qproperty(QString, info_json, READ, NOTIFY)]
        #[qproperty(QString, profiles_json, READ, NOTIFY)]
        #[qproperty(QString, backups_json, READ, NOTIFY)]
        #[qproperty(bool, live, READ, NOTIFY)]
        #[qproperty(QString, live_json, READ, NOTIFY)]
        #[qproperty(bool, calibrating, READ, NOTIFY)]
        #[qproperty(QString, calibrated_json, READ, NOTIFY)]
        #[qproperty(QString, last_error, READ, NOTIFY)]
        type Device = super::DeviceRust;

        /// Opens the board if needed and reads everything again.
        #[qinvokable]
        fn refresh(&self);

        /// Writes a whole snapshot (JSON) with a page's edits: only the tables that differ, each read back.
        #[qinvokable]
        fn apply(&self, snapshot: &QString);

        /// What `apply` would change, as `{ changes: [...] }` or `{ error }`.
        #[qinvokable]
        fn preview(&self, snapshot: &QString) -> QString;

        #[qinvokable]
        fn set_music_rhythm(&self, on: bool);

        /// Empty when `name` can be used for a profile, else the reason.
        #[qinvokable]
        fn profile_name_problem(&self, name: &QString) -> QString;

        #[qinvokable]
        fn save_profile(&self, name: &QString, overwrite: bool);

        #[qinvokable]
        fn profile_preview(&self, name: &QString) -> QString;

        #[qinvokable]
        fn apply_profile(&self, name: &QString);

        #[qinvokable]
        fn delete_profile(&self, name: &QString);

        #[qinvokable]
        fn backup_now(&self);

        #[qinvokable]
        fn restore_preview(&self, path: &QString) -> QString;

        /// Restores a backup file. Only called after the confirmation dialog.
        #[qinvokable]
        fn restore(&self, path: &QString);

        #[qinvokable]
        fn start_live(&self);

        #[qinvokable]
        fn stop_live(&self);

        /// Clears depths, peaks and press counts of the live view.
        #[qinvokable]
        fn reset_live(&self);

        /// Starts any-key calibration. Only called after the confirmation dialog.
        #[qinvokable]
        fn start_calibration(&self);

        /// Ends calibration; the board stores the result.
        #[qinvokable]
        fn stop_calibration(&self);

        /// Resets every key's trigger settings. Only called after the confirmation dialog.
        #[qinvokable]
        fn reset_triggers(&self);

        /// Factory reset. Only called after the confirmation dialog.
        #[qinvokable]
        fn factory_reset(&self);

        /// Emitted for every error, also when the message repeats.
        #[qsignal]
        fn error_occurred(self: Pin<&mut Self>, message: QString);

        /// A message for the snackbar: i18n key and `%1` argument.
        #[qsignal]
        fn notice(self: Pin<&mut Self>, key: QString, arg: QString);

        /// `finished`, `failed` (with the board's code) or `unknown` (the board said nothing).
        #[qsignal]
        fn calibration_result(self: Pin<&mut Self>, outcome: QString, code: i32);

        /// SIGINT / SIGTERM arrived and any stream is closed: the window should quit.
        #[qsignal]
        fn quit_requested(self: Pin<&mut Self>);
    }

    impl cxx_qt::Threading for Device {}
    impl cxx_qt::Initialize for Device {}
}

use super::update;
use crate::config;
use crate::files::{self, Dirs};
use crate::worker::{self, Command, Event, Info, Settings, State};
use aula_core::layout;
use aula_core::lighting::ModeList;
use aula_core::perkey::key_name;
use aula_core::snapshot;
use aula_core::store::{self, Profiles};
use aula_core::trigger;
use aula_core::Snapshot;
use core::pin::Pin;
use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::QString;
use serde_json::{json, Value};
use std::path::Path;
use std::sync::mpsc::{self, Sender};

/// Whether this run uses the simulator (`--sim` or `AULA_GUI_SIM=1`), set by main before Qt starts.
pub static SIM: std::sync::OnceLock<bool> = std::sync::OnceLock::new();

pub struct DeviceRust {
    present: bool,
    connected: bool,
    supported: bool,
    sim: bool,
    node: QString,
    product_code: QString,
    firmware: QString,
    build_date: QString,
    loaded: bool,
    busy: bool,
    snapshot_json: QString,
    info_json: QString,
    profiles_json: QString,
    backups_json: QString,
    live: bool,
    live_json: QString,
    calibrating: bool,
    calibrated_json: QString,
    last_error: QString,
    /// The snapshot the pages show, for previews.
    current: Option<Snapshot>,
    commands: Option<Sender<Command>>,
}

impl Default for DeviceRust {
    fn default() -> Self {
        Self {
            present: false,
            connected: false,
            supported: false,
            sim: SIM.get().copied().unwrap_or(false),
            node: QString::default(),
            product_code: QString::default(),
            firmware: QString::default(),
            build_date: QString::default(),
            loaded: false,
            busy: false,
            snapshot_json: QString::default(),
            info_json: QString::default(),
            profiles_json: QString::from("[]"),
            backups_json: QString::from("[]"),
            live: false,
            live_json: QString::from("{}"),
            calibrating: false,
            calibrated_json: QString::from("[]"),
            last_error: QString::default(),
            current: None,
            commands: None,
        }
    }
}

fn modes_json(m: &ModeList) -> Value {
    json!({ "modes": m.modes, "selectable": m.selectable(), "maxSpeed": m.max_speed, "maxBrightness": m.max_brightness })
}

/// Board details for the pages; per-key lists use key codes like the snapshot.
pub fn info_json(i: &Info) -> Value {
    let features: Vec<Value> = i.features.iter().map(|(n, on)| json!({ "name": n, "on": on })).collect();
    let switches: Vec<Value> = i
        .switches
        .iter()
        .map(|id| json!({ "id": id, "name": trigger::switch_name(*id).map_or_else(|| format!("#{id}"), str::to_string) }))
        .collect();
    let fn_fixed: Vec<String> = i.fn_fixed.iter().map(|k| key_name(*k)).collect();
    json!({
        "features": features,
        "musicRhythm": i.music_rhythm,
        "musicRhythmOn": i.music_rhythm_on,
        "calibration": i.calibration,
        "modes": modes_json(&i.modes),
        "switches": switches,
        "fnFixed": fn_fixed,
        "socdHotkey": i.socd_hotkey.map(|h| format!("{:02x} {:02x}", h[0], h[1])),
    })
}

/// `{ changes: [...] }` from `old` to `new`, or `{ error }`.
pub fn preview_json(old: Option<&Snapshot>, new: Result<Snapshot, String>) -> Value {
    let Some(old) = old else { return json!({ "error": "the settings have not been read yet" }) };
    match new {
        Ok(new) => {
            let changes: Vec<String> = snapshot::diff(old, &new).iter().map(ToString::to_string).collect();
            json!({ "changes": changes, "tables": snapshot::changed_tables(&new, old) })
        }
        Err(e) => json!({ "error": e }),
    }
}

fn qs(v: &Value) -> QString {
    QString::from(&v.to_string())
}

impl qobject::Device {
    pub fn refresh(&self) {
        self.send(Command::Refresh);
    }

    pub fn apply(&self, snapshot: &QString) {
        self.send(Command::Apply(snapshot.to_string()));
    }

    pub fn preview(&self, snapshot: &QString) -> QString {
        let new = Snapshot::from_json(&snapshot.to_string()).map_err(String::from);
        qs(&preview_json(self.current.as_ref(), new))
    }

    pub fn set_music_rhythm(&self, on: bool) {
        self.send(Command::SetMusicRhythm(on));
    }

    pub fn profile_name_problem(&self, name: &QString) -> QString {
        QString::from(&store::validate_profile_name(&name.to_string()).err().map(String::from).unwrap_or_default())
    }

    pub fn save_profile(&self, name: &QString, overwrite: bool) {
        self.send(Command::SaveProfile { name: name.to_string(), overwrite });
    }

    pub fn profile_preview(&self, name: &QString) -> QString {
        let new = Dirs::for_run(self.sim)
            .and_then(|d| Profiles::in_dir(d.profiles).load(&name.to_string()).map_err(String::from));
        qs(&preview_json(self.current.as_ref(), new))
    }

    pub fn apply_profile(&self, name: &QString) {
        self.send(Command::ApplyProfile(name.to_string()));
    }

    pub fn delete_profile(&self, name: &QString) {
        self.send(Command::DeleteProfile(name.to_string()));
    }

    pub fn backup_now(&self) {
        self.send(Command::BackupNow);
    }

    pub fn restore_preview(&self, path: &QString) -> QString {
        let new = store::load_snapshot(Path::new(&path.to_string())).map_err(String::from);
        qs(&preview_json(self.current.as_ref(), new))
    }

    pub fn restore(&self, path: &QString) {
        self.send(Command::Restore(path.to_string().into()));
    }

    pub fn start_live(&self) {
        self.send(Command::StartLive);
    }

    pub fn stop_live(&self) {
        self.send(Command::StopLive);
    }

    pub fn reset_live(&self) {
        self.send(Command::ResetLive);
    }

    pub fn start_calibration(&self) {
        self.send(Command::StartCalibration);
    }

    pub fn stop_calibration(&self) {
        self.send(Command::StopCalibration);
    }

    pub fn reset_triggers(&self) {
        self.send(Command::ResetTriggers);
    }

    pub fn factory_reset(&self) {
        self.send(Command::FactoryReset);
    }

    fn send(&self, command: Command) {
        if let Some(tx) = &self.commands {
            // Fails only when the worker has exited, which it reports itself.
            let _ = tx.send(command);
        }
    }
}

fn apply_state(mut device: Pin<&mut qobject::Device>, s: State) {
    update!(device, present, present_changed, s.present);
    update!(device, connected, connected_changed, s.connected);
    update!(device, supported, supported_changed, s.supported);
    update!(device, node, node_changed, QString::from(&s.node));
    update!(device, product_code, product_code_changed, QString::from(&s.product_code));
    update!(device, firmware, firmware_changed, QString::from(&s.firmware));
    update!(device, build_date, build_date_changed, QString::from(&s.build_date));
    let info = s.info.as_ref().map(|i| info_json(i).to_string()).unwrap_or_default();
    update!(device, info_json, info_json_changed, QString::from(&info));
    let snap = s.snapshot.as_ref().and_then(|s| serde_json::to_string(s).ok()).unwrap_or_default();
    device.as_mut().rust_mut().current = s.snapshot;
    update!(device, snapshot_json, snapshot_json_changed, QString::from(&snap));
    let loaded = !snap.is_empty() && !info.is_empty();
    update!(device, loaded, loaded_changed, loaded);
}

fn apply(mut device: Pin<&mut qobject::Device>, event: Event) {
    match event {
        Event::State(s) => apply_state(device, *s),
        Event::Busy(busy) => update!(device, busy, busy_changed, busy),
        Event::Error(message) => {
            let message = QString::from(&message);
            update!(device, last_error, last_error_changed, message.clone());
            device.as_mut().error_occurred(message);
        }
        Event::Notice(key, arg) => device.as_mut().notice(QString::from(key), QString::from(&arg)),
        Event::Profiles(list) => update!(device, profiles_json, profiles_json_changed, qs(&json!(list))),
        Event::Backups(list) => update!(device, backups_json, backups_json_changed, qs(&files::backups_json(&list))),
        Event::LiveRunning(on) => {
            if !on {
                update!(device, live_json, live_json_changed, QString::from("{}"));
            }
            update!(device, live, live_changed, on);
        }
        Event::LiveFrame(j) => update!(device, live_json, live_json_changed, QString::from(&j)),
        Event::CalibrationRunning(on) => {
            if on {
                update!(device, calibrated_json, calibrated_json_changed, QString::from("[]"));
            }
            update!(device, calibrating, calibrating_changed, on);
        }
        Event::CalibrationKeys(keys) => {
            let codes: Vec<&str> = keys.iter().filter_map(|k| layout::by_index(*k)).map(|k| k.code).collect();
            update!(device, calibrated_json, calibrated_json_changed, qs(&json!(codes)));
        }
        Event::CalibrationResult(r) => {
            let (outcome, code) = match r {
                Some((true, _)) => ("finished", 0),
                Some((false, code)) => ("failed", i32::from(code)),
                None => ("unknown", 0),
            };
            device.as_mut().calibration_result(QString::from(outcome), code);
        }
        Event::QuitRequested => device.as_mut().quit_requested(),
    }
}

impl cxx_qt::Initialize for qobject::Device {
    fn initialize(mut self: Pin<&mut Self>) {
        let (tx, rx) = mpsc::channel();
        let qt = self.qt_thread();
        let cfg = config::get();
        let settings = Settings { sim: self.sim, poll: cfg.device_poll, live_publish: cfg.live_publish, dirs: None, sim_board: None };
        let spawned = worker::spawn(tx.clone(), move || {
            worker::run(rx, settings, |event| qt.queue(move |device| apply(device, event)).is_ok());
        });
        match spawned {
            Ok(()) => self.as_mut().rust_mut().commands = Some(tx),
            Err(e) => apply(self.as_mut(), Event::Error(format!("cannot start the device thread: {e}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Snapshot {
        Snapshot::from_json(include_str!("../../../aula-core/tests/fixtures/owner-2026-11-01.json")).unwrap()
    }

    #[test]
    fn preview_lists_changes() {
        let old = fixture();
        let mut new = old.clone();
        new.poll_rate_khz = Some(1);
        let v = preview_json(Some(&old), Ok(new));
        assert_eq!(v["tables"], json!(["poll rate"]));
        assert_eq!(v["changes"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn preview_without_changes_or_settings() {
        let old = fixture();
        let v = preview_json(Some(&old), Ok(old.clone()));
        assert_eq!(v["changes"], json!([]));
        assert!(preview_json(None, Ok(old)).get("error").is_some());
        assert_eq!(preview_json(Some(&fixture()), Err("bad file".into()))["error"], "bad file");
    }

    #[test]
    fn info_uses_key_codes_and_switch_names() {
        let i = Info {
            features: vec![("socd", true)],
            music_rhythm: true,
            music_rhythm_on: false,
            calibration: true,
            modes: ModeList { modes: vec![0, 2], max_speed: 4, max_brightness: 4 },
            switches: vec![1, 99],
            fn_fixed: vec![22, 46],
            socd_hotkey: Some([0xfa, 0xe5]),
        };
        let v = info_json(&i);
        assert_eq!(v["fnFixed"], json!(["Escape", "KeyW"]));
        assert_eq!(v["switches"][0]["name"], "HM1");
        assert_eq!(v["switches"][1]["name"], "#99");
        assert_eq!(v["modes"]["selectable"], json!([0, 2, 10]));
        assert_eq!(v["socdHotkey"], "fa e5");
        assert_eq!(v["features"][0]["on"], true);
    }
}

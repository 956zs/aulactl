//! `Sessions` QML singleton: recorded feed sessions (`aulactl feed --record`) and their replay reports. Files are
//! read and reports written on a short-lived thread; results come back through the Qt thread queue.

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
        #[qproperty(QString, sessions_json, READ, NOTIFY)]
        #[qproperty(bool, working, READ, NOTIFY)]
        type Sessions = super::SessionsRust;

        /// Reads the newest sessions again.
        #[qinvokable]
        fn refresh(self: Pin<&mut Self>);

        /// Aligns session `file` with its exported osu! replay, writes the HTML report and opens it.
        #[qinvokable]
        fn report(self: Pin<&mut Self>, file: &QString);

        /// A report finished: `ok` with the report path, or the reason it failed.
        #[qsignal]
        fn report_done(self: Pin<&mut Self>, ok: bool, message: QString);
    }

    impl cxx_qt::Threading for Sessions {}
}

use super::update;
use aula_core::{replay, session};
use core::pin::Pin;
use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::QString;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// Sessions listed.
const LISTED: usize = 10;

pub struct SessionsRust {
    sessions_json: QString,
    working: bool,
}

impl Default for SessionsRust {
    fn default() -> Self {
        Self { sessions_json: QString::from("[]"), working: false }
    }
}

/// The newest `n` sessions in `dir`, newest first: file, start (unix ms), label, duration (ms), presses, keys.
/// Files that do not read are listed with an `error`.
pub fn summaries(dir: &Path, n: usize) -> Vec<Value> {
    let mut files = session::list(dir).unwrap_or_default();
    files.reverse();
    files.truncate(n);
    files
        .iter()
        .map(|p| {
            let file = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            match session::read(p) {
                Ok(s) => json!({
                    "file": file,
                    "start": s.header.start_unix_ms,
                    "label": s.header.label,
                    "duration": s.end_ms.round(),
                    "presses": s.press_count(),
                    "keys": s.presses.iter().filter(|(_, v)| !v.is_empty()).map(|(k, _)| k.clone()).collect::<Vec<_>>(),
                }),
                Err(e) => json!({ "file": file, "error": e.to_string() }),
            }
        })
        .collect()
}

/// Writes the report of session `file` in `sessions_dir` (replay looked up in `exports`) and returns its path.
pub fn make_report(sessions_dir: &Path, file: &str, exports: &Path, reports: &Path) -> Result<PathBuf, String> {
    // only names listed in the sessions directory
    if file.contains('/') || file.starts_with('.') {
        return Err(format!("{file:?} is not a session file name"));
    }
    let path = session::resolve(sessions_dir, file)?;
    Ok(replay::make_report(&path, None, exports, reports)?.path)
}

fn dirs() -> Result<(PathBuf, PathBuf, PathBuf), String> {
    Ok((session::sessions_dir()?, replay::default_exports_dir()?, replay::reports_dir()?))
}

impl qobject::Sessions {
    pub fn refresh(self: Pin<&mut Self>) {
        let qt = self.qt_thread();
        let spawned = std::thread::Builder::new().name("sessions".into()).spawn(move || {
            let list = match session::sessions_dir() {
                Ok(dir) => summaries(&dir, LISTED),
                Err(_) => Vec::new(),
            };
            let text = QString::from(&Value::Array(list).to_string());
            let _ = qt.queue(move |mut obj| update!(obj, sessions_json, sessions_json_changed, text));
        });
        // without a thread the list just stays as it is
        drop(spawned);
    }

    pub fn report(mut self: Pin<&mut Self>, file: &QString) {
        if self.working {
            return;
        }
        update!(self, working, working_changed, true);
        let file = file.to_string();
        let qt = self.qt_thread();
        let spawned = std::thread::Builder::new().name("report".into()).spawn(move || {
            let r = dirs().and_then(|(s, e, r)| make_report(&s, &file, &e, &r)).and_then(|p| {
                aula_core::replay::open(&p).map_err(|e| e.to_string())?;
                Ok(p)
            });
            let _ = qt.queue(move |mut obj| {
                update!(obj, working, working_changed, false);
                match r {
                    Ok(p) => obj.as_mut().report_done(true, QString::from(&p.display().to_string())),
                    Err(e) => obj.as_mut().report_done(false, QString::from(&e)),
                }
            });
        });
        if let Err(e) = spawned {
            update!(self, working, working_changed, false);
            self.as_mut().report_done(false, QString::from(&format!("cannot start the report thread: {e}")));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aula_core::session::{Header, Writer, SCHEMA};

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("aula-gui-test-{}-sessions-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    fn header(label: &str, start: u64) -> Header {
        Header {
            schema: SCHEMA,
            label: label.into(),
            start_unix_ms: start,
            start_mono_ms: 0.0,
            product: "P".into(),
            firmware: "F".into(),
            max_mm: 3.4,
            keys: Vec::new(),
        }
    }

    #[test]
    fn lists_newest_first() {
        let d = scratch("list");
        assert!(summaries(&d, 10).is_empty(), "missing dir");
        for (i, label) in ["old", "new"].iter().enumerate() {
            let mut w = Writer::create(&d, &header(label, 1_793_491_200_000 + i as u64 * 60_000)).unwrap();
            w.line(r#"{"type":"press","t":12.0,"key":"KeyZ"}"#).unwrap();
            w.finish().unwrap();
        }
        std::fs::write(d.join("20261101-000500-bad.ndjson.gz"), "x").unwrap();
        let l = summaries(&d, 10);
        assert_eq!(l.len(), 3);
        assert!(l[0]["error"].is_string(), "{l:?}");
        assert_eq!(l[1]["label"], "new");
        assert_eq!(l[1]["presses"], 1);
        assert_eq!(l[1]["keys"], json!(["KeyZ"]));
        assert_eq!(l[2]["label"], "old");
        assert_eq!(summaries(&d, 1).len(), 1);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn report_needs_a_listed_name() {
        let d = scratch("report");
        let e = make_report(&d, "../x", &d, &d).unwrap_err();
        assert!(e.contains("not a session file name"), "{e}");
        assert!(make_report(&d, ".hidden", &d, &d).is_err());
        assert!(make_report(&d, "20261101-000000", &d, &d).unwrap_err().contains("no session"));
    }
}

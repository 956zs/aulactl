//! `KeyTables` QML singleton: constant tables (layout, actions, lighting modes, switches) and pure helpers the
//! pages call synchronously (selection, mm formatting, labels, recording). No device access.

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
        #[qproperty(QString, layout_json, READ, CONSTANT)]
        #[qproperty(QString, actions_json, READ, CONSTANT)]
        #[qproperty(QString, usages_json, READ, CONSTANT)]
        #[qproperty(QString, modes_json, READ, CONSTANT)]
        #[qproperty(QString, switches_json, READ, CONSTANT)]
        type KeyTables = super::KeyTablesRust;

        /// Key codes for a core selector (`all`, `wasd`, `letters`, `row:2`), as JSON; empty on error.
        #[qinvokable]
        fn select(&self, selector: &QString) -> QString;

        /// Key codes under the rectangle between two corners in u, as JSON.
        #[qinvokable]
        fn keys_in_rect(&self, x0: f64, y0: f64, x1: f64, y1: f64) -> QString;

        /// Selection (JSON codes) after a click on `code`: replace, or toggle with a modifier held.
        #[qinvokable]
        fn click(&self, selection: &QString, code: &QString, toggle: bool) -> QString;

        /// Selection (JSON codes) after a drag over `keys`: replace, or add with a modifier held.
        #[qinvokable]
        fn merge(&self, selection: &QString, keys: &QString, add: bool) -> QString;

        /// Raw trigger units as mm with two decimals.
        #[qinvokable]
        fn mm(&self, raw: f64, unit_mm: f64) -> QString;

        /// Nearest raw value for typed text ("1.5", "1,5 mm") within `min..=max`, or -1.
        #[qinvokable]
        fn raw_from_text(&self, text: &QString, unit_mm: f64, min: i32, max: i32) -> i32;

        /// Key-cap text for an action.
        #[qinvokable]
        fn label(&self, action: &QString) -> QString;

        /// `{ name }` with the canonical action text, or `{ error }`.
        #[qinvokable]
        fn parse_action(&self, text: &QString) -> QString;

        /// Base-layer factory action of a key.
        #[qinvokable]
        fn base_default(&self, code: &QString) -> QString;

        /// Core keycode name of a `KeyEvent.nativeScanCode`, or empty.
        #[qinvokable]
        fn usage_from_scan_code(&self, scan_code: i32) -> QString;

        /// Layout code of the key sending this scan code with the factory keymap, or empty.
        #[qinvokable]
        fn code_from_scan_code(&self, scan_code: i32) -> QString;

        /// Macro events from recorded `[{ name, down, t }]`: `{ events, dropped }`.
        #[qinvokable]
        fn macro_from_recording(&self, recording: &QString, last_delay_ms: i32) -> QString;

        /// Empty when a macro (core JSON) can be written, else the reason.
        #[qinvokable]
        fn macro_problem(&self, macro_json: &QString) -> QString;
    }
}

use crate::actions;
use crate::evdev;
use crate::format;
use crate::geometry;
use crate::macro_edit;
use aula_core::layout;
use aula_core::lighting::{Colors, MODES};
use aula_core::trigger::SWITCHES;
use cxx_qt_lib::QString;
use serde_json::{json, Value};

pub struct KeyTablesRust {
    layout_json: QString,
    actions_json: QString,
    usages_json: QString,
    modes_json: QString,
    switches_json: QString,
}

/// `[{ id, name, speed, colors, directions: [{ id, name }] }]`.
pub fn modes() -> Value {
    Value::Array(
        MODES
            .iter()
            .map(|m| {
                let colors = match m.colors {
                    Colors::None => "none",
                    Colors::Fg => "fg",
                    Colors::FgBg => "fg-bg",
                };
                let dirs: Vec<Value> = m.directions.iter().map(|(id, n)| json!({ "id": id, "name": n })).collect();
                json!({ "id": m.id, "name": m.name, "speed": m.speed, "colors": colors, "directions": dirs })
            })
            .collect(),
    )
}

fn switches() -> Value {
    Value::Array(SWITCHES.iter().map(|(id, name, cap)| json!({ "id": id, "name": name, "travel": cap })).collect())
}

impl Default for KeyTablesRust {
    fn default() -> Self {
        let s = |v: Value| QString::from(&v.to_string());
        Self {
            layout_json: s(geometry::layout_json()),
            actions_json: s(actions::catalogue()),
            usages_json: s(actions::usages()),
            modes_json: s(modes()),
            switches_json: s(switches()),
        }
    }
}

fn codes(indices: &[usize]) -> Vec<&'static str> {
    indices.iter().filter_map(|i| layout::by_index(*i)).map(|k| k.code).collect()
}

/// Indices of the codes in a JSON list; unknown codes are dropped.
pub fn indices(json: &str) -> Vec<usize> {
    serde_json::from_str::<Vec<String>>(json)
        .unwrap_or_default()
        .iter()
        .filter_map(|c| layout::by_code(c))
        .map(|k| k.index as usize)
        .collect()
}

fn qjson<T: serde::Serialize + ?Sized>(v: &T) -> QString {
    QString::from(&serde_json::to_string(v).unwrap_or_default())
}

impl qobject::KeyTables {
    pub fn select(&self, selector: &QString) -> QString {
        match geometry::select(&selector.to_string()) {
            Ok(v) => qjson(&codes(&v)),
            Err(_) => QString::default(),
        }
    }

    pub fn keys_in_rect(&self, x0: f64, y0: f64, x1: f64, y1: f64) -> QString {
        qjson(&codes(&geometry::keys_in_rect(x0, y0, x1, y1)))
    }

    pub fn click(&self, selection: &QString, code: &QString, toggle: bool) -> QString {
        let sel = indices(&selection.to_string());
        let Some(k) = layout::by_code(&code.to_string()) else { return qjson(&codes(&sel)) };
        qjson(&codes(&geometry::click(&sel, k.index as usize, toggle)))
    }

    pub fn merge(&self, selection: &QString, keys: &QString, add: bool) -> QString {
        let merged = geometry::merge(&indices(&selection.to_string()), &indices(&keys.to_string()), add);
        qjson(&codes(&merged))
    }

    pub fn mm(&self, raw: f64, unit_mm: f64) -> QString {
        QString::from(&format::mm(raw, unit_mm))
    }

    pub fn raw_from_text(&self, text: &QString, unit_mm: f64, min: i32, max: i32) -> i32 {
        let (Ok(min), Ok(max)) = (u16::try_from(min), u16::try_from(max)) else { return -1 };
        format::parse_mm(&text.to_string())
            .and_then(|mm| format::raw_from_mm(mm, unit_mm, min, max))
            .map_or(-1, i32::from)
    }

    pub fn label(&self, action: &QString) -> QString {
        QString::from(&actions::short_label(&action.to_string()))
    }

    pub fn parse_action(&self, text: &QString) -> QString {
        let v = match actions::parse(&text.to_string()) {
            Ok(name) => json!({ "name": name }),
            Err(e) => json!({ "error": e }),
        };
        QString::from(&v.to_string())
    }

    pub fn base_default(&self, code: &QString) -> QString {
        QString::from(&actions::base_default(&code.to_string()).unwrap_or_default())
    }

    pub fn usage_from_scan_code(&self, scan_code: i32) -> QString {
        QString::from(evdev::usage_name(scan_code).unwrap_or_default())
    }

    pub fn code_from_scan_code(&self, scan_code: i32) -> QString {
        QString::from(evdev::layout_code(scan_code).unwrap_or_default())
    }

    pub fn macro_from_recording(&self, recording: &QString, last_delay_ms: i32) -> QString {
        let last = u16::try_from(last_delay_ms.max(0)).unwrap_or(u16::MAX);
        let v = match macro_edit::parse_recording(&recording.to_string()) {
            Ok(rec) => macro_edit::conversion_json(&macro_edit::from_recording(&rec, last)),
            Err(e) => json!({ "error": e }),
        };
        QString::from(&v.to_string())
    }

    pub fn macro_problem(&self, macro_json: &QString) -> QString {
        QString::from(&macro_edit::problem(&macro_json.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes_table() {
        let m = modes();
        let list = m.as_array().unwrap();
        assert_eq!(list.len(), MODES.len());
        let wave = list.iter().find(|m| m["name"] == "wave").unwrap();
        assert_eq!(wave["colors"], "fg-bg");
        assert_eq!(wave["directions"].as_array().unwrap().len(), 6);
        assert_eq!(list.iter().find(|m| m["id"] == 10).unwrap()["colors"], "none");
    }

    #[test]
    fn code_lists() {
        assert_eq!(indices(r#"["KeyQ", "Nope", "Escape"]"#), vec![45, 22]);
        assert_eq!(indices("not json"), Vec::<usize>::new());
        assert_eq!(indices("[]"), Vec::<usize>::new());
        assert_eq!(codes(&[22, 0, 46]), vec!["Escape", "KeyW"]);
    }

    #[test]
    fn switch_table() {
        let s = switches();
        assert_eq!(s[0], json!({ "id": 1, "name": "HM1", "travel": 34 }));
    }
}

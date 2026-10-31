//! Key actions for the keymap picker and the key caps: the grouped catalogue (core keycode, media and named
//! tables) and short labels that fit on a key cap. Names are the core text form (`KeyA`, `VolumeUp`, `Macro3`).

use aula_core::keycodes::{self, is_modifier, MEDIA, USAGES};
use aula_core::keymap::{KeyAction, ADVANCED_SLOTS, MACRO_SLOTS, NAMED};
use aula_core::layout;
use serde_json::{json, Value};

/// Picker groups in display order; the QML translates `group.<id>`.
pub const GROUPS: &[&str] = &[
    "special", "letters", "digits", "modifiers", "editing", "symbols", "function", "numpad", "international", "media",
    "lighting", "mouse", "shortcuts", "macros", "advanced",
];

fn usage_group(usage: u8, name: &str) -> &'static str {
    match usage {
        0x04..=0x1d => "letters",
        0x1e..=0x27 => "digits",
        0xe0..=0xe7 => "modifiers",
        0x3a..=0x45 | 0x68..=0x73 => "function",
        0x53..=0x63 => "numpad",
        0x64 | 0x87..=0x8b => "international",
        0x2d..=0x38 => "symbols",
        _ if name.starts_with("Numpad") => "numpad",
        _ => "editing",
    }
}

fn named_group(entry: [u8; 4]) -> &'static str {
    match entry[0] {
        0x12 => "mouse",
        0x13 => "shortcuts",
        _ => "lighting",
    }
}

/// `[{ id, items: [{ name, label }] }]`, groups in `GROUPS` order.
pub fn catalogue() -> Value {
    let mut groups: Vec<(&str, Vec<Value>)> = GROUPS.iter().map(|g| (*g, Vec::new())).collect();
    let mut push = |group: &str, name: String| {
        let label = short_label(&name);
        if let Some((_, items)) = groups.iter_mut().find(|(g, _)| *g == group) {
            items.push(json!({ "name": name, "label": label }));
        }
    };
    push("special", "Default".into());
    push("special", "Fn".into());
    for (u, n) in USAGES {
        push(usage_group(*u, n), (*n).to_string());
    }
    for (_, n) in MEDIA {
        push("media", (*n).to_string());
    }
    for (e, n) in NAMED {
        push(named_group(*e), (*n).to_string());
    }
    for s in 0..MACRO_SLOTS {
        push("macros", format!("Macro{s}"));
    }
    for s in 0..ADVANCED_SLOTS {
        push("advanced", format!("Adv{s}"));
    }
    Value::Array(groups.into_iter().map(|(id, items)| json!({ "id": id, "items": items })).collect())
}

/// Key-cap text for a keyboard usage name.
fn usage_label(usage: u8) -> String {
    let fixed = match usage {
        0x28 => "Enter",
        0x29 => "Esc",
        0x2a => "Bksp",
        0x2b => "Tab",
        0x2c => "Space",
        0x2d => "-",
        0x2e => "=",
        0x2f => "[",
        0x30 => "]",
        0x31 => "\\",
        0x33 => ";",
        0x34 => "'",
        0x35 => "`",
        0x36 => ",",
        0x37 => ".",
        0x38 => "/",
        0x39 => "Caps",
        0x46 => "PrtSc",
        0x47 => "ScrLk",
        0x48 => "Pause",
        0x49 => "Ins",
        0x4a => "Home",
        0x4b => "PgUp",
        0x4c => "Del",
        0x4d => "End",
        0x4e => "PgDn",
        0x4f => "→",
        0x50 => "←",
        0x51 => "↓",
        0x52 => "↑",
        0x53 => "NumLk",
        0x54 => "Num/",
        0x55 => "Num*",
        0x56 => "Num-",
        0x57 => "Num+",
        0x58 => "NumEnt",
        0x63 => "Num.",
        0x64 => "ISO\\",
        0x65 => "Menu",
        0xe0 => "LCtrl",
        0xe1 => "LShift",
        0xe2 => "LAlt",
        0xe3 => "LWin",
        0xe4 => "RCtrl",
        0xe5 => "RShift",
        0xe6 => "RAlt",
        0xe7 => "RWin",
        _ => "",
    };
    if !fixed.is_empty() {
        return fixed.into();
    }
    let name = keycodes::name_of_usage(usage).unwrap_or("?");
    if let Some(rest) = name.strip_prefix("Key").or_else(|| name.strip_prefix("Digit")) {
        return rest.into();
    }
    if let Some(rest) = name.strip_prefix("Numpad") {
        return format!("Num{rest}");
    }
    name.into()
}

/// Modifier text inside a combination ("Ctrl+C").
fn combo_part(usage: u8) -> String {
    match usage {
        0xe0 | 0xe4 => "Ctrl".into(),
        0xe1 | 0xe5 => "Shift".into(),
        0xe2 | 0xe6 => "Alt".into(),
        0xe3 | 0xe7 => "Win".into(),
        u => usage_label(u),
    }
}

fn media_label(name: &str) -> &str {
    match name {
        "VolumeUp" => "Vol+",
        "VolumeDown" => "Vol-",
        "Mute" => "Mute",
        "PlayPause" => "Play",
        "NextTrack" => "Next",
        "PrevTrack" => "Prev",
        "MediaStop" => "Stop",
        "Mail" => "Mail",
        "BrowserSearch" => "Search",
        "BrowserFavorites" => "Fav",
        "BrowserHome" => "WWW",
        "BrowserForward" => "Fwd",
        "BrowserBack" => "Back",
        "BrowserStop" => "WStop",
        "BrowserRefresh" => "Reload",
        "MediaPlayer" => "Player",
        "Calculator" => "Calc",
        "MyComputer" => "PC",
        other => other,
    }
}

fn named_label(name: &str) -> &str {
    match name {
        "LightNextMode" => "LED+",
        "BrightnessUp" => "Bri+",
        "BrightnessDown" => "Bri-",
        "LightSpeedUp" => "Spd+",
        "LightSpeedDown" => "Spd-",
        "LightNextDirection" => "Dir",
        "LightFullColor" => "RGB",
        "LightNextColor" => "Color",
        "LightNextBgColor" => "BgCol",
        "LockKeyboard" => "Lock",
        "WinLock" => "WinLk",
        "LightsOff" => "LEDoff",
        "MouseLeft" => "LMB",
        "MouseRight" => "RMB",
        "MouseMiddle" => "MMB",
        "MouseForward" => "MB5",
        "MouseBack" => "MB4",
        "WheelUp" => "Whl↑",
        "WheelDown" => "Whl↓",
        "ShowDesktop" => "Desk",
        "CloseWindow" => "Close",
        "LockPc" => "LockPC",
        other => other,
    }
}

/// Short key-cap text for an action in text form; empty for `Default`, the text itself when it does not
/// parse.
pub fn short_label(text: &str) -> String {
    let Ok(action) = text.parse::<KeyAction>() else { return text.to_string() };
    match &action {
        KeyAction::Default => String::new(),
        KeyAction::Key(u) => usage_label(*u),
        KeyAction::Fn => "Fn".into(),
        KeyAction::Named(_) => named_label(&action.to_string()).to_string(),
        KeyAction::Media(_) => media_label(&action.to_string()).to_string(),
        KeyAction::Combo(keys) => keys.iter().map(|u| combo_part(*u)).collect::<Vec<_>>().join("+"),
        KeyAction::Macro(s) => format!("M{s}"),
        KeyAction::Advanced(s) => format!("A{s}"),
        KeyAction::Raw(_) => "raw".into(),
    }
}

/// The canonical text of an action typed by the user (`ctrl+c` -> `Copy`), or an error message.
pub fn parse(text: &str) -> Result<String, String> {
    let t = text.trim();
    if t.is_empty() {
        return Err("empty action".into());
    }
    t.parse::<KeyAction>().map(|a| a.to_string()).map_err(String::from)
}

/// Base-layer action of a key with the factory keymap (`Escape`, `Fn`); None for unknown codes.
pub fn base_default(code: &str) -> Option<String> {
    let k = layout::by_code(code)?;
    Some(KeyAction::from_entry(k.default_entry, Some(k.index as usize)).to_string())
}

/// Keyboard usages only (SOCD keys, macro steps), as `[{ name, label }]`.
pub fn usages() -> Value {
    Value::Array(USAGES.iter().map(|(u, n)| json!({ "name": n, "label": usage_label(*u), "modifier": is_modifier(*u) })).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn group<'a>(cat: &'a Value, id: &str) -> &'a Vec<Value> {
        cat.as_array().unwrap().iter().find(|g| g["id"] == id).unwrap()["items"].as_array().unwrap()
    }

    #[test]
    fn catalogue_covers_every_table_once() {
        let cat = catalogue();
        let names: Vec<&str> =
            cat.as_array().unwrap().iter().flat_map(|g| g["items"].as_array().unwrap()).map(|i| i["name"].as_str().unwrap()).collect();
        let expected = 2 + USAGES.len() + MEDIA.len() + NAMED.len() + MACRO_SLOTS as usize + ADVANCED_SLOTS as usize;
        assert_eq!(names.len(), expected);
        let mut unique = names.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), names.len());
        // Every name parses back to itself.
        for n in names {
            assert_eq!(parse(n).as_deref(), Ok(n), "{n}");
        }
    }

    #[test]
    fn groups_hold_what_they_say() {
        let cat = catalogue();
        assert_eq!(group(&cat, "letters").len(), 26);
        assert_eq!(group(&cat, "digits").len(), 10);
        assert_eq!(group(&cat, "modifiers").len(), 8);
        assert_eq!(group(&cat, "macros").len(), 10);
        assert_eq!(group(&cat, "advanced").len(), 40);
        assert!(group(&cat, "mouse").iter().all(|i| i["name"].as_str().unwrap().contains("Mouse") || i["name"].as_str().unwrap().starts_with("Wheel")));
        assert!(group(&cat, "shortcuts").iter().any(|i| i["name"] == "Copy"));
        assert!(group(&cat, "numpad").iter().any(|i| i["name"] == "NumpadEnter"));
        assert!(group(&cat, "editing").iter().any(|i| i["name"] == "ArrowUp"));
        assert_eq!(cat.as_array().unwrap().len(), GROUPS.len());
    }

    #[test]
    fn labels() {
        assert_eq!(short_label("KeyA"), "A");
        assert_eq!(short_label("Digit0"), "0");
        assert_eq!(short_label("Escape"), "Esc");
        assert_eq!(short_label("F12"), "F12");
        assert_eq!(short_label("Numpad7"), "Num7");
        assert_eq!(short_label("ArrowLeft"), "←");
        assert_eq!(short_label("VolumeUp"), "Vol+");
        assert_eq!(short_label("MouseBack"), "MB4");
        assert_eq!(short_label("Macro3"), "M3");
        assert_eq!(short_label("Adv12"), "A12");
        assert_eq!(short_label("Fn"), "Fn");
        assert_eq!(short_label("Copy"), "Copy");
        assert_eq!(short_label("ControlLeft+ShiftLeft+Escape"), "Ctrl+Shift+Esc");
        assert_eq!(short_label("raw:10040300"), "raw");
    }

    #[test]
    fn label_edge_cases() {
        assert_eq!(short_label("Default"), "");
        assert_eq!(short_label(""), "");
        assert_eq!(short_label("nonsense"), "nonsense");
    }

    #[test]
    fn parse_canonicalises_or_explains() {
        assert_eq!(parse("ctrl+c").as_deref(), Ok("Copy"));
        assert_eq!(parse(" esc ").as_deref(), Ok("Escape"));
        assert_eq!(parse("ControlLeft+ShiftLeft+Escape").as_deref(), Ok("ControlLeft+ShiftLeft+Escape"));
        assert!(parse("").is_err());
        assert!(parse("   ").is_err());
        assert!(parse("Macro10").is_err());
        assert!(parse("Ctrl+Shift").is_err());
        assert!(parse("blah").unwrap_err().contains("blah"));
    }

    #[test]
    fn base_defaults() {
        assert_eq!(base_default("Escape").as_deref(), Some("Escape"));
        assert_eq!(base_default("KeyFn").as_deref(), Some("Fn"));
        assert_eq!(base_default("ShiftLeft").as_deref(), Some("ShiftLeft"));
        assert_eq!(base_default("Nope"), None);
        assert_eq!(base_default(""), None);
    }

    #[test]
    fn usage_list() {
        let u = usages();
        let list = u.as_array().unwrap();
        assert_eq!(list.len(), USAGES.len());
        assert!(list.iter().any(|i| i["name"] == "ShiftLeft" && i["modifier"] == true));
        assert!(list.iter().any(|i| i["name"] == "KeyA" && i["modifier"] == false && i["label"] == "A"));
    }
}

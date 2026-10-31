//! Commands end to end against the in-memory keyboard (`aula-core` feature `sim`).

use crate::{dispatch, Cli, Outcome};
use aula_core::advanced::AdvancedKey;
use aula_core::keymap::{KeyAction, Layer};
use aula_core::lighting::Rgb;
use aula_core::macros::PlayMode;
use aula_core::sim::SimKeyboard;
use aula_core::socd::SocdMode;
use aula_core::trigger::TriggerMode;
use aula_core::Keyboard;
use std::path::PathBuf;

fn kb() -> Keyboard<SimKeyboard> {
    let mut k = Keyboard::with_link(SimKeyboard::owner_board()).unwrap();
    k.set_auto_backup(None);
    k
}

/// A fresh directory for files a test writes.
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let d =
            std::env::temp_dir().join(format!("aulactl-cli-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        TempDir(d)
    }

    fn file(&self, name: &str) -> String {
        self.0.join(name).to_string_lossy().into_owned()
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn run_in(
    kb: &mut Keyboard<SimKeyboard>,
    profiles: Option<&TempDir>,
    args: &[&str],
) -> Result<(Outcome, String), String> {
    let mut out = Vec::new();
    let mut c = Cli::new(kb, &mut out);
    c.profiles = profiles.map(|p| p.0.clone());
    let r = dispatch(&mut c, args)?;
    Ok((r, String::from_utf8(out).unwrap()))
}

fn run(kb: &mut Keyboard<SimKeyboard>, args: &[&str]) -> Result<String, String> {
    run_in(kb, None, args).map(|(_, out)| out)
}

/// Frames sent since `from` that change the board (everything except reads).
fn writes_since(kb: &Keyboard<SimKeyboard>, from: usize) -> usize {
    kb.link().sent[from..]
        .iter()
        .filter(|b| match b[0] {
            0x07 => b[1] == 0,
            0x09 => b[1] <= 1,
            0x14 | 0x17 => true,
            0x18 | 0x19 | 0x22 => b[1] & 0x80 == 0,
            0x24 => b[1] == 0 || b[1] == 3,
            0x25 | 0x26 => b[1] == 1,
            0x21 => matches!((b[4], b[5]), (0x18, 0 | 6 | 12 | 13 | 0x0f | 0x10) | (0, 9)),
            _ => false,
        })
        .count()
}

#[test]
fn read_commands_send_no_writes() {
    let mut k = kb();
    let start = k.link().sent.len();
    for args in [
        &["info"][..],
        &["actuation", "show"],
        &["actuation", "show", "wasd"],
        &["deadband", "show"],
        &["switch", "show"],
        &["switch", "list"],
        &["pollrate"],
        &["keymap", "show"],
        &["keymap", "show", "--layer", "fn", "--all"],
        &["macro", "list"],
        &["adv", "list"],
        &["socd"],
        &["winlock"],
        &["light"],
        &["light", "modes"],
        &["light", "custom"],
        &["light", "custom", "1"],
        &["light", "slot"],
        &["dump"],
    ] {
        run(&mut k, args).unwrap_or_else(|e| panic!("{args:?}: {e}"));
    }
    assert_eq!(writes_since(&k, start), 0);
}

#[test]
fn info_output() {
    let out = run(&mut kb(), &["info"]).unwrap();
    assert!(
        out.starts_with(
            "product:    SI2825KZHEARGB\nfirmware:   3.17.07 (built Apr 15 2026,11:20:35)\n"
        ),
        "{out}"
    );
    assert!(
        out.contains("travel:     0.08..3.40 mm in 0.01 mm steps\n"),
        "{out}"
    );
    assert!(
        out.contains("poll rate:  8 kHz\n") && out.contains("rollover:   NKRO\n"),
        "{out}"
    );
    assert!(
        out.contains("features:   socd, any-key calibration, per-key rgb"),
        "{out}"
    );
}

#[test]
fn unsupported_board_is_only_identified() {
    let mut k = Keyboard::with_link(SimKeyboard::other_board()).unwrap();
    let out = run(&mut k, &["info"]).unwrap();
    assert!(
        out.contains("SI2828HEARGB") && out.contains("not a WIN 60 HE"),
        "{out}"
    );
    assert!(run(&mut k, &["actuation", "set", "wasd", "--point", "1"])
        .unwrap_err()
        .contains("unsupported board"));
    assert!(k.link().sent.iter().all(|b| b[0] == 0x0d));
}

#[test]
fn actuation_set_and_show() {
    let mut k = kb();
    let out = run(&mut k, &["actuation", "set", "wasd", "--point", "1.2"]).unwrap();
    assert_eq!(
        out,
        "set rt from 1.20 mm, sensitivity 0.50 mm: KeyW KeyA KeyS KeyD\n"
    );
    assert_eq!(k.link().triggers[46], (12, 120, 50, 50));
    assert_eq!(k.link().triggers[45], (12, 100, 50, 50), "KeyQ untouched");

    run(&mut k, &["actuation", "set", "W", "--rt", "0.3/0.45"]).unwrap();
    assert_eq!(k.link().triggers[46], (13, 120, 30, 45));
    run(&mut k, &["actuation", "set", "W", "--mode", "fixed"]).unwrap();
    let t = k.read_trigger(46).unwrap();
    assert_eq!((t.mode, t.actuation), (TriggerMode::Fixed, 120));

    let start = k.link().sent.len();
    assert_eq!(
        run(&mut k, &["actuation", "set", "W", "--mode", "fixed"]).unwrap(),
        "nothing to change: the keys already have these settings\n"
    );
    assert_eq!(writes_since(&k, start), 0);

    let view = run(&mut k, &["actuation", "show"]).unwrap();
    assert!(
        view.contains("|1.20 |") && view.contains("|1.20r|1.20r|1.20r|"),
        "{view}"
    );
    let table = run(&mut k, &["actuation", "show", "W,Q"]).unwrap();
    assert_eq!(table, "key   legend  mode   actuation  press  release\nKeyQ  Q       rt     1.00       0.50   0.50\nKeyW  W       fixed  1.20       -      -\n");
}

#[test]
fn actuation_errors_write_nothing() {
    let mut k = kb();
    let start = k.link().sent.len();
    for (args, msg) in [
        (&["actuation", "set", "wasd"][..], "give --point"),
        (&["actuation", "set", "wasd", "--point", "5"], "outside"),
        (&["actuation", "set", "wasd", "--point", "0.01"], "outside"),
        (
            &["actuation", "set", "nokey", "--point", "1"],
            "unknown key",
        ),
        (
            &[
                "actuation",
                "set",
                "wasd",
                "--rt",
                "0.3/0.5",
                "--mode",
                "rt",
            ],
            "rt-split",
        ),
        (
            &["actuation", "set", "wasd", "--point", "x"],
            "not a length",
        ),
        (
            &["actuation", "set", "wasd", "--bogus", "1"],
            "unknown option",
        ),
        (&["actuation"], "unknown arguments"),
    ] {
        let e = run(&mut k, args).unwrap_err();
        assert!(e.contains(msg), "{args:?}: {e}");
    }
    assert_eq!(writes_since(&k, start), 0);
}

#[test]
fn dead_band_and_switches() {
    let mut k = kb();
    let out = run(
        &mut k,
        &["deadband", "set", "wasd", "--top", "0.1", "--bottom", "0.2"],
    )
    .unwrap();
    assert_eq!(
        out,
        "set top 0.10 mm, bottom 0.20 mm: KeyW KeyA KeyS KeyD\n"
    );
    assert_eq!(&k.link().dead_band[46 * 2..46 * 2 + 2], &[10, 20]);
    assert!(run(&mut k, &["deadband", "set", "W", "--top", "1.5"])
        .unwrap_err()
        .contains("outside 0..1.00 mm"));
    assert!(run(&mut k, &["deadband", "show"])
        .unwrap()
        .contains("top 0.10 mm, bottom 0.20 mm: KeyW KeyA KeyS KeyD"));

    let out = run(&mut k, &["switch", "set", "W", "cy1"]).unwrap();
    assert!(
        out.starts_with("set CY1 (id 3): KeyW\n") && out.contains("recalibrate"),
        "{out}"
    );
    assert_eq!(k.link().switches[46], 3);
    assert!(run(&mut k, &["switch", "set", "W", "7"])
        .unwrap_err()
        .contains("not offered"));
    assert_eq!(
        run(&mut k, &["switch", "list"]).unwrap(),
        "HM1 (id 1)\nHH1 (id 2)\nCY1 (id 3)\nTC1 (TTC Kom) (id 5)\n"
    );
}

#[test]
fn poll_rate_change_asks_for_a_restart() {
    let mut k = kb();
    let (outcome, out) = run_in(&mut k, None, &["pollrate", "4"]).unwrap();
    assert_eq!(outcome, Outcome::Restarting(Some(4)));
    assert!(out.contains("4 kHz; the keyboard restarts"));
    assert_eq!(k.link().poll_khz, 4);
    let mut k = kb();
    assert_eq!(
        run_in(&mut k, None, &["pollrate", "8000"]).unwrap().0,
        Outcome::Done
    );
    assert!(run(&mut k, &["pollrate", "3"]).is_err());
}

#[test]
fn keymap_set_and_reset() {
    let mut k = kb();
    let out = run(&mut k, &["keymap", "set", "Caps", "Esc"]).unwrap();
    assert_eq!(out, "base layer CapsLock: Escape\n");
    assert_eq!(
        k.read_keymap(Layer::Base).unwrap().get(66),
        Some(&KeyAction::Key(0x29))
    );
    let show = run(&mut k, &["keymap", "show"]).unwrap();
    assert!(
        show.contains("CapsLock  Caps    Escape") && show.contains("1 of 61 keys remapped"),
        "{show}"
    );
    run(&mut k, &["keymap", "set", "KeyQ", "Ctrl+Shift+Esc"]).unwrap();
    run(&mut k, &["keymap", "reset", "Caps"]).unwrap();
    let base = k.read_keymap(Layer::Base).unwrap();
    assert_eq!(base.get(66), Some(&KeyAction::Key(0x39)));
    assert_eq!(
        base.get(45).unwrap().to_string(),
        "ControlLeft+ShiftLeft+Escape"
    );

    run(
        &mut k,
        &["keymap", "set", "KeyE", "VolumeUp", "--layer", "fn"],
    )
    .unwrap();
    assert_eq!(
        k.read_keymap(Layer::Fn).unwrap().get(47),
        Some(&KeyAction::Media(0x10))
    );
    // KeyW is Fn-fixed on this board
    assert!(run(&mut k, &["keymap", "set", "W", "Mute", "--layer", "fn"]).is_err());
    assert!(run(&mut k, &["keymap", "set", "Caps", "Nope"])
        .unwrap_err()
        .contains("unknown key action"));
    assert!(run(&mut k, &["keymap", "set", "Caps", "Esc", "--layer", "fn2"]).is_err());
}

#[test]
fn macros_with_binding() {
    let mut k = kb();
    let out = run(
        &mut k,
        &[
            "macro",
            "set",
            "2",
            "tap a, wait 30, tap b",
            "--repeat",
            "3",
            "--bind",
            "F",
        ],
    )
    .unwrap();
    assert!(
        out.starts_with("macro slot 2: repeat 3x, 4 events: down KeyA, wait 10, up KeyA, wait 30"),
        "{out}"
    );
    assert!(out.ends_with("base layer KeyF: Macro2\n"), "{out}");
    let m = k.read_macro(2).unwrap().unwrap();
    assert_eq!((m.mode, m.repeat, m.events.len()), (PlayMode::Repeat, 3, 4));
    assert_eq!(m.events[1].delay_ms, 30);
    let list = run(&mut k, &["macro", "list"]).unwrap();
    assert!(
        list.lines().nth(2).unwrap().ends_with("[on KeyF]"),
        "{list}"
    );
    assert!(list.starts_with("0: empty\n"));
    let show = run(&mut k, &["macro", "show", "2"]).unwrap();
    assert!(show.contains("aulactl macro set 2 \"down KeyA, wait 10, up KeyA, wait 30, down KeyB, wait 10, up KeyB, wait 10\" --mode repeat --repeat 3 --delay 0"), "{show}");

    let out = run(&mut k, &["macro", "clear", "2"]).unwrap();
    assert!(out.contains("still bound to Macro2: KeyF"), "{out}");
    assert_eq!(k.read_macro(2).unwrap(), None);
    for (args, msg) in [
        (&["macro", "set", "10", "tap a"][..], "macro slot (0..9)"),
        (&["macro", "set", "1", "wait 5"], "cannot start with a wait"),
        (
            &["macro", "set", "1", "tap a", "--mode", "repeat"],
            "needs --repeat",
        ),
        (
            &[
                "macro", "set", "1", "tap a", "--mode", "toggle", "--repeat", "2",
            ],
            "only goes with",
        ),
        (
            &["macro", "set", "1", "tap a", "--repeat", "0"],
            "repeat count",
        ),
    ] {
        let e = run(&mut k, args).unwrap_err();
        assert!(e.contains(msg), "{args:?}: {e}");
    }
}

#[test]
fn advanced_keys() {
    let mut k = kb();
    let out = run(
        &mut k,
        &[
            "adv",
            "set",
            "0",
            "mt",
            "LCtrl",
            "Esc",
            "--threshold",
            "200",
            "--bind",
            "Caps",
        ],
    )
    .unwrap();
    assert!(
        out.starts_with("advanced-key slot 0: mt hold ControlLeft, tap Escape, after 200 ms\n"),
        "{out}"
    );
    assert_eq!(
        k.read_keymap(Layer::Base).unwrap().get(66),
        Some(&KeyAction::Advanced(0))
    );
    run(
        &mut k,
        &["adv", "set", "1", "dks", "KeyA:p--h", "Space:t---"],
    )
    .unwrap();
    run(&mut k, &["adv", "set", "2", "tgl", "LShift"]).unwrap();
    run(
        &mut k,
        &["adv", "set", "3", "dks", "KeyB:t---", "--point", "1.0"],
    )
    .unwrap();
    let adv = k.read_advanced().unwrap();
    assert!(matches!(&adv[0], AdvancedKey::ModTap { threshold: 20, .. }));
    match &adv[1] {
        AdvancedKey::Dks { routes, keys } => {
            assert_eq!(*routes, [15, 30, 30, 15]);
            assert_eq!(keys.len(), 2);
        }
        other => panic!("{other:?}"),
    }
    assert!(matches!(
        &adv[3],
        AdvancedKey::Dks {
            routes: [10, 30, 30, 10],
            ..
        }
    ));
    let list = run(&mut k, &["adv", "list"]).unwrap();
    assert!(
        list.contains("0: mt hold ControlLeft, tap Escape, after 200 ms  [on CapsLock]"),
        "{list}"
    );
    assert!(
        list.contains("1: dks at 1.5/3.0/3.0/1.5 mm: KeyA:p--h Space:t---  [not bound]"),
        "{list}"
    );
    assert!(list.ends_with("4 of 40 advanced-key slots used\n"));
    run(&mut k, &["adv", "clear", "1"]).unwrap();
    assert_eq!(k.read_advanced().unwrap()[1], AdvancedKey::Empty);
    for (args, msg) in [
        (&["adv", "set", "40", "tgl", "a"][..], "advanced-key slot"),
        (&["adv", "set", "0", "dks"], "1 to 4 output keys"),
        (
            &[
                "adv", "set", "0", "dks", "a:t---", "b:t---", "c:t---", "d:t---", "e:t---",
            ],
            "1 to 4 output keys",
        ),
        (
            &["adv", "set", "0", "mt", "a", "b", "--threshold", "100"],
            "hold threshold",
        ),
        (
            &["adv", "set", "0", "tgl", "Macro1"],
            "cannot be used inside",
        ),
        (&["adv", "set", "0", "spin", "a"], "unknown arguments"),
    ] {
        let e = run(&mut k, args).unwrap_err();
        assert!(e.contains(msg), "{args:?}: {e}");
    }
}

#[test]
fn socd_pairs_and_power() {
    let mut k = kb();
    let out = run(&mut k, &["socd", "add", "a", "d", "--mode", "neutral"]).unwrap();
    assert_eq!(
        out,
        "SOCD is off\n#  key1  key2  mode\n1  KeyA  KeyD  neutral\n"
    );
    run(&mut k, &["socd", "add", "w", "s"]).unwrap();
    let pairs = k.read_socd_pairs().unwrap();
    assert_eq!(pairs.len(), 2);
    assert_eq!(pairs[1].mode, SocdMode::LastWins);
    assert!(run(&mut k, &["socd", "add", "a", "a"]).is_err());
    run(&mut k, &["socd", "remove", "d", "a"]).unwrap();
    assert_eq!(k.read_socd_pairs().unwrap().len(), 1);
    assert!(run(&mut k, &["socd", "remove", "5"])
        .unwrap_err()
        .contains("not a pair number"));
    run(&mut k, &["socd", "remove", "1"]).unwrap();
    assert!(k.read_socd_pairs().unwrap().is_empty());
    assert_eq!(run(&mut k, &["socd", "on"]).unwrap(), "SOCD is on\n");
    assert!(k.link().socd_on);
}

#[test]
fn win_lock_options() {
    let mut k = kb();
    assert_eq!(run(&mut k, &["winlock"]).unwrap(), "win-lock blocks: win\n");
    assert_eq!(
        run(&mut k, &["winlock", "win,alt-tab"]).unwrap(),
        "win-lock blocks: win, alt-tab\n"
    );
    assert_eq!(k.link().caps[20], 0x09);
    assert!(run(&mut k, &["winlock", "win", "alt-tab"])
        .unwrap()
        .starts_with("nothing to change"));
    assert!(run(&mut k, &["winlock", "ctrl"]).is_err());
}

#[test]
fn lighting() {
    let mut k = kb();
    let out = run(
        &mut k,
        &[
            "light",
            "effect",
            "wave",
            "--direction",
            "left",
            "--speed",
            "3",
            "--color",
            "#00ff00",
        ],
    )
    .unwrap();
    assert_eq!(
        out,
        "lighting: wave, brightness 1, speed 3, colour #00ff00 on #000000, direction left\n"
    );
    let e = k.read_effect().unwrap();
    assert_eq!(
        (e.mode, e.direction, e.speed, e.fg),
        (2, 1, 3, Rgb(0, 255, 0))
    );
    run(&mut k, &["light", "brightness", "4"]).unwrap();
    run(&mut k, &["light", "rainbow", "on"]).unwrap();
    run(&mut k, &["light", "off"]).unwrap();
    let e = k.read_effect().unwrap();
    assert_eq!((e.brightness, e.full_color, e.on), (4, true, false));
    assert!(run(&mut k, &["light", "brightness", "9"])
        .unwrap_err()
        .contains("brightness (0..4)"));
    assert!(run(&mut k, &["light", "brightness", "2", "--speed", "1"])
        .unwrap_err()
        .contains("light effect MODE"));

    let out = run(
        &mut k,
        &["light", "custom", "set", "wasd", "#00ff00", "--slot", "1"],
    )
    .unwrap();
    assert!(
        out.starts_with("custom slot 1: #00ff00 on KeyW KeyA KeyS KeyD\n")
            && out.contains("custom slot 0 is still shown")
            && out.contains("light effect custom"),
        "{out}"
    );
    assert_eq!(
        k.read_custom_colors(1).unwrap().get(46),
        Some(&Rgb(0, 255, 0))
    );
    assert_eq!(
        run(&mut k, &["light", "slot"]).unwrap(),
        "custom slot 0\n",
        "editing the other slot does not switch the shown one"
    );
    assert_eq!(
        k.read_custom_colors(0).unwrap().get(46),
        Some(&Rgb(0xff, 0, 0))
    );
    assert_eq!(
        run(&mut k, &["light", "slot", "1"]).unwrap(),
        "custom slot 1 selected\n"
    );
    let out = run(&mut k, &["light", "custom", "set", "W", "#0000ff"]).unwrap();
    assert!(!out.contains("still shown"), "{out}");
    assert_eq!(k.read_active_custom_slot().unwrap(), 1);
    assert_eq!(
        run(&mut k, &["light", "slot", "0"]).unwrap(),
        "custom slot 0 selected\n"
    );
    assert_eq!(k.read_active_custom_slot().unwrap(), 0);
    let shown = run(&mut k, &["light", "custom"]).unwrap();
    assert_eq!(
        shown,
        "custom slot 0:\n#ff0000: KeyW KeyA KeyS KeyD\n57 other keys are black (off)\n"
    );
    run(&mut k, &["light", "music", "on"]).unwrap();
    assert!(k.link().music);
    assert!(run(&mut k, &["light", "custom", "show", "2"]).is_err());
    assert!(run(&mut k, &["light", "--slot", "1"])
        .unwrap_err()
        .contains("--slot goes with"));
}

#[test]
fn backup_diff_restore() {
    let dir = TempDir::new("restore");
    let mut k = kb();
    let file = dir.file("before.json");
    let out = run(&mut k, &["backup", &file]).unwrap();
    assert!(
        out.contains("SI2825KZHEARGB firmware 3.17.07")
            && out.contains("trigger: rt from 1.00 mm, sensitivity 0.50 mm: all keys"),
        "{out}"
    );
    assert!(run(&mut k, &["backup", &file])
        .unwrap_err()
        .contains("already exists"));
    assert!(run(&mut k, &["diff", &file])
        .unwrap()
        .starts_with("the keyboard matches"));

    run(&mut k, &["actuation", "set", "W", "--point", "2"]).unwrap();
    run(&mut k, &["keymap", "set", "Caps", "Esc"]).unwrap();
    let diff = run(&mut k, &["diff", &file]).unwrap();
    assert!(
        diff.contains("KeyW") && diff.contains("CapsLock") && diff.ends_with("2 differences\n"),
        "{diff}"
    );

    let start = k.link().sent.len();
    let preview = run(&mut k, &["restore", &file]).unwrap();
    assert!(
        preview.contains("nothing was written") && preview.contains("--yes"),
        "{preview}"
    );
    assert_eq!(writes_since(&k, start), 0);

    let out = run(&mut k, &["restore", &file, "--yes"]).unwrap();
    assert!(
        out.contains("written and read back: base layer, trigger"),
        "{out}"
    );
    assert_eq!(k.link().triggers[46], (12, 100, 50, 50));
    assert!(run(&mut k, &["restore", &file, "--yes"])
        .unwrap()
        .contains("nothing to change"));

    let dump = dir.file("dump.json");
    run(&mut k, &["dump", &dump]).unwrap();
    assert!(aula_core::store::load_snapshot(std::path::Path::new(&dump)).is_ok());
    let json = run(&mut k, &["dump"]).unwrap();
    assert!(json.contains("\"format\""));
    assert!(run(&mut k, &["restore", &dir.file("missing.json")]).is_err());
}

#[test]
fn profiles() {
    let dir = TempDir::new("profiles");
    let mut k = kb();
    let p =
        |k: &mut Keyboard<SimKeyboard>, args: &[&str]| run_in(k, Some(&dir), args).map(|(_, o)| o);
    assert!(p(&mut k, &["profile", "list"])
        .unwrap()
        .starts_with("no profiles yet"));
    assert!(p(&mut k, &["profile", "save", "game"])
        .unwrap()
        .starts_with("saved profile game"));
    assert!(p(&mut k, &["profile", "save", "game"])
        .unwrap_err()
        .contains("--force"));
    p(&mut k, &["profile", "save", "game", "--force"]).unwrap();
    assert!(p(&mut k, &["profile", "save", "../x"]).is_err());
    assert_eq!(p(&mut k, &["profile", "list"]).unwrap(), "game\n");
    assert!(p(&mut k, &["profile", "show", "game"])
        .unwrap()
        .contains("macros: 0 of 10 slots used"));

    run(&mut k, &["light", "brightness", "3"]).unwrap();
    assert!(p(&mut k, &["profile", "diff", "game"])
        .unwrap()
        .starts_with("lighting:"));
    assert!(p(&mut k, &["profile", "apply", "game"])
        .unwrap()
        .contains("nothing was written"));
    assert_eq!(k.read_effect().unwrap().brightness, 3);
    p(&mut k, &["profile", "apply", "game", "--yes"]).unwrap();
    assert_eq!(k.read_effect().unwrap().brightness, 1);

    assert!(p(&mut k, &["profile", "delete", "game"])
        .unwrap_err()
        .contains("--yes"));
    p(&mut k, &["profile", "delete", "game", "--yes"]).unwrap();
    assert!(p(&mut k, &["profile", "show", "game"]).is_err());
}

#[test]
fn destructive_commands_need_yes() {
    let mut k = kb();
    let start = k.link().sent.len();
    assert!(run(&mut k, &["reset", "triggers"])
        .unwrap_err()
        .contains("--yes"));
    assert!(run(&mut k, &["reset", "factory"])
        .unwrap_err()
        .contains("--yes"));
    assert!(run(&mut k, &["calibrate"]).unwrap_err().contains("--yes"));
    assert!(run(&mut k, &["reset"]).is_err());
    assert_eq!(k.link().sent.len(), start, "nothing at all was sent");

    run(&mut k, &["actuation", "set", "W", "--point", "2"]).unwrap();
    assert!(run(&mut k, &["reset", "triggers", "--yes"])
        .unwrap()
        .contains("back to factory"));
    assert_eq!(k.link().triggers[46], (0, 100, 1, 1));
    run(&mut k, &["keymap", "set", "Caps", "Esc"]).unwrap();
    assert!(run(&mut k, &["reset", "factory", "--yes"])
        .unwrap()
        .contains("back to factory settings"));
    assert_eq!(
        k.read_keymap(Layer::Base).unwrap().get(66),
        Some(&KeyAction::Key(0x39))
    );
}

/// Sim link that answers the calibration start with `event`, and drops every other `0x21` reply (so stop()
/// gets no final status).
struct EarlyStatus {
    sim: SimKeyboard,
    event: aula_core::frame::Body,
}

impl aula_core::hidraw::Link for EarlyStatus {
    fn send(&mut self, b: &aula_core::frame::Body) -> aula_core::Result<()> {
        self.sim.send(b)?;
        if b[0] == 0x21 && b[4] == 0x18 && b[5] == 0x0f {
            self.sim.push_input(self.event);
        }
        Ok(())
    }
    fn recv(&mut self, t: std::time::Duration) -> aula_core::Result<Option<aula_core::frame::Body>> {
        self.sim.recv(t)
    }
    fn drain(&mut self) -> aula_core::Result<()> {
        self.sim.drain()
    }
}

fn hex_body(s: &str) -> aula_core::frame::Body {
    let mut b = [0u8; aula_core::frame::BODY_LEN];
    for (i, h) in s.split_whitespace().enumerate() {
        b[i] = u8::from_str_radix(h, 16).unwrap();
    }
    b
}

fn calibrate_with_early(event: &str) -> Result<String, String> {
    let mut sim = SimKeyboard::owner_board();
    sim.mute_cmd = Some(0x21);
    let mut k = Keyboard::with_link(EarlyStatus {
        sim,
        event: hex_body(event),
    })
    .unwrap();
    k.set_auto_backup(None);
    let mut out = Vec::new();
    let mut c = Cli::new(&mut k, &mut out);
    dispatch(&mut c, &["calibrate", "--yes"])?;
    assert!(!k.link().sim.calibrating, "stop was sent");
    Ok(String::from_utf8(out).unwrap())
}

#[test]
fn calibration_keeps_an_early_final_status() {
    let out = calibrate_with_early("21 00 00 00 02 0f 00").unwrap();
    assert!(out.ends_with("calibration finished\n"), "{out}");
    let e = calibrate_with_early("21 00 00 00 02 0f 04").unwrap_err();
    assert!(e.contains("failure (code 4)"), "{e}");
}

#[test]
fn bind_targets_are_checked_before_the_slot_is_written() {
    let mut k = kb();
    let start = k.link().sent.len();
    // KeyW is Fn-fixed
    assert!(run(&mut k, &["macro", "set", "3", "tap a", "--bind", "W", "--layer", "fn"]).is_err());
    assert!(run(
        &mut k,
        &["adv", "set", "4", "tgl", "ShiftLeft", "--bind", "W", "--layer", "fn"]
    )
    .is_err());
    assert_eq!(writes_since(&k, start), 0, "nothing written");
    assert_eq!(k.read_macro(3).unwrap(), None);
    assert_eq!(k.read_advanced().unwrap()[4], AdvancedKey::Empty);
    // a valid fn-layer bind still works
    let out = run(&mut k, &["macro", "set", "3", "tap a", "--bind", "E", "--layer", "fn"]).unwrap();
    assert!(out.ends_with("fn layer KeyE: Macro3\n"), "{out}");
}

#[test]
fn travel_opens_and_closes_the_stream() {
    let mut k = kb();
    let out = run(&mut k, &["travel", "wasd", "--raw", "--seconds", "1"]).unwrap();
    assert_eq!(out, "");
    let sent = &k.link().sent;
    assert!(sent.iter().any(|b| b[0] == 0x21 && b[5] == 0x02), "opened");
    assert_eq!(
        sent.last().map(|b| (b[0], b[5])),
        Some((0x21, 0x03)),
        "closed last"
    );
    assert!(!k.link().travel_open);
    assert!(run(&mut k, &["travel", "--seconds", "0"]).is_err());
}

#[test]
fn output_files_are_never_clobbered_without_force() {
    use std::os::unix::fs::PermissionsExt;
    let dir = TempDir::new("noclobber");
    let mut k = kb();
    let file = dir.file("dump.json");
    std::fs::write(&file, "keep").unwrap();
    for cmd in ["dump", "backup"] {
        let e = run(&mut k, &[cmd, &file]).unwrap_err();
        assert!(e.contains("already exists") && e.contains("--force"), "{e}");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "keep");
    }
    run(&mut k, &["dump", &file, "--force"]).unwrap();
    assert!(aula_core::store::load_snapshot(std::path::Path::new(&file)).is_ok());
    let mode = std::fs::metadata(&file).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
    let other = dir.file("b.json");
    run(&mut k, &["backup", &other]).unwrap();
    run(&mut k, &["backup", &other, "--force"]).unwrap();
    assert!(run(&mut k, &["dump", &file, &other]).is_err());
}

#[test]
fn raw_entries_cannot_be_typed_or_restored() {
    let dir = TempDir::new("raw");
    let mut k = kb();
    let start = k.link().sent.len();
    for args in [
        &["keymap", "set", "Caps", "raw:10040300"][..],
        &["keymap", "set", "Caps", "raw:00000000", "--layer", "fn"],
        &["adv", "set", "1", "tgl", "raw:00040000"],
        &["adv", "set", "1", "mt", "raw:00040000", "KeyA"],
        &["adv", "set", "1", "dks", "raw:00040000:t---"],
    ] {
        let e = run(&mut k, args).unwrap_err();
        assert!(e.contains(aula_core::keymap::RAW_NOT_KEPT), "{args:?}: {e}");
    }
    assert_eq!(writes_since(&k, start), 0);

    // a crafted restore file with a raw entry and a raw advanced slot
    let mut snap = k.read_snapshot().unwrap();
    snap.keymap.base.0.insert(66, KeyAction::Raw([0x99, 0, 0, 0]));
    let file = dir.file("crafted.json");
    aula_core::store::save_snapshot(std::path::Path::new(&file), &snap).unwrap();
    let e = run(&mut k, &["restore", &file, "--yes"]).unwrap_err();
    assert!(e.contains(aula_core::keymap::RAW_NOT_KEPT), "{e}");
    let mut snap = k.read_snapshot().unwrap();
    snap.advanced[4] = AdvancedKey::Other { payload: "07".repeat(40) };
    aula_core::store::save_snapshot(std::path::Path::new(&file), &snap).unwrap();
    assert!(run(&mut k, &["restore", &file, "--yes"]).is_err());
    assert_eq!(writes_since(&k, start), 0, "nothing was written");
}

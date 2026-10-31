//! `Keyboard` against the in-memory board: guards, read-back verification, snapshot apply / diff.

use crate::advanced::AdvancedKey;
use crate::frame::{self, body};
use crate::info::WinLock;
use crate::keymap::{KeyAction, Layer};
use crate::lighting::Rgb;
use crate::macros::simple_sequence;
use crate::perkey::PerKey;
use crate::sim::SimKeyboard;
use crate::snapshot::{changed_tables, diff, Snapshot};
use crate::socd::{SocdMode, SocdPair};
use crate::trigger::{DeadBand, KeyTrigger, TriggerMode};
use crate::travel::{CalibrationEvent, Confirm};
use crate::{keycodes::Usage, layout, store, Error, Keyboard};
use std::ops::ControlFlow;
use std::time::Duration;

fn kb() -> Keyboard<SimKeyboard> {
    let mut k = Keyboard::with_link(SimKeyboard::owner_board()).unwrap();
    k.set_auto_backup(None);
    k
}

fn writes(kb: &Keyboard<SimKeyboard>) -> Vec<frame::Body> {
    kb.link().sent.iter().filter(|b| is_write(b)).copied().collect()
}

/// Frames that change state (everything but reads and travel open/close).
fn is_write(b: &frame::Body) -> bool {
    match b[0] {
        0x07 => b[1] == 0,
        0x09 => b[1] <= 1,
        0x14 | 0x17 => true,
        0x18 | 0x19 | 0x22 => b[1] & 0x80 == 0,
        0x24 => b[1] == 0 || b[1] == 3,
        0x25 | 0x26 => b[1] == 1,
        0x21 => matches!((b[4], b[5]), (0x18, 0 | 6 | 12 | 13 | 0x0f | 0x10) | (0, 9)),
        _ => false,
    }
}

#[test]
fn identifies_the_owner_board() {
    let k = kb();
    assert!(k.is_supported());
    assert_eq!(k.identity().product_code, "SI2825KZHEARGB");
    assert!(k.capabilities().unwrap().socd);
    assert_eq!(&k.link().sent[0][..2], &[0x0d, 0x00]);
    assert_eq!(&k.link().sent[1][..2], &[0x01, 0x00]);
}

#[test]
fn other_boards_only_get_identification() {
    let mut k = Keyboard::with_link(SimKeyboard::other_board()).unwrap();
    assert!(!k.is_supported());
    assert_eq!(k.link().sent.len(), 1, "not even 0x01 after a foreign product code");
    assert!(matches!(k.capabilities(), Err(Error::UnsupportedProduct { .. })));
    assert!(matches!(k.read_effect(), Err(Error::UnsupportedProduct { .. })));
    assert!(matches!(k.read_keymap(Layer::Base), Err(Error::UnsupportedProduct { .. })));
    assert!(matches!(k.set_win_lock(WinLock::default()), Err(Error::UnsupportedProduct { .. })));
    assert!(matches!(k.travel_stream(&[46]).map(|_| ()), Err(Error::UnsupportedProduct { .. })));
    assert!(k.read_snapshot().is_err());
    assert_eq!(k.link().sent.len(), 1, "nothing else was sent");
}

#[test]
fn reads_match_the_probe() {
    let mut k = kb();
    let e = k.read_effect().unwrap();
    assert_eq!((e.mode, e.brightness, e.fg), (0, 1, Rgb(0xff, 0, 0xff)));
    assert_eq!(k.read_mode_list().unwrap().modes.len(), 14);
    assert!(k.read_side_mode_list().unwrap().modes.is_empty());
    assert_eq!(k.read_active_custom_slot().unwrap(), 0);
    let c = k.read_custom_colors(0).unwrap();
    assert_eq!(c.iter().filter(|(_, c)| **c != Rgb::BLACK).count(), 4);
    assert_eq!(k.read_keymap(Layer::Base).unwrap().get(122), Some(&KeyAction::Fn));
    assert_eq!(k.read_fn_fixed().unwrap().len(), 35, "physical keys only");
    assert!(k.read_macros().unwrap().iter().all(Option::is_none));
    assert!(k.read_advanced().unwrap().iter().all(|a| *a == AdvancedKey::Empty));
    let l = k.travel_limits().unwrap();
    assert_eq!((l.min, l.max), (8, 340));
    assert_eq!(k.read_trigger(22).unwrap(), KeyTrigger { mode: TriggerMode::Rapid, actuation: 100, press: 50, release: 50 });
    assert!(k.read_trigger(0).is_err());
    assert_eq!(k.read_poll_rate().unwrap(), 8);
    assert_eq!(k.read_switch_list().unwrap(), vec![1, 2, 3, 5]);
    assert!(k.read_switch_types().unwrap().iter().all(|(_, s)| *s == 1));
    assert!(k.read_dead_band().unwrap().iter().all(|(_, d)| *d == DeadBand::default()));
    assert!(!k.read_socd_power().unwrap().enabled);
    assert!(k.read_socd_pairs().unwrap().is_empty());
    assert_eq!(k.read_win_lock().unwrap(), WinLock { win: true, ..Default::default() });
    assert!(writes(&k).is_empty(), "reads send no writes");
}

#[test]
fn snapshot_of_a_fresh_board_and_json_round_trip() {
    let mut k = kb();
    let s = k.read_snapshot().unwrap();
    assert!(writes(&k).is_empty());
    assert_eq!(s.trigger.len(), 61);
    assert_eq!(s.keymap.base.get(66), Some(&KeyAction::Key(0x39)));
    assert_eq!(s.poll_rate_khz, Some(8));
    let json = s.to_json().unwrap();
    let back = Snapshot::from_json(&json).unwrap();
    assert_eq!(back, s);
    assert!(diff(&s, &back).is_empty());
    assert!(changed_tables(&back, &s).is_empty());
}

#[test]
fn snapshot_json_errors() {
    let s = kb().read_snapshot().unwrap();
    let json = s.to_json().unwrap();
    assert!(Snapshot::from_json("").is_err());
    assert!(Snapshot::from_json("{}").is_err());
    assert!(Snapshot::from_json(&json.replace("\"schema\": 1", "\"schema\": 2")).is_err());
    assert!(Snapshot::from_json(&json.replace("aulactl snapshot", "other")).is_err());
    assert!(Snapshot::from_json(&json.replace("SI2825KZHEARGB", "SI2828HEARGB")).is_err());
    assert!(Snapshot::from_json(&json.replace("\"win_lock\"", "\"extra\": 1, \"win_lock\"")).is_err());
    assert!(Snapshot::from_json(&json.replace("\"CapsLock\": \"CapsLock\"", "\"CapsLock\": \"Bogus\"")).is_err());
    let mut short = s.clone();
    short.macros.pop();
    assert!(Snapshot::from_json(&short.to_json().unwrap()).is_err());
}

#[test]
fn applying_an_unchanged_snapshot_writes_nothing() {
    let mut k = kb();
    let s = k.read_snapshot().unwrap();
    let r = k.apply_snapshot(&s, &s).unwrap();
    assert!(r.written.is_empty() && !r.reconnect_needed);
    assert!(writes(&k).is_empty());
}

#[test]
fn apply_writes_only_changed_tables_and_verifies() {
    let mut k = kb();
    let old = k.read_snapshot().unwrap();
    let mut new = old.clone();
    new.keymap.base.0.insert(66, KeyAction::Key(0x29)); // Caps -> Esc
    new.keymap.base.0.insert(68, KeyAction::Macro(0));
    new.macros[0] = Some(simple_sequence(0, &[Usage(4), Usage(5)], 20));
    new.trigger.0.insert(46, KeyTrigger { mode: TriggerMode::RapidSplit, actuation: 40, press: 10, release: 20 });
    new.dead_band.as_mut().unwrap().0.insert(46, DeadBand { top: 5, bottom: 10 });
    new.socd = Some(crate::snapshot::Socd {
        enabled: true,
        pairs: vec![SocdPair { mode: SocdMode::LastWins, key1: Usage(4), key2: Usage(7) }],
    });
    new.win_lock.alt_tab = true;
    new.advanced[3] = AdvancedKey::Toggle { action: KeyAction::Key(0xe1) };
    let tables = changed_tables(&new, &old);
    assert_eq!(
        tables,
        vec!["macro slot 0", "advanced keys", "base layer", "trigger", "dead band", "SOCD pairs", "SOCD power", "win-lock"]
    );
    let changes = diff(&old, &new);
    assert!(changes.iter().any(|c| c.to_string() == "base layer: CapsLock: CapsLock -> Escape"), "{changes:?}");
    assert!(changes.iter().any(|c| c.to_string().starts_with("trigger: KeyW: rapid actuation 100")));
    let report = k.apply_snapshot(&new, &old).unwrap();
    assert_eq!(report.written, tables);
    let after = k.read_snapshot().unwrap();
    assert!(diff(&after, &new).is_empty(), "{:?}", diff(&after, &new));
    // only the changed trigger key was written, in one frame
    let trig: Vec<_> = writes(&k).into_iter().filter(|b| b[0] == 0x21).collect();
    assert_eq!(trig.len(), 1);
    assert_eq!(frame::decode_mask(&trig[0][6..28]), vec![46]);
    // only the changed advanced slot was written
    assert_eq!(writes(&k).iter().filter(|b| b[0] == 0x22).count(), 1);
}

#[test]
fn read_back_mismatch_is_reported() {
    let mut k = kb();
    k.link_mut().corrupt_next_write = Some(0x18);
    let mut m = k.read_keymap(Layer::Base).unwrap();
    m.0.insert(66, KeyAction::Key(0x29));
    assert!(matches!(k.write_keymap(Layer::Base, &m), Err(Error::Verify(_))));
    k.link_mut().corrupt_next_write = Some(0x22);
    assert!(matches!(
        k.write_advanced(&[(0, AdvancedKey::Toggle { action: KeyAction::Key(4) })]),
        Err(Error::Verify(_))
    ));
}

#[test]
fn missing_replies_time_out() {
    let mut k = kb();
    k.link_mut().mute_cmd = Some(0x18);
    assert!(matches!(k.read_keymap(Layer::Base), Err(Error::Timeout(_))));
}

#[test]
fn fn_fixed_keys_cannot_be_remapped() {
    let mut k = kb();
    let mut m = PerKey::new();
    m.0.insert(46, KeyAction::Key(0x52)); // W is Fn-fixed
    assert!(matches!(k.write_keymap(Layer::Fn, &m), Err(Error::Invalid(_))));
    m.0.clear();
    m.0.insert(45, KeyAction::Key(0x52)); // Q is free
    k.write_keymap(Layer::Fn, &m).unwrap();
    assert_eq!(k.read_keymap(Layer::Fn).unwrap().get(45), Some(&KeyAction::Key(0x52)));
}

#[test]
fn invalid_values_are_refused_before_any_write() {
    let mut k = kb();
    let mut t = PerKey::new();
    t.0.insert(46, KeyTrigger { mode: TriggerMode::Fixed, actuation: 341, press: 1, release: 1 });
    assert!(k.write_triggers(&t).is_err());
    let mut e = k.read_effect().unwrap();
    e.brightness = 9;
    assert!(k.set_effect(&e).is_err());
    assert!(k.write_custom_colors(2, &PerKey::new()).is_err());
    assert!(k.write_macro(10, None).is_err());
    assert!(k.set_poll_rate(3).is_err());
    assert!(writes(&k).is_empty());
}

#[test]
fn lighting_writes() {
    let mut k = kb();
    let mut e = k.read_effect().unwrap();
    e.mode = 2;
    e.direction = 1;
    k.set_effect(&e).unwrap();
    let w = writes(&k);
    assert_eq!(w[0], body("17 00 00 00 02 00 00"), "music off first");
    assert_eq!(&w[1][..6], &[0x07, 0, 0, 0, 0x0e, 2]);
    let mut c = PerKey::new();
    c.0.insert(22, Rgb(1, 2, 3));
    k.write_custom_colors(1, &c).unwrap();
    assert_eq!(k.read_custom_colors(1).unwrap().get(22), Some(&Rgb(1, 2, 3)));
    assert_eq!(writes(&k).iter().filter(|b| b[0] == 0x09).count(), 8);
}

#[test]
fn switch_types_and_socd_and_win_lock() {
    let mut k = kb();
    let mut s = k.read_switch_types().unwrap();
    s.0.insert(46, 5);
    k.write_switch_types(&s).unwrap();
    assert_eq!(k.read_switch_types().unwrap().get(46), Some(&5));
    s.0.insert(46, 4);
    assert!(k.write_switch_types(&s).is_err(), "4 is not offered");
    k.set_socd_enabled(true).unwrap();
    assert!(k.read_socd_power().unwrap().enabled);
    let w = WinLock { win: false, shift_tab: true, alt_f4: true, alt_tab: false };
    k.set_win_lock(w).unwrap();
    assert_eq!(k.read_win_lock().unwrap(), w);
}

#[test]
fn poll_rate_change_kills_the_handle() {
    let mut k = kb();
    k.set_poll_rate(1).unwrap();
    assert_eq!(k.link().poll_khz, 1);
    assert!(k.read_effect().is_err());
}

#[test]
fn travel_stream_is_closed_on_drop() {
    let mut k = kb();
    {
        let mut s = k.travel_stream(&[46, 110]).unwrap();
        assert!(s.kb_link_open());
        s.push(body("21 00 00 00 05 01 05 00 6e"));
        s.push(body("21 00 00 00 01 00")); // stray ACK is not a sample
        s.push(body("21 00 00 00 03 01 02 02 00 00 ff ff"));
        let mut got = Vec::new();
        s.run(Duration::from_millis(1), |x| match x {
            Some(x) => {
                got.push(x);
                ControlFlow::Continue(())
            }
            None => ControlFlow::Break(()),
        })
        .unwrap();
        assert_eq!(got.len(), 2);
        assert_eq!((got[0].index, got[0].depth), (110, 110));
        assert!(got[1].at_rest);
    }
    assert!(!k.link().travel_open);
    assert_eq!(k.link().sent.last().unwrap(), &body("21 00 00 00 18 03"));
    // explicit close
    let s = k.travel_stream(&layout::all_indices()).unwrap();
    s.close().unwrap();
    assert!(!k.link().travel_open);
}

#[test]
fn calibration_needs_confirmation_and_is_stopped() {
    let mut k = kb();
    {
        let mut c = k.calibrate(Confirm::user_confirmed()).unwrap();
        let mut p = body("21 00 00 00 18 0f 01");
        p[7..29].copy_from_slice(&frame::encode_mask(&[22]).unwrap());
        c.push(p);
        assert_eq!(c.next_event(Duration::from_millis(1)).unwrap(), Some(CalibrationEvent::Progress(vec![22])));
    }
    assert!(!k.link().calibrating, "dropped guard sent stop");
    let c = k.calibrate(Confirm::user_confirmed()).unwrap();
    assert_eq!(c.stop().unwrap(), Some(CalibrationEvent::Finished));
}

#[test]
fn resets() {
    let mut k = kb();
    let mut t = PerKey::new();
    t.0.insert(46, KeyTrigger { mode: TriggerMode::Fixed, actuation: 200, press: 5, release: 5 });
    k.write_triggers(&t).unwrap();
    assert!(k.reset_triggers(Confirm::user_confirmed()).unwrap());
    assert_eq!(k.read_trigger(46).unwrap().actuation, 100);
    k.set_socd_enabled(true).unwrap();
    assert!(k.factory_reset(Confirm::user_confirmed()).unwrap().is_empty());
    assert!(!k.read_socd_power().unwrap().enabled);
}

#[test]
fn auto_backup_before_the_first_write_only() {
    let dir = store::temp_dir("autobackup");
    let mut k = kb();
    k.set_auto_backup(Some(dir.clone()));
    k.read_effect().unwrap();
    assert!(store::list_auto_backups(&dir).unwrap().is_empty(), "reads do not back up");
    k.set_socd_enabled(true).unwrap();
    k.set_socd_enabled(false).unwrap();
    let backups = store::list_auto_backups(&dir).unwrap();
    assert_eq!(backups.len(), 1);
    assert_eq!(k.last_backup(), Some(backups[0].as_path()));
    let snap = store::load_snapshot(&backups[0]).unwrap();
    assert!(!snap.socd.unwrap().enabled, "backup holds the state before the write");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn profiles_round_trip() {
    let dir = store::temp_dir("profiles-rt");
    let p = store::Profiles::in_dir(dir.clone());
    let s = kb().read_snapshot().unwrap();
    p.save("game", &s, false).unwrap();
    assert!(p.save("game", &s, false).is_err(), "no silent overwrite");
    p.save("game", &s, true).unwrap();
    assert_eq!(p.list().unwrap(), vec!["game".to_string()]);
    assert_eq!(p.load("game").unwrap(), s);
    std::fs::remove_dir_all(&dir).unwrap();
}

/// Test-only access to the sim behind a guard.
trait PushInput {
    fn push(&mut self, b: frame::Body);
}

impl PushInput for crate::TravelStream<'_, SimKeyboard> {
    fn push(&mut self, b: frame::Body) {
        self.link_for_test().push_input(b);
    }
}

impl PushInput for crate::Calibration<'_, SimKeyboard> {
    fn push(&mut self, b: frame::Body) {
        self.link_for_test().push_input(b);
    }
}

impl crate::TravelStream<'_, SimKeyboard> {
    fn kb_link_open(&mut self) -> bool {
        self.link_for_test().travel_open
    }
}

/// Snapshot dumped by `examples/probe.rs` from the owner's board (READ-only, 2026-11-01).
const OWNER_SNAPSHOT: &str = include_str!("../tests/fixtures/owner-2026-11-01.json");

#[test]
fn real_board_snapshot_parses_and_matches_the_sim() {
    let real = Snapshot::from_json(OWNER_SNAPSHOT).unwrap();
    assert_eq!(real.device.firmware, "3.17.07");
    let blue = real.lighting.custom.as_ref().unwrap()[1].iter().filter(|(_, c)| **c == Rgb(0, 0, 0xff)).count();
    assert_eq!(blue, 34);
    let mut sim = kb().read_snapshot().unwrap();
    sim.created_unix = real.created_unix;
    sim.lighting.custom.as_mut().unwrap()[1] = real.lighting.custom.as_ref().unwrap()[1].clone();
    assert!(diff(&real, &sim).is_empty(), "{:?}", diff(&real, &sim));
    assert_eq!(sim, real);
}

#[test]
fn changing_the_active_custom_slot_rewrites_that_slot() {
    let mut k = kb();
    let old = k.read_snapshot().unwrap();
    let mut new = old.clone();
    new.lighting.active_custom_slot = 1;
    assert_eq!(changed_tables(&new, &old), vec!["custom colours slot 1"]);
    assert_eq!(diff(&old, &new)[0].to_string(), "lighting: active custom slot 0 -> 1");
    k.apply_snapshot(&new, &old).unwrap();
    assert_eq!(k.read_active_custom_slot().unwrap(), 1);
}

#[test]
fn diff_texts() {
    let old = kb().read_snapshot().unwrap();
    let mut new = old.clone();
    new.lighting.effect.mode = 2;
    new.poll_rate_khz = Some(1);
    new.macros[9] = Some(simple_sequence(9, &[Usage(4)], 0));
    let d: Vec<String> = diff(&old, &new).iter().map(|c| c.to_string()).collect();
    assert_eq!(d[0], "lighting: static brightness 1 speed 2 fg #ff00ff bg #000000 direction 0 -> wave brightness 1 speed 2 fg #ff00ff bg #000000 direction 0");
    assert!(d.contains(&"macros: slot 9: empty -> 2 events, Once, repeat 1".to_string()), "{d:?}");
    assert!(d.contains(&"poll rate: 8 kHz -> 1 kHz (the keyboard restarts)".to_string()));
    assert_eq!(changed_tables(&new, &old).last().unwrap(), "poll rate", "poll rate goes last");
}

// ---- security fixes ----------------------------------------------------------------------------------------

/// The sim board with a raw (undecodable) entry on KeyA in the base layer and a raw advanced slot 5.
fn kb_with_raw() -> Keyboard<SimKeyboard> {
    let mut k = kb();
    k.link_mut().keymap[0][68 * 4..68 * 4 + 4].copy_from_slice(&[0x10, 0x05, 0, 0]);
    let mut slot = [0u8; 40];
    slot[0] = 9;
    slot[1] = 0xaa;
    k.link_mut().advanced[5] = slot;
    k
}

#[test]
fn raw_keymap_entries_are_only_kept() {
    let mut k = kb_with_raw();
    let mut m = k.read_keymap(Layer::Base).unwrap();
    assert_eq!(m.get(68), Some(&KeyAction::Raw([0x10, 5, 0, 0])));
    // keeping it while changing another key is fine
    m.0.insert(66, KeyAction::Key(0x29));
    k.write_keymap(Layer::Base, &m).unwrap();
    assert_eq!(k.read_keymap(Layer::Base).unwrap().get(68), Some(&KeyAction::Raw([0x10, 5, 0, 0])));
    // a new raw entry is refused before anything is written
    let before = writes(&k).len();
    m.0.insert(69, KeyAction::Raw([0x99, 1, 2, 3]));
    let e = k.write_keymap(Layer::Base, &m).unwrap_err();
    assert!(e.to_string().contains(crate::keymap::RAW_NOT_KEPT), "{e}");
    // changed raw bytes on a key that holds raw are refused too
    m.0.insert(69, KeyAction::Default);
    m.0.insert(68, KeyAction::Raw([0x10, 6, 0, 0]));
    assert!(k.write_keymap(Layer::Base, &m).is_err());
    assert_eq!(writes(&k).len(), before, "nothing was written");
}

#[test]
fn raw_advanced_slots_are_only_kept() {
    let mut k = kb_with_raw();
    let current = k.read_advanced().unwrap();
    assert!(current[5].has_raw());
    // writing it back unchanged is allowed
    k.write_advanced(&[(5, current[5].clone())]).unwrap();
    let before = writes(&k).len();
    let forged = AdvancedKey::Other { payload: "ff".repeat(40) };
    assert!(k.write_advanced(&[(5, forged.clone())]).unwrap_err().to_string().contains(crate::keymap::RAW_NOT_KEPT));
    assert!(k.write_advanced(&[(6, current[5].clone())]).is_err(), "not what slot 6 holds");
    let raw_action = AdvancedKey::Toggle { action: KeyAction::Raw([0x99, 0, 0, 0]) };
    assert!(k.write_advanced(&[(7, raw_action)]).is_err());
    assert_eq!(writes(&k).len(), before, "nothing was written");
}

#[test]
fn apply_refuses_raw_that_the_board_does_not_hold() {
    let mut k = kb_with_raw();
    let old = k.read_snapshot().unwrap();
    // a round trip of a snapshot with raw entries is fine
    let mut keep = old.clone();
    keep.keymap.base.0.insert(66, KeyAction::Key(0x29));
    k.apply_snapshot(&keep, &old).unwrap();
    let old = k.read_snapshot().unwrap();
    let forges: [fn(&mut Snapshot); 4] = [
        |s: &mut Snapshot| {
            s.keymap.fn_layer.0.insert(45, KeyAction::Raw([0x14, 0, 0, 0]));
        },
        |s: &mut Snapshot| {
            s.keymap.base.0.insert(68, KeyAction::Raw([0x10, 7, 0, 0]));
        },
        |s: &mut Snapshot| s.advanced[1] = AdvancedKey::Other { payload: "01".repeat(40) },
        |s: &mut Snapshot| {
            s.advanced[2] = AdvancedKey::ModTap { hold: KeyAction::Raw([1, 2, 3, 4]), tap: KeyAction::Key(4), threshold: 20 }
        },
    ];
    for forge in forges {
        let mut new = old.clone();
        forge(&mut new);
        let before = writes(&k).len();
        let e = k.apply_snapshot(&new, &old).unwrap_err();
        assert!(e.to_string().contains(crate::keymap::RAW_NOT_KEPT), "{e}");
        assert_eq!(writes(&k).len(), before);
    }
}

#[test]
fn dks_routes_and_dead_band_are_checked_against_the_travel() {
    let mut k = kb();
    let dks = |r: u8| AdvancedKey::Dks {
        routes: [15, r, r, 15],
        keys: vec![crate::advanced::DksKey { action: KeyAction::Key(4), phases: [1, 0, 0, 0] }],
    };
    k.write_advanced(&[(0, dks(34))]).unwrap(); // 3.4 mm = full travel
    let before = writes(&k).len();
    assert!(matches!(k.write_advanced(&[(0, dks(35))]), Err(Error::Invalid(_))));
    let mut d = PerKey::new();
    d.0.insert(46, DeadBand { top: 100, bottom: 100 });
    k.write_dead_band(&d).unwrap(); // 2.00 mm of 3.40
    d.0.insert(46, DeadBand { top: 0, bottom: 101 });
    assert!(matches!(k.write_dead_band(&d), Err(Error::Invalid(_))));
    assert_eq!(writes(&k).len(), before + 5, "only the good dead band (5 pages) was written");
    // the same checks run in apply, before the first write
    let old = k.read_snapshot().unwrap();
    let mut new = old.clone();
    new.win_lock.alt_tab = !old.win_lock.alt_tab;
    new.advanced[1] = dks(200);
    let before = writes(&k).len();
    assert!(k.apply_snapshot(&new, &old).unwrap_err().to_string().contains("beyond the full travel"));
    assert_eq!(writes(&k).len(), before);
}

#[test]
fn apply_validates_every_table_before_the_first_write() {
    let mut k = kb();
    let old = k.read_snapshot().unwrap();
    type Forge = fn(&mut Snapshot);
    let forges: [(Forge, &str); 7] = [
        (|s| { s.keymap.fn_layer.0.insert(46, KeyAction::Key(0x52)); }, "Fn-fixed key"),
        (|s| { s.dead_band.as_mut().unwrap().0.insert(46, DeadBand { top: 200, bottom: 0 }); }, "dead band"),
        (|s| { s.switch_types.as_mut().unwrap().0.insert(46, 4); }, "switch"),
        (|s| s.socd.as_mut().unwrap().pairs = vec![SocdPair { mode: SocdMode::LastWins, key1: Usage(1), key2: Usage(7) }; 1], "SOCD"),
        (|s| s.socd.as_mut().unwrap().pairs = vec![SocdPair { mode: SocdMode::LastWins, key1: Usage(4), key2: Usage(7) }; 21], "SOCD"),
        (|s| { s.lighting.custom.as_mut().unwrap()[1].0.insert(200, Rgb(1, 2, 3)); }, "custom colours"),
        (|s| s.macros[3] = Some(simple_sequence(3, &[Usage(4), Usage(0xa5)], 0)), "macro slot 3"),
    ];
    for (forge, what) in forges {
        let mut new = old.clone();
        new.win_lock.alt_tab = !old.win_lock.alt_tab; // applied first if validation were per table
        new.lighting.effect.brightness = 3;
        forge(&mut new);
        let before = writes(&k).len();
        let e = k.apply_snapshot(&new, &old);
        assert!(e.is_err(), "{what} accepted");
        assert_eq!(writes(&k).len(), before, "{what}: something was written before validation failed");
    }
}

#[test]
fn macro_usages_must_be_in_the_keycode_table() {
    let mut k = kb();
    assert!(k.write_macro(0, Some(&simple_sequence(0, &[Usage(0xa5)], 0))).is_err());
    assert!(k.write_macro(0, Some(&simple_sequence(0, &[Usage(0)], 0))).is_err());
    assert!(writes(&k).is_empty());
    k.write_macro(0, Some(&simple_sequence(0, &[Usage(4)], 0))).unwrap();
}

#[test]
fn writes_are_refused_without_a_backup_directory() {
    let mut k = Keyboard::with_link(SimKeyboard::owner_board()).unwrap();
    k.read_effect().unwrap();
    let e = k.set_socd_enabled(true).unwrap_err();
    assert!(matches!(e, Error::Refused(_)), "{e}");
    assert!(writes(&k).is_empty());
    k.set_auto_backup(None);
    k.set_socd_enabled(true).unwrap();
}

#[test]
fn backups_and_profiles_are_owner_only() {
    use std::os::unix::fs::PermissionsExt;
    let mode = |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    let dir = store::temp_dir("private");
    let backups = dir.join("backups");
    std::fs::create_dir(&backups).unwrap();
    std::fs::set_permissions(&backups, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut k = kb();
    k.set_auto_backup(Some(backups.clone()));
    k.set_socd_enabled(true).unwrap();
    assert_eq!(mode(&backups), 0o700, "existing directory narrowed");
    assert_eq!(mode(k.last_backup().unwrap()), 0o600);
    let p = store::Profiles::in_dir(dir.join("profiles"));
    let path = p.save("x", &k.read_snapshot().unwrap(), false).unwrap();
    assert_eq!(mode(p.dir()), 0o700);
    assert_eq!(mode(&path), 0o600);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn writing_the_other_custom_slot_keeps_the_shown_slot() {
    let mut k = kb();
    let old = k.read_snapshot().unwrap();
    let mut new = old.clone();
    new.lighting.custom.as_mut().unwrap()[1].0.insert(22, Rgb(1, 2, 3));
    assert_eq!(changed_tables(&new, &old), vec!["custom colours slot 1", "custom colours slot 0"], "active slot last");
    k.apply_snapshot(&new, &old).unwrap();
    assert_eq!(k.read_active_custom_slot().unwrap(), 0);
    assert_eq!(k.read_custom_colors(1).unwrap().get(22), Some(&Rgb(1, 2, 3)));
    assert_eq!(k.read_custom_colors(0).unwrap(), old.lighting.custom.as_ref().unwrap()[0], "slot 0 rewritten unchanged");
    // switching to slot 1 while changing slot 0: slot 0 first, slot 1 last
    let old = k.read_snapshot().unwrap();
    let mut new = old.clone();
    new.lighting.active_custom_slot = 1;
    new.lighting.custom.as_mut().unwrap()[0].0.insert(22, Rgb(4, 5, 6));
    assert_eq!(changed_tables(&new, &old), vec!["custom colours slot 0", "custom colours slot 1"]);
    k.apply_snapshot(&new, &old).unwrap();
    assert_eq!(k.read_active_custom_slot().unwrap(), 1);
    // nothing changed: nothing written
    let old = k.read_snapshot().unwrap();
    assert!(changed_tables(&old, &old).is_empty());
}

#[test]
fn single_custom_slot_write_keeps_the_active_slot() {
    let mut k = kb();
    let shown = k.read_custom_colors(0).unwrap();
    let mut c = PerKey::new();
    c.0.insert(46, Rgb(9, 9, 9));
    k.write_custom_colors_keep_active(1, &c).unwrap();
    assert_eq!(k.read_active_custom_slot().unwrap(), 0);
    assert_eq!(k.read_custom_colors(1).unwrap().get(46), Some(&Rgb(9, 9, 9)));
    assert_eq!(k.read_custom_colors(0).unwrap(), shown);
    let custom_writes = |k: &Keyboard<SimKeyboard>| writes(k).iter().filter(|b| b[0] == 0x09).count();
    let before = custom_writes(&k);
    k.write_custom_colors_keep_active(0, &shown).unwrap();
    assert_eq!(custom_writes(&k) - before, 8, "active slot: one table, no extra rewrite");
    assert!(k.write_custom_colors_keep_active(2, &c).is_err());
    let e = k.verify_active_custom_slot(1).unwrap_err();
    assert!(matches!(e, Error::Verify(_)), "{e}");
}

#[test]
fn out_of_range_triggers_on_the_board_do_not_block_other_keys() {
    let mut k = kb();
    k.link_mut().triggers[30].1 = 2000; // beyond the 3.40 mm travel
    let old = k.read_snapshot().unwrap();
    let mut new = old.clone();
    new.trigger.0.get_mut(&46).unwrap().actuation = 150;
    k.apply_snapshot(&new, &old).unwrap();
    assert_eq!(k.read_trigger(46).unwrap().actuation, 150);
    // a changed key is still checked
    let old = k.read_snapshot().unwrap();
    let mut bad = old.clone();
    bad.trigger.0.get_mut(&46).unwrap().actuation = 2000;
    assert!(matches!(k.apply_snapshot(&bad, &old), Err(Error::Invalid(_))));
    assert_eq!(k.read_trigger(46).unwrap().actuation, 150);
}

#[test]
fn snapshot_reads_the_current_win_lock() {
    let mut k = kb();
    let before = k.read_snapshot().unwrap().win_lock;
    k.link_mut().caps[20] = 0b0100; // changed on the board (Fn combo) after connect
    let now = k.read_snapshot().unwrap().win_lock;
    assert_ne!(now, before);
    assert_eq!(now, WinLock { alt_f4: true, ..Default::default() });
}

#[test]
fn calibration_ignores_start_echo_and_acks() {
    let mut k = kb();
    let mut c = k.calibrate(Confirm::user_confirmed()).unwrap();
    c.push(body("21 00 00 00 18 0f 00"));
    c.push(body("21 00 00 00 01 0f 00"));
    assert_eq!(c.next_event(Duration::from_millis(1)).unwrap(), None, "echo and ACK are not Finished");
    c.push(body("21 00 00 00 02 0f 04"));
    assert_eq!(c.next_event(Duration::from_millis(1)).unwrap(), Some(CalibrationEvent::Failed(4)));
    assert_eq!(c.stop().unwrap(), Some(CalibrationEvent::Finished));
}

# aulactl — SESSION

Goal: Linux CLI + cxx-qt QML GUI driver for the AULA WIN 60 HE keyboard (USB 2e3c:c365, product string "WIN 60 HE"),
mirroring ~/Desktop/github/hecatectl (Rust workspace: core + CLI + GUI, ii matugen colours, scripts/install.sh, udev rule).
Scope (owner-confirmed 2026-11-01): actuation / rapid trigger / SOCD etc, keymap / layers / macros, RGB, live travel view + calibration,
plus own small improvements. Private repo 956zs/aulactl.

## Device facts
- Interfaces: if0 boot keyboard (hidraw5), if1 mouse/consumer/system/NKRO (hidraw6), if2 vendor (hidraw7):
  usage page 0xFF1B usage 0x91, report ID 1, 63-byte IN + OUT, no feature reports.
- VID:PID 2e3c:c365 is shared by other boards (AULA WIN68 HE, Redragon K686...) -> match product string too.
- udev: /etc/udev/rules.d/70-aulactl.rules (uaccess) installed.

## References (untrusted data, read only)
- ~/aula-re/dl/aether : Aether-HE (Python, NO license -> facts only, no code copying).
  driver_src/ = official AULA WebHID driver JS (device.aulacn.com, 403 to curl) incl. decoded/ versions.

## Progress
- 🔄 Task 1 protocol research -> docs/protocol.md
- ✅ Task 1 protocol research -> docs/protocol.md (product code SI2825KZHEARGB, fw V3.17.07; no onboard profiles; calibration/reset/adv-key writes JS-only)
- ✅ Task 2 plan written (plans/PLAN.md)
- ✅ Phase A: crates/aula-core (126 tests, clippy clean; probe READ-only on the real board OK, snapshot read 265 ms). No write was sent to the device yet.

## Core API (crates/aula-core, frozen during phase B; ask the lead for changes)

Errors: `aula_core::Error` (NotFound, Io, Busy, Timeout, Protocol, UnsupportedProduct{product_code}, Unsupported,
Invalid, Refused, Verify), `Result<T>`; `impl From<Error> for String`. Units: travel values are raw trigger units
(0.01 mm; `TravelLimits::{unit_mm,to_mm,from_mm}`). Key index = `row*22+col`; slots are 0-based everywhere.

- `hidraw::{find_devices() -> Vec<DeviceInfo{node,name,phys}>, wait_for_device(timeout), DEBUG_ENV="AULACTL_DEBUG"}`
- `Keyboard::{open_first(), open(&DeviceInfo), open_path(&Path)}` -> identifies via 0x0D; other boards get no
  other command (`UnsupportedProduct`). `identity() -> &Identity{raw,product_code,firmware,build_date}`,
  `is_supported()`, `capabilities() -> &Capabilities` (`features()`, `socd`, `custom_light()`, `poll_rate()`,
  `dead_band()`, `switch_types()`, `music_rhythm()`, `any_key_calibration()`, `win_lock()`, ...),
  `refresh_capabilities()`, `set_auto_backup(Option<PathBuf>)` (on by default for `open*`; `with_link` refuses writes until it is called, None = explicit opt-out for sim/tests), `last_backup()`,
  `with_lock(|kb| ..)` (one flock'ed operation).
- Lighting: `read_mode_list() -> ModeList{modes,max_speed,max_brightness}` (`selectable()`), `read_effect()`,
  `set_effect(&Effect{mode,brightness,speed,fg,bg,direction,full_color,on})`, `read_active_custom_slot()`,
  `read_custom_colors(slot)`, `write_custom_colors(slot, &PerKey<Rgb>)`, `set_music_rhythm(bool)`.
  `lighting::{MODES, mode_info, mode_by_name, ModeInfo{name,speed,colors,directions}, Rgb (parse "#rrggbb")}`.
- Keymap: `read_keymap(Layer::{Base,Fn}) -> Keymap (=PerKey<KeyAction>)`, `write_keymap(layer, &map)` (whole layer,
  missing keys = default), `read_fn_fixed() -> Vec<usize>`. `KeyAction` parses/prints text: `KeyA`, `Esc`, `Fn`,
  `Default`, `VolumeUp`, `BrightnessUp`, `MouseLeft`, `Copy`, `Ctrl+Shift+Esc`, `Macro0..9`, `Adv0..39`, `raw:xxxxxxxx`.
  Tables: `keycodes::{USAGES, MEDIA, usage_by_name, name_of_usage, Usage}`, `keymap::NAMED`.
- Macros: `read_macro(slot)`, `read_macros()`, `write_macro(slot, Option<&Macro>)`;
  `Macro{id,mode: PlayMode::{Once,Repeat,Toggle,WhileHeld},repeat,events: Vec<MacroEvent{input: MacroInput::{Key(Usage),Mouse(u8)},down,delay_ms}>}`,
  `macros::simple_sequence(id, &[Usage], delay_ms)`. Bind with keymap action `MacroN` (same slot).
- Advanced: `read_advanced() -> Vec<AdvancedKey>` (40), `write_advanced(&[(slot, AdvancedKey)])`;
  `AdvancedKey::{Empty, Dks{routes,keys: Vec<DksKey{action,phases}>}, ModTap{hold,tap,threshold}, Toggle{action}, Other{payload}}`,
  `advanced::dks_default_routes`. Bind with keymap action `AdvN`.
- Trigger: `travel_limits() -> TravelLimits{min,max,..}`, `read_trigger(idx)`, `read_triggers(&[idx]) -> PerKey<KeyTrigger>`,
  `write_triggers(&PerKey<KeyTrigger{mode: TriggerMode::{Fixed,Rapid,RapidSplit},actuation,press,release}>)` (grouped
  frames, read back), `read_dead_band()/write_dead_band(&PerKey<DeadBand{top,bottom}>)`, `read_switch_list()`,
  `read_switch_types()/write_switch_types(&PerKey<u8>)` (`trigger::SWITCHES`, `switch_name`), `read_poll_rate()`,
  `set_poll_rate(khz)` (board restarts: handle dead, `hidraw::wait_for_device` + reopen).
- SOCD / win-lock: `read_socd_power() -> SocdPower{enabled,hotkey}`, `set_socd_enabled(bool)`, `read_socd_pairs()`,
  `write_socd_pairs(&[SocdPair{mode: SocdMode::{LastWins,Key1Wins,Key2Wins,Neutral},key1,key2}])`,
  `read_win_lock()/set_win_lock(WinLock{win,shift_tab,alt_f4,alt_tab})`.
- Live/destructive: `travel_stream(&[idx]) -> TravelStream` (`next_sample(timeout) -> Option<TravelSample{index,depth,at_rest}>`,
  `run(tick, |Option<sample>| ControlFlow)`, `close()`; Drop closes; holds the device lock),
  `calibrate(Confirm) -> Calibration` (`next_event(timeout) -> Option<CalibrationEvent::{Progress(keys),Finished,Failed(code)}>`,
  `stop()`; Drop stops), `reset_triggers(Confirm) -> bool acked`, `factory_reset(Confirm) -> Vec<unacked steps>`.
  `Confirm::user_confirmed()` only after explicit confirmation (CLI `--yes`, GUI dialog).
- Snapshot: `read_snapshot() -> Snapshot` (serde JSON, `to_json()/from_json()`, schema 1), `apply_snapshot(&new, &old)
  -> ApplyReport{written, reconnect_needed}` (only changed tables, poll rate last), `snapshot::{diff(old,new) -> Vec<Change>
  (Display "section: detail"), changed_tables(new, old)}`.
- Store: `store::{data_dir, backups_dir, profiles_dir, save_snapshot, load_snapshot, save_auto_backup, list_auto_backups,
  validate_profile_name, Profiles::{open_default, in_dir, list, exists, save(name,&snap,overwrite), load, delete, path}}`.
- Layout: `layout::{KEYS (Key{index,label,code,default_entry,visual_row,x_px,y_px,w_px,fn_fixed}; x(),y(),width() in u),
  by_index, by_code, all_indices, select("wasd,row:2,Esc,idx:46,all,letters,digits,mods")}`; `perkey::{PerKey, key_name}`.
- Tests without hardware: feature `sim` -> `sim::SimKeyboard` (owner's board in memory) + `Keyboard::with_link(sim)`;
  real snapshot fixture `crates/aula-core/tests/fixtures/owner-2026-11-01.json`.

## CLI (crates/aulactl) ✅

Commits 7c7b972, 2edf358. 54 tests (pure helpers + end-to-end against `sim`), clippy -D warnings clean.
Real board: only read commands and `travel` were run (no write sent).
- device: `list`, `info`, `dump [FILE]`, `backup [FILE]`, `diff FILE`, `restore FILE [--yes]`,
  `profile list|save NAME [--force]|show|diff|apply NAME [--yes]|delete NAME --yes`
- actuation: `actuation show [KEYS] [--table]`, `actuation set KEYS [--point MM] [--rt P[/R]] [--mode fixed|rt|rt-split]`,
  `deadband show|set KEYS --top/--bottom MM`, `switch show|list|set KEYS TYPE`, `pollrate [1|2|4|8]` (waits for restart,
  reads back), `travel [KEYS] [--raw] [--seconds N]` (SIGINT/SIGTERM close the stream), `calibrate --yes`,
  `reset triggers|factory --yes`
- keys: `keymap show [--layer] [--all]|set KEYS ACTION|reset KEYS|codes [FILTER]`, `macro list|show|set SLOT "STEPS"
  [--mode] [--repeat] [--delay] [--bind KEYS]|clear`, `adv list|set SLOT dks|mt|tgl ..|clear`, `socd [list]|add|remove|on|off`,
  `winlock [opts]`
- light: `light [show]|modes|effect MODE [opts]|brightness|speed|direction|rainbow|color|on|off|custom [show] [SLOT]|
  custom set KEYS #rgb [--slot]|slot [N]|music on|off`

## GUI (crates/aula-gui, phase B) ✅
- cxx-qt 0.10 + QML, hecatectl look (ii matugen theme, en + zh_TW with live switch). Pages: Overview (status, capabilities,
  profiles with diff preview, backup/restore, poll rate, win-lock, language, factory reset), Actuation, Keymap, Macros,
  Advanced (DKS/MT/TGL + SOCD), Lighting (effects, per-key paint, music rhythm), Live (heatmap, key tester, calibration).
- Write path: pages edit snapshot sections; worker re-reads, refuses if the board changed since it was shown, then
  `apply_snapshot` (changed tables only) and re-reads. Destructive actions only from confirm dialogs.
- `--sim` / `AULA_GUI_SIM=1` = core simulator (scratch profile/backup dirs). `--page N`, `--screenshot PATH`.
- 93 tests, clippy clean. Installed with scripts/install.sh (~/.local/bin/aula-gui + desktop entry). Real board: only
  read pages and a short live stream were used (open/close ACKed); no write was sent.

## Security fixes ✅
- Review findings P2-2..P3-10 fixed (1472c03, ce3dc0a, 8c7c66a, 23cf7c9): raw entries preserve-only, full validation before
  the first write, mandatory auto-backup, locked restore/apply, 0700/0600 + no-clobber files, bounded drain / 4 MiB snapshots,
  private --sim dir, Confirm-gated calibration. See plans/ISSUES.md. Tests 142/57/96, clippy clean, reinstalled; real board READ-only smoke OK.

## Review fixes ✅
- Correctness review P2-1..P3-6 + test dir cleanup fixed (ca80ddb core, cc1ab90 cli, db699cd sim knob, 9b8a7cd gui); see plans/ISSUES.md. Tests 150/60/101, clippy clean, reinstalled; real board READ-only smoke OK.
- ✅ Phase C: security review (fixed, ISSUES.md), correctness review (fixed), Codex skipped by owner; README/LICENSE; pushed private 956zs/aulactl (2026-11-01)
- ✅ Hardware-test fixes (c3d7f60, 704b781; ISSUES HW-1..3): macros always written as a whole 0..9 sequence (`write_macros`), last
  macro delay 0; SOCD power unacked, verified via `24 02`; custom RGB writes put the previous effect back, apply writes custom
  slots before the effect. Tests 158/60/101, clippy clean, reinstalled. Real board: macro set/clear (slots 9, 3), SOCD on/off,
  custom Esc colour and back OK; restore of the pre-write backup = nothing to change, diff matches.
- Real-device writes verified so far: actuation (one key change+revert), lighting brightness, macros, SOCD power, custom RGB. Not yet on hardware: keymap/SOCD pairs/adv writes, calibration, resets (JS-only), GUI Apply.
- Owner baseline backup: ~/aula-re/backup-pre-write-2026-11-01.json

## Feed (`aulactl feed`) ✅ (2026-11-01)
- Contract: docs/feed.md (NDJSON: status/hello/depth/press/release/error). Core `feed` module: `KeyState` (fixed / rt /
  rt-split press/release mirroring the firmware, raw units), `DepthCoalescer` (per-key N Hz, final 0 at once, 0 = off),
  `Line` + `hello()` (serde). `hidraw::Device::set_lock_wait`; sim knobs `travel_script`, `unplug_after_script`.
- CLI `feed [--keys KEYS] [--depth-hz N]`: connect loop (waiting no keyboard/busy every 1 s, 200 ms lock wait),
  `Keyboard::with_link` without backup dir (writes impossible), hello after the stream opened, lost -> waiting,
  SIGINT/SIGTERM -> close (ACKed) exit 0, EPIPE exit 0, fatal -> error line + exit 1.
- Latency tooling: `scripts/latency-check.sh` (usbmon, two runs feed off/on) + analyzer example
  `crates/aula-core/examples/usbmon_latency.rs` (unit-tested, `test = true`). Only a 2 s dry run done; the real
  measurement needs the owner typing.
- Tests 174 core / 7 example / 69 cli / 101 gui, clippy clean, reinstalled. Real board: feed KeyZ,KeyX 5 s (open/close
  ACKed), busy -> hello once `travel` finished, `info` OK after.

## Replay link (feed --record + osu! replays) ✅ (2026-11-01)
- Design: plans/PLAN.md "Replay link". Contract: docs/feed.md "Recording sessions" + "Replay reports" (stdout unchanged).
- Core: `session` (gzip NDJSON writer/reader, truncated-file tolerant, retention 300 / 2 GiB, label rules,
  `resolve`), `osr` (legacy .osr parser, lzma-rs, caps, spans per input, `encode` test helpers under test/sim),
  `align` (offset voting + greedy matching ±6 ms, best injective input→key mapping, refuses < 60 % / < 10),
  `report` (per-key stats + self-contained HTML `report.html`, zh_TW), `replay` (exports scan, pairing by map
  name + timestamp window, `make_report`, `open` via xdg-open). Deps: flate2 (miniz_oxide), lzma-rs.
- CLI: `feed --record [DIR] [--label TEXT]` (`Recorder`: file at first hello, flush at unplug, finish on exit,
  write errors stop recording only); `replay sessions|match|report` (offline, no keyboard).
- GUI: `Sessions` singleton (bridge/sessions.rs) + Live page card: newest 10 sessions, Report button (thread,
  opens the HTML). i18n en + zh_TW.
- lazer .osr timestamp = end of play (checked on a real export: ~0.3 s before osu-state's cooldown start).
- aula_keys module flags: `aulactl feed --keys ... --depth-hz ... --record --label "<beatmap>"`.
- Limitations: one offset per play (a pause inside the play breaks alignment after it); taiko/catch refused;
  custom lazer rates (not DT/HT) not known from the legacy format.

## Key overlay video (`aulactl replay overlay`) + osu-render step ✅ (2026-11-01)
- Contract: docs/feed.md "Key overlay video" (CLI, `--json-only` schema, raw `--out -`, exit 3 = no session) and
  "danser video time → song time" (formula + empirical check).
- Core `overlay`: `Data` (presses/peaks/depth on the song clock, serde, schema 1), `build`, `session_candidates`
  (4 newest sessions started ≤ replay ts + 1 min), `find_session` (first that aligns).
- CLI `crates/aulactl/src/overlay/`: `draw` (pure `render(scene, style, font, t, pixmap)`: keycaps with travel fill,
  actuation line, counts, KPS / peak KPS / 1/4 BPM, KeyOverlay bars scaled by peak depth, fade at the top), `font`
  (fontdue via fc-match, built-in 5×7 pixel font fallback = tests), `mod` (args, `Timing` with pre-roll, ffmpeg
  qtrle/vp9/ffv1 or raw RGBA stdout). Deps: tiny-skia 0.12 (no png), fontdue 0.9.
- Example `aula-core/examples/synth_session.rs`: synthetic session from a real .osr (testing without a recording).
- danser's bundled ffmpeg has no qtrle/ffv1/libvpx: osu-render pipes `--out -` raw frames into danser's ffmpeg.
- osu-render PR (branch feat/keyboard-overlay): `src/key-overlay.ts` (timeline from danser settings, composite,
  encoder fallback to libx264), options keyOverlay/Corner/Scale/Opacity, `/health keyOverlay`, UI + i18n, README x3.
  E2E on a test instance (port 3031, synthetic sessions): nomod quickstart, HDDT, full intro all within 0..1 frame of
  danser's own key overlay. Live service not touched.

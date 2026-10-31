# aulactl — ISSUES

## Security review 2026-11-01

| id | finding | status | commit |
|---|---|---|---|
| P2-1 | udev rule granted access to every interface | fixed (lead) | b4f15d1 |
| P2-2 | restore file / profile could write unvalidated raw bytes (`KeyAction::Raw`, `AdvancedKey::Other`) | fixed: raw entries are preserve-only (must equal the board's current entry) in `write_keymap`, `write_advanced` and snapshot apply; CLI refuses typed `raw:` actions | ce3dc0a, 8c7c66a |
| P2-3 | `Keyboard::link_mut()` bypassed the allowlist / product guard | fixed: `#[cfg(test)] pub(crate)` | ce3dc0a |
| P3-1 | DKS routes and dead band not checked against travel limits | fixed: `AdvancedKey::check_travel`, `DeadBand::validate` (in core writes and apply) | ce3dc0a |
| P3-2 | macro key usages not checked against the keycode table | fixed in `Macro::validate` | ce3dc0a |
| P3-3 | apply validated some tables only inside each write | fixed: `snapshot::validate` checks keymaps (+Fn-fixed), changed macros/advanced, colours, dead band, switch types, SOCD, poll rate before the first write | ce3dc0a |
| P3-4 | auto-backup silently off when no backup dir | fixed: core refuses writes without a backup dir (`set_auto_backup(None)` = explicit opt-out for sim/tests); GUI only opts out for `--sim` | ce3dc0a, 23cf7c9 |
| P3-5 | TOCTOU: `old` read outside the lock apply takes | fixed: CLI restore/profile apply and GUI apply read + compare + write inside one `with_lock` | 8c7c66a, 23cf7c9 |
| P3-6 | backups/profiles world-readable | fixed: dirs 0700 (existing narrowed), files 0600 | 1472c03 |
| P3-7 | CLI dump/backup/profile save could clobber files | fixed: hard-link no-clobber write unless `--force` | 1472c03, 8c7c66a |
| P3-8 | GUI `--sim` dir in shared /tmp | fixed: `$XDG_RUNTIME_DIR/aula-gui-sim-<pid>`, created 0700 once, existing path refused, removed only if created | 23cf7c9 |
| P3-9 | unbounded hidraw drain / snapshot file size | fixed: drain 256 frames / 100 ms; load_snapshot max 4 MiB | 1472c03 |
| P3-10 | `Keyboard::calibrate` ignored its `Confirm` | fixed: `calibration_start_frame(Confirm)` needs the token | ce3dc0a |

Tests after the fixes: aula-core 142, aulactl 57, aula-gui 96; clippy -D warnings clean.

## Correctness review 2026-11-01

| id | finding | status | commit |
|---|---|---|---|
| P2-1 | CLI calibrate dropped an early Finished/Failed and then failed on stop()'s empty result | fixed: early final status wins over stop()'s | cc1ab90 |
| P2-2 | custom slots written 0 then 1, so the last-written (shown) slot could switch | fixed: other slot first, active slot last (rewritten even if unchanged), `09 20` verified; CLI `light custom set --slot N` keeps the shown slot (`write_custom_colors_keep_active`) and says so | ca80ddb, cc1ab90 |
| P2-3 | apply validated every trigger key, so bad values already on the board blocked unrelated applies | fixed: only `changed_keys` validated | ca80ddb |
| P2-4 | stopLive only sent when `Device.live` was already true | fixed: page leave / nav change / window close always send it (stopCalibration on close too) | 9b8a7cd |
| P3-1 | snapshot used cached capabilities (stale win-lock) | fixed: `refresh_capabilities` in read | ca80ddb |
| P3-2 | GUI kept a dead handle after an I/O error (same hidrawN re-enumeration) | fixed: GuiLink records I/O errors, command() disconnects so the poll reopens | db699cd, 9b8a7cd |
| P3-3 | GUI reconnect did not update `state.node` | fixed | 9b8a7cd |
| P3-4 | an echo/ACK of the calibration start could read as Finished | fixed: `b[4] < 2` ignored, exact start-frame echo ignored for 100 ms; protocol.md open question 9 (re-check on first real calibration) | ca80ddb |
| P3-5 | DKS decode dropped Default gaps (positions shifted on rewrite) | fixed: gaps and phases kept, only all-zero trailing positions dropped; non-zero bytes past the keys keep the slot raw | ca80ddb |
| P3-6 | CLI: `--bind` validated after the slot write; pollrate reconnect ignored `--device`; Ctrl-C during the lock wait still started travel/calibration | fixed: keymap plan checked before the slot write; reconnect follows the same board (USB port path, else node); `hidraw::set_lock_abort` from the signal handler aborts the wait before anything is sent | ca80ddb, cc1ab90 |
| — | GUI worker tests left `$XDG_RUNTIME_DIR/aula-gui-sim-<pid>` | fixed: per-test scratch dirs removed on drop; sim-dir test removes the process dir | 9b8a7cd |

Tests after the fixes: aula-core 150, aulactl 60, aula-gui 101; clippy -D warnings clean; reinstalled; real board READ-only smoke (info, dump, diff vs pre-write backup) OK.

## Hardware-test findings 2026-11-01

| id | finding | status | commit |
|---|---|---|---|
| HW-1 | single macro-slot writes (5 pages, all ACKed `19 00`) are silently dropped by the firmware; a full 0..9 sequence is stored | fixed: `write_macro` / new `write_macros` read all slots and write all 10 in order 0..9 (unchanged slots with their current image), then read all back; snapshot apply has one "macros" table; encoder writes delay 0 for the last event (CLI text: no trailing wait); sim only commits a full 0..9 sequence | c3d7f60, 704b781 |
| HW-2 | SOCD power `24 03` takes effect but gets no ACK (protocol said `24 00`) | fixed: `set_socd_enabled` / factory reset send it without waiting, verify via `24 02` (up to 5 reads, 20 ms apart); sim sends no ACK | c3d7f60, 704b781 |
| HW-3 | writing a custom RGB slot switches the effect to Custom (mode 10, brightness 4, speed 3) | fixed: `write_custom_colors*` put the previous effect back (`07 00`, verified by `07 01`); snapshot apply writes custom slots before the effect; sim switches the effect | c3d7f60, 704b781 |

Real board after the fixes: macro set/clear on slots 9 and 3 (6 runs, all stored, other slot kept), SOCD on/off, `light custom set Esc '#00ff00' --slot 1` and back (effect stayed static); restore of the pre-write backup: nothing to change, `diff` matches.

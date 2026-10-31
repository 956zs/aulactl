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

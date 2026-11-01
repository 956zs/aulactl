# aulactl — implementation plan

Source of truth for the wire format: `docs/protocol.md`. Reference implementation style: `~/Desktop/github/hecatectl`
(same owner, same conventions: Rust 2021 workspace, `libc` hidraw ioctls, no hidapi; cxx-qt 0.10 + QML GUI;
ii matugen colours; i18n en + zh_TW; `scripts/install.sh` user-local install).

## Crates

| crate | role |
|---|---|
| `crates/aula-core` | hidraw discovery + transport, framing, identification guard, every feature model (read/write), key layout table, keycode table, config snapshot (serde JSON) for backup/restore and host-side profiles, travel stream, calibration |
| `crates/aulactl` | CLI |
| `crates/aula-gui` | cxx-qt QML GUI |

## Safety rules (all crates)

1. Writes are only allowed after `0x0D` returns `SI2825HEARGB` / `SI2825KZHEARGB` (protocol 1.1). Otherwise read-only, and only identification commands are sent.
2. Never send a command byte not listed in protocol 2.6. Never send anything marked DO NOT USE (side-light write `08 00`, gamepad `14 03`, onboard profile `21 sub 11/12`, sleep timers).
3. DESTRUCTIVE commands (factory reset `14 00 00 00 01 01`, trigger reset `21 .. 18 06`, calibration) need explicit confirmation (CLI `--yes`, GUI confirm dialog).
4. Automatic backup: before the first WRITE of a process, the full config snapshot is saved to `$XDG_DATA_HOME/aulactl/backups/auto-<timestamp>.json` (keep last 20).
5. One request in flight; drain stale input first; 200 ms per frame / 1 s per table timeouts; travel stream always closed on exit (Drop guard + signal handling in CLI).
6. Writes are read back and compared when the protocol allows reading that table.

## Features

- info: product string, firmware, capability bitmap
- actuation: per-key actuation point, rapid trigger (modes 0/12/13, press/release sensitivity), dead band (top/bottom), switch type, poll rate
- SOCD pairs + power; advanced keys DKS / MT / TGL (40 slots)
- keymap: base + Fn layer, full keycode table (names usable in CLI: `KeyA`, `Escape`, `VolumeUp`, `Macro3`, ...), Fn-fixed keys respected
- macros: 10 slots, play modes, repeat count, delays
- lighting: effects (mode/brightness/speed/colour/direction), per-key RGB two custom slots, active slot, music-rhythm toggle
- win-lock flags, NKRO/6KRO only if capability says so
- live travel stream; calibration with progress
- backup / restore (whole snapshot, diff preview); factory reset
- **own improvements**: host-side named profiles (board has no onboard profiles) with `profile save/apply/list/diff`; `diff` between device and a file; key-group selectors (`wasd`, `row:2`, `all`, labels) for per-key commands; GUI live travel heatmap + key tester; read-back verification

## Phases

- A (1 agent): `aula-core` complete with unit tests (pure encode/decode functions: normal / empty / boundary / error), `examples/probe.rs` read-only.
- B (parallel, file-partitioned): CLI agent owns `crates/aulactl/**`; GUI agent owns `crates/aula-gui/**`, `scripts/**`, desktop entry/icon. Core is frozen during B (requests for core changes go to the lead).
- C (parallel): lead verification on the real device (reads, round-trip writes of current values, one change+revert on one key with backup), clippy/tests, security review, Codex review, handle feedback; README; push private repo 956zs/aulactl.

## Build rules

- `CARGO_TARGET_DIR=~/.cache/build-targets/aulactl` (GUI agent may use `aulactl-gui`), never repo `target/`.
- Run builds with `nice -n 19 ionice -c3`. Before heavy builds check `hyprctl activewindow -j` — if class is `osu!` (owner playing), wait.
- Commits: conventional format, no AI attribution.

## Replay link (feed recording + osu! replay alignment) — design 2026-11-01

Scope (owner-confirmed): record + align + report. All Rust (no scripts in other languages).

- **Recording** (`aulactl feed --record [DIR] [--label TEXT]`): stdout unchanged. The session file
  `<dir>/<UTC YYYYmmdd-HHMMSS>-<sanitized label>.ndjson.gz` (default dir `$XDG_DATA_HOME/aulactl/sessions`, 0700;
  file 0600, created new, never replaced) is opened at the first `hello`. Line 1 = `{"type":"session","schema":1,
  "label","start_unix_ms","start_mono_ms","product","firmware","max_mm","keys":[hello keys]}`; then lines shaped like the
  feed's: `hello` (each reconnect), every raw travel sample as `depth` (t 0.01 ms, mm 3 decimals, not coalesced),
  `press`/`release`, `status` lost. `t` = ms since the feed started (same clock as stdout). gzip via `flate2`
  (miniz_oxide, pure Rust), sync-flushed every 2 s; finished on SIGINT/SIGTERM/EPIPE/fatal error; a record write error
  stops recording (stderr once), never the feed. Readers accept a truncated file (SIGKILL). Retention after each new
  file: newest 300 sessions and at most 2 GiB in total (constants in `aula_core::session`). Label: control chars
  removed, ≤ 200 chars in the header; file-name part = alphanumerics and `-_.()[]`, spaces → `_`, no leading dot,
  ≤ 80 chars.
- **core `osr`**: legacy replay header (strings ULEB128, caps) + LZMA frames via `lzma-rs` (pure Rust, memlimit).
  Frames `w|x|y|z` with cumulative w, seed frame -12345 skipped, lazer trailing data ignored. Inputs: standard
  bit0 (M1/K1) = input 1, bit1 (M2/K2) = input 2; mania x = column bitmask. Other modes: refused. Rate mods DT/NC
  ×1.5, HT ×0.75 (song time → real time). Test encoder in tests (lzma-rs compress).
- **core `align`** (pure): presses per recorded key vs replay key-downs per input. Offset search: votes over
  (session press − replay down) differences in 1 ms bins (replay anchors capped to 400, spread), limited to the
  wall-clock window when the osr timestamp is known; the top bins are rescored by greedy nearest matching within
  ±6 ms per (input, key) and the offset refined to the median residual; mapping = best injective assignment of
  inputs to keys (brute force ≤ 3 inputs, greedy beyond). Result: offset, mapping, match rate, residual
  median/p95 (abs), unmatched counts; refused below 60 % match rate or < 10 matches.
- **core `report`**: per-key stats (presses, depth at the replay's key-down p10/median/p90, peak per press,
  bottom-out ratio ≥ max−0.1 mm, hold, RT re-presses without returning above the point, shallow presses = peak <
  point + 0.5 mm) and a self-contained HTML (inline CSS/JS, data as embedded JSON with `</` escaped, canvas
  timeline with wheel zoom / drag scroll, zh_TW labels, dark theme). Written to `$XDG_DATA_HOME/aulactl/reports/`.
- **CLI `aulactl replay`**: `sessions`, `match [--exports DIR]`, `report [SESSION|latest] [--osr FILE]
  [--exports DIR] [--open]`. Pairing: osr file name `<player> playing <map> (<date>).osr` → normalized map name vs
  session label (alphanumeric, lowercase), and osr timestamp within [session start − 1 min, session end + 5 min].
- **GUI**: Live page "Sessions" list (newest 10) with a Report button → core report + xdg-open; follow-up if it grows.

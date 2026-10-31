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

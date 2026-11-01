# `aulactl feed` — live key event stream (NDJSON)

Consumers (e.g. the `aula_keys` Quickshell ii module) run `aulactl feed [--keys KEYS] [--depth-hz N]` (optionally
`--record`, see below) as a child
process and read one JSON object per line from stdout. stderr is for humans only.

The feed holds the keyboard's travel-test stream open; while it runs, other aulactl/aula-gui writes are refused
("busy"). Consumers should only run it while needed (e.g. while a rhythm game is focused) and stop it with
SIGTERM/SIGINT; the stream is always closed on exit.

## Lines

All lines have `"type"`. `t` is milliseconds since the feed started (monotonic, float).

```json
{"type":"status","state":"waiting","reason":"no keyboard"}
{"type":"status","state":"waiting","reason":"busy"}
{"type":"hello","product":"SI2825KZHEARGB","firmware":"3.17.07","max_mm":3.40,
 "keys":[{"code":"KeyZ","label":"Z","idx":89,"x":1.75,"y":3,"w":1.0,
          "mode":"rt","point":1.00,"press":0.50,"release":0.50}]}
{"type":"depth","t":1234.5,"key":"KeyZ","mm":1.23}
{"type":"press","t":1234.6,"key":"KeyZ"}
{"type":"release","t":1301.0,"key":"KeyZ"}
{"type":"status","state":"lost","reason":"keyboard unplugged"}
```

- `status`: `waiting` (no board or device busy; the feed retries every second and repeats a status only when it
  changes), `lost` (board went away; the feed goes back to waiting and emits a new `hello` when it reconnects).
- `hello`: once per (re)connection, before any key event. `keys` lists only the selected keys (default: all 61),
  with layout geometry in key units (`x`, `y` row, `w` width) and the key's current trigger settings.
- `depth`: key travel in mm (2 decimals). Coalesced to at most `--depth-hz` per key (default 120, 0..1000;
  0 = no `depth` lines); the last depth of a burst follows once its interval has passed, and the final depth
  when a key returns to rest (0.00) is always sent at once.
- `press` / `release`: derived on the host from depth and the key's trigger settings, mirroring the firmware:
  - fixed: press when depth >= point, release when depth < point.
  - rt / rt-split: below `point` the key is released; above it, press when the key moved down by `press` mm
    from its lowest point since the last release, release when it moved up by `release` mm from its highest
    point since the last press.
  Never throttled. They are an approximation of what the firmware reports over USB (the stream is sampled every
  ~1.5–5 ms while keys move), suitable for counters and KPS, not for timing judgements.
- Exit codes: 0 on SIGTERM/SIGINT, 1 on a fatal error (also printed as `{"type":"error","msg":"..."}`).
- Numbers are plain JSON numbers (`3.4`, not `3.40`). Unknown `type`s and fields may be added later; ignore them.
- Only the travel-test open/close frames (RAM only) and READ commands are sent; the feed never writes settings.

## Recording sessions (`--record`)

```sh
aulactl feed [--keys KEYS] [--depth-hz N] --record [DIR] [--label TEXT]
```

Optional; without `--record` nothing changes. stdout is exactly the same with or without it. The recording
is a gzip'd NDJSON file per feed run, `DIR/<UTC YYYYmmdd-HHMMSS>[-<label>].ndjson.gz` (default DIR
`$XDG_DATA_HOME/aulactl/sessions`, created 0700; files 0600, never replaced: `-1`, `-2`, ... on a clash).
`--label` (e.g. the beatmap) is stored in full (control characters removed, at most 200 characters); the
file-name part keeps only letters/digits and `-_.()[]`, spaces become `_`, at most 80 characters. One run
may hold several plays (retries); the replay report finds the right one.

- The file is created at the first `hello` (no file while the feed only waits). stderr says where it records.
- Line 1: `{"type":"session","schema":1,"label":"...","start_unix_ms":...,"start_mono_ms":...,"product":"...",
  "firmware":"...","max_mm":3.4,"keys":[{"code","label","idx","mode","point","press","release"}]}` —
  `start_unix_ms` / `start_mono_ms` are the wall clock and CLOCK_MONOTONIC at `t` = 0 (the same `t` as
  stdout).
- Then the feed's own line shapes: `hello` on every (re)connection, every raw travel sample as `depth`
  (`t` to 0.01 ms, `mm` to 0.001 mm, **not** coalesced, rest frames as `0`), `press` / `release`, and
  `{"type":"status","state":"lost",...,"t":...}` when the board goes away.
- Data is pushed to the file every 2 s and at an unplug; SIGINT/SIGTERM, a closed stdout (EPIPE) and fatal
  errors complete the gzip stream. A killed feed leaves a readable prefix. A write error (disk full) stops
  the recording with one stderr line; the feed keeps running.
- Retention: after each new file the oldest sessions beyond the newest 300 or beyond 2 GiB in total are
  deleted (the file being written is kept).

The `aula_keys` module enables it with `--record --label "<beatmap from osu-state>"` (keep its usual
`--keys` / `--depth-hz`).

### Replay reports (`aulactl replay`)

```sh
aulactl replay sessions [--limit N]                    # newest sessions: start, length, presses, keys, label
aulactl replay match [--exports DIR] [--limit N]       # exported .osr files that belong to each session
aulactl replay report [SESSION|latest] [--osr FILE] [--exports DIR] [--open]
```

- Replays: the legacy `.osr` format osu!lazer exports (default DIR `~/.local/share/osu/exports`, read only),
  osu!standard (K1/M1 = input 1, K2/M2 = input 2) and mania (columns). DT/NC and HT are taken into account.
- Pairing: the map in the file name (`<player> playing <map> (<date>).osr`) equals the session label
  (letters/digits only, case-insensitive) and the replay's timestamp lies between 1 min before the session
  start and 5 min after its last line. Without a label, time only. Newest first; the first replay that
  lines up is used (`--osr` picks one).
- Alignment: the offset between the replay's song clock and the session clock is found from the press times
  (± 6 ms per matched press, wall-clock window from the replay timestamp), together with which recorded key
  played which input. Refused (with the numbers) when fewer than 60 % of the replay's key-downs match —
  wrong session, keys not recorded (e.g. mouse buttons), or a pause in the play.
- The report is one self-contained HTML file in `$XDG_DATA_HOME/aulactl/reports/` (no network): alignment
  quality, per key the depth when the replay registered each press (p10/median/p90), peak depth, bottom-out
  ratio (peak ≥ max − 0.1 mm), hold time, rapid-trigger re-presses (the key did not rise above the
  actuation point since the previous release), shallow presses (peak < point + 0.5 mm, listed with links),
  and a zoomable timeline of each key's depth with the replay's key-down spans and the actuation point.
  Labels are in Traditional Chinese.

## Does the feed slow down key reports? (`scripts/latency-check.sh`)

Whether the board's normal key reports change while the travel test is open is not known (protocol 10.1).
`scripts/latency-check.sh` measures it with usbmon on the real board:

```sh
scripts/latency-check.sh 20            # two 20 s runs: feed off, then feed on (all keys)
scripts/latency-check.sh 20 --keys KeyZ,KeyX
```

Before each run it waits for Enter; then type or mash keys (e.g. alternate Z/X fast) for the whole run. It
needs `sudo -A` (SUDO_ASKPASS) to load `usbmon` and read `/sys/kernel/debug/usb/usbmon/<bus>u`, captures the
whole bus, and keeps the board's interrupt-IN completions: 0x81 (interface 0, boot keyboard), 0x82
(interface 1, report 5 = NKRO bitmap) and 0x83 (vendor interface, travel frames). The analyzer is the Rust
example `crates/aula-core/examples/usbmon_latency.rs`. Per run it prints:

- key report counts on if0 / if1 and travel frames on 0x83;
- report interval distribution (µs, min/p10/p50/p90/max) per interface, and how many reports came within
  250 µs of the previous one (at 8 kHz the floor is 125 µs; fewer fast intervals or a higher minimum with the
  feed on would mean a reduced report rate);
- key-downs and the down-to-down interval of the same key (mashing speed);
- feed-on run only: travel-frame interval per key, and the delay from the first travel frame at or past the
  key's actuation point (from the feed's `hello`) to the matching HID key-down (same usage, nearest within
  ±50 ms). Positive = the HID report came after the frame. With rapid trigger the firmware presses at the
  point only from rest, so re-presses above the point are not counted.

Timestamps are the kernel's URB completion times on one clock, so the delay compares the two interfaces
directly. Captures and reports are kept in `~/.cache/aulactl/latency-<time>/`.

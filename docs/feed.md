# `aulactl feed` — live key event stream (NDJSON)

Consumers (e.g. the `aula_keys` Quickshell ii module) run `aulactl feed [--keys KEYS] [--depth-hz N]` as a child
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

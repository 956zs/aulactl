# aulactl

> **中文摘要（zh-TW）**
>
> 在 Linux 上取代 AULA 官方網頁驅動，設定 **AULA WIN 60 HE** 磁軸鍵盤（USB `2E3C:C365`，產品碼 `SI2825HEARGB` / `SI2825KZHEARGB`）。
> 包含命令列工具 `aulactl` 和圖形介面 `aula-gui`。
>
> - 安裝：`scripts/install.sh`。udev 規則只開放設定用的 interface 2，打字用的 interface 不開放。
> - 每次執行的第一個寫入前會自動備份到 `~/.local/share/aulactl/backups/`；手動備份 `aulactl backup`，還原 `aulactl restore FILE --yes`。
> - 鍵盤沒有板載 profile，`aulactl profile` 在電腦端存多組設定。
> - 校準、恢復出廠、重設觸發都需要 `--yes`（GUI 會跳確認）。

A Linux replacement for AULA's WebHID driver for the **AULA WIN 60 HE** hall-effect keyboard. It talks to the
vendor HID interface (interface 2, usage page `0xFF1B`) with the packets the official driver sends.
The protocol is documented in [`docs/protocol.md`](docs/protocol.md).

- `aulactl`: command-line tool (`aulactl --help`)
- `aula-gui`: settings app (Qt Quick + Rust, cxx-qt), follows the Quickshell *ii* (matugen) colours
- `aula-core`: the library both use

`2E3C:C365` is shared with other boards (WIN 68 HE, KP-TE153, ...). aulactl identifies the board by the product
code it reports and refuses to write to anything else.

## Features

- Actuation point per key (0.08–3.40 mm), rapid trigger (shared or separate press/release sensitivity),
  dead band, switch type, poll rate (1/2/4/8 kHz)
- Keymap for the base and Fn layers (Fn-fixed keys respected), 10 macros, 40 advanced keys (DKS / mod-tap /
  toggle), SOCD pairs, win-lock options
- Lighting: effects, brightness/speed/direction/colours, per-key colours in two custom slots, music rhythm
- Live key travel (terminal map or GUI heatmap with peak hold), key tester, any-key calibration
- `aulactl feed`: live key depth and press/release as JSON lines for other programs (e.g. an on-screen key
  overlay); contract in [docs/feed.md](docs/feed.md)
- osu! replay link: `aulactl feed --record` saves every raw travel sample of a session; `aulactl replay report`
  aligns it with the replay osu!lazer exports and writes an HTML report (depth at each press, peak depth,
  bottom-out ratio, hold time, rapid-trigger re-presses, shallow presses, zoomable timeline); also from the GUI's
  Live page. See [docs/feed.md](docs/feed.md#recording-sessions---record)
- Key overlay video: `aulactl replay overlay` draws the recorded keys of a replay (live travel, presses, KPS /
  BPM, KeyOverlay-style bars) as a transparent video on the replay's clock, e.g. for osu-render to lay over its
  danser render. See [docs/feed.md](docs/feed.md#key-overlay-video-aulactl-replay-overlay)
- Backup / restore / diff, host-side named profiles

## Install

```sh
scripts/install.sh            # builds release, installs to ~/.local/bin + desktop entry
```

The udev rule `udev/70-aulactl.rules` gives the logged-in user access to the vendor interface only.

## Safety model

- Writes only after the board identifies as WIN 60 HE; only command bytes the official driver uses are sent,
  and commands with unknown effects on this board (gamepad mode, onboard profiles, side lights, sleep timers)
  are never sent.
- One request at a time with timeouts; every write is read back and compared.
- The first write of each run saves a full backup (kept: last 20, mode 0600).
- Restores and profiles are validated completely before the first frame is written.
- Calibration, factory reset and trigger reset need explicit confirmation.

## Development

```sh
CARGO_TARGET_DIR=~/.cache/build-targets/aulactl cargo test --workspace --all-features
aula-gui --sim                 # GUI against an in-memory board
```

## License

MIT

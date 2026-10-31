#!/usr/bin/env bash
# Does `aulactl feed` (travel test open) change the keyboard's key reports? Captures the board's USB
# interrupt-IN traffic with usbmon twice, once without and once with the feed running, while you type or
# mash keys, and reports per run: key report counts, report intervals, and (feed run only) the delay between
# the first travel frame at or past a key's actuation point and the matching HID key-down.
#
#   scripts/latency-check.sh [SECONDS] [--keys KEYS] [--yes]
#
# SECONDS per run (default 20). --keys is passed to `aulactl feed` (default: all keys). --yes starts each run
# without waiting for Enter (for a dry run). Needs sudo -A (SUDO_ASKPASS) for modprobe usbmon and reading
# debugfs. Captures and reports go to ${XDG_CACHE_HOME:-~/.cache}/aulactl/latency-<time>/.
set -euo pipefail

repo=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
export SUDO_ASKPASS="${SUDO_ASKPASS:-$HOME/.local/bin/sudo-askpass}"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$HOME/.cache/build-targets/aulactl}"
aulactl="${AULACTL:-$(command -v aulactl || true)}"

seconds=20
keys=all
ask=1
while [ $# -gt 0 ]; do
    case "$1" in
        --keys)
            keys="${2:?--keys needs a value}"
            shift
            ;;
        --yes) ask=0 ;;
        -h | --help)
            sed -n '2,11s/^# \{0,1\}//p' "${BASH_SOURCE[0]}"
            exit 0
            ;;
        *[!0-9]* | '')
            echo "latency-check: unknown argument '$1'" >&2
            exit 2
            ;;
        *) seconds="$1" ;;
    esac
    shift
done
[ -n "$aulactl" ] || { echo "latency-check: aulactl not found (scripts/install.sh, or AULACTL=path)" >&2; exit 1; }

# the keyboard's bus and device number
board=
for d in /sys/bus/usb/devices/*; do
    if [ "$(cat "$d/idVendor" 2>/dev/null)" = 2e3c ] && [ "$(cat "$d/idProduct" 2>/dev/null)" = c365 ]; then
        board="$(cat "$d/busnum"):$(cat "$d/devnum")"
        break
    fi
done
[ -n "$board" ] || { echo "latency-check: no WIN 60 HE (2e3c:c365) on USB" >&2; exit 1; }
bus=${board%%:*}
mon="/sys/kernel/debug/usb/usbmon/${bus}u"

echo "building the analyzer..."
nice -n 19 ionice -c3 cargo build --quiet --release --manifest-path "$repo/Cargo.toml" -p aula-core --example usbmon_latency
analyzer="$CARGO_TARGET_DIR/release/examples/usbmon_latency"

sudo -A modprobe usbmon
sudo -A test -r "$mon" || { echo "latency-check: $mon missing (is debugfs mounted?)" >&2; exit 1; }

out="${XDG_CACHE_HOME:-$HOME/.cache}/aulactl/latency-$(date +%Y%m%d-%H%M%S)"
mkdir -p "$out"
feed_pid=

cleanup() {
    if [ -n "$feed_pid" ]; then
        kill -TERM "$feed_pid" 2>/dev/null || true
        wait "$feed_pid" 2>/dev/null || true
    fi
}
trap cleanup EXIT

capture() { # name
    if [ "$ask" = 1 ]; then
        read -r -p "run '$1': press Enter, then type / mash keys for ${seconds} s... "
    fi
    echo "capturing $1 for ${seconds} s (bus $bus, device ${board#*:})"
    # usbmon text output of the whole bus; the analyzer keeps this device's interrupt-IN completions
    sudo -A timeout -s INT "$seconds" cat "$mon" >"$out/$1.usbmon" || true
}

capture feed-off

"$aulactl" feed --keys "$keys" --depth-hz 0 >"$out/feed.ndjson" 2>"$out/feed.stderr" &
feed_pid=$!
for _ in $(seq 50); do
    grep -q '"type":"hello"' "$out/feed.ndjson" 2>/dev/null && break
    sleep 0.1
done
grep -q '"type":"hello"' "$out/feed.ndjson" || {
    echo "latency-check: the feed did not start:" >&2
    cat "$out/feed.ndjson" "$out/feed.stderr" >&2
    exit 1
}
capture feed-on
cleanup
feed_pid=

for run in feed-off feed-on; do
    {
        echo "== $run"
        "$analyzer" --dev "$board" --hello "$out/feed.ndjson" "$out/$run.usbmon"
    } | tee "$out/$run.report"
done
echo "feed events: $(grep -c '"type":"press"' "$out/feed.ndjson" || true) presses; files in $out"

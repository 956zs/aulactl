# AULA keyboard overlay — osu-render plugin

Adds the keyboard you recorded with `aulactl feed --record` to [osu-render](https://github.com/956zs/osu-render)
videos: one keycap per key that played the replay (live travel, actuation notches, press count), KPS / peak KPS /
tapping BPM, and KeyOverlay-style bars rising from each key. The overlay is drawn by
`aulactl replay overlay auto` (contract: [docs/feed.md](../../docs/feed.md#key-overlay-video-aulactl-replay-overlay))
on the replay's song clock and laid over danser's video by osu-render's plugin host (plugin API 1,
osu-render `docs/plugins.md`).

- Linux only (aulactl is a Linux tool). Needs aulactl on the machine running osu-render: `AULACTL=/path/to/aulactl`
  in osu-render's environment, else `aulactl` on `PATH`, else `~/.local/bin/aulactl` (`scripts/install.sh`).
- The service user must be able to read the recordings (`$XDG_DATA_HOME/aulactl/sessions`, default
  `~/.local/share/aulactl/sessions`): run osu-render as the same user that records.
- A render whose replay has no matching recording (another keyboard, recording was off) is skipped with a note in
  the job log; the video is danser's as usual.

## Install

From the aulactl repo, either the folder or a zip:

```sh
scripts/package-osu-render-plugin.sh          # → dist/aula-keys-osu-render-plugin-<version>.zip
cd ~/services/osu-render
npm run plugin -- install ~/Desktop/github/aulactl/integrations/osu-render-plugin
# or: npm run plugin -- install /path/to/aula-keys-osu-render-plugin-1.0.0.zip   (--force to update)
systemctl --user restart osu-render
```

Then in the web page: **All settings → Plugins → AULA keyboard overlay → Use for this render**, and pick the
position, size (% of the video height), opacity and whether to show the travel in mm. `npm run plugin -- remove
aula-keys` uninstalls it.

## Options

| key | | default |
|---|---|---|
| `corner` | `top-left`, `top-right`, `bottom-left`, `bottom-right` | `bottom-right` |
| `size` | overlay height in % of the video height (width = 9/16 of it) | 40 |
| `opacity` | 20..100 % | 90 |
| `showDepth` | travel in mm above each legend (`--show-depth`) | off |

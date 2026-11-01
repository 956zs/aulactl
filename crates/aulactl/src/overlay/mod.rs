//! `replay overlay`: a transparent key overlay video of a replay, from the recorded session that played it,
//! timed on the replay's song clock (`docs/feed.md` "Key overlay video"). Frames are drawn by `draw` and piped
//! as raw RGBA into ffmpeg, which encodes them with alpha (QuickTime Animation `.mov`, VP9 `.webm`, FFV1
//! `.mkv`), or written to stdout as they are (`--out -`) for a consumer's own ffmpeg.

mod draw;
mod font;

use crate::args::{parse_num, Args};
use crate::replay::Dirs;
use crate::{bad_args, say};
use aula_core::osr;
use aula_core::overlay::{self, Data};
use aula_core::replay::align_replay;
use aula_core::session;
use draw::{render, Scene, Style};
use font::Font;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use tiny_skia::Pixmap;

/// Error text of `auto` when no recorded session lines up with the replay; `main` exits with
/// `NO_SESSION_EXIT` for it.
pub const NO_SESSION: &str = "no recorded session lines up with this replay";
pub const NO_SESSION_EXIT: u8 = 3;
/// Shown before the first and after the last replay frame by default (real ms).
const DEFAULT_LEAD_MS: f64 = 1000.0;
/// Longest video made (real ms).
const MAX_DURATION_MS: f64 = 4.0 * 3600.0 * 1000.0;

/// Frame and time settings.
#[derive(Debug, Clone, PartialEq)]
pub struct Timing {
    pub fps: u32,
    /// Song ms shown on frame 0.
    pub start_ms: f64,
    /// Video length (real ms).
    pub duration_ms: f64,
    /// The first `preroll_ms` of video run at speed 1 (danser's lead-in before the map's clock starts), the
    /// rest at the song rate.
    pub preroll_ms: f64,
    pub rate: f64,
}

impl Timing {
    pub fn frames(&self) -> u64 {
        (self.duration_ms * self.fps as f64 / 1000.0)
            .ceil()
            .max(0.0) as u64
    }

    /// Song time shown on frame `n`.
    pub fn song_time(&self, n: u64) -> f64 {
        let video = n as f64 * 1000.0 / self.fps as f64;
        if video < self.preroll_ms {
            self.start_ms + video
        } else {
            self.start_ms + self.preroll_ms + (video - self.preroll_ms) * self.rate
        }
    }
}

/// ffmpeg output options for the container `out` names.
pub fn encoder_args(out: &Path) -> Result<&'static [&'static str], String> {
    let ext = out
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    match ext.as_deref() {
        Some("mov") => Ok(&["-c:v", "qtrle", "-pix_fmt", "argb"]),
        Some("webm") => Ok(&[
            "-c:v",
            "libvpx-vp9",
            "-pix_fmt",
            "yuva420p",
            "-b:v",
            "0",
            "-crf",
            "30",
            "-row-mt",
            "1",
            "-auto-alt-ref",
            "0",
        ]),
        Some("mkv") => Ok(&["-c:v", "ffv1", "-pix_fmt", "bgra"]),
        _ => Err(format!(
            "{}: the overlay video must be .mov (QuickTime Animation), .webm (VP9), .mkv (FFV1) or - (raw RGBA to stdout)",
            out.display()
        )),
    }
}

fn parse_ms(v: &str, what: &str) -> Result<f64, String> {
    match v.parse::<f64>() {
        Ok(x) if x.is_finite() && x.abs() <= MAX_DURATION_MS => Ok(x),
        _ => Err(format!("{what}: '{v}' is not a time in ms")),
    }
}

/// `replay overlay [SESSION|latest|auto] --osr FILE --out FILE [--width W] [--height H] [--fps N]
/// [--start-ms S] [--duration-ms D] [--preroll-ms P] [--bar-speed PX] [--font FILE] [--ffmpeg BIN]
/// [--show-depth] [--json-only]`
pub fn run(out: &mut dyn Write, dirs: &Dirs, rest: &[&str]) -> Result<(), String> {
    let mut a = Args::new(rest);
    let osr_path = a
        .opt("--osr")?
        .map(PathBuf::from)
        .ok_or("replay overlay needs --osr FILE")?;
    let out_path = a.opt("--out")?.map(PathBuf::from);
    let width = a
        .opt("--width")?
        .map(|v| parse_num(v, 16..=4096, "width"))
        .transpose()?
        .unwrap_or(360);
    let height = a
        .opt("--height")?
        .map(|v| parse_num(v, 16..=4096, "height"))
        .transpose()?
        .unwrap_or(640);
    let fps = a
        .opt("--fps")?
        .map(|v| parse_num(v, 1..=240, "fps"))
        .transpose()?
        .unwrap_or(60);
    let start = a
        .opt("--start-ms")?
        .map(|v| parse_ms(v, "--start-ms"))
        .transpose()?;
    let duration = a
        .opt("--duration-ms")?
        .map(|v| parse_ms(v, "--duration-ms"))
        .transpose()?;
    let preroll = a
        .opt("--preroll-ms")?
        .map(|v| parse_ms(v, "--preroll-ms"))
        .transpose()?
        .unwrap_or(0.0);
    let bar_speed = a
        .opt("--bar-speed")?
        .map(|v| parse_num(v, 1..=100_000, "bar speed (px/s)"))
        .transpose()?;
    let font_path = a.opt("--font")?.map(PathBuf::from);
    let ffmpeg = a.opt("--ffmpeg")?.unwrap_or("ffmpeg").to_string();
    let json_only = a.flag("--json-only");
    let show_depth = a.flag("--show-depth");
    let which = match a.positional()?.as_slice() {
        [] => "auto",
        [s] => s,
        _ => return Err(bad_args(&[&["replay", "overlay"], rest].concat())),
    };
    let out_path = match out_path {
        Some(p) => p,
        None if json_only => PathBuf::from("-"),
        None => return Err("replay overlay needs --out FILE".into()),
    };
    if !json_only && out_path != Path::new("-") {
        encoder_args(&out_path)?;
    }
    if duration.is_some_and(|d| d <= 0.0) {
        return Err("--duration-ms must be more than 0".into());
    }
    if preroll < 0.0 {
        return Err("--preroll-ms must not be negative".into());
    }

    let data = load(dirs, which, &osr_path)?;
    if json_only {
        let json = data.to_json();
        if out_path == Path::new("-") {
            return say(out, &json);
        }
        std::fs::write(&out_path, format!("{json}\n"))
            .map_err(|e| format!("{}: {e}", out_path.display()))?;
        return summary(out, &data, &format!("wrote {}", out_path.display()));
    }
    let first = data.replay_range.0 - DEFAULT_LEAD_MS * data.rate;
    let start_ms = start.unwrap_or(first);
    let timing = Timing {
        fps,
        start_ms,
        duration_ms: duration.unwrap_or_else(|| {
            let song = data.replay_range.1 + DEFAULT_LEAD_MS * data.rate - start_ms;
            (song / data.rate).clamp(1000.0, MAX_DURATION_MS)
        }),
        preroll_ms: preroll,
        rate: data.rate,
    };
    let font = match font_path {
        Some(p) => Font::load(&p)?,
        None => Font::system(),
    };
    let style = Style {
        width,
        height,
        bar_speed: bar_speed.map(|v| v as f32),
        show_depth,
    };
    let scene = Scene::new(data)?;
    let raw = out_path == Path::new("-");
    let frames = if raw {
        write_frames(&scene, &style, font, &timing, out).map_err(|e| format!("output: {e}"))?
    } else {
        encode(&scene, &style, font, &timing, &ffmpeg, &out_path)?
    };
    // raw frames own stdout: the summary goes to stderr
    let mut stderr = std::io::stderr();
    summary(
        if raw { &mut stderr } else { out },
        &scene.data,
        &format!(
            "wrote {} ({frames} frames, {width}x{height} at {fps} fps, song {:.0}..{:.0} ms)",
            if raw {
                "raw RGBA frames to stdout".into()
            } else {
                out_path.display().to_string()
            },
            timing.song_time(0),
            timing.song_time(frames.saturating_sub(1))
        ),
    )
}

/// The overlay data of the replay in session `which` (`auto` = the session that lines up with it).
fn load(dirs: &Dirs, which: &str, osr_path: &Path) -> Result<Data, String> {
    let replay = osr::read(osr_path)?;
    let name = osr_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let dir = dirs.sessions()?;
    let (s, a) = if which == "auto" {
        overlay::find_session(&dir, &replay)?.ok_or(NO_SESSION)?
    } else {
        let s = session::read(&session::resolve(&dir, which)?)?;
        let (_, a) = align_replay(&s, &replay)?;
        (s, a)
    };
    Ok(overlay::build(&s, &replay, &name, &a))
}

fn summary(out: &mut dyn Write, d: &Data, what: &str) -> Result<(), String> {
    let keys: Vec<String> = d
        .keys
        .iter()
        .map(|k| format!("{}={}", k.input, k.code))
        .collect();
    say(
        out,
        &format!(
            "session {}: matched {} of {} replay presses ({:.1} %), keys {}",
            d.session_file,
            d.matched,
            d.replay_downs,
            d.match_rate * 100.0,
            keys.join(" ")
        ),
    )?;
    say(out, what)
}

/// Draws every frame and pipes it into ffmpeg; returns the frame count.
fn encode(
    scene: &Scene,
    style: &Style,
    font: Font,
    timing: &Timing,
    ffmpeg: &str,
    out: &Path,
) -> Result<u64, String> {
    let size = format!("{}x{}", style.width, style.height);
    let fps = timing.fps.to_string();
    let mut child = Command::new(ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-y"])
        .args([
            "-f", "rawvideo", "-pix_fmt", "rgba", "-s", &size, "-r", &fps, "-i", "pipe:0",
        ])
        .args(encoder_args(out)?)
        .arg(out)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("{ffmpeg}: {e}"))?;
    let mut stderr = child.stderr.take();
    let reader = std::thread::spawn(move || {
        let mut s = String::new();
        if let Some(e) = stderr.as_mut() {
            let _ = e.take(64 * 1024).read_to_string(&mut s);
        }
        s
    });
    let mut write_err = None;
    let mut frames = 0;
    if let Some(mut stdin) = child.stdin.take() {
        match write_frames(scene, style, font, timing, &mut stdin) {
            Ok(n) => frames = n,
            Err(e) => write_err = Some(e),
        }
    }
    let status = child.wait().map_err(|e| format!("{ffmpeg}: {e}"))?;
    let log = reader.join().unwrap_or_default();
    if !status.success() || write_err.is_some() {
        let _ = std::fs::remove_file(out);
        return Err(format!(
            "ffmpeg failed ({status}): {}",
            log.lines().last().unwrap_or("no output")
        ));
    }
    Ok(frames)
}

/// Draws every frame as straight-alpha RGBA rows into `sink`; returns the frame count.
fn write_frames(
    scene: &Scene,
    style: &Style,
    mut font: Font,
    timing: &Timing,
    sink: &mut dyn Write,
) -> std::io::Result<u64> {
    let mut pm = Pixmap::new(style.width, style.height)
        .ok_or_else(|| std::io::Error::other("bad overlay size"))?;
    let mut buf = vec![0u8; style.width as usize * style.height as usize * 4];
    let frames = timing.frames();
    for n in 0..frames {
        render(scene, style, &mut font, timing.song_time(n), &mut pm);
        for (px, out) in pm.pixels().iter().zip(buf.chunks_exact_mut(4)) {
            let c = px.demultiply();
            out.copy_from_slice(&[c.red(), c.green(), c.blue(), c.alpha()]);
        }
        sink.write_all(&buf)?;
    }
    sink.flush()?;
    Ok(frames)
}

#[cfg(test)]
mod tests {
    use super::*;
    use aula_core::feed::Edge;
    use aula_core::osr::encode;
    use aula_core::session::{Header, SessionKey, Writer, SCHEMA};

    const START: u64 = 1_793_491_200_000;

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(tag: &str) -> Self {
            let d = std::env::temp_dir().join(format!(
                "aulactl-cli-test-{}-overlay-{tag}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&d);
            std::fs::create_dir_all(&d).unwrap();
            Scratch(d)
        }

        fn dirs(&self) -> Dirs {
            Dirs {
                sessions: Some(self.0.join("sessions")),
                reports: Some(self.0.join("reports")),
            }
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A session with 30 alternating Z/X taps at session 20 s + song time, and the replay of them.
    fn setup(s: &Scratch) -> PathBuf {
        let taps: Vec<i64> = (0..30).map(|i| 1500 + i * 160 + (i * i) % 13).collect();
        let mut frames = Vec::new();
        let header = Header {
            schema: SCHEMA,
            label: "Map".into(),
            start_unix_ms: START,
            start_mono_ms: 0.0,
            product: "SI2825KZHEARGB".into(),
            firmware: "3.17.07".into(),
            max_mm: 3.4,
            keys: ["KeyZ", "KeyX"]
                .iter()
                .map(|c| SessionKey {
                    code: (*c).into(),
                    label: c[3..].into(),
                    idx: 0,
                    mode: "rt".into(),
                    point: 1.0,
                    press: 0.3,
                    release: 0.3,
                })
                .collect(),
        };
        let mut w = Writer::create(&s.0.join("sessions"), &header).unwrap();
        for (i, t) in taps.iter().enumerate() {
            frames.push((*t, if i % 2 == 0 { 5 } else { 10 }));
            frames.push((*t + 40, 0));
            let key = if i % 2 == 0 { "KeyZ" } else { "KeyX" };
            let st = 20_000.0 + *t as f64;
            w.depth(st, key, 1.1).unwrap();
            w.edge(st, key, Edge::Press).unwrap();
            w.depth(st + 8.0, key, 3.0).unwrap();
            w.depth(st + 40.0, key, 0.0).unwrap();
            w.edge(st + 40.0, key, Edge::Release).unwrap();
        }
        w.finish().unwrap();
        let osr = s.0.join("upload.osr");
        let end = START as i64 + 20_000 + taps[29] + 40;
        std::fs::write(&osr, encode::osr(0, 0, end, &encode::frames_text(&frames))).unwrap();
        osr
    }

    fn run_args(s: &Scratch, args: &[&str]) -> Result<String, String> {
        let mut out = Vec::new();
        run(&mut out, &s.dirs(), args)?;
        Ok(String::from_utf8(out).unwrap())
    }

    #[test]
    fn json_only_auto_finds_the_session() {
        let s = Scratch::new("json");
        let osr = setup(&s);
        let osr = osr.to_str().unwrap();
        let text = run_args(&s, &["auto", "--osr", osr, "--json-only"]).unwrap();
        let d: Data = serde_json::from_str(text.trim()).unwrap();
        assert_eq!(d.keys.len(), 2);
        assert_eq!(d.keys[0].presses.len(), 15);
        assert!((d.keys[0].presses[0].down - 1500.0).abs() < 1.0);
        assert!((d.offset_ms - 20_000.0).abs() < 1.0);
        // to a file, the default session argument is auto, named session works too
        let file = s.0.join("o.json");
        let text = run_args(
            &s,
            &["--osr", osr, "--json-only", "--out", file.to_str().unwrap()],
        )
        .unwrap();
        assert!(text.contains("matched 30 of 30"), "{text}");
        let back: Data = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(back, d);
        let text = run_args(&s, &["latest", "--osr", osr, "--json-only"]).unwrap();
        assert!(text.starts_with('{'));
    }

    #[test]
    fn auto_without_a_session_says_so() {
        let s = Scratch::new("none");
        let osr = setup(&s);
        std::fs::remove_dir_all(s.0.join("sessions")).unwrap();
        let e = run_args(&s, &["auto", "--osr", osr.to_str().unwrap(), "--json-only"]).unwrap_err();
        assert_eq!(e, NO_SESSION);
    }

    #[test]
    fn bad_arguments() {
        let s = Scratch::new("args");
        let osr = setup(&s);
        let osr = osr.to_str().unwrap();
        let e = run_args(&s, &["--out", "x.mov"]).unwrap_err();
        assert!(e.contains("--osr"), "{e}");
        let e = run_args(&s, &["--osr", osr]).unwrap_err();
        assert!(e.contains("--out"), "{e}");
        let e = run_args(&s, &["--osr", osr, "--out", "x.mp4"]).unwrap_err();
        assert!(e.contains(".mov"), "{e}");
        let e = run_args(&s, &["--osr", osr, "--out", "x.mov", "--fps", "0"]).unwrap_err();
        assert!(e.contains("fps"), "{e}");
        let e = run_args(&s, &["--osr", osr, "--out", "x.mov", "--duration-ms", "0"]).unwrap_err();
        assert!(e.contains("duration"), "{e}");
        let e = run_args(&s, &["--osr", osr, "--out", "x.mov", "--start-ms", "nan"]).unwrap_err();
        assert!(e.contains("start"), "{e}");
        let e = run_args(&s, &["--osr", osr, "--out", "x.mov", "--preroll-ms", "-5"]).unwrap_err();
        assert!(e.contains("preroll"), "{e}");
        let e = run_args(&s, &["a", "b", "--osr", osr, "--json-only"]).unwrap_err();
        assert!(e.contains("unknown arguments"), "{e}");
        let e = run_args(&s, &["--osr", "/nonexistent.osr", "--json-only"]).unwrap_err();
        assert!(e.contains("nonexistent"), "{e}");
        let e = run_args(
            &s,
            &[
                "--osr",
                osr,
                "--out",
                s.0.join("x.mov").to_str().unwrap(),
                "--ffmpeg",
                "/nonexistent/ffmpeg",
            ],
        )
        .unwrap_err();
        assert!(e.contains("/nonexistent/ffmpeg"), "{e}");
    }

    #[test]
    fn timing_maps_frames_to_song_time() {
        let t = Timing {
            fps: 60,
            start_ms: -1000.0,
            duration_ms: 1000.0,
            preroll_ms: 0.0,
            rate: 1.0,
        };
        assert_eq!(t.frames(), 60);
        assert_eq!(t.song_time(0), -1000.0);
        assert!((t.song_time(60) - 0.0).abs() < 1e-9);
        // DT after a 500 ms pre-roll at speed 1
        let dt = Timing {
            preroll_ms: 500.0,
            rate: 1.5,
            ..t.clone()
        };
        assert!((dt.song_time(30) - -500.0).abs() < 1e-9);
        assert!((dt.song_time(60) - (-500.0 + 750.0)).abs() < 1e-9);
        assert!((dt.song_time(15) - -750.0).abs() < 1e-9);
        let empty = Timing {
            duration_ms: 0.0,
            ..t
        };
        assert_eq!(empty.frames(), 0);
    }

    #[test]
    fn containers() {
        assert_eq!(encoder_args(Path::new("a.MOV")).unwrap()[1], "qtrle");
        assert_eq!(encoder_args(Path::new("a.webm")).unwrap()[1], "libvpx-vp9");
        assert_eq!(encoder_args(Path::new("a.mkv")).unwrap()[1], "ffv1");
        assert!(encoder_args(Path::new("a")).is_err());
        assert!(encoder_args(Path::new("a.mp4")).is_err());
    }

    #[test]
    fn raw_frames_to_stdout() {
        let s = Scratch::new("raw");
        let osr = setup(&s);
        let mut out = Vec::new();
        run(
            &mut out,
            &s.dirs(),
            &[
                "--osr",
                osr.to_str().unwrap(),
                "--out",
                "-",
                "--width",
                "20",
                "--height",
                "30",
                "--fps",
                "10",
                "--start-ms",
                "1400",
                "--duration-ms",
                "300",
            ],
        )
        .unwrap();
        assert_eq!(out.len(), 3 * 20 * 30 * 4);
        assert!(out.chunks_exact(4).any(|p| p[3] > 0), "something is drawn");
        assert!(
            out.chunks_exact(4).any(|p| p[3] == 0),
            "and the rest is transparent"
        );
    }

    /// Needs ffmpeg on PATH; skipped without it.
    #[test]
    fn encodes_a_short_video() {
        if Command::new("ffmpeg").arg("-version").output().is_err() {
            return;
        }
        let s = Scratch::new("encode");
        let osr = setup(&s);
        let video = s.0.join("o.mov");
        let text = run_args(
            &s,
            &[
                "--osr",
                osr.to_str().unwrap(),
                "--out",
                video.to_str().unwrap(),
                "--width",
                "64",
                "--height",
                "96",
                "--fps",
                "10",
                "--start-ms",
                "1400",
                "--duration-ms",
                "500",
                "--font",
                "/nonexistent.ttf",
            ],
        )
        .unwrap_err();
        assert!(text.contains("nonexistent.ttf"), "{text}");
        let text = run_args(
            &s,
            &[
                "--osr",
                osr.to_str().unwrap(),
                "--out",
                video.to_str().unwrap(),
                "--width",
                "64",
                "--height",
                "96",
                "--fps",
                "10",
                "--start-ms",
                "1400",
                "--duration-ms",
                "500",
            ],
        )
        .unwrap();
        assert!(text.contains("5 frames"), "{text}");
        assert!(std::fs::metadata(&video).unwrap().len() > 0);
    }
}

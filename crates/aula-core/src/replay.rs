//! Linking recorded sessions with exported osu! replays: which `.osr` files belong to a session (map name in
//! the file name + replay timestamp), and the whole report pipeline (read, align, write the HTML).

use crate::align::{self, Alignment, Params};
use crate::error::{invalid, Error, Result};
use crate::osr::{self, Replay};
use crate::report;
use crate::session::{self, Session};
use crate::store::{private_dir, write_atomic};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

/// A replay may be stamped this long before the session started (clock differences, ms).
const BEFORE_START_MS: i64 = 60_000;
/// ... and this long after the last recorded line (the export happens after the result screen).
const AFTER_END_MS: i64 = 300_000;
/// Extra room around the offset the replay's timestamp implies (ms).
const WINDOW_SLACK_MS: f64 = 90_000.0;
/// Most `.osr` files looked at in an exports directory.
const MAX_EXPORTS: usize = 5000;

/// `~/.local/share/osu/exports` (osu!lazer's export folder).
pub fn default_exports_dir() -> Result<PathBuf> {
    let base = match std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).filter(|p| p.is_absolute()) {
        Some(x) => x,
        None => match std::env::var_os("HOME").map(PathBuf::from).filter(|p| p.is_absolute()) {
            Some(h) => h.join(".local/share"),
            None => return Err(Error::Io("neither XDG_DATA_HOME nor HOME is set".into())),
        },
    };
    Ok(base.join("osu/exports"))
}

/// `$XDG_DATA_HOME/aulactl/reports`.
pub fn reports_dir() -> Result<PathBuf> {
    Ok(crate::store::data_dir()?.join("reports"))
}

/// An exported replay that may belong to a session.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub path: PathBuf,
    /// Map name from the file name.
    pub map: String,
    pub unix_ms: i64,
    /// The map name equals the session label (normalized).
    pub name_match: bool,
}

/// `.osr` files in `dir` with their header timestamps (files that do not parse are skipped).
pub fn exports(dir: &Path) -> Result<Vec<(PathBuf, String, i64)>> {
    let rd = match fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(Error::Io(format!("{}: {e}", dir.display()))),
    };
    let mut out = Vec::new();
    for e in rd.flatten().take(MAX_EXPORTS * 4) {
        let p = e.path();
        let Some(name) = p.file_name().and_then(|n| n.to_str()) else { continue };
        let Some(map) = osr::map_from_file_name(name) else { continue };
        if !e.file_type().is_ok_and(|t| t.is_file()) {
            continue;
        }
        if let Ok(Replay { timestamp_unix_ms: Some(ts), .. }) = osr::read_header(&p) {
            out.push((p, map, ts));
            if out.len() >= MAX_EXPORTS {
                break;
            }
        }
    }
    out.sort();
    Ok(out)
}

/// Replays from `exports` stamped within the session (from 1 min before its start to 5 min after its last line)
/// whose map name matches the label; with an empty label, every replay in that time. Newest first.
pub fn candidates(header: &session::Header, end_unix_ms: u64, exports: &[(PathBuf, String, i64)]) -> Vec<Candidate> {
    let from = header.start_unix_ms as i64 - BEFORE_START_MS;
    let to = end_unix_ms as i64 + AFTER_END_MS;
    let label = osr::normalize_name(&header.label);
    let mut out: Vec<Candidate> = exports
        .iter()
        .filter(|(_, _, ts)| (from..=to).contains(ts))
        .map(|(p, map, ts)| Candidate { path: p.clone(), map: map.clone(), unix_ms: *ts, name_match: osr::normalize_name(map) == label })
        .filter(|c| c.name_match || label.is_empty())
        .collect();
    out.sort_by(|a, b| b.unix_ms.cmp(&a.unix_ms).then(a.path.cmp(&b.path)));
    out
}

/// Offsets (session ms of song time 0) the replay's timestamp allows. The timestamp is taken as the end of
/// the play, or its start, whichever fits, with 90 s of slack.
pub fn offset_window(replay: &Replay, session_start_unix_ms: u64) -> Option<(f64, f64)> {
    let ts = replay.timestamp_unix_ms? as f64 - session_start_unix_ms as f64;
    let (first, last) = replay.time_range()?;
    let rate = replay.rate();
    Some((ts - last as f64 / rate - WINDOW_SLACK_MS, ts - first as f64 / rate + WINDOW_SLACK_MS))
}

/// Session presses per key: down times in session ms, sorted.
pub fn press_times(s: &Session) -> BTreeMap<String, Vec<f64>> {
    s.presses
        .iter()
        .map(|(k, v)| {
            let mut t: Vec<f64> = v.iter().map(|p| p.0).collect();
            t.sort_by(f64::total_cmp);
            (k.clone(), t)
        })
        .collect()
}

/// Aligns a replay with a session (within the window its timestamp allows).
pub fn align_replay(s: &Session, r: &Replay) -> Result<(Vec<osr::Span>, Alignment)> {
    let spans = r.spans()?;
    let params = Params { window: offset_window(r, s.header.start_unix_ms), ..Params::default() };
    let a = align::align(&spans, r.input_count()?, r.rate(), &press_times(s), &params).map_err(|e| Error::Invalid(e.to_string()))?;
    Ok((spans, a))
}

/// A written report.
#[derive(Debug, Clone, PartialEq)]
pub struct Written {
    pub path: PathBuf,
    pub replay: PathBuf,
    pub match_rate: f64,
    pub matched: usize,
    pub replay_downs: usize,
}

/// Reads `session_path`, aligns it with `osr_path` (or the newest matching replay in `exports_dir` that lines
/// up) and writes `<reports_dir>/<session stem>[-<n>].html` (0600, replaced when it exists).
pub fn make_report(session_path: &Path, osr_path: Option<&Path>, exports_dir: &Path, reports_dir: &Path) -> Result<Written> {
    let s = session::read(session_path)?;
    let tried: Vec<PathBuf> = match osr_path {
        Some(p) => vec![p.to_path_buf()],
        None => {
            let c = candidates(&s.header, s.end_unix_ms(), &exports(exports_dir)?);
            if c.is_empty() {
                return invalid(format!(
                    "no exported replay in {} matches this session ({}); export the replay in osu! or pass --osr FILE",
                    exports_dir.display(),
                    if s.header.label.is_empty() { "no label, time window only" } else { s.header.label.as_str() }
                ));
            }
            c.into_iter().map(|c| c.path).collect()
        }
    };
    let mut last_err = None;
    for p in &tried {
        let r = osr::read(p)?;
        match align_replay(&s, &r) {
            Ok((spans, a)) => {
                let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let rep = report::build(&s, &r, &name, &spans, &a, now_unix_ms());
                let out = report_path(reports_dir, session_path, r.timestamp_unix_ms)?;
                write_atomic(&out, &report::html(&rep))?;
                return Ok(Written {
                    path: out,
                    replay: p.clone(),
                    match_rate: a.match_rate,
                    matched: a.pairs.len(),
                    replay_downs: a.replay_downs,
                });
            }
            Err(e) => last_err = Some(format!("{}: {e}", p.file_name().map(|n| n.to_string_lossy()).unwrap_or_default())),
        }
    }
    Err(Error::Invalid(last_err.unwrap_or_else(|| "no replay to align".into())))
}

/// `<reports>/<session stem>--<replay time, UTC>.html`.
fn report_path(dir: &Path, session_path: &Path, replay_unix_ms: Option<i64>) -> Result<PathBuf> {
    private_dir(dir)?;
    let name = session_path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let base = session::file_label(name.strip_suffix(session::EXT).unwrap_or(&name));
    Ok(dir.join(match replay_unix_ms {
        Some(ms) if ms >= 0 => format!("{base}--{}.html", crate::store::timestamp(ms as u64 / 1000)),
        _ => format!("{base}.html"),
    }))
}

/// Opens a file with the desktop's default application (`xdg-open`) without waiting for it; a thread reaps the
/// child when it exits.
pub fn open(path: &Path) -> Result<()> {
    let mut child = std::process::Command::new("xdg-open")
        .arg(path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| Error::Io(format!("xdg-open: {e}")))?;
    std::thread::spawn(move || child.wait());
    Ok(())
}

fn now_unix_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::feed::Edge;
    use crate::osr::encode;
    use crate::session::{Header, SessionKey, Writer, SCHEMA};
    use crate::store::temp_dir;

    const START: u64 = 1_793_491_200_000;

    fn header(label: &str) -> Header {
        Header {
            schema: SCHEMA,
            label: label.into(),
            start_unix_ms: START,
            start_mono_ms: 0.0,
            product: "SI2825KZHEARGB".into(),
            firmware: "3.17.07".into(),
            max_mm: 3.4,
            keys: ["KeyZ", "KeyX"]
                .iter()
                .enumerate()
                .map(|(i, c)| SessionKey {
                    code: (*c).into(),
                    label: c[3..].into(),
                    idx: 89 + i,
                    mode: "rt".into(),
                    point: 1.0,
                    press: 0.3,
                    release: 0.3,
                })
                .collect(),
        }
    }

    /// Writes a session in which the replay's 60 taps were played twice (a failed try with sloppy timing at
    /// 10 s, the real play at 40 s), and the replay of the real play (stamped at its end).
    fn setup(d: &Path, label: &str) -> (PathBuf, PathBuf) {
        let taps: Vec<i64> = (0..60).map(|i| 2000 + i * 137 + (i * i) % 23).collect();
        let mut frames = Vec::new();
        for (i, t) in taps.iter().enumerate() {
            frames.push((*t, if i % 2 == 0 { 5 } else { 10 }));
            frames.push((*t + 30, 0));
        }
        let end = *taps.last().unwrap() + 30;
        let mut w = Writer::create(&d.join("sessions"), &header(label)).unwrap();
        for (base, sloppy) in [(10_000.0, true), (40_000.0, false)] {
            for (i, t) in taps.iter().enumerate() {
                let key = if i % 2 == 0 { "KeyZ" } else { "KeyX" };
                let t = base + *t as f64 + if sloppy { ((i * 7) % 30) as f64 - 15.0 } else { 2.0 };
                w.depth(t - 2.0, key, 0.8).unwrap();
                w.depth(t, key, 1.1).unwrap();
                w.edge(t, key, Edge::Press).unwrap();
                w.depth(t + 8.0, key, 3.4).unwrap();
                w.depth(t + 30.0, key, 0.0).unwrap();
                w.edge(t + 30.0, key, Edge::Release).unwrap();
            }
        }
        w.finish().unwrap();
        let session = w.path().to_path_buf();
        let exports = d.join("exports");
        fs::create_dir_all(&exports).unwrap();
        let osr = exports.join(format!("n1cat playing {label} (2026-11-01_08-00).osr"));
        fs::write(&osr, encode::osr(0, 0, START as i64 + 40_000 + end, &encode::frames_text(&frames))).unwrap();
        // another map at the same time, and the same map a day later
        fs::write(exports.join("n1cat playing Other - Song (x) [y] (2026-11-01_08-00).osr"), encode::osr(0, 0, START as i64 + 40_000, "")).unwrap();
        fs::write(exports.join(format!("n1cat playing {label} (2026-11-02_08-00).osr")), encode::osr(0, 0, START as i64 + 86_400_000, "")).unwrap();
        fs::write(exports.join("broken.osr"), b"nope").unwrap();
        (session, osr)
    }

    #[test]
    fn pairs_and_reports() {
        let d = temp_dir("replay");
        let label = "Artist - Title (mapper) [Diff]";
        let (session_path, osr_path) = setup(&d, label);
        let s = session::read(&session_path).unwrap();
        let all = exports(&d.join("exports")).unwrap();
        assert_eq!(all.len(), 3, "broken file skipped");
        let c = candidates(&s.header, s.end_unix_ms(), &all);
        assert_eq!(c.len(), 1, "{c:?}");
        assert_eq!(c[0].path, osr_path);
        assert!(c[0].name_match);
        assert_eq!(c[0].map, label);
        // no label: time only (both replays of that day's session)
        let c = candidates(&header(""), s.end_unix_ms(), &all);
        assert_eq!(c.len(), 2);
        let w = make_report(&session_path, None, &d.join("exports"), &d.join("reports")).unwrap();
        assert_eq!(w.replay, osr_path);
        assert_eq!((w.matched, w.replay_downs), (60, 60));
        let html = fs::read_to_string(&w.path).unwrap();
        assert!(html.contains("\"matched\":60"));
        assert!(w.path.file_name().unwrap().to_string_lossy().ends_with(".html"));
        assert!(w.path.starts_with(d.join("reports")));
        // explicit replay; a replay that does not belong is refused with the reason
        let other = d.join("exports/n1cat playing Other - Song (x) [y] (2026-11-01_08-00).osr");
        let e = make_report(&session_path, Some(&other), &d.join("exports"), &d.join("reports")).unwrap_err();
        assert!(e.to_string().contains("no key presses"), "{e}");
        // nothing in the exports dir
        let e = make_report(&session_path, None, &d.join("missing"), &d.join("reports")).unwrap_err();
        assert!(e.to_string().contains("no exported replay"), "{e}");
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn windows() {
        let frames = encode::frames_text(&[(1000, 1), (61_000, 0)]);
        let r = osr::parse(&encode::osr(0, 0, START as i64 + 100_000, &frames), true).unwrap();
        let (lo, hi) = offset_window(&r, START).unwrap();
        // stamped at the end: offset 39 s; at the start: 100 s
        assert!(lo <= 39_000.0 && hi >= 100_000.0 && lo > -60_000.0 && hi < 200_000.0, "{lo} {hi}");
        let dt = osr::parse(&encode::osr(0, osr::MOD_DOUBLE_TIME, START as i64 + 100_000, &frames), true).unwrap();
        assert!(offset_window(&dt, START).unwrap().0 > lo, "DT plays are shorter");
        let empty = osr::parse(&encode::osr(0, 0, 0, ""), true).unwrap();
        assert_eq!(offset_window(&empty, START), None);
    }

    #[test]
    fn report_names() {
        let d = temp_dir("replay-names");
        let p = report_path(&d, Path::new("/s/20261101-000000-a_[b].ndjson.gz"), Some(START as i64 + 61_000)).unwrap();
        assert_eq!(p.file_name().unwrap(), "20261101-000000-a_[b]--20261101-000101.html");
        let p = report_path(&d, Path::new("../../x"), None).unwrap();
        assert_eq!(p, d.join("x.html"));
        fs::remove_dir_all(&d).unwrap();
    }
}

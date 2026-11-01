//! replay: recorded feed sessions and osu! replays (`aulactl feed --record`): list sessions, pair them with
//! exported `.osr` files, and write HTML reports. No keyboard needed.

use crate::args::{parse_num, Args};
use crate::{bad_args, say};
use aula_core::replay::{self, Candidate};
use aula_core::session;
use std::io::Write;
use std::path::{Path, PathBuf};

const DEFAULT_LIMIT: u32 = 20;

/// Directories the replay commands use (tests point them elsewhere).
#[derive(Debug, Clone, Default)]
pub struct Dirs {
    pub sessions: Option<PathBuf>,
    pub reports: Option<PathBuf>,
}

impl Dirs {
    fn sessions(&self) -> Result<PathBuf, String> {
        match &self.sessions {
            Some(d) => Ok(d.clone()),
            None => Ok(session::sessions_dir()?),
        }
    }

    fn reports(&self) -> Result<PathBuf, String> {
        match &self.reports {
            Some(d) => Ok(d.clone()),
            None => Ok(replay::reports_dir()?),
        }
    }
}

pub fn run(out: &mut dyn Write, dirs: &Dirs, rest: &[&str]) -> Result<(), String> {
    match rest {
        ["sessions", more @ ..] => sessions(out, dirs, more),
        ["match", more @ ..] => matches(out, dirs, more),
        ["report", more @ ..] => report(out, dirs, more),
        _ => Err(bad_args(&[&["replay"], rest].concat())),
    }
}

fn limit(a: &mut Args) -> Result<usize, String> {
    Ok(a.opt("--limit")?
        .map(|s| parse_num(s, 1..=10_000, "number of sessions"))
        .transpose()?
        .unwrap_or(DEFAULT_LIMIT) as usize)
}

fn exports_dir(a: &mut Args) -> Result<PathBuf, String> {
    match a.opt("--exports")? {
        Some(d) => Ok(PathBuf::from(d)),
        None => Ok(replay::default_exports_dir()?),
    }
}

/// Local time `YYYY-mm-dd HH:MM:SS` of unix ms.
pub fn local_time(unix_ms: u64) -> String {
    let secs = (unix_ms / 1000) as libc::time_t;
    // SAFETY: localtime_r only writes the tm it is given.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    if unsafe { libc::localtime_r(&secs, &mut tm) }.is_null() {
        return aula_core::store::timestamp(unix_ms / 1000);
    }
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min,
        tm.tm_sec
    )
}

/// `m:ss` of milliseconds.
pub fn duration(ms: f64) -> String {
    let s = (ms.max(0.0) / 1000.0).round() as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

/// The newest `n` session files, newest first.
fn newest(dir: &Path, n: usize) -> Result<Vec<PathBuf>, String> {
    let mut all = session::list(dir)?;
    all.reverse();
    all.truncate(n);
    Ok(all)
}

/// `replay sessions [--limit N]`
fn sessions(out: &mut dyn Write, dirs: &Dirs, rest: &[&str]) -> Result<(), String> {
    let mut a = Args::new(rest);
    let n = limit(&mut a)?;
    if !a.positional()?.is_empty() {
        return Err(bad_args(&[&["replay", "sessions"], rest].concat()));
    }
    let dir = dirs.sessions()?;
    let files = newest(&dir, n)?;
    if files.is_empty() {
        return say(
            out,
            &format!(
                "no recorded sessions in {} (record with 'aulactl feed --record')",
                dir.display()
            ),
        );
    }
    for p in files {
        let name = p
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        match session::read(&p) {
            Ok(s) => {
                let keys: Vec<&str> = s
                    .presses
                    .iter()
                    .filter(|(_, v)| !v.is_empty())
                    .map(|(k, _)| k.as_str())
                    .collect();
                say(
                    out,
                    &format!(
                        "{}  {:>6}  {:>6} presses  {:<16}  {}{}\n    {name}",
                        local_time(s.header.start_unix_ms),
                        duration(s.end_ms),
                        s.press_count(),
                        if keys.is_empty() {
                            "-".to_string()
                        } else {
                            keys.join(",")
                        },
                        if s.header.label.is_empty() {
                            "(no label)"
                        } else {
                            &s.header.label
                        },
                        if s.truncated { "  [cut off]" } else { "" }
                    ),
                )?;
            }
            Err(e) => say(out, &format!("{name}: {e}"))?,
        }
    }
    Ok(())
}

/// `replay match [--exports DIR] [--limit N]`
fn matches(out: &mut dyn Write, dirs: &Dirs, rest: &[&str]) -> Result<(), String> {
    let mut a = Args::new(rest);
    let n = limit(&mut a)?;
    let exports_dir = exports_dir(&mut a)?;
    if !a.positional()?.is_empty() {
        return Err(bad_args(&[&["replay", "match"], rest].concat()));
    }
    let files = newest(&dirs.sessions()?, n)?;
    let exports = replay::exports(&exports_dir)?;
    for p in files {
        let name = p
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let s = match session::read(&p) {
            Ok(s) => s,
            Err(e) => {
                say(out, &format!("{name}: {e}"))?;
                continue;
            }
        };
        say(out, &name)?;
        let c = replay::candidates(&s.header, s.end_unix_ms(), &exports);
        if c.is_empty() {
            say(out, "    no exported replay")?;
        }
        for Candidate {
            path,
            unix_ms,
            name_match,
            ..
        } in c
        {
            say(
                out,
                &format!(
                    "    {}  {}{}",
                    local_time(unix_ms.max(0) as u64),
                    path.file_name()
                        .map(|n| n.to_string_lossy())
                        .unwrap_or_default(),
                    if name_match { "" } else { "  (time only)" }
                ),
            )?;
        }
    }
    Ok(())
}

/// `replay report [SESSION|latest] [--osr FILE] [--exports DIR] [--open]`
fn report(out: &mut dyn Write, dirs: &Dirs, rest: &[&str]) -> Result<(), String> {
    let mut a = Args::new(rest);
    let osr = a.opt("--osr")?.map(PathBuf::from);
    let exports_dir = exports_dir(&mut a)?;
    let open = a.flag("--open");
    let which = match a.positional()?.as_slice() {
        [] => "latest",
        [s] => s,
        _ => return Err(bad_args(&[&["replay", "report"], rest].concat())),
    };
    let path = session::resolve(&dirs.sessions()?, which)?;
    let w = replay::make_report(&path, osr.as_deref(), &exports_dir, &dirs.reports()?)?;
    say(
        out,
        &format!(
            "matched {} of {} replay presses ({:.1} %) with {}",
            w.matched,
            w.replay_downs,
            w.match_rate * 100.0,
            w.replay
                .file_name()
                .map(|n| n.to_string_lossy())
                .unwrap_or_default()
        ),
    )?;
    say(out, &w.path.display().to_string())?;
    if open {
        replay::open(&w.path)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use aula_core::feed::Edge;
    use aula_core::osr::encode;
    use aula_core::session::{Header, SessionKey, Writer, SCHEMA};

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(tag: &str) -> Self {
            let d = std::env::temp_dir().join(format!(
                "aulactl-cli-test-{}-replay-{tag}",
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

    const START: u64 = 1_793_491_200_000;

    /// A session that played 30 alternating taps 20 s after its start, and the replay of it.
    fn setup(s: &Scratch, label: &str) {
        let header = Header {
            schema: SCHEMA,
            label: label.into(),
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
        let mut frames = Vec::new();
        for i in 0..30i64 {
            let t = 1000 + i * 173 + (i * i) % 17;
            frames.push((t, if i % 2 == 0 { 5 } else { 10 }));
            frames.push((t + 30, 0));
            let key = if i % 2 == 0 { "KeyZ" } else { "KeyX" };
            let st = 20_000.0 + t as f64 + 3.0;
            w.depth(st, key, 1.4).unwrap();
            w.edge(st, key, Edge::Press).unwrap();
            w.depth(st + 30.0, key, 0.0).unwrap();
            w.edge(st + 30.0, key, Edge::Release).unwrap();
        }
        w.finish().unwrap();
        let exports = s.0.join("exports");
        std::fs::create_dir_all(&exports).unwrap();
        std::fs::write(
            exports.join(format!("n1cat playing {label} (2026-11-01_08-00).osr")),
            encode::osr(0, 0, START as i64 + 30_000, &encode::frames_text(&frames)),
        )
        .unwrap();
    }

    fn run_text(dirs: &Dirs, args: &[&str]) -> Result<String, String> {
        let mut out = Vec::new();
        run(&mut out, dirs, args)?;
        Ok(String::from_utf8(out).unwrap())
    }

    #[test]
    fn sessions_match_and_report() {
        let s = Scratch::new("all");
        let dirs = s.dirs();
        let empty = run_text(&dirs, &["sessions"]).unwrap();
        assert!(empty.contains("no recorded sessions"), "{empty}");
        setup(&s, "Artist - Song (me) [Hard]");
        let list = run_text(&dirs, &["sessions"]).unwrap();
        assert!(
            list.contains("30 presses") && list.contains("KeyX,KeyZ"),
            "{list}"
        );
        assert!(list.contains("Artist - Song (me) [Hard]"));
        assert!(list.contains("20261101-000000-Artist_-_Song_(me)_[Hard].ndjson.gz"));
        let exports = s.0.join("exports");
        let ex = exports.to_str().unwrap();
        let m = run_text(&dirs, &["match", "--exports", ex]).unwrap();
        assert!(
            m.contains("n1cat playing Artist - Song (me) [Hard] (2026-11-01_08-00).osr"),
            "{m}"
        );
        assert!(!m.contains("time only"));
        let r = run_text(&dirs, &["report", "latest", "--exports", ex]).unwrap();
        assert!(r.contains("matched 30 of 30"), "{r}");
        let html = PathBuf::from(r.lines().last().unwrap());
        assert!(html.starts_with(s.0.join("reports")));
        assert!(std::fs::read_to_string(&html)
            .unwrap()
            .contains("Artist - Song (me) [Hard]"));
        // explicit session name and replay file
        let osr = std::fs::read_dir(&exports)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let r = run_text(
            &dirs,
            &[
                "report",
                "20261101-000000-Artist_-_Song_(me)_[Hard]",
                "--osr",
                osr.to_str().unwrap(),
                "--exports",
                "/nonexistent",
            ],
        )
        .unwrap();
        assert!(r.contains("matched 30 of 30"), "{r}");
        // errors
        assert!(run_text(&dirs, &["report", "nope", "--exports", ex])
            .unwrap_err()
            .contains("no session"));
        let e = run_text(&dirs, &["report", "--exports", "/nonexistent"]).unwrap_err();
        assert!(e.contains("no exported replay"), "{e}");
        assert!(run_text(&dirs, &["report", "a", "b"]).is_err());
        assert!(run_text(&dirs, &["sessions", "--limit", "0"]).is_err());
        assert!(run_text(&dirs, &["bogus"]).is_err());
        assert!(run_text(&dirs, &[]).is_err());
    }

    #[test]
    fn formatting() {
        assert_eq!(duration(0.0), "0:00");
        assert_eq!(duration(-5.0), "0:00");
        assert_eq!(duration(61_499.0), "1:01");
        assert_eq!(duration(3_600_000.0), "60:00");
        let t = local_time(START);
        assert_eq!(t.len(), 19, "{t}");
        assert!(t.starts_with("2026-1"), "{t}");
    }
}

//! Data for a key overlay video of a replay (`aulactl replay overlay`): the recorded presses and depth samples of
//! the keys that played the replay's inputs, moved onto the replay's song clock (ms, as in `.osr` frames), and
//! the session that holds a replay (`find_session`). Pure apart from `find_session`'s file reads.

use crate::align::Alignment;
use crate::error::{invalid, Result};
use crate::osr::Replay;
use crate::replay::align_replay;
use crate::report::peak;
use crate::session::{self, Session};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const SCHEMA: u32 = 1;
/// Samples kept before the first and after the last replay frame (song ms).
const MARGIN_MS: f64 = 10_000.0;
/// Sessions (started before the replay was stamped) tried by `find_session`, newest first.
const MAX_SESSIONS_TRIED: usize = 4;
/// A session may start this long after the replay's timestamp (clock differences, ms).
const START_SLACK_MS: u64 = 60_000;

/// One press of a key in song ms; `peak` = deepest travel (mm) while it was held.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Press {
    pub down: f64,
    /// None when the recording ended while the key was held.
    pub up: Option<f64>,
    pub peak: f64,
}

/// A recorded key that played one replay input.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Key {
    pub code: String,
    pub label: String,
    /// Replay input name (`K1/M1`, `Col 2`).
    pub input: String,
    pub mode: String,
    /// Actuation point (mm).
    pub point: f64,
    /// Presses in time order.
    pub presses: Vec<Press>,
    /// Travel samples `(song ms, mm)` in time order; the depth holds until the next sample.
    pub depth: Vec<(f64, f64)>,
}

/// Everything the overlay draws, on the replay's song clock.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Data {
    pub schema: u32,
    pub session_file: String,
    pub replay_file: String,
    /// Song-time speed (1.5 with DT/NC, 0.75 with HT): song ms per real ms.
    pub rate: f64,
    /// Session ms of song time 0.
    pub offset_ms: f64,
    pub matched: usize,
    pub replay_downs: usize,
    pub match_rate: f64,
    pub residual_median_ms: f64,
    pub max_mm: f64,
    /// First and last replay frame (song ms).
    pub replay_range: (f64, f64),
    /// Ordered by replay input.
    pub keys: Vec<Key>,
}

impl Data {
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{}".into())
    }
}

/// `v` rounded to 1/`scale` (100.0 = two decimals).
fn round(v: f64, scale: f64) -> f64 {
    (v * scale).round() / scale
}

/// Builds the overlay data of `replay` played in `session`, aligned by `a`.
pub fn build(session: &Session, replay: &Replay, replay_file: &str, a: &Alignment) -> Data {
    let song = |t: f64| (t - a.offset_ms) * a.rate;
    let (first, last) = replay.time_range().map_or((0.0, 0.0), |(f, l)| (f as f64, l as f64));
    let from = a.to_session(first - MARGIN_MS);
    let to = a.to_session(last + MARGIN_MS);
    let mut keys = Vec::new();
    for (&input, code) in &a.mapping {
        let Some(k) = session.key(code) else { continue };
        let samples: &[(f64, f64)] = session.depth.get(code).map_or(&[], Vec::as_slice);
        let presses = session
            .presses
            .get(code)
            .map_or(&[][..], Vec::as_slice)
            .iter()
            .filter(|(down, up)| up.unwrap_or(f64::INFINITY) >= from && *down <= to)
            .map(|&(down, up)| Press {
                down: round(song(down), 100.0),
                up: up.map(|u| round(song(u), 100.0)),
                peak: round(peak(samples, down, up.unwrap_or(session.end_ms)), 1000.0),
            })
            .collect();
        let lo = samples.partition_point(|s| s.0 < from);
        let hi = samples.partition_point(|s| s.0 <= to);
        let mut depth = Vec::with_capacity(hi - lo + 1);
        // the depth already reached at the window start
        if lo > 0 {
            depth.push((round(song(from), 100.0), samples[lo - 1].1));
        }
        depth.extend(samples[lo..hi].iter().map(|s| (round(song(s.0), 100.0), round(s.1, 1000.0))));
        keys.push(Key {
            code: k.code.clone(),
            label: k.label.clone(),
            input: replay.input_name(input),
            mode: k.mode.clone(),
            point: k.point,
            presses,
            depth,
        });
    }
    Data {
        schema: SCHEMA,
        session_file: session.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        replay_file: replay_file.to_string(),
        rate: a.rate,
        offset_ms: round(a.offset_ms, 100.0),
        matched: a.pairs.len(),
        replay_downs: a.replay_downs,
        match_rate: round(a.match_rate, 1000.0),
        residual_median_ms: round(a.residual_median, 100.0),
        max_mm: session.header.max_mm,
        replay_range: (first, last),
        keys,
    }
}

/// Session files in `dir` that may hold a replay stamped at `replay_unix_ms`: started at most 1 min after it
/// (the stamp is the end of the play), newest first, at most `MAX_SESSIONS_TRIED`.
pub fn session_candidates(dir: &Path, replay_unix_ms: i64) -> Result<Vec<PathBuf>> {
    let latest = crate::store::timestamp((replay_unix_ms.max(0) as u64 + START_SLACK_MS) / 1000);
    let mut out: Vec<PathBuf> = session::list(dir)?
        .into_iter()
        .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n[..15] <= *latest))
        .collect();
    out.reverse();
    out.truncate(MAX_SESSIONS_TRIED);
    Ok(out)
}

/// The recorded session that holds `replay` (the newest candidate that aligns), with its alignment; None when
/// no session lines up. A replay without a timestamp is refused.
pub fn find_session(dir: &Path, replay: &Replay) -> Result<Option<(Session, Alignment)>> {
    let Some(ts) = replay.timestamp_unix_ms else {
        return invalid("the replay has no timestamp; name the session instead of 'auto'");
    };
    for p in session_candidates(dir, ts)? {
        let Ok(s) = session::read(&p) else { continue };
        if (s.end_unix_ms() as i64) + 300_000 < ts {
            continue;
        }
        if let Ok((_, a)) = align_replay(&s, replay) {
            return Ok(Some((s, a)));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::feed::Edge;
    use crate::osr::{self, encode};
    use crate::session::{Header, SessionKey, Writer, SCHEMA as SESSION_SCHEMA};
    use crate::store::temp_dir;
    use std::fs;

    const START: u64 = 1_793_491_200_000;

    fn header(label: &str, start: u64) -> Header {
        Header {
            schema: SESSION_SCHEMA,
            label: label.into(),
            start_unix_ms: start,
            start_mono_ms: 0.0,
            product: "SI2825KZHEARGB".into(),
            firmware: "3.17.07".into(),
            max_mm: 3.4,
            keys: ["KeyZ", "KeyX", "KeyC"]
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

    /// 40 alternating taps from song 2000 ms; returns the replay bytes (stamped at the end of the play when it
    /// starts at session `play_at` ms) and writes the session (keys Z/X, peak 3.0 / 2.0 mm) into `dir`.
    fn setup(dir: &Path, start: u64, play_at: f64, rate_mods: u32) -> Vec<u8> {
        let rate = if rate_mods & osr::MOD_DOUBLE_TIME != 0 { 1.5 } else { 1.0 };
        let taps: Vec<i64> = (0..40).map(|i| 2000 + i * 150 + (i * i) % 17).collect();
        let mut frames = Vec::new();
        for (i, t) in taps.iter().enumerate() {
            frames.push((*t, if i % 2 == 0 { 5 } else { 10 }));
            frames.push((*t + 45, 0));
        }
        let mut w = Writer::create(dir, &header("Map", start)).unwrap();
        for (i, t) in taps.iter().enumerate() {
            let (key, deep) = if i % 2 == 0 { ("KeyZ", 3.0) } else { ("KeyX", 2.0) };
            let t = play_at + *t as f64 / rate;
            w.depth(t - 1.0, key, 0.6).unwrap();
            w.depth(t, key, 1.2).unwrap();
            w.edge(t, key, Edge::Press).unwrap();
            w.depth(t + 10.0, key, deep).unwrap();
            w.depth(t + 30.0 / rate, key, 0.0).unwrap();
            w.edge(t + 30.0 / rate, key, Edge::Release).unwrap();
        }
        w.finish().unwrap();
        let end = start as i64 + play_at as i64 + ((taps[39] + 45) as f64 / rate) as i64;
        encode::osr(0, rate_mods, end, &encode::frames_text(&frames))
    }

    #[test]
    fn builds_song_time_data() {
        let d = temp_dir("overlay-build");
        let bytes = setup(&d, START, 30_000.0, 0);
        let r = osr::parse(&bytes, true).unwrap();
        let (s, a) = find_session(&d, &r).unwrap().expect("aligned");
        let data = build(&s, &r, "x.osr", &a);
        assert_eq!(data.keys.len(), 2);
        assert_eq!((data.keys[0].code.as_str(), data.keys[0].input.as_str()), ("KeyZ", "K1/M1"));
        assert_eq!(data.keys[1].code, "KeyX");
        assert_eq!(data.keys[0].presses.len(), 20);
        let p = data.keys[0].presses[0];
        assert!((p.down - 2000.0).abs() < 0.5, "{p:?}");
        assert!((p.up.unwrap() - 2030.0).abs() < 0.5, "{p:?}");
        assert_eq!(p.peak, 3.0);
        assert_eq!(data.keys[1].presses[0].peak, 2.0);
        // depth samples moved to song time, the curve rests at 0
        assert!(data.keys[0].depth.iter().any(|&(t, mm)| (t - 2010.0).abs() < 0.5 && mm == 3.0));
        assert!((data.offset_ms - 30_000.0).abs() < 1.0);
        assert_eq!((data.matched, data.replay_downs), (40, 40));
        let json = data.to_json();
        let back: Data = serde_json::from_str(&json).unwrap();
        assert_eq!(back, data);
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn rate_mods_scale_to_song_time() {
        let d = temp_dir("overlay-dt");
        let bytes = setup(&d, START, 20_000.0, osr::MOD_DOUBLE_TIME);
        let r = osr::parse(&bytes, true).unwrap();
        let (s, a) = find_session(&d, &r).unwrap().expect("aligned");
        let data = build(&s, &r, "x.osr", &a);
        assert_eq!(data.rate, 1.5);
        let p = data.keys[0].presses[0];
        assert!((p.down - 2000.0).abs() < 1.0 && (p.up.unwrap() - 2030.0).abs() < 1.0, "{p:?}");
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn no_session_lines_up() {
        let d = temp_dir("overlay-none");
        // empty dir
        let bytes = setup(&temp_dir("overlay-none-src"), START, 30_000.0, 0);
        let r = osr::parse(&bytes, true).unwrap();
        assert!(find_session(&d, &r).unwrap().is_none());
        // a session with other presses
        let mut w = Writer::create(&d, &header("Other", START)).unwrap();
        for i in 0..50 {
            w.edge(1000.0 + i as f64 * 333.0, "KeyC", Edge::Press).unwrap();
        }
        w.finish().unwrap();
        assert!(find_session(&d, &r).unwrap().is_none());
        // the session started after the replay was stamped
        let late = temp_dir("overlay-late");
        let _ = setup(&late, START + 3_600_000, 30_000.0, 0);
        assert!(find_session(&late, &r).unwrap().is_none());
        // no timestamp
        let frames = encode::frames_text(&[(1000, 1), (1100, 0)]);
        let mut r = osr::parse(&encode::osr(0, 0, 0, &frames), true).unwrap();
        r.timestamp_unix_ms = None;
        assert!(find_session(&d, &r).is_err());
        for x in [d, late, temp_dir("overlay-none-src")] {
            let _ = fs::remove_dir_all(x);
        }
    }

    #[test]
    fn candidates_newest_first_and_bounded() {
        let d = temp_dir("overlay-cands");
        for i in 0..6u64 {
            fs::write(d.join(session::file_name(START + i * 600_000, "m")), b"").unwrap();
        }
        // replay stamped 25 min after START: sessions at 0, 10, 20 min (and none after)
        let c = session_candidates(&d, START as i64 + 25 * 60_000).unwrap();
        assert_eq!(c.len(), 3);
        assert!(c[0].to_string_lossy().contains(&crate::store::timestamp((START + 1_200_000) / 1000)));
        let c = session_candidates(&d, START as i64 + 100 * 60_000).unwrap();
        assert_eq!(c.len(), MAX_SESSIONS_TRIED);
        assert!(session_candidates(&d, 0).unwrap().is_empty());
        fs::remove_dir_all(&d).unwrap();
    }
}

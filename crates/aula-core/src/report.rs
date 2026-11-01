//! Replay reports: per-key press statistics of an aligned replay + session, and a single self-contained HTML
//! file (inline CSS/JS, data as embedded JSON, canvas timeline, zh_TW labels, no network).

use crate::align::{quantile, Alignment};
use crate::osr::{Replay, Span};
use crate::session::Session;
use serde::Serialize;
use std::collections::BTreeSet;

/// Margin around the play in the timeline (ms).
const TIMELINE_MARGIN_MS: f64 = 1500.0;
/// A press counts as bottomed out when its peak is within this of full travel (mm).
pub const BOTTOM_OUT_MARGIN_MM: f64 = 0.1;
/// A press is shallow when its peak stays below actuation point + this (mm).
pub const SHALLOW_MARGIN_MM: f64 = 0.5;
/// Shallow presses listed per key.
const MAX_SHALLOW_LISTED: usize = 200;

/// Everything the HTML shows.
#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub title: String,
    pub session_file: String,
    pub replay_file: String,
    pub player: String,
    pub mode: &'static str,
    pub mods: u32,
    pub rate: f64,
    pub score: i32,
    pub max_combo: u16,
    pub counts: [u16; 6],
    pub replay_unix_ms: Option<i64>,
    pub session_start_unix_ms: u64,
    pub max_mm: f64,
    pub align: AlignSummary,
    pub keys: Vec<KeyReport>,
    /// Song-time range shown first (ms).
    pub view: (f64, f64),
    pub generated_unix_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct AlignSummary {
    pub offset_ms: f64,
    pub matched: usize,
    pub replay_downs: usize,
    pub session_presses: usize,
    pub match_rate: f64,
    pub residual_median: f64,
    pub residual_p95: f64,
    pub unmatched_replay: usize,
    pub unmatched_session: usize,
}

/// One recorded key that played a replay input.
#[derive(Debug, Clone, Serialize)]
pub struct KeyReport {
    pub code: String,
    pub label: String,
    pub input: String,
    pub mode: String,
    pub point: f64,
    pub press: f64,
    pub release: f64,
    pub stats: KeyStats,
    pub shallow: Vec<Shallow>,
    /// Depth samples in the timeline window: song ms and mm (parallel arrays).
    pub t: Vec<f64>,
    pub mm: Vec<f64>,
    /// Replay key-down spans in song ms, and whether a recorded press matched them.
    pub spans: Vec<(f64, f64, bool)>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct KeyStats {
    pub presses: usize,
    pub matched: usize,
    /// Depth when the replay registered the press (mm).
    pub at_down_p10: Option<f64>,
    pub at_down_median: Option<f64>,
    pub at_down_p90: Option<f64>,
    /// Deepest point of each press (mm).
    pub peak_median: Option<f64>,
    pub peak_p10: Option<f64>,
    pub bottom_out_ratio: Option<f64>,
    /// Real ms the replay held the input.
    pub hold_median: Option<f64>,
    pub hold_p10: Option<f64>,
    pub hold_p90: Option<f64>,
    /// Presses that came without the key returning above the actuation point since the previous release.
    pub rt_repress: usize,
    pub shallow: usize,
    /// Presses the travel stream has no samples for (left out of the depth figures).
    pub no_data: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Shallow {
    /// Song ms of the replay key-down.
    pub t: f64,
    pub peak: f64,
}

/// Depth (mm) at session time `t`: the last sample at or before it (the stream only reports movement), 0 before
/// the first.
pub fn depth_at(samples: &[(f64, f64)], t: f64) -> f64 {
    match samples.partition_point(|s| s.0 <= t) {
        0 => 0.0,
        i => samples[i - 1].1,
    }
}

/// Deepest depth during [from, to] (session ms), including the depth already reached at `from`.
pub fn peak(samples: &[(f64, f64)], from: f64, to: f64) -> f64 {
    let start = samples.partition_point(|s| s.0 <= from);
    let end = samples.partition_point(|s| s.0 <= to);
    samples[start..end.max(start)].iter().map(|s| s.1).fold(depth_at(samples, from), f64::max)
}

/// Shallowest depth during [from, to] (session ms), including the depth at `from`.
pub fn lowest(samples: &[(f64, f64)], from: f64, to: f64) -> f64 {
    let start = samples.partition_point(|s| s.0 <= from);
    let end = samples.partition_point(|s| s.0 <= to);
    samples[start..end.max(start)].iter().map(|s| s.1).fold(depth_at(samples, from), f64::min)
}

/// How far the travel stream may trail the HID report the replay recorded (ms). The board sends travel frames
/// every ~1.5–5 ms while a key moves, so at the aligned press instant the newest sample can still be from
/// before the key reached the actuation point.
pub const STREAM_LAG_MS: f64 = 8.0;

/// Depth (mm) when the replay registered a press at session time `t`, allowing for the stream lag: the depth
/// at `t` if it is at or past `point`; else the first sample in (t, t + `STREAM_LAG_MS`] that reaches the point;
/// else the deepest sample in that window (or the depth at `t` when there is none).
pub fn judgement_depth(samples: &[(f64, f64)], t: f64, point: f64) -> f64 {
    let d = depth_at(samples, t);
    if d >= point {
        return d;
    }
    let from = samples.partition_point(|s| s.0 <= t);
    let to = samples.partition_point(|s| s.0 <= t + STREAM_LAG_MS);
    let window = &samples[from..to.max(from)];
    window.iter().find(|s| s.1 >= point).map_or_else(|| window.iter().map(|s| s.1).fold(d, f64::max), |s| s.1)
}

/// Whether any sample lies in (from − `STREAM_LAG_MS`, to + `STREAM_LAG_MS`]: a key that moved has samples there
/// (the stream only reports movement), so a press without any was not seen by the stream at all.
pub fn has_samples(samples: &[(f64, f64)], from: f64, to: f64) -> bool {
    let a = samples.partition_point(|s| s.0 <= from - STREAM_LAG_MS);
    let b = samples.partition_point(|s| s.0 <= to + STREAM_LAG_MS);
    b > a
}

/// Statistics of one key over the replay spans it played (`spans` in session ms, time order; `matched` flags).
/// Presses without travel samples count as `no_data` and are left out of the depth figures. Returns the
/// shallow presses as (span index, peak).
pub fn key_stats(samples: &[(f64, f64)], spans: &[(f64, f64)], matched: &[bool], point: f64, max_mm: f64) -> (KeyStats, Vec<(usize, f64)>) {
    let mut at_down = Vec::new();
    let mut peaks = Vec::new();
    let mut holds = Vec::new();
    let mut shallow = Vec::new();
    let mut rt_repress = 0;
    let mut no_data = 0;
    for (i, &(down, up)) in spans.iter().enumerate() {
        let up = up.max(down);
        holds.push(up - down);
        if !has_samples(samples, down, up) {
            no_data += 1;
            continue;
        }
        at_down.push(judgement_depth(samples, down, point));
        // the stream trails the replay: the press span is extended by the lag
        let p = peak(samples, down, up + STREAM_LAG_MS);
        peaks.push(p);
        if p < point + SHALLOW_MARGIN_MM {
            shallow.push((i, p));
        }
        if i > 0 {
            let prev_up = spans[i - 1].1;
            if prev_up <= down && lowest(samples, prev_up + STREAM_LAG_MS, down + STREAM_LAG_MS) >= point {
                rt_repress += 1;
            }
        }
    }
    let bottomed = peaks.iter().filter(|p| **p >= max_mm - BOTTOM_OUT_MARGIN_MM).count();
    let r2 = |v: Option<f64>| v.map(|x| (x * 100.0).round() / 100.0);
    let r1 = |v: Option<f64>| v.map(|x| (x * 10.0).round() / 10.0);
    let stats = KeyStats {
        presses: spans.len(),
        matched: matched.iter().filter(|m| **m).count(),
        at_down_p10: r2(quantile(&at_down, 0.1)),
        at_down_median: r2(quantile(&at_down, 0.5)),
        at_down_p90: r2(quantile(&at_down, 0.9)),
        peak_median: r2(quantile(&peaks, 0.5)),
        peak_p10: r2(quantile(&peaks, 0.1)),
        bottom_out_ratio: (!peaks.is_empty()).then(|| (bottomed as f64 / peaks.len() as f64 * 1000.0).round() / 1000.0),
        hold_median: r1(quantile(&holds, 0.5)),
        hold_p10: r1(quantile(&holds, 0.1)),
        hold_p90: r1(quantile(&holds, 0.9)),
        rt_repress,
        shallow: shallow.len(),
        no_data,
    };
    (stats, shallow)
}

/// Builds the report of `replay` (its `spans`) played in `session`, aligned by `a`.
pub fn build(session: &Session, replay: &Replay, replay_file: &str, spans: &[Span], a: &Alignment, now_unix_ms: u64) -> Report {
    let song = |t: f64| (t - a.offset_ms) * a.rate;
    let matched: BTreeSet<usize> = a.pairs.iter().map(|p| p.span).collect();
    let (first, last) = spans.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(f, l), s| (f.min(s.down), l.max(s.up)));
    let (first, last) = if first.is_finite() { (first, last) } else { (0.0, 0.0) };
    let from = a.to_session(first) - TIMELINE_MARGIN_MS;
    let to = a.to_session(last) + TIMELINE_MARGIN_MS;
    let max_mm = session.header.max_mm;
    let mut keys = Vec::new();
    for (&input, code) in &a.mapping {
        let Some(k) = session.key(code) else { continue };
        let samples: &[(f64, f64)] = session.depth.get(code).map_or(&[], Vec::as_slice);
        let mine: Vec<(usize, &Span)> = spans.iter().enumerate().filter(|(_, s)| s.input == input).collect();
        let sess_spans: Vec<(f64, f64)> = mine.iter().map(|(_, s)| (a.to_session(s.down), a.to_session(s.up))).collect();
        let flags: Vec<bool> = mine.iter().map(|(n, _)| matched.contains(n)).collect();
        let (stats, shallow) = key_stats(samples, &sess_spans, &flags, k.point, max_mm);
        let lo = samples.partition_point(|s| s.0 < from);
        let hi = samples.partition_point(|s| s.0 <= to);
        // the depth at the window start, so the curve does not start mid-air
        let mut t = vec![(song(from) * 10.0).round() / 10.0];
        let mut mm = vec![depth_at(samples, from)];
        for s in &samples[lo..hi] {
            t.push((song(s.0) * 10.0).round() / 10.0);
            mm.push((s.1 * 1000.0).round() / 1000.0);
        }
        keys.push(KeyReport {
            code: k.code.clone(),
            label: k.label.clone(),
            input: replay.input_name(input),
            mode: k.mode.clone(),
            point: k.point,
            press: k.press,
            release: k.release,
            shallow: shallow
                .into_iter()
                .take(MAX_SHALLOW_LISTED)
                .map(|(i, p)| Shallow { t: mine[i].1.down.round(), peak: (p * 100.0).round() / 100.0 })
                .collect(),
            spans: mine.iter().zip(&flags).map(|((_, s), m)| (s.down, s.up, *m)).collect(),
            stats,
            t,
            mm,
        });
    }
    let c = replay.counts;
    Report {
        title: if session.header.label.is_empty() { replay_file.to_string() } else { session.header.label.clone() },
        session_file: session.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        replay_file: replay_file.to_string(),
        player: replay.player.clone(),
        mode: replay.mode.name(),
        mods: replay.mods,
        rate: a.rate,
        score: replay.score,
        max_combo: replay.max_combo,
        counts: [c.n300, c.n100, c.n50, c.geki, c.katu, c.miss],
        replay_unix_ms: replay.timestamp_unix_ms,
        session_start_unix_ms: session.header.start_unix_ms,
        max_mm,
        align: AlignSummary {
            offset_ms: (a.offset_ms * 10.0).round() / 10.0,
            matched: a.pairs.len(),
            replay_downs: a.replay_downs,
            session_presses: a.session_presses,
            match_rate: (a.match_rate * 1000.0).round() / 1000.0,
            residual_median: (a.residual_median * 100.0).round() / 100.0,
            residual_p95: (a.residual_p95 * 100.0).round() / 100.0,
            unmatched_replay: a.unmatched_replay(),
            unmatched_session: a.unmatched_session(),
        },
        keys,
        view: (first - TIMELINE_MARGIN_MS * a.rate, last + TIMELINE_MARGIN_MS * a.rate),
        generated_unix_ms: now_unix_ms,
    }
}

/// The report as one HTML document. The data is embedded as JSON with `<` escaped, so no label can close the
/// script element; the page only sets text through `textContent`.
pub fn html(r: &Report) -> String {
    let json = serde_json::to_string(r).unwrap_or_else(|_| "{}".into()).replace('<', "\\u003c");
    TEMPLATE.replace("/*DATA*/", &json)
}

const TEMPLATE: &str = include_str!("report.html");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::align::{align, Params};
    use crate::osr::{encode, parse};
    use crate::session::{Header, SessionKey, SCHEMA};
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    #[test]
    fn depth_lookups() {
        let s = [(10.0, 0.5), (20.0, 2.0), (30.0, 3.4), (40.0, 1.0), (50.0, 0.0)];
        assert_eq!(depth_at(&s, 5.0), 0.0);
        assert_eq!(depth_at(&s, 10.0), 0.5);
        assert_eq!(depth_at(&s, 25.0), 2.0);
        assert_eq!(depth_at(&s, 99.0), 0.0);
        assert_eq!(depth_at(&[], 1.0), 0.0);
        assert_eq!(peak(&s, 15.0, 45.0), 3.4);
        assert_eq!(peak(&s, 41.0, 49.0), 1.0, "no sample inside: the depth held at the start");
        assert_eq!(peak(&s, 45.0, 41.0), 1.0, "reversed range");
        assert_eq!(lowest(&s, 20.0, 45.0), 1.0);
        assert_eq!(lowest(&[], 0.0, 10.0), 0.0);
    }

    #[test]
    fn stats_of_a_key() {
        // press 1: to the bottom; press 2: shallow re-press without rising above the point (1.0); press 3 after
        // returning to rest
        let s = [
            (0.0, 0.3),
            (5.0, 1.2),
            (10.0, 3.35),
            (40.0, 1.5),
            (45.0, 1.1),
            (50.0, 1.3),
            (60.0, 1.2),
            (80.0, 0.0),
            (100.0, 1.0),
            (105.0, 2.0),
            (130.0, 0.0),
        ];
        let spans = [(5.0, 42.0), (50.0, 75.0), (105.0, 125.0)];
        let (st, shallow) = key_stats(&s, &spans, &[true, true, false], 1.0, 3.4);
        assert_eq!(st.presses, 3);
        assert_eq!(st.matched, 2);
        assert_eq!(st.at_down_median, Some(1.3));
        assert_eq!((st.at_down_p10, st.at_down_p90), (Some(1.2), Some(2.0)));
        assert_eq!(st.peak_median, Some(2.0));
        assert_eq!(st.bottom_out_ratio, Some(0.333));
        assert_eq!(st.hold_median, Some(25.0));
        assert_eq!(st.rt_repress, 1);
        assert_eq!(st.shallow, 1);
        assert_eq!(shallow, vec![(1, 1.3)]);
        let (empty, none) = key_stats(&s, &[], &[], 1.0, 3.4);
        assert_eq!(empty, KeyStats::default());
        assert!(none.is_empty());
        let (no_depth, shallow) = key_stats(&[], &spans, &[false; 3], 1.0, 3.4);
        assert_eq!((no_depth.no_data, no_depth.presses), (3, 3));
        assert_eq!((no_depth.peak_median, no_depth.at_down_median), (None, None), "no data is not 0.00 mm");
        assert_eq!(no_depth.shallow, 0);
        assert!(shallow.is_empty());
        assert_eq!(no_depth.hold_median, Some(25.0), "hold times come from the replay");
    }

    /// The travel stream trails the HID report: at the aligned press instant the newest sample is still the
    /// key at rest.
    #[test]
    fn lagged_stream() {
        // press registered at 100 (point 1.0); samples arrive 3 ms later
        let s = [(103.0, 0.6), (105.0, 1.4), (108.0, 2.5), (140.0, 1.0), (146.0, 0.0)];
        assert_eq!(depth_at(&s, 100.0), 0.0);
        assert_eq!(judgement_depth(&s, 100.0, 1.0), 1.4, "first sample past the point within the lag");
        assert_eq!(judgement_depth(&s, 106.0, 1.0), 1.4, "already past the point at t");
        assert_eq!(judgement_depth(&s, 100.0, 3.0), 2.5, "never reaches the point: deepest in the window");
        assert_eq!(judgement_depth(&s, 120.0, 3.0), 2.5, "nothing in the window: depth at t");
        assert_eq!(judgement_depth(&[], 1.0, 1.0), 0.0);
        assert!(has_samples(&s, 100.0, 130.0));
        assert!(has_samples(&s, 150.0, 160.0), "the last sample within the lag before the press");
        assert!(!has_samples(&s, 160.0, 170.0));
        assert!(!has_samples(&[], 0.0, 1.0));
        // replay span 100..137, release seen by the stream 3 ms late; the peak counts the lagged samples
        let spans = [(100.0, 137.0), (300.0, 330.0)];
        let (st, shallow) = key_stats(&s, &spans, &[true, true], 1.0, 3.4);
        assert_eq!(st.at_down_median, Some(1.4));
        assert_eq!(st.at_down_p10, Some(1.4), "no 0.00 from the lag");
        assert_eq!(st.peak_median, Some(2.5));
        assert_eq!(st.no_data, 1, "the second press has no samples");
        assert!(shallow.is_empty());
        // a real shallow press, peaking just after its replay release
        let s = [(98.0, 0.5), (101.0, 1.1), (131.0, 1.3), (134.0, 0.0)];
        let (st, shallow) = key_stats(&s, &[(100.0, 128.0)], &[true], 1.0, 3.4);
        assert_eq!(shallow, vec![(0, 1.3)]);
        assert_eq!(st.at_down_median, Some(1.1));
    }

    fn header() -> Header {
        let key = |code: &str, label: &str, idx| SessionKey {
            code: code.into(),
            label: label.into(),
            idx,
            mode: "rt".into(),
            point: 1.0,
            press: 0.3,
            release: 0.3,
        };
        Header {
            schema: SCHEMA,
            label: "Artist - Title (mapper) [Diff] </script><b>".into(),
            start_unix_ms: 1_793_491_200_000,
            start_mono_ms: 0.0,
            product: "SI2825KZHEARGB".into(),
            firmware: "3.17.07".into(),
            max_mm: 3.4,
            keys: vec![key("KeyZ", "Z", 89), key("KeyX", "X", 90)],
        }
    }

    /// A replay of 40 alternating taps and a session in which they were played 5 s after the session start.
    pub(crate) fn fixture() -> (Session, Replay) {
        let mut frames = Vec::new();
        for i in 0..40 {
            let t = 1000 + i * 150;
            frames.push((t, if i % 2 == 0 { 5 } else { 10 }));
            frames.push((t + 40, 0));
        }
        let replay = parse(&encode::osr(0, 0, 1_793_491_260_000, &encode::frames_text(&frames)), true).unwrap();
        let mut depth: BTreeMap<String, Vec<(f64, f64)>> = BTreeMap::new();
        let mut presses: BTreeMap<String, Vec<(f64, Option<f64>)>> = BTreeMap::new();
        for i in 0..40 {
            let key = if i % 2 == 0 { "KeyZ" } else { "KeyX" };
            let t = 5000.0 + 1000.0 + i as f64 * 150.0 + 1.5;
            let d = depth.entry(key.into()).or_default();
            d.extend([(t - 3.0, 0.6), (t, 1.2), (t + 10.0, 3.4), (t + 40.0, 0.4), (t + 45.0, 0.0)]);
            presses.entry(key.into()).or_default().push((t, Some(t + 40.0)));
        }
        let session = Session {
            path: PathBuf::from("/x/20261101-000000-a.ndjson.gz"),
            header: header(),
            depth,
            presses,
            end_ms: 13_000.0,
            truncated: false,
        };
        (session, replay)
    }

    /// Writes a demo report from a synthetic play (streams, shallow taps, re-presses that stay below the point)
    /// to `$AULA_SAMPLE_REPORT`: `AULA_SAMPLE_REPORT=/tmp/x.html cargo test -p aula-core sample_report -- --ignored`.
    #[test]
    #[ignore]
    fn sample_report() {
        let Some(out) = std::env::var_os("AULA_SAMPLE_REPORT") else { return };
        let mut seed = 99u64;
        let mut rnd = || {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
            (seed >> 11) as f64 / (1u64 << 53) as f64
        };
        let mut frames = Vec::new();
        let mut depth: BTreeMap<String, Vec<(f64, f64)>> = BTreeMap::new();
        let mut presses: BTreeMap<String, Vec<(f64, Option<f64>)>> = BTreeMap::new();
        let offset = 20_000.0;
        let mut t = 1500.0;
        for i in 0..500 {
            // bursts of 16th-note streams with pauses
            t += if i % 24 < 16 { 90.0 } else { 180.0 + (rnd() * 200.0).round() };
            let input = i % 2;
            let hold = 35.0 + (rnd() * 25.0).round();
            frames.push((t as i64, if input == 0 { 5 } else { 10 }));
            frames.push((t as i64 + hold as i64, 0));
            let key = if input == 0 { "KeyZ" } else { "KeyX" };
            let st = offset + t + 2.0 + rnd() * 2.0;
            let peak = if rnd() < 0.1 { 1.1 + rnd() * 0.3 } else { 2.2 + rnd() * 1.2 };
            let peak = peak.min(3.4);
            // released to just below the point in streams sometimes (re-press without a full return)
            let rest = if i % 24 < 16 && rnd() < 0.3 { 1.05 } else { 0.0 };
            let d = depth.entry(key.into()).or_default();
            let start = d.last().map_or(0.0, |s: &(f64, f64)| s.1);
            for k in 0..6 {
                let f = (k + 1) as f64 / 6.0;
                d.push((st - 6.0 + k as f64 * 2.0, start + (peak - start) * f));
            }
            for k in 0..5 {
                let f = (k + 1) as f64 / 5.0;
                d.push((st + hold - 4.0 + k as f64 * 2.5, peak + (rest - peak) * f));
            }
            presses.entry(key.into()).or_default().push((st - 2.0, Some(st + hold)));
        }
        let replay = parse(&encode::osr(0, 0, 1_793_491_260_000, &encode::frames_text(&frames)), true).unwrap();
        let session = Session {
            path: PathBuf::from("/x/20261101-000000-sample.ndjson.gz"),
            header: Header { label: "Sample Artist - Sample Song (mapper) [Insane]".into(), ..header() },
            depth,
            presses,
            end_ms: offset + t + 2000.0,
            truncated: false,
        };
        let spans = replay.spans().unwrap();
        let p = session.presses.iter().map(|(k, v)| (k.clone(), v.iter().map(|p| p.0).collect())).collect();
        let a = align(&spans, 2, replay.rate(), &p, &Params::default()).unwrap();
        let r = build(&session, &replay, "tester playing Sample Artist - Sample Song (mapper) [Insane] (2026-11-01_08-01).osr", &spans, &a, 1_793_491_300_000);
        std::fs::write(&out, html(&r)).unwrap();
    }

    #[test]
    fn report_of_an_aligned_play() {
        let (session, replay) = fixture();
        let spans = replay.spans().unwrap();
        let presses = session.presses.iter().map(|(k, v)| (k.clone(), v.iter().map(|p| p.0).collect())).collect();
        let a = align(&spans, 2, replay.rate(), &presses, &Params::default()).unwrap();
        assert!((a.offset_ms - 5001.5).abs() < 0.01, "{}", a.offset_ms);
        let r = build(&session, &replay, "x.osr", &spans, &a, 1);
        assert_eq!(r.keys.len(), 2);
        let z = r.keys.iter().find(|k| k.code == "KeyZ").unwrap();
        assert_eq!(z.input, "K1/M1");
        assert_eq!(z.stats.presses, 20);
        assert_eq!(z.stats.matched, 20);
        assert_eq!(z.stats.at_down_median, Some(1.2));
        assert_eq!(z.stats.bottom_out_ratio, Some(1.0));
        assert_eq!(z.stats.hold_median, Some(40.0));
        assert_eq!(z.stats.rt_repress, 0);
        assert!(z.shallow.is_empty());
        assert_eq!(z.t.len(), z.mm.len());
        assert_eq!(z.mm[0], 0.0);
        assert!((z.t[1] - 997.0).abs() < 0.2, "song time of the first sample: {}", z.t[1]);
        assert_eq!(z.spans.len(), 20);
        assert_eq!(r.align.matched, 40);
        assert_eq!(r.counts, [300, 20, 3, 50, 10, 2]);
        let page = html(&r);
        assert!(page.starts_with("<!doctype html>"));
        assert!(!page.contains("</script><b>"), "label cannot close the script element");
        assert!(page.contains("\\u003c/script>\\u003cb>"));
        assert!(!page.contains("/*DATA*/"));
        assert!(!page.contains("http://") && !page.contains("https://"), "no network references");
    }
}

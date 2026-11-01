//! Aligning an osu! replay with a recorded session (pure): the time offset between the replay's song clock and
//! the session clock, and which recorded key played which replay input.
//!
//! Session time of a replay event at song time `r`: `offset + r / rate` (rate 1.5 with DT, 0.75 with HT).
//! Offsets are found by voting over (session press − replay key-down) differences, rescored by matching each
//! replay key-down to the nearest session press of the mapped key within `tolerance_ms`.

use crate::osr::Span;
use std::collections::{BTreeMap, HashMap};
use std::fmt;

/// Alignment settings.
#[derive(Debug, Clone, PartialEq)]
pub struct Params {
    /// Largest |residual| of a matched press.
    pub tolerance_ms: f64,
    /// Smallest share of replay key-downs that must match.
    pub min_rate: f64,
    pub min_matches: usize,
    /// Offsets to search (ms), e.g. from the replay's wall-clock timestamp; None = any.
    pub window: Option<(f64, f64)>,
}

impl Default for Params {
    fn default() -> Self {
        Params { tolerance_ms: 6.0, min_rate: 0.6, min_matches: 10, window: None }
    }
}

/// Replay key-downs used to vote for offsets (spread over the replay).
const MAX_ANCHORS: usize = 400;
/// Vote bin width (ms).
const BIN_MS: f64 = 2.0;
/// Offset candidates rescored.
const CANDIDATES: usize = 12;

/// One replay key-down matched to a session press.
#[derive(Debug, Clone, PartialEq)]
pub struct Pair {
    pub input: usize,
    pub key: String,
    /// Index into the replay's spans.
    pub span: usize,
    /// Session time of the replay key-down (offset applied).
    pub replay_t: f64,
    /// Session time of the recorded press.
    pub session_t: f64,
}

impl Pair {
    pub fn residual(&self) -> f64 {
        self.session_t - self.replay_t
    }
}

/// A successful alignment.
#[derive(Debug, Clone, PartialEq)]
pub struct Alignment {
    pub offset_ms: f64,
    pub rate: f64,
    /// Input index -> recorded key code (inputs without a key are not listed).
    pub mapping: BTreeMap<usize, String>,
    pub pairs: Vec<Pair>,
    /// Replay key-downs (all inputs).
    pub replay_downs: usize,
    /// Session presses of the mapped keys during the play.
    pub session_presses: usize,
    pub match_rate: f64,
    /// |residual| median and 95th percentile (ms).
    pub residual_median: f64,
    pub residual_p95: f64,
    /// Median signed residual (session press − replay key-down, after the offset; ~0 by construction).
    pub residual_bias: f64,
}

impl Alignment {
    pub fn unmatched_replay(&self) -> usize {
        self.replay_downs - self.pairs.len()
    }

    pub fn unmatched_session(&self) -> usize {
        self.session_presses.saturating_sub(self.pairs.len())
    }

    /// Session time of song time `r`.
    pub fn to_session(&self, r: f64) -> f64 {
        self.offset_ms + r / self.rate
    }
}

/// Why no alignment was found.
#[derive(Debug, Clone, PartialEq)]
pub enum Refusal {
    NoReplayInput,
    NoSessionPresses,
    /// The best candidate, too weak to trust.
    TooFew { matched: usize, replay_downs: usize, rate: f64, offset_ms: f64 },
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Refusal::NoReplayInput => f.write_str("the replay has no key presses"),
            Refusal::NoSessionPresses => f.write_str("the session has no presses (in the searched time range)"),
            Refusal::TooFew { matched, replay_downs, rate, .. } => write!(
                f,
                "the replay does not line up with this session: best match {matched} of {replay_downs} key presses \
                 ({:.0} %); is it the right session, were the replay's keys recorded, or was the game paused?",
                rate * 100.0
            ),
        }
    }
}

/// p-quantile (0..=1, nearest rank) of unsorted values; None when empty.
pub fn quantile(values: &[f64], p: f64) -> Option<f64> {
    let mut v: Vec<f64> = values.iter().copied().filter(|x| x.is_finite()).collect();
    if v.is_empty() {
        return None;
    }
    v.sort_by(f64::total_cmp);
    let i = ((v.len() - 1) as f64 * p.clamp(0.0, 1.0)).round() as usize;
    Some(v[i])
}

/// Greedy in-order matching of sorted `a` to sorted `b` within `tol`: pairs of indices.
fn match_sorted(a: &[f64], b: &[f64], tol: f64) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        let d = b[j] - a[i];
        if d < -tol {
            j += 1;
        } else if d > tol {
            i += 1;
        } else {
            // take the closer of b[j] and b[j+1] for a[i]
            if j + 1 < b.len() && (b[j + 1] - a[i]).abs() < d.abs() {
                j += 1;
                continue;
            }
            out.push((i, j));
            i += 1;
            j += 1;
        }
    }
    out
}

struct Scored {
    offset: f64,
    mapping: BTreeMap<usize, String>,
    pairs: Vec<Pair>,
}

/// Aligns replay `spans` (song ms, `inputs` inputs, song `rate`) with session `presses` (key code -> press
/// times in session ms, sorted).
pub fn align(
    spans: &[Span],
    inputs: usize,
    rate: f64,
    presses: &BTreeMap<String, Vec<f64>>,
    params: &Params,
) -> Result<Alignment, Refusal> {
    let rate = if rate.is_finite() && rate > 0.0 { rate } else { 1.0 };
    // replay key-downs per input, in real ms from song time 0
    let mut downs: Vec<Vec<(f64, usize)>> = vec![Vec::new(); inputs];
    for (n, s) in spans.iter().enumerate() {
        if let Some(d) = downs.get_mut(s.input) {
            d.push((s.down / rate, n));
        }
    }
    for d in &mut downs {
        d.sort_by(|a, b| a.0.total_cmp(&b.0));
    }
    let replay_downs: usize = downs.iter().map(Vec::len).sum();
    if replay_downs == 0 {
        return Err(Refusal::NoReplayInput);
    }
    let mut all: Vec<f64> = presses.values().flatten().copied().filter(|t| t.is_finite()).collect();
    all.sort_by(f64::total_cmp);
    if all.is_empty() {
        return Err(Refusal::NoSessionPresses);
    }
    let mut anchors: Vec<f64> = downs.iter().flatten().map(|d| d.0).collect();
    anchors.sort_by(f64::total_cmp);
    if anchors.len() > MAX_ANCHORS {
        let step = anchors.len() as f64 / MAX_ANCHORS as f64;
        anchors = (0..MAX_ANCHORS).map(|i| anchors[(i as f64 * step) as usize]).collect();
    }
    let (lo, hi) = params.window.unwrap_or((f64::NEG_INFINITY, f64::INFINITY));
    // votes per bin
    let mut votes: HashMap<i64, u32> = HashMap::new();
    for &a in &anchors {
        let from = all.partition_point(|&s| s - a < lo);
        for &s in &all[from..] {
            let d = s - a;
            if d > hi {
                break;
            }
            *votes.entry((d / BIN_MS).floor() as i64).or_default() += 1;
        }
    }
    if votes.is_empty() {
        return Err(Refusal::NoSessionPresses);
    }
    // smoothed over ± tolerance
    let reach = (params.tolerance_ms / BIN_MS).ceil() as i64;
    let mut smoothed: Vec<(i64, u32)> = votes
        .keys()
        .map(|&b| (b, (b - reach..=b + reach).map(|x| votes.get(&x).copied().unwrap_or(0)).sum()))
        .collect();
    smoothed.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let mut candidates: Vec<f64> = Vec::new();
    for (b, _) in smoothed {
        let off = (b as f64 + 0.5) * BIN_MS;
        if candidates.iter().all(|c| (c - off).abs() > 2.0 * params.tolerance_ms) {
            candidates.push(off);
            if candidates.len() >= CANDIDATES {
                break;
            }
        }
    }
    let mut best: Option<Scored> = None;
    for c in candidates {
        let mut s = score(&downs, presses, c, params.tolerance_ms);
        // refine to the median residual and rescore
        let res: Vec<f64> = s.pairs.iter().map(Pair::residual).collect();
        if let Some(m) = quantile(&res, 0.5) {
            let r = score(&downs, presses, c + m, params.tolerance_ms);
            if r.pairs.len() >= s.pairs.len() {
                s = r;
            }
        }
        if best.as_ref().is_none_or(|b| s.pairs.len() > b.pairs.len()) {
            best = Some(s);
        }
    }
    let Some(best) = best else { return Err(Refusal::NoSessionPresses) };
    let matched = best.pairs.len();
    let match_rate = matched as f64 / replay_downs as f64;
    if matched < params.min_matches || match_rate < params.min_rate {
        return Err(Refusal::TooFew { matched, replay_downs, rate: match_rate, offset_ms: best.offset });
    }
    // session presses of the mapped keys during the play
    let first = downs.iter().filter_map(|d| d.first()).map(|d| d.0).fold(f64::INFINITY, f64::min) + best.offset;
    let last = downs.iter().filter_map(|d| d.last()).map(|d| d.0).fold(f64::NEG_INFINITY, f64::max) + best.offset;
    let session_presses = best
        .mapping
        .values()
        .filter_map(|k| presses.get(k))
        .map(|v| v.iter().filter(|t| **t >= first - params.tolerance_ms && **t <= last + params.tolerance_ms).count())
        .sum();
    let res: Vec<f64> = best.pairs.iter().map(Pair::residual).collect();
    let abs: Vec<f64> = res.iter().map(|r| r.abs()).collect();
    let mut pairs = best.pairs;
    pairs.sort_by(|a, b| a.replay_t.total_cmp(&b.replay_t));
    Ok(Alignment {
        offset_ms: best.offset,
        rate,
        mapping: best.mapping,
        replay_downs,
        session_presses,
        match_rate,
        residual_median: quantile(&abs, 0.5).unwrap_or(0.0),
        residual_p95: quantile(&abs, 0.95).unwrap_or(0.0),
        residual_bias: quantile(&res, 0.5).unwrap_or(0.0),
        pairs,
    })
}

/// Matches at `offset` for every (input, key) and picks the best injective input -> key mapping.
fn score(downs: &[Vec<(f64, usize)>], presses: &BTreeMap<String, Vec<f64>>, offset: f64, tol: f64) -> Scored {
    let keys: Vec<&String> = presses.keys().collect();
    // pairs[i][k]
    let table: Vec<Vec<Vec<Pair>>> = downs
        .iter()
        .enumerate()
        .map(|(input, d)| {
            let shifted: Vec<f64> = d.iter().map(|x| x.0 + offset).collect();
            keys.iter()
                .map(|k| {
                    let s = &presses[*k];
                    match_sorted(&shifted, s, tol)
                        .into_iter()
                        .map(|(i, j)| Pair { input, key: (*k).clone(), span: d[i].1, replay_t: shifted[i], session_t: s[j] })
                        .collect()
                })
                .collect()
        })
        .collect();
    let counts: Vec<Vec<usize>> = table.iter().map(|r| r.iter().map(Vec::len).collect()).collect();
    let assign = best_assignment(&counts, keys.len());
    let mut mapping = BTreeMap::new();
    let mut pairs = Vec::new();
    for (i, k) in assign.into_iter().enumerate() {
        if let Some(k) = k {
            if counts[i][k] > 0 {
                mapping.insert(i, keys[k].clone());
                pairs.extend(table[i][k].iter().cloned());
            }
        }
    }
    Scored { offset, mapping, pairs }
}

/// Input -> key (or none), each key used once, maximizing the summed counts. Exhaustive for up to 3 inputs,
/// greedy (largest count first) beyond.
fn best_assignment(counts: &[Vec<usize>], keys: usize) -> Vec<Option<usize>> {
    let n = counts.len();
    if n <= 3 && keys <= 64 {
        fn go(i: usize, counts: &[Vec<usize>], keys: usize, used: &mut Vec<bool>, cur: &mut Vec<Option<usize>>, best: &mut (usize, Vec<Option<usize>>), sum: usize) {
            if i == counts.len() {
                if sum > best.0 || best.1.is_empty() {
                    *best = (sum, cur.clone());
                }
                return;
            }
            cur.push(None);
            go(i + 1, counts, keys, used, cur, best, sum);
            cur.pop();
            for k in 0..keys {
                if !used[k] && counts[i][k] > 0 {
                    used[k] = true;
                    cur.push(Some(k));
                    go(i + 1, counts, keys, used, cur, best, sum + counts[i][k]);
                    cur.pop();
                    used[k] = false;
                }
            }
        }
        let mut best = (0, Vec::new());
        go(0, counts, keys, &mut vec![false; keys], &mut Vec::new(), &mut best, 0);
        return best.1;
    }
    let mut cells: Vec<(usize, usize, usize)> =
        counts.iter().enumerate().flat_map(|(i, r)| r.iter().enumerate().map(move |(k, c)| (*c, i, k))).collect();
    cells.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
    let mut out = vec![None; n];
    let mut used = vec![false; keys];
    for (c, i, k) in cells {
        if c > 0 && out[i].is_none() && !used[k] {
            out[i] = Some(k);
            used[k] = true;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Deterministic pseudo-random numbers in [0, 1).
    struct Lcg(u64);
    impl Lcg {
        fn next(&mut self) -> f64 {
            self.0 = self.0.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
            (self.0 >> 11) as f64 / (1u64 << 53) as f64
        }
    }

    /// A synthetic standard replay: `n` alternating presses on inputs 0/1 with irregular gaps (song ms).
    fn replay(n: usize, seed: u64) -> Vec<Span> {
        let mut rng = Lcg(seed);
        let mut t = 1000.0;
        (0..n)
            .map(|i| {
                t += 60.0 + (rng.next() * 200.0).floor();
                Span { input: i % 2, down: t, up: t + 30.0 }
            })
            .collect()
    }

    /// The session presses a player of `spans` would leave: input 0 on `k0`, 1 on `k1`, at `offset + r/rate`
    /// with ± `jitter` ms, dropping every `drop_every`-th press.
    #[allow(clippy::too_many_arguments)]
    fn session(spans: &[Span], rate: f64, offset: f64, jitter: f64, drop_every: usize, k0: &str, k1: &str, seed: u64) -> BTreeMap<String, Vec<f64>> {
        let mut rng = Lcg(seed);
        let mut m: BTreeMap<String, Vec<f64>> = BTreeMap::new();
        for (n, s) in spans.iter().enumerate() {
            let j = (rng.next() * 2.0 - 1.0) * jitter;
            if drop_every > 0 && n % drop_every == drop_every - 1 {
                continue;
            }
            let k = if s.input == 0 { k0 } else { k1 };
            m.entry(k.to_string()).or_default().push(offset + s.down / rate + j);
        }
        for v in m.values_mut() {
            v.sort_by(f64::total_cmp);
        }
        m
    }

    fn merge(a: BTreeMap<String, Vec<f64>>, b: BTreeMap<String, Vec<f64>>) -> BTreeMap<String, Vec<f64>> {
        let mut out = a;
        for (k, v) in b {
            out.entry(k).or_default().extend(v);
        }
        for v in out.values_mut() {
            v.sort_by(f64::total_cmp);
        }
        out
    }

    #[test]
    fn finds_offset_and_mapping_with_jitter() {
        let r = replay(300, 1);
        let s = session(&r, 1.0, 12_345.6, 3.0, 0, "KeyZ", "KeyX", 2);
        let a = align(&r, 2, 1.0, &s, &Params::default()).unwrap();
        assert!((a.offset_ms - 12_345.6).abs() < 2.0, "{}", a.offset_ms);
        assert_eq!(a.mapping, BTreeMap::from([(0, "KeyZ".to_string()), (1, "KeyX".to_string())]));
        assert_eq!(a.pairs.len(), 300);
        assert_eq!(a.match_rate, 1.0);
        assert!(a.residual_median <= 3.0 && a.residual_p95 <= 5.0, "{a:?}");
        assert_eq!((a.unmatched_replay(), a.unmatched_session()), (0, 0));
        assert!((a.to_session(1000.0) - (a.offset_ms + 1000.0)).abs() < 1e-9);
        assert!(a.pairs.windows(2).all(|w| w[0].replay_t <= w[1].replay_t));
    }

    #[test]
    fn swapped_keys_and_double_time() {
        let r = replay(200, 3);
        let s = session(&r, 1.5, -400.0, 2.0, 0, "KeyX", "KeyZ", 4);
        let a = align(&r, 2, 1.5, &s, &Params::default()).unwrap();
        assert_eq!(a.mapping[&0], "KeyX");
        assert_eq!(a.mapping[&1], "KeyZ");
        assert!((a.offset_ms + 400.0).abs() < 2.0);
        assert_eq!(a.rate, 1.5);
    }

    #[test]
    fn missing_presses_and_extra_keys() {
        let r = replay(300, 5);
        let mut s = session(&r, 1.0, 5000.0, 2.0, 5, "KeyZ", "KeyX", 6);
        // an unrelated key typed during the play
        s.insert("KeyC".into(), (0..100).map(|i| 5000.0 + i as f64 * 397.0).collect());
        let a = align(&r, 2, 1.0, &s, &Params::default()).unwrap();
        assert_eq!(a.pairs.len(), 240);
        assert!((a.match_rate - 0.8).abs() < 1e-9);
        assert_eq!(a.unmatched_replay(), 60);
        assert!(!a.mapping.values().any(|k| k == "KeyC"));
    }

    #[test]
    fn picks_the_right_play_among_retries() {
        let r = replay(250, 7);
        // two failed attempts (another player's timing: the same map with different jitter and missing chunks)
        // and the real play
        let fail1 = session(&r[..80], 1.0, 10_000.0, 25.0, 0, "KeyZ", "KeyX", 8);
        let fail2 = session(&r[..150], 1.0, 60_000.0, 25.0, 0, "KeyZ", "KeyX", 9);
        let real = session(&r, 1.0, 120_000.0, 2.0, 0, "KeyZ", "KeyX", 10);
        let s = merge(merge(fail1, fail2), real);
        let a = align(&r, 2, 1.0, &s, &Params::default()).unwrap();
        assert!((a.offset_ms - 120_000.0).abs() < 2.0, "{}", a.offset_ms);
        assert_eq!(a.pairs.len(), 250);
        // a wall-clock window around the first attempt finds only that attempt: too weak, refused
        let p = Params { window: Some((5_000.0, 15_000.0)), ..Params::default() };
        match align(&r, 2, 1.0, &s, &p) {
            Err(Refusal::TooFew { offset_ms, matched, .. }) => {
                assert!((offset_ms - 10_000.0).abs() < 30.0, "{offset_ms}");
                assert!(matched < 150);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn refuses_without_overlap() {
        let r = replay(100, 11);
        let other = replay(100, 12);
        let s = session(&other, 1.0, 0.0, 0.0, 0, "KeyZ", "KeyX", 13);
        let e = align(&r, 2, 1.0, &s, &Params::default()).unwrap_err();
        assert!(matches!(e, Refusal::TooFew { .. }), "{e:?}");
        assert!(e.to_string().contains("does not line up"), "{e}");
        assert_eq!(align(&[], 2, 1.0, &s, &Params::default()), Err(Refusal::NoReplayInput));
        assert_eq!(align(&r, 2, 1.0, &BTreeMap::new(), &Params::default()), Err(Refusal::NoSessionPresses));
        let empty_key = BTreeMap::from([("KeyZ".to_string(), vec![])]);
        assert_eq!(align(&r, 2, 1.0, &empty_key, &Params::default()), Err(Refusal::NoSessionPresses));
        // a window with no session press in reach
        let s = session(&r, 1.0, 0.0, 0.0, 0, "KeyZ", "KeyX", 13);
        let p = Params { window: Some((1e9, 2e9)), ..Params::default() };
        assert_eq!(align(&r, 2, 1.0, &s, &p), Err(Refusal::NoSessionPresses));
        // too few presses in total
        let short = replay(5, 14);
        let s = session(&short, 1.0, 0.0, 0.0, 0, "KeyZ", "KeyX", 15);
        assert!(matches!(align(&short, 2, 1.0, &s, &Params::default()), Err(Refusal::TooFew { matched: 5, .. })));
    }

    #[test]
    fn one_input_on_the_mouse() {
        // input 0 played with the mouse (not recorded): half the presses cannot match
        let r = replay(200, 16);
        let mut s = session(&r, 1.0, 777.0, 1.0, 0, "KeyZ", "KeyX", 17);
        s.remove("KeyZ");
        let e = align(&r, 2, 1.0, &s, &Params::default()).unwrap_err();
        assert!(matches!(e, Refusal::TooFew { matched: 100, replay_downs: 200, .. }), "{e:?}");
        let p = Params { min_rate: 0.4, ..Params::default() };
        let a = align(&r, 2, 1.0, &s, &p).unwrap();
        assert_eq!(a.mapping, BTreeMap::from([(1, "KeyX".to_string())]));
    }

    #[test]
    fn mania_many_columns() {
        let mut rng = Lcg(18);
        let mut t = 0.0;
        let spans: Vec<Span> = (0..400)
            .map(|_| {
                t += 40.0 + (rng.next() * 150.0).floor();
                Span { input: (rng.next() * 7.0) as usize, down: t, up: t + 20.0 }
            })
            .collect();
        let keys = ["KeyS", "KeyD", "KeyF", "Space", "KeyJ", "KeyK", "KeyL"];
        let mut s: BTreeMap<String, Vec<f64>> = BTreeMap::new();
        for sp in &spans {
            s.entry(keys[sp.input].to_string()).or_default().push(3000.0 + sp.down + 1.0);
        }
        let a = align(&spans, 7, 1.0, &s, &Params::default()).unwrap();
        for (i, k) in keys.iter().enumerate() {
            assert_eq!(a.mapping.get(&i).map(String::as_str), Some(*k));
        }
        assert_eq!(a.pairs.len(), 400);
    }

    #[test]
    fn helpers() {
        assert_eq!(quantile(&[], 0.5), None);
        assert_eq!(quantile(&[3.0, 1.0, 2.0], 0.5), Some(2.0));
        assert_eq!(quantile(&[3.0, 1.0, 2.0], 0.0), Some(1.0));
        assert_eq!(quantile(&[3.0, 1.0, 2.0], 1.0), Some(3.0));
        assert_eq!(quantile(&[1.0, f64::NAN], 2.0), Some(1.0));
        assert_eq!(match_sorted(&[10.0, 20.0, 30.0], &[9.0, 11.0, 26.0, 31.0], 4.0), vec![(0, 0), (2, 3)]);
        assert_eq!(match_sorted(&[], &[1.0], 1.0), vec![]);
        assert_eq!(match_sorted(&[10.0], &[10.5, 10.1], 1.0).len(), 1);
        assert_eq!(best_assignment(&[vec![5, 9], vec![8, 1]], 2), vec![Some(1), Some(0)]);
        assert_eq!(best_assignment(&[vec![0, 0]], 2), vec![None]);
        assert_eq!(best_assignment(&[], 3), Vec::<Option<usize>>::new());
        let big: Vec<Vec<usize>> = (0..5).map(|i| (0..5).map(|k| if i == k { 10 } else { 1 }).collect()).collect();
        assert_eq!(best_assignment(&big, 5), (0..5).map(Some).collect::<Vec<_>>());
    }
}

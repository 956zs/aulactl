//! Reads a usbmon text capture (`/sys/kernel/debug/usb/usbmon/<bus>u`) of the keyboard and reports HID key
//! report timing, and, when the travel test was open (`aulactl feed` running), the delay between the first
//! travel frame at or past a key's actuation point and the key-down in the HID reports. Used by
//! `scripts/latency-check.sh`; see `docs/feed.md`.
//!
//! usage: usbmon_latency --dev BUS:DEV [--hello FEED.ndjson] CAPTURE
//!
//! Endpoints (owner's board): 0x81 = interface 0 boot keyboard (8 bytes: modifiers, 0, six usages),
//! 0x82 = interface 1 report 5 NKRO bitmap (bit n of bytes 1.. = usage n, 0..0x9f), 0x83 = vendor interface
//! (travel frames `01 21 00 00 00 05 01 row col lo hi`, rest frames with `b[4] = 3`).

use aula_core::layout;
use std::collections::{BTreeMap, BTreeSet};

/// One completed interrupt-IN transfer.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Event {
    /// Kernel timestamp in µs.
    ts: u64,
    ep: u8,
    data: Vec<u8>,
}

/// Parses one usbmon text line (`tag ts C Ii:bus:dev:ep status len = words..`). Only completions of interrupt
/// IN transfers of `bus:dev` are returned.
fn parse_line(line: &str, bus: u16, dev: u16) -> Option<Event> {
    let mut f = line.split_whitespace();
    let _tag = f.next()?;
    let ts: u64 = f.next()?.parse().ok()?;
    if f.next()? != "C" {
        return None;
    }
    let mut addr = f.next()?.split(':');
    if addr.next()? != "Ii" {
        return None;
    }
    let (b, d): (u16, u16) = (addr.next()?.parse().ok()?, addr.next()?.parse().ok()?);
    let ep: u8 = addr.next()?.parse().ok()?;
    if (b, d) != (bus, dev) {
        return None;
    }
    let status = f.next()?;
    // interrupt transfers print `status:interval`
    if status.split(':').next() != Some("0") {
        return None;
    }
    let _len = f.next()?;
    if f.next()? != "=" {
        return None;
    }
    let mut data = Vec::new();
    for word in f {
        if word.len() % 2 != 0 {
            return None;
        }
        for i in (0..word.len()).step_by(2) {
            data.push(u8::from_str_radix(&word[i..i + 2], 16).ok()?);
        }
    }
    Some(Event { ts, ep, data })
}

/// Usages held down according to a key report, or None for reports that are not key reports.
fn held(ep: u8, data: &[u8]) -> Option<BTreeSet<u8>> {
    match ep {
        1 if data.len() >= 8 => {
            let mods = (0..8).filter(|i| data[0] & (1 << i) != 0).map(|i| 0xe0 + i);
            Some(mods.chain(data[2..8].iter().copied().filter(|u| *u > 1)).collect())
        }
        2 if data.first() == Some(&5) => Some(
            data[1..]
                .iter()
                .take(20)
                .enumerate()
                .flat_map(|(byte, bits)| (0..8).filter(move |i| bits & (1 << i) != 0).map(move |i| (byte * 8 + i) as u8))
                .collect(),
        ),
        _ => None,
    }
}

/// A travel frame: (key index, depth in 0.01 mm). Rest frames have depth 0.
fn travel(ep: u8, data: &[u8]) -> Option<(usize, u16)> {
    if ep != 3 || data.len() < 9 || data[0] != 1 || data[1] != 0x21 || data[6] != 1 {
        return None;
    }
    let idx = data[7] as usize * 22 + data[8] as usize;
    match data[5] {
        5 if data.len() >= 11 => Some((idx, u16::from_le_bytes([data[9], data[10]]))),
        3 => Some((idx, 0)),
        _ => None,
    }
}

/// Key-downs in the HID reports: (time, endpoint, usage).
fn hid_downs(events: &[Event]) -> Vec<(u64, u8, u8)> {
    let mut prev: BTreeMap<u8, BTreeSet<u8>> = BTreeMap::new();
    let mut out = Vec::new();
    for e in events {
        let Some(now) = held(e.ep, &e.data) else { continue };
        let before = prev.entry(e.ep).or_default();
        out.extend(now.difference(before).map(|u| (e.ts, e.ep, *u)));
        *before = now;
    }
    out
}

/// Actuations in the travel stream: (time, key index) of the first frame at or past the point after being
/// below it. `points` are in 0.01 mm per key index; keys without one are skipped.
fn stream_downs(events: &[Event], points: &BTreeMap<usize, u16>) -> Vec<(u64, usize)> {
    let mut down: BTreeSet<usize> = BTreeSet::new();
    let mut out = Vec::new();
    for e in events {
        let Some((idx, depth)) = travel(e.ep, &e.data) else { continue };
        let Some(point) = points.get(&idx) else { continue };
        if depth >= *point {
            if down.insert(idx) {
                out.push((e.ts, idx));
            }
        } else {
            down.remove(&idx);
        }
    }
    out
}

/// For each stream actuation, the signed delay in µs to the nearest unused HID key-down of the same usage
/// within ±`window` µs (positive: the HID report came after the stream frame). Returns (delays, unmatched).
fn match_downs(stream: &[(u64, usize)], hid: &[(u64, u8, u8)], window: u64) -> (Vec<i64>, usize) {
    let mut used = vec![false; hid.len()];
    let mut delays = Vec::new();
    let mut unmatched = 0;
    for (t, idx) in stream {
        let usage = layout::by_index(*idx).map_or(0, |k| k.default_usage());
        let best = hid
            .iter()
            .enumerate()
            .filter(|(i, (ht, _, u))| !used[*i] && *u == usage && ht.abs_diff(*t) <= window)
            .min_by_key(|(_, (ht, _, _))| ht.abs_diff(*t));
        match best {
            Some((i, (ht, _, _))) => {
                used[i] = true;
                delays.push(*ht as i64 - *t as i64);
            }
            None => unmatched += 1,
        }
    }
    (delays, unmatched)
}

/// min / p10 / p50 / p90 / max of a sample, or "-" when empty.
fn summary(v: &[i64]) -> String {
    if v.is_empty() {
        return "-".into();
    }
    let mut s = v.to_vec();
    s.sort_unstable();
    let p = |q: f64| s[((s.len() - 1) as f64 * q).round() as usize];
    format!("n={} min={} p10={} p50={} p90={} max={}", s.len(), s[0], p(0.1), p(0.5), p(0.9), s[s.len() - 1])
}

/// Gaps in µs between consecutive events of one endpoint that satisfy `keep`.
fn intervals(events: &[Event], ep: u8, keep: impl Fn(&Event) -> bool) -> Vec<i64> {
    let ts: Vec<u64> = events.iter().filter(|e| e.ep == ep && keep(e)).map(|e| e.ts).collect();
    ts.windows(2).map(|w| w[1] as i64 - w[0] as i64).collect()
}

/// Actuation points (0.01 mm) per key index from the feed's `hello` line.
fn points_from_hello(text: &str) -> BTreeMap<usize, u16> {
    let mut out = BTreeMap::new();
    for line in text.lines() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else { continue };
        if v["type"] != "hello" {
            continue;
        }
        for k in v["keys"].as_array().into_iter().flatten() {
            if let (Some(idx), Some(point)) = (k["idx"].as_u64(), k["point"].as_f64()) {
                out.insert(idx as usize, (point * 100.0).round() as u16);
            }
        }
    }
    out
}

fn report(events: &[Event], points: &BTreeMap<usize, u16>) -> String {
    let mut out = String::new();
    let count = |ep: u8| events.iter().filter(|e| e.ep == ep && held(ep, &e.data).is_some()).count();
    let frames = events.iter().filter(|e| travel(e.ep, &e.data).is_some()).count();
    out.push_str(&format!("key reports: if0 (0x81) {}, if1 (0x82, NKRO) {}; travel frames (0x83) {frames}\n", count(1), count(2)));
    for (ep, name) in [(1u8, "if0"), (2, "if1")] {
        let all = intervals(events, ep, |e| held(e.ep, &e.data).is_some());
        out.push_str(&format!("{name} report interval µs: {}\n", summary(&all)));
        let fast = all.iter().filter(|g| **g < 250).count();
        out.push_str(&format!("{name} intervals < 250 µs (two reports within 2 polls at 8 kHz): {fast}\n"));
    }
    let hid = hid_downs(events);
    let mut per_usage: BTreeMap<u8, Vec<u64>> = BTreeMap::new();
    for (t, _, u) in &hid {
        per_usage.entry(*u).or_default().push(*t);
    }
    let repeat: Vec<i64> =
        per_usage.values().flat_map(|ts| ts.windows(2).map(|w| w[1] as i64 - w[0] as i64)).collect();
    out.push_str(&format!("key-downs {}; same key down-to-down µs: {}\n", hid.len(), summary(&repeat)));
    let mut per_key: BTreeMap<usize, Vec<u64>> = BTreeMap::new();
    for e in events {
        if let Some((idx, _)) = travel(e.ep, &e.data) {
            per_key.entry(idx).or_default().push(e.ts);
        }
    }
    let gaps: Vec<i64> = per_key
        .values()
        .flat_map(|ts| ts.windows(2).map(|w| w[1] as i64 - w[0] as i64).filter(|g| *g < 20_000))
        .collect();
    out.push_str(&format!("travel frame interval per key µs (gaps < 20 ms): {}\n", summary(&gaps)));
    if frames == 0 {
        out.push_str("stream-to-report delay: no travel frames (feed not running, or no key moved)\n");
    } else if points.is_empty() {
        out.push_str("stream-to-report delay: no actuation points (pass --hello)\n");
    } else {
        let (delays, unmatched) = match_downs(&stream_downs(events, points), &hid, 50_000);
        out.push_str(&format!(
            "stream-to-report delay µs (HID key-down minus first frame >= point; negative = HID first): {}; unmatched {unmatched}\n",
            summary(&delays)
        ));
    }
    out
}

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut dev = None;
    let mut hello = None;
    let mut file = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--dev" => dev = it.next().cloned(),
            "--hello" => hello = it.next().cloned(),
            _ if file.is_none() => file = Some(a.clone()),
            _ => return Err(format!("unexpected argument '{a}'")),
        }
    }
    let usage = "usage: usbmon_latency --dev BUS:DEV [--hello FEED.ndjson] CAPTURE";
    let (Some(dev), Some(file)) = (dev, file) else { return Err(usage.into()) };
    let (bus, dev) = dev
        .split_once(':')
        .and_then(|(b, d)| Some((b.parse().ok()?, d.parse().ok()?)))
        .ok_or(usage)?;
    let text = std::fs::read_to_string(&file).map_err(|e| format!("{file}: {e}"))?;
    let events: Vec<Event> = text.lines().filter_map(|l| parse_line(l, bus, dev)).collect();
    let points = match hello {
        Some(h) => points_from_hello(&std::fs::read_to_string(&h).map_err(|e| format!("{h}: {e}"))?),
        None => BTreeMap::new(),
    };
    print!("{}", report(&events, &points));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const Z: usize = 90; // KeyZ, usage 0x1d

    fn ev(ts: u64, ep: u8, data: &[u8]) -> Event {
        Event { ts, ep, data: data.to_vec() }
    }

    fn frame(ts: u64, idx: usize, depth: u16) -> Event {
        let d = depth.to_le_bytes();
        ev(ts, 3, &[1, 0x21, 0, 0, 0, 5, 1, (idx / 22) as u8, (idx % 22) as u8, d[0], d[1]])
    }

    fn nkro(ts: u64, usages: &[u8]) -> Event {
        let mut d = vec![0u8; 21];
        d[0] = 5;
        for u in usages {
            d[1 + *u as usize / 8] |= 1 << (u % 8);
        }
        ev(ts, 2, &d)
    }

    #[test]
    fn parses_usbmon_lines() {
        let l = "ffff8a1c0b2d6f00 3575914555 C Ii:3:015:2 0:1 21 = 05000000 20000000 00000000 00000000 00000000 00";
        let e = parse_line(l, 3, 15).unwrap();
        assert_eq!((e.ts, e.ep, e.data.len()), (3575914555, 2, 21));
        assert_eq!(held(e.ep, &e.data).unwrap(), BTreeSet::from([0x1d]));
        assert_eq!(parse_line(l, 3, 16), None, "other device");
        assert_eq!(parse_line(&l.replace(" C ", " S "), 3, 15), None, "submission");
        assert_eq!(parse_line(&l.replace("Ii", "Io"), 3, 15), None, "OUT");
        assert_eq!(parse_line(&l.replace(" 0:1 ", " -2:1 "), 3, 15), None, "error status");
        assert_eq!(parse_line("ffff 1 C Ii:3:015:2 0:1 0 <", 3, 15), None, "no data");
        assert_eq!(parse_line("", 3, 15), None);
        assert_eq!(parse_line("ffff 1 C Ii:3:015:2 0:1 1 = 0", 3, 15), None, "odd hex");
    }

    #[test]
    fn boot_reports_include_modifiers() {
        let h = held(1, &[0x02, 0, 0x1d, 0, 0, 0, 0, 0]).unwrap();
        assert_eq!(h, BTreeSet::from([0x1d, 0xe1]));
        assert_eq!(held(1, &[0, 0, 1, 1, 1, 1, 1, 1]).unwrap(), BTreeSet::new(), "rollover error");
        assert_eq!(held(1, &[0; 4]), None, "short");
        assert_eq!(held(2, &[3, 0xe9, 0]), None, "consumer report");
        assert_eq!(held(3, &[0; 8]), None);
    }

    #[test]
    fn travel_frames() {
        assert_eq!(travel(3, &frame(0, Z, 340).data), Some((Z, 340)));
        assert_eq!(travel(3, &[1, 0x21, 0, 0, 0, 3, 1, 4, 2, 8, 0]), Some((Z, 0)));
        assert_eq!(travel(3, &[1, 0x21, 0, 0, 0, 1, 0]), None, "ACK");
        assert_eq!(travel(2, &frame(0, Z, 340).data), None);
    }

    #[test]
    fn key_downs_and_matching() {
        let events = vec![
            frame(1000, Z, 50),
            frame(1300, Z, 120),
            nkro(1450, &[0x1d]),
            frame(1600, Z, 200),
            nkro(1700, &[0x1d, 0x1b]),
            frame(2000, Z, 30),
            nkro(2100, &[0x1b]),
            nkro(2300, &[0x1d]),
            frame(2400, Z, 110),
        ];
        let hid = hid_downs(&events);
        assert_eq!(hid, vec![(1450, 2, 0x1d), (1700, 2, 0x1b), (2300, 2, 0x1d)]);
        let points = BTreeMap::from([(Z, 100)]);
        let stream = stream_downs(&events, &points);
        assert_eq!(stream, vec![(1300, Z), (2400, Z)]);
        let (delays, unmatched) = match_downs(&stream, &hid, 50_000);
        assert_eq!((delays, unmatched), (vec![150, -100], 0));
        assert_eq!(match_downs(&stream, &hid, 90), (vec![], 2), "outside the window");
        assert_eq!(match_downs(&[], &hid, 50_000), (vec![], 0));
        assert_eq!(stream_downs(&events, &BTreeMap::new()), vec![]);
    }

    #[test]
    fn summaries_and_intervals() {
        assert_eq!(summary(&[]), "-");
        assert_eq!(summary(&[5]), "n=1 min=5 p10=5 p50=5 p90=5 max=5");
        assert_eq!(summary(&[3, 1, 2]), "n=3 min=1 p10=1 p50=2 p90=3 max=3");
        let e = [nkro(0, &[]), nkro(125, &[4]), ev(130, 2, &[3, 0, 0]), nkro(500, &[])];
        assert_eq!(intervals(&e, 2, |e| held(e.ep, &e.data).is_some()), vec![125, 375]);
        assert_eq!(intervals(&e, 1, |_| true), Vec::<i64>::new());
    }

    #[test]
    fn hello_points() {
        let text = "{\"type\":\"status\",\"state\":\"waiting\",\"reason\":\"busy\"}\n\
                    {\"type\":\"hello\",\"keys\":[{\"idx\":90,\"point\":1.0},{\"idx\":91,\"point\":0.35}]}\n\
                    not json\n";
        assert_eq!(points_from_hello(text), BTreeMap::from([(90, 100), (91, 35)]));
        assert_eq!(points_from_hello(""), BTreeMap::new());
    }

    #[test]
    fn report_without_stream() {
        let r = report(&[nkro(0, &[0x1d]), nkro(200, &[])], &BTreeMap::new());
        assert!(r.contains("if1 (0x82, NKRO) 2"), "{r}");
        assert!(r.contains("no travel frames"), "{r}");
        assert!(r.contains("if1 intervals < 250 µs (two reports within 2 polls at 8 kHz): 1"), "{r}");
    }
}

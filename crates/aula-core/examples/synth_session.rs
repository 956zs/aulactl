//! Writes a synthetic recorded session that plays an osu! replay's key presses, for testing the replay tools
//! (`aulactl replay report|overlay`) end to end without a recording: every key-down span of input 1 / 2 becomes
//! a press of KeyZ / KeyX with a plausible travel curve, placed so the replay's timestamp is the end of the play.
//!
//! cargo run -p aula-core --example synth_session -- REPLAY.osr SESSIONS_DIR [--offset-ms MS] [--label TEXT]

use aula_core::feed::Edge;
use aula_core::osr;
use aula_core::session::{Header, SessionKey, Writer, SCHEMA};
use std::path::PathBuf;

const KEYS: [&str; 2] = ["KeyZ", "KeyX"];
const POINT_MM: f64 = 1.0;
const MAX_MM: f64 = 3.4;

fn main() {
    if let Err(e) = run() {
        eprintln!("synth_session: {e}");
        std::process::exit(1);
    }
}

/// Deterministic jitter in [-1, 1] from an index.
fn noise(i: usize, salt: u64) -> f64 {
    let mut x = (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ salt;
    x ^= x >> 29;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 32;
    (x % 2001) as f64 / 1000.0 - 1.0
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (mut pos, mut offset_ms, mut label) = (Vec::new(), 30_000.0, None);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--offset-ms" => offset_ms = it.next().and_then(|v| v.parse().ok()).ok_or("--offset-ms needs a number")?,
            "--label" => label = Some(it.next().ok_or("--label needs a value")?.clone()),
            _ => pos.push(a.clone()),
        }
    }
    let [replay_path, dir] = pos.as_slice() else {
        return Err("usage: synth_session REPLAY.osr SESSIONS_DIR [--offset-ms MS] [--label TEXT]".into());
    };
    let replay_path = PathBuf::from(replay_path);
    let r = osr::read(&replay_path)?;
    let spans = r.spans()?;
    let rate = r.rate();
    let (_, last) = r.time_range().ok_or("the replay has no frames")?;
    let ts = r.timestamp_unix_ms.ok_or("the replay has no timestamp")?;
    // the play's song time 0 sits at session `offset_ms`; the replay is stamped at its end
    let start_unix_ms = (ts as f64 - offset_ms - last as f64 / rate).max(0.0) as u64;
    let label = label.unwrap_or_else(|| {
        let name = replay_path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        osr::map_from_file_name(&name).unwrap_or_default()
    });
    let header = Header {
        schema: SCHEMA,
        label,
        start_unix_ms,
        start_mono_ms: 0.0,
        product: "SI2825KZHEARGB".into(),
        firmware: "3.17.07".into(),
        max_mm: MAX_MM,
        keys: KEYS
            .iter()
            .map(|c| SessionKey {
                code: (*c).into(),
                label: c[3..].into(),
                idx: 0,
                mode: "rt".into(),
                point: POINT_MM,
                press: 0.3,
                release: 0.3,
            })
            .collect(),
    };
    let mut w = Writer::create(std::path::Path::new(dir), &header)?;
    let mut n = 0;
    for (i, s) in spans.iter().enumerate() {
        let Some(key) = KEYS.get(s.input) else { continue };
        let down = offset_ms + s.down / rate + noise(i, 1) * 2.0;
        let up = (offset_ms + s.up / rate + noise(i, 2) * 2.0).max(down + 12.0);
        let peak = 2.0 + (noise(i, 3) + 1.0) * 0.7;
        let points = [
            (down - 6.0, 0.5),
            (down, POINT_MM + 0.05),
            (down + 4.0, (POINT_MM + peak) / 2.0),
            (down + 8.0, peak),
            ((up - 6.0).max(down + 9.0), peak - 0.2),
            (up, POINT_MM - 0.2),
            (up + 8.0, 0.0),
        ];
        for (k, (t, mm)) in points.iter().enumerate() {
            w.depth(*t, key, *mm)?;
            if k == 1 {
                w.edge(*t, key, Edge::Press)?;
            }
            if k == 5 {
                w.edge(*t, key, Edge::Release)?;
            }
        }
        n += 1;
    }
    w.finish()?;
    println!("{} ({n} presses, song 0 at session {offset_ms} ms)", w.path().display());
    Ok(())
}

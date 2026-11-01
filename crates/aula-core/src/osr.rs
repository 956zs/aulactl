//! osu! replay files (`.osr`, the legacy format osu!lazer exports): header, LZMA-compressed input frames, and
//! the key-down spans per input that `align` matches against a recorded session.
//!
//! Layout (little-endian): mode u8, version i32, beatmap MD5 str, player str, replay MD5 str, counts 6 × u16
//! (300, 100, 50, geki, katu, miss), score i32, max combo u16, perfect u8, mods u32, life bar str, timestamp
//! i64 (.NET ticks, UTC), compressed length i32, LZMA data (`w|x|y|z,` frames, `w` = ms since the previous
//! frame), online score id i64, then lazer-only data that is ignored here. A str is `0x00` (none) or `0x0b`,
//! ULEB128 length, UTF-8.

use crate::error::{Error, Result};
use std::io::{Read, Write};
use std::path::Path;

/// Largest .osr file read (real ones are 20 KiB .. 1 MiB).
pub const MAX_FILE_BYTES: u64 = 32 << 20;
/// Largest decompressed frame text.
pub const MAX_FRAME_BYTES: usize = 64 << 20;
/// Most frames accepted.
pub const MAX_FRAMES: usize = 2_000_000;
/// Longest header string (the life bar graph is the long one).
const MAX_STR: usize = 1 << 20;
/// LZMA decoder memory limit (dictionary).
const LZMA_MEMLIMIT: usize = 64 << 20;
/// `w` of the trailing frame that carries the RNG seed.
const SEED_FRAME: i64 = -12345;
/// .NET ticks (100 ns since 0001-01-01) at the unix epoch.
const UNIX_EPOCH_TICKS: i64 = 621_355_968_000_000_000;

pub const MOD_DOUBLE_TIME: u32 = 1 << 6;
pub const MOD_HALF_TIME: u32 = 1 << 8;
pub const MOD_NIGHTCORE: u32 = 1 << 9;

/// Game mode of a replay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Standard,
    Taiko,
    Catch,
    Mania,
}

impl Mode {
    fn from_byte(b: u8) -> Result<Mode> {
        Ok(match b {
            0 => Mode::Standard,
            1 => Mode::Taiko,
            2 => Mode::Catch,
            3 => Mode::Mania,
            b => return Err(Error::Invalid(format!("unknown game mode {b}"))),
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Mode::Standard => "osu!",
            Mode::Taiko => "taiko",
            Mode::Catch => "catch",
            Mode::Mania => "mania",
        }
    }
}

/// Judgement counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Counts {
    pub n300: u16,
    pub n100: u16,
    pub n50: u16,
    pub geki: u16,
    pub katu: u16,
    pub miss: u16,
}

/// One input frame: `t` is song time in ms (cumulative `w`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frame {
    pub t: i64,
    pub x: f64,
    pub y: f64,
    pub keys: u32,
}

/// A parsed replay. `frames` is empty when only the header was read.
#[derive(Debug, Clone, PartialEq)]
pub struct Replay {
    pub mode: Mode,
    pub version: i32,
    pub beatmap_md5: String,
    pub player: String,
    pub replay_md5: String,
    pub counts: Counts,
    pub score: i32,
    pub max_combo: u16,
    pub perfect: bool,
    pub mods: u32,
    /// When the score was set (unix ms, UTC); None when the file has no timestamp.
    pub timestamp_unix_ms: Option<i64>,
    pub frames: Vec<Frame>,
    pub seed: Option<i64>,
}

/// One key-down span of an input, in song ms.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Span {
    pub input: usize,
    pub down: f64,
    pub up: f64,
}

impl Replay {
    /// Song-time speed: 1.5 with DT/NC, 0.75 with HT, else 1. Real ms = song ms / rate.
    pub fn rate(&self) -> f64 {
        if self.mods & (MOD_DOUBLE_TIME | MOD_NIGHTCORE) != 0 {
            1.5
        } else if self.mods & MOD_HALF_TIME != 0 {
            0.75
        } else {
            1.0
        }
    }

    /// Input bits of a frame: standard M1/K1 = bit 0, M2/K2 = bit 1 (K1 = 5 and K2 = 10 include them; smoke
    /// is ignored); mania: the column bitmask in `x`. Taiko and catch are not supported.
    fn input_bits(&self, f: &Frame) -> Result<u32> {
        match self.mode {
            Mode::Standard => Ok(f.keys & 0b11),
            Mode::Mania => Ok(if f.x.is_finite() && f.x >= 0.0 && f.x < (1u64 << 20) as f64 { f.x as u32 } else { 0 }),
            m => Err(Error::Unsupported(format!("{} replays are not supported (osu! and mania only)", m.name()))),
        }
    }

    /// Number of inputs (2 for standard, the highest pressed column + 1 for mania).
    pub fn input_count(&self) -> Result<usize> {
        let mut bits = 0;
        for f in &self.frames {
            bits |= self.input_bits(f)?;
        }
        Ok(match self.mode {
            Mode::Standard => 2,
            _ => (32 - bits.leading_zeros()) as usize,
        })
    }

    /// Display name of input `i`.
    pub fn input_name(&self, i: usize) -> String {
        match self.mode {
            Mode::Standard => ["K1/M1", "K2/M2"].get(i).map_or_else(|| format!("#{}", i + 1), |s| s.to_string()),
            _ => format!("Col {}", i + 1),
        }
    }

    /// Key-down spans of every input in song ms, ordered by down time. A key still held at the last frame ends
    /// there.
    pub fn spans(&self) -> Result<Vec<Span>> {
        let mut out = Vec::new();
        let mut held = [None::<f64>; 32];
        let mut prev = 0u32;
        let mut last_t = 0.0;
        for f in &self.frames {
            let bits = self.input_bits(f)?;
            let t = f.t as f64;
            last_t = t;
            let changed = bits ^ prev;
            for (i, h) in held.iter_mut().enumerate() {
                if changed & (1 << i) == 0 {
                    continue;
                }
                if bits & (1 << i) != 0 {
                    *h = Some(t);
                } else if let Some(down) = h.take() {
                    out.push(Span { input: i, down, up: t });
                }
            }
            prev = bits;
        }
        for (i, h) in held.iter().enumerate() {
            if let Some(down) = h {
                out.push(Span { input: i, down: *down, up: last_t });
            }
        }
        out.sort_by(|a, b| a.down.total_cmp(&b.down).then(a.input.cmp(&b.input)));
        Ok(out)
    }

    /// Song time of the first and last frame.
    pub fn time_range(&self) -> Option<(i64, i64)> {
        Some((self.frames.first()?.t, self.frames.last()?.t))
    }
}

/// Reads a whole replay (header and frames).
pub fn read(path: &Path) -> Result<Replay> {
    parse(&read_file(path)?, true).map_err(|e| prefix(path, e))
}

/// Reads only the header (no decompression); `frames` is empty.
pub fn read_header(path: &Path) -> Result<Replay> {
    parse(&read_file(path)?, false).map_err(|e| prefix(path, e))
}

fn prefix(path: &Path, e: Error) -> Error {
    match e {
        Error::Invalid(m) => Error::Invalid(format!("{}: {m}", path.display())),
        Error::Unsupported(m) => Error::Unsupported(format!("{}: {m}", path.display())),
        e => e,
    }
}

fn read_file(path: &Path) -> Result<Vec<u8>> {
    let io = |e: std::io::Error| Error::Io(format!("{}: {e}", path.display()));
    let f = std::fs::File::open(path).map_err(io)?;
    let mut buf = Vec::new();
    f.take(MAX_FILE_BYTES + 1).read_to_end(&mut buf).map_err(io)?;
    if buf.len() as u64 > MAX_FILE_BYTES {
        return Err(Error::Invalid(format!("{}: larger than {} MiB, not a replay", path.display(), MAX_FILE_BYTES >> 20)));
    }
    Ok(buf)
}

struct Cursor<'a> {
    b: &'a [u8],
    at: usize,
}

impl<'a> Cursor<'a> {
    fn take(&mut self, n: usize, what: &str) -> Result<&'a [u8]> {
        if self.b.len() - self.at < n {
            return Err(Error::Invalid(format!("file ends inside the {what} (not a replay?)")));
        }
        let s = &self.b[self.at..self.at + n];
        self.at += n;
        Ok(s)
    }

    fn u8(&mut self, what: &str) -> Result<u8> {
        Ok(self.take(1, what)?[0])
    }

    fn u16(&mut self, what: &str) -> Result<u16> {
        Ok(u16::from_le_bytes(self.take(2, what)?.try_into().unwrap_or_default()))
    }

    fn u32(&mut self, what: &str) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4, what)?.try_into().unwrap_or_default()))
    }

    fn i64(&mut self, what: &str) -> Result<i64> {
        Ok(i64::from_le_bytes(self.take(8, what)?.try_into().unwrap_or_default()))
    }

    fn string(&mut self, what: &str) -> Result<String> {
        match self.u8(what)? {
            0x00 => Ok(String::new()),
            0x0b => {
                let mut len = 0usize;
                let mut shift = 0;
                loop {
                    let b = self.u8(what)?;
                    if shift > 28 {
                        return Err(Error::Invalid(format!("bad string length in the {what}")));
                    }
                    len |= ((b & 0x7f) as usize) << shift;
                    shift += 7;
                    if b & 0x80 == 0 {
                        break;
                    }
                }
                if len > MAX_STR {
                    return Err(Error::Invalid(format!("the {what} is too long ({len} bytes)")));
                }
                let s = self.take(len, what)?;
                String::from_utf8(s.to_vec()).map_err(|_| Error::Invalid(format!("the {what} is not UTF-8")))
            }
            b => Err(Error::Invalid(format!("bad string marker 0x{b:02x} in the {what} (not a replay?)"))),
        }
    }
}

/// Parses replay bytes; `frames` false stops after the header.
pub fn parse(bytes: &[u8], frames: bool) -> Result<Replay> {
    let mut c = Cursor { b: bytes, at: 0 };
    let mode = Mode::from_byte(c.u8("game mode")?)?;
    let version = c.u32("version")? as i32;
    let beatmap_md5 = c.string("beatmap hash")?;
    let player = c.string("player name")?;
    let replay_md5 = c.string("replay hash")?;
    let counts = Counts {
        n300: c.u16("counts")?,
        n100: c.u16("counts")?,
        n50: c.u16("counts")?,
        geki: c.u16("counts")?,
        katu: c.u16("counts")?,
        miss: c.u16("counts")?,
    };
    let score = c.u32("score")? as i32;
    let max_combo = c.u16("max combo")?;
    let perfect = c.u8("perfect flag")? != 0;
    let mods = c.u32("mods")?;
    let _life = c.string("life bar")?;
    let ticks = c.i64("timestamp")?;
    let timestamp_unix_ms = (ticks > 0).then(|| (ticks - UNIX_EPOCH_TICKS) / 10_000);
    let mut r = Replay {
        mode,
        version,
        beatmap_md5,
        player,
        replay_md5,
        counts,
        score,
        max_combo,
        perfect,
        mods,
        timestamp_unix_ms,
        frames: Vec::new(),
        seed: None,
    };
    if !frames {
        return Ok(r);
    }
    let len = c.u32("frame data length")? as i32;
    if len < 0 {
        return Err(Error::Invalid(format!("negative frame data length {len}")));
    }
    let data = c.take(len as usize, "frame data")?;
    let text = decompress(data)?;
    (r.frames, r.seed) = parse_frames(&text)?;
    Ok(r)
}

/// A `Write` that refuses more than `left` bytes.
struct Capped {
    out: Vec<u8>,
    left: usize,
}

impl Write for Capped {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if buf.len() > self.left {
            return Err(std::io::Error::other("too large"));
        }
        self.left -= buf.len();
        self.out.extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn decompress(data: &[u8]) -> Result<String> {
    if data.is_empty() {
        return Ok(String::new());
    }
    let mut out = Capped { out: Vec::new(), left: MAX_FRAME_BYTES };
    let opts = lzma_rs::decompress::Options { memlimit: Some(LZMA_MEMLIMIT), ..Default::default() };
    lzma_rs::lzma_decompress_with_options(&mut &data[..], &mut out, &opts).map_err(|e| match e {
        lzma_rs::error::Error::IoError(e) if e.to_string() == "too large" => {
            Error::Invalid(format!("frame data larger than {} MiB", MAX_FRAME_BYTES >> 20))
        }
        e => Error::Invalid(format!("frame data does not decompress: {e}")),
    })?;
    String::from_utf8(out.out).map_err(|_| Error::Invalid("frame data is not text".into()))
}

/// `w|x|y|z,` frames with cumulative `w` (song ms); the seed frame (`w` -12345) is returned apart.
pub fn parse_frames(text: &str) -> Result<(Vec<Frame>, Option<i64>)> {
    let mut frames = Vec::new();
    let mut seed = None;
    let mut t: i64 = 0;
    for (n, item) in text.split(',').enumerate() {
        let item = item.trim();
        if item.is_empty() {
            continue;
        }
        let bad = || Error::Invalid(format!("frame {} is malformed: {:?}", n + 1, item.chars().take(40).collect::<String>()));
        let mut parts = item.split('|');
        let (Some(w), Some(x), Some(y), Some(z), None) = (parts.next(), parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return Err(bad());
        };
        let w: i64 = w.parse().map_err(|_| bad())?;
        let x: f64 = x.parse().map_err(|_| bad())?;
        let y: f64 = y.parse().map_err(|_| bad())?;
        // keys is an integer, but some writers print it as a float
        let z: f64 = z.parse().map_err(|_| bad())?;
        if w == SEED_FRAME {
            seed = Some(z as i64);
            continue;
        }
        t = t.checked_add(w).ok_or_else(bad)?;
        if frames.len() >= MAX_FRAMES {
            return Err(Error::Invalid(format!("more than {MAX_FRAMES} frames")));
        }
        let keys = if z.is_finite() && z >= 0.0 && z <= u32::MAX as f64 { z as u32 } else { 0 };
        frames.push(Frame { t, x, y, keys });
    }
    Ok((frames, seed))
}

/// The map name in an exported replay's file name: `<player> playing <map> (<YYYY-MM-DD_HH-MM>).osr`.
pub fn map_from_file_name(name: &str) -> Option<String> {
    let stem = name.strip_suffix(".osr")?;
    let rest = stem.split_once(" playing ").map_or(stem, |(_, r)| r);
    // drop the trailing date in parentheses
    let rest = match rest.rfind(" (") {
        Some(i) if rest.ends_with(')') && rest[i + 2..rest.len() - 1].bytes().all(|b| b.is_ascii_digit() || b == b'-' || b == b'_') => {
            &rest[..i]
        }
        _ => rest,
    };
    Some(rest.to_string())
}

/// Lowercase alphanumerics only, for comparing map names from file names and labels.
pub fn normalize_name(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// Test helpers: build replays in memory.
#[cfg(any(test, feature = "sim"))]
pub mod encode {
    use super::*;

    fn string(out: &mut Vec<u8>, s: &str) {
        if s.is_empty() {
            out.push(0);
            return;
        }
        out.push(0x0b);
        let mut n = s.len();
        loop {
            let b = (n & 0x7f) as u8;
            n >>= 7;
            out.push(if n > 0 { b | 0x80 } else { b });
            if n == 0 {
                break;
            }
        }
        out.extend_from_slice(s.as_bytes());
    }

    /// Frames text for key-down spans (ms) of standard inputs (bit per input), with stable's two leading
    /// frames and a seed frame.
    pub fn frames_text(frames: &[(i64, u32)]) -> String {
        let mut s = String::from("0|256|-500|0,-1|256|-500|0,");
        let mut prev = -1;
        for &(t, keys) in frames {
            s.push_str(&format!("{}|100.5|200.25|{keys},", t - prev));
            prev = t;
        }
        s.push_str("-12345|0|0|7291,");
        s
    }

    /// A whole .osr file.
    pub fn osr(mode: u8, mods: u32, unix_ms: i64, frames_text: &str) -> Vec<u8> {
        let mut out = vec![mode];
        out.extend_from_slice(&30000001i32.to_le_bytes());
        string(&mut out, "d41d8cd98f00b204e9800998ecf8427e");
        string(&mut out, "tester");
        string(&mut out, "");
        for c in [300u16, 20, 3, 50, 10, 2] {
            out.extend_from_slice(&c.to_le_bytes());
        }
        out.extend_from_slice(&123456i32.to_le_bytes());
        out.extend_from_slice(&456u16.to_le_bytes());
        out.push(0);
        out.extend_from_slice(&mods.to_le_bytes());
        string(&mut out, "0|1,1000|0.9,");
        out.extend_from_slice(&(unix_ms * 10_000 + UNIX_EPOCH_TICKS).to_le_bytes());
        let mut data = Vec::new();
        lzma_rs::lzma_compress(&mut frames_text.as_bytes(), &mut data).expect("compress");
        out.extend_from_slice(&(data.len() as i32).to_le_bytes());
        out.extend_from_slice(&data);
        out.extend_from_slice(&0i64.to_le_bytes());
        out
    }
}

#[cfg(test)]
mod tests {
    use super::encode::*;
    use super::*;

    #[test]
    fn parses_a_synthetic_replay() {
        let text = frames_text(&[(1000, 0), (1500, 5), (1550, 0), (1600, 10), (1620, 15), (1700, 10), (1750, 0)]);
        let bytes = osr(0, MOD_DOUBLE_TIME, 1_793_491_200_000, &text);
        let r = parse(&bytes, true).unwrap();
        assert_eq!(r.mode, Mode::Standard);
        assert_eq!(r.player, "tester");
        assert_eq!(r.counts, Counts { n300: 300, n100: 20, n50: 3, geki: 50, katu: 10, miss: 2 });
        assert_eq!((r.score, r.max_combo, r.perfect), (123456, 456, false));
        assert_eq!(r.timestamp_unix_ms, Some(1_793_491_200_000));
        assert_eq!(r.seed, Some(7291));
        assert_eq!(r.rate(), 1.5);
        assert_eq!(r.frames.len(), 9);
        assert_eq!((r.frames[0].t, r.frames[1].t, r.frames[2].t), (0, -1, 1000));
        assert_eq!((r.frames[3].x, r.frames[3].y), (100.5, 200.25));
        assert_eq!(r.time_range(), Some((0, 1750)));
        assert_eq!(r.input_count().unwrap(), 2);
        let s = r.spans().unwrap();
        assert_eq!(
            s,
            vec![
                Span { input: 0, down: 1500.0, up: 1550.0 },
                Span { input: 1, down: 1600.0, up: 1750.0 },
                Span { input: 0, down: 1620.0, up: 1700.0 },
            ]
        );
        assert_eq!(r.input_name(0), "K1/M1");
        // header only
        let h = parse(&bytes, false).unwrap();
        assert!(h.frames.is_empty());
        assert_eq!(h.player, "tester");
    }

    #[test]
    fn mania_columns_and_unsupported_modes() {
        // x is the column bitmask: col 1 + 3 down, then col 3 only, then none
        let text = "0|5|0|0,10|1|0|0,5|5|0|0,5|4|0|0,10|0|0|0,";
        let r = parse(&osr(3, 0, 0, text), true).unwrap();
        assert_eq!(r.timestamp_unix_ms, Some(0));
        assert_eq!(r.input_count().unwrap(), 3);
        let s = r.spans().unwrap();
        assert_eq!(
            s,
            vec![
                Span { input: 0, down: 0.0, up: 20.0 },
                Span { input: 2, down: 0.0, up: 10.0 },
                Span { input: 2, down: 15.0, up: 30.0 },
            ]
        );
        assert_eq!(r.input_name(2), "Col 3");
        let taiko = parse(&osr(1, 0, 0, text), true).unwrap();
        assert!(matches!(taiko.spans(), Err(Error::Unsupported(_))));
        assert!(parse(&osr(9, 0, 0, text), true).is_err(), "unknown mode");
    }

    #[test]
    fn held_at_the_end_and_rates() {
        let r = parse(&osr(0, MOD_HALF_TIME, 0, &frames_text(&[(10, 1), (20, 1)])), true).unwrap();
        assert_eq!(r.spans().unwrap(), vec![Span { input: 0, down: 10.0, up: 20.0 }]);
        assert_eq!(r.rate(), 0.75);
        let r = parse(&osr(0, MOD_NIGHTCORE | MOD_DOUBLE_TIME, 0, &frames_text(&[])), true).unwrap();
        assert_eq!(r.rate(), 1.5);
        assert!(r.spans().unwrap().is_empty());
        let r = parse(&osr(0, 0, 0, ""), true).unwrap();
        assert!(r.frames.is_empty() && r.time_range().is_none());
        assert_eq!(r.rate(), 1.0);
    }

    #[test]
    fn frame_text_errors() {
        assert_eq!(parse_frames("").unwrap(), (vec![], None));
        assert!(parse_frames("1|2|3").is_err());
        assert!(parse_frames("1|2|3|4|5").is_err());
        assert!(parse_frames("a|2|3|4").is_err());
        assert!(parse_frames("1|2|3|x").is_err());
        assert!(parse_frames(&format!("{}|0|0|0,1|0|0|0", i64::MAX)).is_err(), "time overflow");
        let (f, _) = parse_frames("1|0|0|-3,1|0|0|1e20,1|0|0|2.0").unwrap();
        assert_eq!(f.iter().map(|f| f.keys).collect::<Vec<_>>(), vec![0, 0, 2]);
    }

    #[test]
    fn malformed_files_never_panic() {
        let good = osr(0, 0, 1000, &frames_text(&[(10, 1), (20, 0)]));
        // every truncation is an error, never a panic
        for n in 0..good.len() - 8 {
            assert!(parse(&good[..n], true).is_err(), "cut at {n}");
        }
        // corrupt the compressed data
        let mut bad = good.clone();
        let data_at = good.len() - 8 - 20;
        for b in &mut bad[data_at..good.len() - 8] {
            *b ^= 0x5a;
        }
        assert!(parse(&bad, true).is_err());
        // a bad string marker, an overlong string length
        let mut bad = good.clone();
        bad[5] = 0x07;
        assert!(parse(&bad, true).unwrap_err().to_string().contains("string marker"));
        let mut bad = good[..5].to_vec();
        bad.extend_from_slice(&[0x0b, 0xff, 0xff, 0xff, 0xff, 0x0f]);
        assert!(parse(&bad, true).is_err());
        // negative length
        let mut bad = good.clone();
        let header_len = parse_header_len(&good);
        bad[header_len..header_len + 4].copy_from_slice(&(-5i32).to_le_bytes());
        assert!(parse(&bad, true).unwrap_err().to_string().contains("negative"));
        // random bytes
        let mut x = 12345u32;
        for len in [1usize, 10, 100, 1000] {
            let v: Vec<u8> = (0..len)
                .map(|_| {
                    x = x.wrapping_mul(1_103_515_245).wrapping_add(12345);
                    (x >> 16) as u8
                })
                .collect();
            let _ = parse(&v, true);
        }
    }

    /// Offset of the frame data length field.
    fn parse_header_len(bytes: &[u8]) -> usize {
        let mut c = Cursor { b: bytes, at: 0 };
        c.u8("").unwrap();
        c.u32("").unwrap();
        for _ in 0..3 {
            c.string("").unwrap();
        }
        c.take(12 + 4 + 2 + 1 + 4, "").unwrap();
        c.string("").unwrap();
        c.i64("").unwrap();
        c.at
    }

    #[test]
    fn file_names() {
        let n = "n1cat playing II-L - SPUTNIK-3 (DeviousPanda) [Insane] (2026-11-01_12-30).osr";
        assert_eq!(map_from_file_name(n).as_deref(), Some("II-L - SPUTNIK-3 (DeviousPanda) [Insane]"));
        assert_eq!(map_from_file_name("x playing A (B) [C].osr").as_deref(), Some("A (B) [C]"));
        assert_eq!(map_from_file_name("plain.osr").as_deref(), Some("plain"));
        assert_eq!(map_from_file_name("x.osk"), None);
        assert_eq!(normalize_name("II-L - SPUTNIK-3 (DeviousPanda) [Insane]"), "iilsputnik3deviouspandainsane");
        assert_eq!(normalize_name(""), "");
    }

    #[test]
    fn missing_and_oversized_files() {
        assert!(matches!(read(Path::new("/nonexistent/x.osr")), Err(Error::Io(_))));
        let d = crate::store::temp_dir("osr");
        let p = d.join("big.osr");
        std::fs::File::create(&p).unwrap().set_len(MAX_FILE_BYTES + 1).unwrap();
        assert!(read_header(&p).unwrap_err().to_string().contains("larger than"));
        std::fs::remove_dir_all(&d).unwrap();
    }

    /// The newest replay the owner exported, if any (read only; skipped when there is none).
    #[test]
    fn reads_a_real_lazer_export() {
        let Some(home) = std::env::var_os("HOME") else { return };
        let dir = Path::new(&home).join(".local/share/osu/exports");
        let Ok(rd) = std::fs::read_dir(&dir) else { return };
        let mut files: Vec<_> = rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "osr")).collect();
        files.sort();
        let Some(p) = files.pop() else { return };
        let r = read(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
        assert!(!r.frames.is_empty(), "{}", p.display());
        assert!(r.timestamp_unix_ms.is_some_and(|t| t > 1_500_000_000_000));
        if matches!(r.mode, Mode::Standard | Mode::Mania) {
            let s = r.spans().unwrap();
            assert!(!s.is_empty());
            assert!(s.iter().all(|s| s.up >= s.down));
        }
    }
}

//! Recorded feed sessions (`aulactl feed --record`, `docs/feed.md`): gzip'd NDJSON files under
//! `$XDG_DATA_HOME/aulactl/sessions`, written while the feed runs and read back for replay reports.
//!
//! Line 1 is the `session` header; the rest are shaped like the feed's own lines (`hello`, `depth`, `press`,
//! `release`, `status`), except that every raw travel sample is a `depth` line (not coalesced) with `t` to
//! 0.01 ms and `mm` to 0.001 mm.

use crate::error::{invalid, Error, Result};
use crate::feed::{Edge, Hello, HelloKey};
use crate::store::{private_dir, timestamp};
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, BufWriter, ErrorKind, Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Version of the session file layout (`schema` in the header).
pub const SCHEMA: u32 = 1;
pub const EXT: &str = ".ndjson.gz";
/// Sessions kept: the newest `KEEP_SESSIONS`, and no more than `KEEP_BYTES` in total (whichever is hit first).
pub const KEEP_SESSIONS: usize = 300;
pub const KEEP_BYTES: u64 = 2 << 30;
/// Longest label in the header, and the label part of a file name (characters).
pub const MAX_LABEL: usize = 200;
pub const MAX_NAME_LABEL: usize = 80;
/// Largest decompressed session a reader accepts (a 10-minute session with two busy keys is about 30 MiB).
pub const MAX_SESSION_BYTES: u64 = 1 << 30;
/// Longest single line a reader accepts.
const MAX_LINE: usize = 1 << 20;
/// How often the writer pushes compressed data to the file, so a crash loses little.
const FLUSH_EVERY: Duration = Duration::from_secs(2);
const FILE_MODE: u32 = 0o600;

/// `$XDG_DATA_HOME/aulactl/sessions`.
pub fn sessions_dir() -> Result<PathBuf> {
    Ok(crate::store::data_dir()?.join("sessions"))
}

/// The header label: control characters removed, trimmed, at most `MAX_LABEL` characters.
pub fn clean_label(label: &str) -> String {
    let s: String = label.chars().filter(|c| !c.is_control()).collect();
    s.trim().chars().take(MAX_LABEL).collect()
}

/// The label part of a file name: alphanumerics (any script) and `-_.()[]`, whitespace as `_`, everything else
/// dropped; no leading dots; at most `MAX_NAME_LABEL` characters. Never contains a path separator.
pub fn file_label(label: &str) -> String {
    let mut out = String::new();
    for c in label.chars() {
        if c.is_alphanumeric() || "-_.()[]".contains(c) {
            out.push(c);
        } else if c.is_whitespace() && !out.ends_with('_') {
            out.push('_');
        }
    }
    let out = out.trim_start_matches(['.', '_']).trim_end_matches('_');
    out.chars().take(MAX_NAME_LABEL).collect()
}

/// `<UTC YYYYmmdd-HHMMSS>[-<file label>].ndjson.gz` for a session started at `start_unix_ms`.
pub fn file_name(start_unix_ms: u64, label: &str) -> String {
    let stamp = timestamp(start_unix_ms / 1000);
    match file_label(label) {
        l if l.is_empty() => format!("{stamp}{EXT}"),
        l => format!("{stamp}-{l}{EXT}"),
    }
}

/// The first line of a session file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Header {
    pub schema: u32,
    pub label: String,
    /// Wall clock (unix ms) at `t` = 0.
    pub start_unix_ms: u64,
    /// CLOCK_MONOTONIC (ms) at `t` = 0.
    pub start_mono_ms: f64,
    pub product: String,
    pub firmware: String,
    pub max_mm: f64,
    pub keys: Vec<SessionKey>,
}

/// A recorded key and its trigger settings (mm), as in the feed's `hello`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionKey {
    pub code: String,
    pub label: String,
    pub idx: usize,
    pub mode: String,
    pub point: f64,
    pub press: f64,
    pub release: f64,
}

impl From<&HelloKey> for SessionKey {
    fn from(k: &HelloKey) -> Self {
        SessionKey {
            code: k.code.into(),
            label: k.label.into(),
            idx: k.idx,
            mode: k.mode.into(),
            point: k.point,
            press: k.press,
            release: k.release,
        }
    }
}

#[derive(Serialize)]
struct HeaderLine<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    #[serde(flatten)]
    header: &'a Header,
}

/// CLOCK_MONOTONIC in ms.
pub fn monotonic_ms() -> f64 {
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    // SAFETY: clock_gettime only writes the timespec it is given.
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
    ts.tv_sec as f64 * 1000.0 + ts.tv_nsec as f64 / 1e6
}

fn round(v: f64, scale: f64) -> f64 {
    (v * scale).round() / scale
}

/// Writes one session file. Lines are compressed as they come and pushed to the file every 2 s; `finish`
/// (or drop) completes the gzip stream.
pub struct Writer {
    path: PathBuf,
    enc: Option<GzEncoder<BufWriter<File>>>,
    flushed: Instant,
}

impl Writer {
    /// Creates a new session file in `dir` (created 0700) for `header` and writes the header line. The file is
    /// 0600 and never replaces an existing one (`-1`, `-2`, ... are appended to the name instead).
    pub fn create(dir: &Path, header: &Header) -> Result<Writer> {
        private_dir(dir)?;
        let name = file_name(header.start_unix_ms, &header.label);
        let stem = name.strip_suffix(EXT).unwrap_or(&name).to_string();
        let mut n = 0;
        let (path, file) = loop {
            let path = if n == 0 { dir.join(&name) } else { dir.join(format!("{stem}-{n}{EXT}")) };
            match OpenOptions::new().write(true).create_new(true).mode(FILE_MODE).open(&path) {
                Ok(f) => break (path, f),
                Err(e) if e.kind() == ErrorKind::AlreadyExists && n < 100 => n += 1,
                Err(e) => return Err(Error::Io(format!("{}: {e}", path.display()))),
            }
        };
        let mut w = Writer {
            path,
            enc: Some(GzEncoder::new(BufWriter::new(file), Compression::fast())),
            flushed: Instant::now(),
        };
        let line = serde_json::to_string(&HeaderLine { kind: "session", header })
            .map_err(|e| Error::Invalid(format!("session header: {e}")))?;
        w.line(&line)?;
        Ok(w)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Appends one line (without its newline).
    pub fn line(&mut self, line: &str) -> Result<()> {
        let path = &self.path;
        let enc = self.enc.as_mut().ok_or_else(|| Error::Io(format!("{}: already finished", path.display())))?;
        let io = |e: std::io::Error| Error::Io(format!("{}: {e}", path.display()));
        enc.write_all(line.as_bytes()).and_then(|_| enc.write_all(b"\n")).map_err(io)?;
        if self.flushed.elapsed() >= FLUSH_EVERY {
            enc.flush().map_err(io)?;
            self.flushed = Instant::now();
        }
        Ok(())
    }

    pub fn hello(&mut self, hello: &Hello) -> Result<()> {
        self.line(&crate::feed::Line::Hello(hello).to_json())
    }

    /// One raw travel sample.
    pub fn depth(&mut self, t: f64, key: &str, mm: f64) -> Result<()> {
        self.line(&serde_json::json!({"type": "depth", "t": round(t, 100.0), "key": key, "mm": round(mm, 1000.0)}).to_string())
    }

    pub fn edge(&mut self, t: f64, key: &str, edge: Edge) -> Result<()> {
        let kind = match edge {
            Edge::Press => "press",
            Edge::Release => "release",
        };
        self.line(&serde_json::json!({"type": kind, "t": round(t, 100.0), "key": key}).to_string())
    }

    /// The keyboard went away (`t` ms after the start).
    pub fn lost(&mut self, t: f64) -> Result<()> {
        self.line(
            &serde_json::json!({"type": "status", "state": "lost", "reason": "keyboard unplugged", "t": round(t, 100.0)})
                .to_string(),
        )
    }

    /// Pushes everything written so far to the file (a readable prefix even if the process dies next).
    pub fn flush(&mut self) -> Result<()> {
        let Some(enc) = self.enc.as_mut() else { return Ok(()) };
        enc.flush().map_err(|e| Error::Io(format!("{}: {e}", self.path.display())))?;
        self.flushed = Instant::now();
        Ok(())
    }

    /// Completes the gzip stream and syncs the file.
    pub fn finish(&mut self) -> Result<()> {
        let Some(enc) = self.enc.take() else { return Ok(()) };
        let io = |e: std::io::Error| Error::Io(format!("{}: {e}", self.path.display()));
        let buf = enc.finish().map_err(io)?;
        let file = buf.into_inner().map_err(|e| io(e.into_error()))?;
        file.sync_all().map_err(io)
    }
}

impl Drop for Writer {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}

/// A recorded session, read back.
#[derive(Debug, Clone, PartialEq)]
pub struct Session {
    pub path: PathBuf,
    pub header: Header,
    /// Per key code: raw samples `(t ms, mm)` in time order.
    pub depth: BTreeMap<String, Vec<(f64, f64)>>,
    /// Per key code: host-derived presses `(down t, up t)`; `up` is None when the file ends while pressed.
    pub presses: BTreeMap<String, Vec<(f64, Option<f64>)>>,
    /// Latest `t` in the file.
    pub end_ms: f64,
    /// The gzip stream or the last line was cut off (killed recorder); everything before it was read.
    pub truncated: bool,
}

impl Session {
    pub fn key(&self, code: &str) -> Option<&SessionKey> {
        self.header.keys.iter().find(|k| k.code == code)
    }

    pub fn press_count(&self) -> usize {
        self.presses.values().map(Vec::len).sum()
    }

    /// Wall clock (unix ms) of the last line.
    pub fn end_unix_ms(&self) -> u64 {
        self.header.start_unix_ms + self.end_ms.max(0.0) as u64
    }
}

#[derive(Deserialize)]
struct AnyLine {
    #[serde(rename = "type")]
    kind: String,
    t: Option<f64>,
    key: Option<String>,
    mm: Option<f64>,
    keys: Option<Vec<SessionKey>>,
}

/// Reads a session file (gzip'd NDJSON, at most `MAX_SESSION_BYTES` decompressed). A file cut off by a killed
/// recorder is read up to the cut (`truncated`); a missing or malformed header is an error.
pub fn read(path: &Path) -> Result<Session> {
    let f = File::open(path).map_err(|e| Error::Io(format!("{}: {e}", path.display())))?;
    read_from(path, f)
}

pub(crate) fn read_from(path: &Path, r: impl Read) -> Result<Session> {
    let bad = |m: String| Error::Invalid(format!("{}: {m}", path.display()));
    let mut lines = BufReader::new(GzDecoder::new(r).take(MAX_SESSION_BYTES + 1));
    let mut buf = Vec::new();
    let mut total = 0u64;
    let mut truncated = false;
    // the header
    match read_line(&mut lines, &mut buf) {
        Ok(true) => {}
        Ok(false) => return Err(bad("empty file, not a session".into())),
        Err(e) => return Err(bad(format!("not a session file ({e})"))),
    }
    total += buf.len() as u64;
    let header = parse_header(&buf).map_err(bad)?;
    let mut s = Session {
        path: path.to_path_buf(),
        depth: BTreeMap::new(),
        presses: BTreeMap::new(),
        end_ms: 0.0,
        truncated: false,
        header,
    };
    loop {
        match read_line(&mut lines, &mut buf) {
            Ok(true) => {}
            Ok(false) => break,
            Err(_) => {
                truncated = true;
                break;
            }
        }
        total += buf.len() as u64;
        if total > MAX_SESSION_BYTES {
            return Err(bad(format!("larger than {} MiB decompressed", MAX_SESSION_BYTES >> 20)));
        }
        if !buf.ends_with(b"\n") {
            // the last line was cut off
            truncated = true;
            break;
        }
        let Ok(l) = serde_json::from_slice::<AnyLine>(&buf) else {
            // a line this reader does not know (or a damaged one): skip
            continue;
        };
        match (l.kind.as_str(), l.t, l.key) {
            ("depth", Some(t), Some(key)) => {
                if let Some(mm) = l.mm.filter(|m| m.is_finite()) {
                    s.depth.entry(key).or_default().push((t, mm));
                    s.end_ms = s.end_ms.max(t);
                }
            }
            ("press", Some(t), Some(key)) => {
                s.presses.entry(key).or_default().push((t, None));
                s.end_ms = s.end_ms.max(t);
            }
            ("release", Some(t), Some(key)) => {
                if let Some(last) = s.presses.get_mut(&key).and_then(|v| v.last_mut()) {
                    if last.1.is_none() {
                        last.1 = Some(t);
                    }
                }
                s.end_ms = s.end_ms.max(t);
            }
            ("hello", _, _) => {
                // settings may change between connections: the newest win
                for k in l.keys.unwrap_or_default() {
                    match s.header.keys.iter_mut().find(|h| h.code == k.code) {
                        Some(h) => *h = k,
                        None => s.header.keys.push(k),
                    }
                }
            }
            _ => {}
        }
    }
    s.truncated = truncated;
    Ok(s)
}

/// One line into `buf` (with its newline, if any); Ok(false) at the end. Lines longer than `MAX_LINE` are an
/// error.
fn read_line(r: &mut impl BufRead, buf: &mut Vec<u8>) -> std::io::Result<bool> {
    buf.clear();
    let n = r.by_ref().take(MAX_LINE as u64).read_until(b'\n', buf)?;
    if n == MAX_LINE && !buf.ends_with(b"\n") {
        return Err(std::io::Error::new(ErrorKind::InvalidData, "line too long"));
    }
    Ok(n > 0)
}

fn parse_header(line: &[u8]) -> std::result::Result<Header, String> {
    let v: serde_json::Value = serde_json::from_slice(line).map_err(|e| format!("not a session file ({e})"))?;
    if v["type"] != "session" {
        return Err("not a session file (no session header)".into());
    }
    match v["schema"].as_u64() {
        Some(s) if s == u64::from(SCHEMA) => {}
        s => return Err(format!("session schema {} is not supported (this aulactl reads {SCHEMA})", s.unwrap_or(0))),
    }
    serde_json::from_value(v).map_err(|e| format!("bad session header: {e}"))
}

/// A file modified this recently counts as still being written.
const RECENT_WRITE: Duration = Duration::from_secs(5);

/// Whether a recorder is probably still writing `path`: modified in the last 5 s, or held open by a process
/// (a running feed that is idle writes nothing, e.g. on a results screen). An unfinished file that is not
/// being written was cut off.
pub fn is_being_written(path: &Path) -> bool {
    let recent = fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok())
        .is_some_and(|age| age < RECENT_WRITE);
    recent || is_open_anywhere(path, Path::new("/proc"))
}

/// Whether any process under `proc` (the user's own; others are unreadable) has `path` open.
fn is_open_anywhere(path: &Path, proc: &Path) -> bool {
    let Ok(target) = fs::canonicalize(path) else { return false };
    let Ok(procs) = fs::read_dir(proc) else { return false };
    procs
        .flatten()
        .filter(|e| e.file_name().to_str().is_some_and(|n| n.bytes().all(|b| b.is_ascii_digit())))
        .any(|e| {
            fs::read_dir(e.path().join("fd"))
                .map(|fds| fds.flatten().any(|fd| fs::read_link(fd.path()).is_ok_and(|l| l == target)))
                .unwrap_or(false)
        })
}

/// Session files in `dir`, oldest first (names start with the UTC start time). A missing dir is empty.
pub fn list(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut v: Vec<PathBuf> = match fs::read_dir(dir) {
        Ok(rd) => rd
            .flatten()
            .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
            .map(|e| e.path())
            .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(is_session_name))
            .collect(),
        Err(e) if e.kind() == ErrorKind::NotFound => Vec::new(),
        Err(e) => return Err(Error::Io(format!("{}: {e}", dir.display()))),
    };
    // start time from the name; files started in the same second by modification time
    let key = |p: &PathBuf| {
        let stamp = p.file_name().and_then(|n| n.to_str()).map(|n| n[..15].to_string()).unwrap_or_default();
        (stamp, fs::metadata(p).and_then(|m| m.modified()).ok(), p.clone())
    };
    v.sort_by_cached_key(key);
    Ok(v)
}

/// `YYYYmmdd-HHMMSS[...].ndjson.gz`
fn is_session_name(n: &str) -> bool {
    let b = n.as_bytes();
    n.ends_with(EXT)
        && b.len() >= 15 + EXT.len()
        && b[..8].iter().all(u8::is_ascii_digit)
        && b[8] == b'-'
        && b[9..15].iter().all(u8::is_ascii_digit)
}

/// Deletes the oldest sessions in `dir` beyond the newest `keep` and beyond `max_bytes` in total (newest
/// counted first). `current` (the file being written) is never deleted. Returns the deleted files.
pub fn prune(dir: &Path, keep: usize, max_bytes: u64, current: Option<&Path>) -> Result<Vec<PathBuf>> {
    let all = list(dir)?;
    let mut kept = 0usize;
    let mut bytes = 0u64;
    let mut deleted = Vec::new();
    for p in all.iter().rev() {
        if Some(p.as_path()) == current {
            kept += 1;
            bytes += fs::metadata(p).map(|m| m.len()).unwrap_or(0);
            continue;
        }
        let len = fs::metadata(p).map(|m| m.len()).unwrap_or(0);
        if kept < keep && bytes + len <= max_bytes {
            kept += 1;
            bytes += len;
        } else {
            fs::remove_file(p).map_err(|e| Error::Io(format!("{}: {e}", p.display())))?;
            deleted.push(p.clone());
        }
    }
    Ok(deleted)
}

/// Resolves a session argument: `latest`, a path, or a file name in `dir`.
pub fn resolve(dir: &Path, arg: &str) -> Result<PathBuf> {
    if arg == "latest" {
        return list(dir)?.pop().ok_or_else(|| Error::Invalid(format!("no recorded sessions in {}", dir.display())));
    }
    let p = Path::new(arg);
    if p.components().count() > 1 || p.is_absolute() {
        return Ok(p.to_path_buf());
    }
    let in_dir = dir.join(arg);
    if in_dir.exists() {
        return Ok(in_dir);
    }
    let with_ext = dir.join(format!("{arg}{EXT}"));
    if with_ext.exists() {
        return Ok(with_ext);
    }
    if p.exists() {
        return Ok(p.to_path_buf());
    }
    invalid(format!("no session {arg:?} in {}", dir.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::temp_dir;
    use std::os::unix::fs::PermissionsExt;

    fn header(label: &str) -> Header {
        Header {
            schema: SCHEMA,
            label: label.into(),
            start_unix_ms: 1_793_491_200_123,
            start_mono_ms: 1234.5,
            product: "SI2825KZHEARGB".into(),
            firmware: "3.17.07".into(),
            max_mm: 3.4,
            keys: vec![SessionKey {
                code: "KeyZ".into(),
                label: "Z".into(),
                idx: 89,
                mode: "rt".into(),
                point: 1.0,
                press: 0.5,
                release: 0.5,
            }],
        }
    }

    #[test]
    fn labels() {
        assert_eq!(clean_label("  a\u{0}b\nc  "), "abc");
        assert_eq!(clean_label(&"x".repeat(500)).len(), MAX_LABEL);
        assert_eq!(clean_label(""), "");
        assert_eq!(file_label("II-L - SPUTNIK-3 (DeviousPanda) [Insane]"), "II-L_-_SPUTNIK-3_(DeviousPanda)_[Insane]");
        assert_eq!(file_label("../../etc/passwd"), "etcpasswd");
        assert_eq!(file_label("a/b\\c:d*e?f\"g<h>i|j"), "abcdefghij");
        assert_eq!(file_label("..hidden"), "hidden");
        assert_eq!(file_label("   "), "");
        assert_eq!(file_label("東方 Project"), "東方_Project");
        assert_eq!(file_label(&"é".repeat(100)).chars().count(), MAX_NAME_LABEL);
        assert_eq!(file_name(1_793_491_200_999, "x y"), "20261101-000000-x_y.ndjson.gz");
        assert_eq!(file_name(0, "/"), "19700101-000000.ndjson.gz");
        assert!(is_session_name("20261101-000000.ndjson.gz"));
        assert!(is_session_name("20261101-000000-a-1.ndjson.gz"));
        assert!(!is_session_name("2026110-000000.ndjson.gz"));
        assert!(!is_session_name("20261101-000000.json"));
        assert!(!is_session_name(".ndjson.gz"));
    }

    fn write_sample(dir: &Path, label: &str) -> PathBuf {
        let mut w = Writer::create(dir, &header(label)).unwrap();
        w.depth(10.0, "KeyZ", 0.5).unwrap();
        w.depth(11.512, "KeyZ", 1.2345).unwrap();
        w.edge(11.5, "KeyZ", Edge::Press).unwrap();
        w.line(r#"{"type":"future","x":1}"#).unwrap();
        w.depth(30.0, "KeyZ", 0.0).unwrap();
        w.edge(30.0, "KeyZ", Edge::Release).unwrap();
        w.lost(31.0).unwrap();
        w.edge(40.0, "KeyZ", Edge::Press).unwrap();
        w.finish().unwrap();
        w.path().to_path_buf()
    }

    #[test]
    fn write_and_read_back() {
        let d = temp_dir("session");
        let p = write_sample(&d.join("s"), "map [Hard]");
        assert_eq!(p.file_name().unwrap(), "20261101-000000-map_[Hard].ndjson.gz");
        assert_eq!(fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(fs::metadata(d.join("s")).unwrap().permissions().mode() & 0o777, 0o700);
        let s = read(&p).unwrap();
        assert_eq!(s.header, header("map [Hard]"));
        assert_eq!(s.depth["KeyZ"], vec![(10.0, 0.5), (11.51, 1.235), (30.0, 0.0)]);
        assert_eq!(s.presses["KeyZ"], vec![(11.5, Some(30.0)), (40.0, None)]);
        assert_eq!(s.end_ms, 40.0);
        assert_eq!(s.press_count(), 2);
        assert!(!s.truncated);
        assert_eq!(s.end_unix_ms(), 1_793_491_200_163);
        // a second file with the same start never replaces the first
        let p2 = write_sample(&d.join("s"), "map [Hard]");
        assert_eq!(p2.file_name().unwrap(), "20261101-000000-map_[Hard]-1.ndjson.gz");
        assert_eq!(list(&d.join("s")).unwrap(), vec![p.clone(), p2]);
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn hello_lines_update_key_settings() {
        let d = temp_dir("session-hello");
        let mut w = Writer::create(&d, &header("")).unwrap();
        w.line(r#"{"type":"hello","product":"P","firmware":"F","max_mm":3.4,"keys":[{"code":"KeyZ","label":"Z","idx":89,"x":1,"y":3,"w":1,"mode":"fixed","point":2.0,"press":0.1,"release":0.1},{"code":"KeyX","label":"X","idx":90,"x":2,"y":3,"w":1,"mode":"rt","point":1.0,"press":0.2,"release":0.2}]}"#).unwrap();
        let p = w.path().to_path_buf();
        drop(w);
        let s = read(&p).unwrap();
        assert_eq!(s.key("KeyZ").unwrap().mode, "fixed");
        assert_eq!(s.key("KeyX").unwrap().point, 1.0);
        assert!(s.key("KeyQ").is_none());
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn truncated_files_are_read_up_to_the_cut() {
        let d = temp_dir("session-cut");
        let p = write_sample(&d, "x");
        let bytes = fs::read(&p).unwrap();
        // without the gzip trailer and some of the deflate data
        let s = read_from(&p, &bytes[..bytes.len() - 12]).unwrap();
        assert!(s.truncated);
        assert_eq!(s.header.label, "x");
        // a cut inside the header is an error
        assert!(read_from(&p, &bytes[..12]).is_err());
        fs::remove_dir_all(&d).unwrap();
    }

    fn gz(text: &str) -> Vec<u8> {
        let mut e = GzEncoder::new(Vec::new(), Compression::fast());
        e.write_all(text.as_bytes()).unwrap();
        e.finish().unwrap()
    }

    #[test]
    fn bad_files() {
        let p = Path::new("x.ndjson.gz");
        assert!(read_from(p, &b""[..]).is_err(), "empty");
        assert!(read_from(p, &b"not gzip at all"[..]).is_err());
        assert!(read_from(p, &gz("")[..]).unwrap_err().to_string().contains("empty"));
        assert!(read_from(p, &gz("{\"type\":\"hello\"}\n")[..]).unwrap_err().to_string().contains("no session header"));
        assert!(read_from(p, &gz("{\"type\":\"session\",\"schema\":2}\n")[..]).unwrap_err().to_string().contains("schema 2"));
        assert!(read_from(p, &gz("{\"type\":\"session\",\"schema\":1}\n")[..]).unwrap_err().to_string().contains("bad session header"));
        assert!(read(Path::new("/nonexistent/x.ndjson.gz")).is_err());
        // an overlong line is treated as a cut
        let mut text = serde_json::to_string(&HeaderLine { kind: "session", header: &header("") }).unwrap();
        text.push('\n');
        text.push_str(&"x".repeat(MAX_LINE + 10));
        let s = read_from(p, &gz(&text)[..]).unwrap();
        assert!(s.truncated);
        // a release without a press, damaged lines and non-finite depths are skipped
        let mut text = serde_json::to_string(&HeaderLine { kind: "session", header: &header("") }).unwrap();
        text.push_str("\n{\"type\":\"release\",\"t\":5,\"key\":\"KeyZ\"}\n{garbage\n{\"type\":\"depth\",\"t\":6,\"key\":\"KeyZ\",\"mm\":null}\n");
        let s = read_from(p, &gz(&text)[..]).unwrap();
        assert!(s.presses.is_empty() && s.depth.is_empty());
        assert!(!s.truncated);
    }

    #[test]
    fn files_being_written() {
        let d = temp_dir("session-live");
        let p = d.join("a.ndjson.gz");
        let f = File::create(&p).unwrap();
        assert!(is_being_written(&p), "just written");
        // this process holds it open (found in /proc even when old)
        assert!(is_open_anywhere(&p, Path::new("/proc")));
        drop(f);
        assert!(!is_open_anywhere(&p, Path::new("/proc")));
        let old = std::time::SystemTime::now() - Duration::from_secs(60);
        File::options().write(true).open(&p).unwrap().set_modified(old).unwrap();
        assert!(!is_being_written(&p), "old and closed: cut off");
        assert!(!is_being_written(&d.join("missing")));
        assert!(!is_open_anywhere(&p, &d.join("no-proc")));
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn retention() {
        let d = temp_dir("session-prune");
        for i in 0..6 {
            fs::write(d.join(format!("2026110{i}-000000-a{EXT}")), vec![0u8; 100]).unwrap();
        }
        fs::write(d.join("notes.txt"), "keep").unwrap();
        // by count
        let gone = prune(&d, 4, u64::MAX, None).unwrap();
        assert_eq!(gone.len(), 2);
        assert!(gone[0].ends_with(format!("20261101-000000-a{EXT}")), "{gone:?}");
        assert_eq!(list(&d).unwrap().len(), 4);
        // by size: 250 bytes keep two of the 100-byte files
        prune(&d, 100, 250, None).unwrap();
        let left = list(&d).unwrap();
        assert_eq!(left.len(), 2);
        assert!(left[1].ends_with(format!("20261105-000000-a{EXT}")));
        // the file being written is kept even when it is old
        let current = left[0].clone();
        prune(&d, 0, 0, Some(&current)).unwrap();
        assert_eq!(list(&d).unwrap(), vec![current]);
        assert!(d.join("notes.txt").exists());
        assert!(prune(&d.join("missing"), 1, 1, None).unwrap().is_empty());
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn resolving_session_arguments() {
        let d = temp_dir("session-resolve");
        assert!(resolve(&d, "latest").unwrap_err().to_string().contains("no recorded sessions"));
        let a = d.join(format!("20261101-000000-a{EXT}"));
        let b = d.join(format!("20261102-000000-b{EXT}"));
        fs::write(&a, "").unwrap();
        fs::write(&b, "").unwrap();
        assert_eq!(resolve(&d, "latest").unwrap(), b);
        assert_eq!(resolve(&d, "20261101-000000-a").unwrap(), a);
        assert_eq!(resolve(&d, &format!("20261101-000000-a{EXT}")).unwrap(), a);
        assert_eq!(resolve(&d, a.to_str().unwrap()).unwrap(), a);
        assert!(resolve(&d, "nope").is_err());
        fs::remove_dir_all(&d).unwrap();
    }
}

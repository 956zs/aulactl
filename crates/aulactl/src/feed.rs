//! feed: live key events as NDJSON on stdout for other programs (`docs/feed.md`). Waits for the keyboard,
//! streams while it is there, waits again when it goes away, and closes the travel test on SIGINT/SIGTERM.

use crate::args::{parse_num, Args};
use crate::{bad_args, put, select_device, signal, wanted_node};
use aula_core::feed::{self, DepthCoalescer, KeyState, Line, Status};
use aula_core::hidraw::{self, Device, Link};
use aula_core::layout;
use aula_core::perkey::key_name;
use aula_core::{Error, Keyboard};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// Time between connection attempts.
const RETRY: Duration = Duration::from_secs(1);
/// Longest wait for a sample before checking the stop flag again.
const TICK: Duration = Duration::from_millis(50);
/// How long opening waits for another aulactl / aula-gui operation before reporting `busy`.
const LOCK_WAIT: Duration = Duration::from_millis(200);
const DEFAULT_DEPTH_HZ: u32 = 120;

/// What the feed streams.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    pub keys: Vec<usize>,
    /// Depth updates per key and second; 0 = none.
    pub depth_hz: u32,
}

pub fn parse_options(rest: &[&str]) -> Result<Options, String> {
    let mut a = Args::new(rest);
    let keys = a.opt("--keys")?;
    let depth_hz = a
        .opt("--depth-hz")?
        .map(|s| parse_num(s, 0..=1000, "depth rate in Hz"))
        .transpose()?
        .unwrap_or(DEFAULT_DEPTH_HZ);
    if !a.positional()?.is_empty() {
        return Err(bad_args(&[&["feed"], rest].concat()));
    }
    let keys = match keys {
        Some(s) => layout::select(s)?,
        None => layout::all_indices(),
    };
    Ok(Options { keys, depth_hz })
}

/// `aulactl feed`: runs until SIGINT/SIGTERM (Ok) or a fatal error, which is also written as an `error` line.
pub fn run(device: Option<PathBuf>, rest: &[&str]) -> Result<(), String> {
    let mut stdout = std::io::stdout().lock();
    let r = run_to(&mut stdout, device, rest);
    if let Err(e) = &r {
        if e != crate::PIPE_CLOSED {
            let _ = emit(&mut stdout, &Line::Error { msg: e });
        }
    }
    r
}

fn run_to(out: &mut dyn Write, device: Option<PathBuf>, rest: &[&str]) -> Result<(), String> {
    let opts = parse_options(rest)?;
    signal::install()?;
    let wanted = wanted_node(device);
    let start = Instant::now();
    let mut shown: Option<Status> = None;
    let mut reported: Option<String> = None;
    loop {
        if signal::interrupted() {
            return Ok(());
        }
        let status = match connect(wanted.as_deref()) {
            Ok(mut kb) => match session(&mut kb, &opts, out, start, &mut signal::interrupted)? {
                End::Stopped => return Ok(()),
                End::Lost => {
                    emit(out, &Line::status(Status::Lost))?;
                    shown = Some(Status::Lost);
                    reported = None;
                    None
                }
                End::NotReady(e) => Some(e),
            },
            Err(e) => Some(e),
        };
        if let Some(e) = status {
            let s = waiting_status(&e)?;
            if shown != Some(s) {
                emit(out, &Line::status(s))?;
                shown = Some(s);
            }
            // humans only: why the board could not be used (e.g. a missing udev rule), once per reason
            let text = e.to_string();
            if !matches!(e, Error::NotFound) && reported.as_ref() != Some(&text) {
                eprintln!("aulactl feed: {text}; retrying");
                reported = Some(text);
            }
        }
        let until = Instant::now() + RETRY;
        while Instant::now() < until && !signal::interrupted() {
            std::thread::sleep(TICK);
        }
    }
}

/// The `waiting` status for a failed connection, or the error when retrying cannot help.
pub fn waiting_status(e: &Error) -> Result<Status, String> {
    match e {
        Error::Busy(_) => Ok(Status::Busy),
        Error::NotFound | Error::Io(_) | Error::Timeout(_) | Error::Protocol(_) => {
            Ok(Status::NoKeyboard)
        }
        e => Err(e.to_string()),
    }
}

fn connect(wanted: Option<&std::path::Path>) -> Result<Keyboard<Device>, Error> {
    let info = select_device(hidraw::find_devices(), wanted).map_err(|_| Error::NotFound)?;
    let mut dev = Device::open(&info.node)?;
    dev.set_lock_wait(LOCK_WAIT);
    // No backup directory: the feed never writes (the travel test open/close only touch RAM).
    Keyboard::with_link(dev)
}

/// How a session ended.
#[derive(Debug, PartialEq, Eq)]
pub enum End {
    /// The stop flag was set; the travel test was closed.
    Stopped,
    /// The keyboard went away after `hello`.
    Lost,
    /// The keyboard could not be set up; nothing was streamed.
    NotReady(Error),
}

/// One connection: reads the selected keys' settings, opens the travel test, writes `hello` and streams events
/// until `stop` returns true or the keyboard goes away. Output errors end it with Err (the stream is closed
/// first).
pub fn session<L: Link>(
    kb: &mut Keyboard<L>,
    opts: &Options,
    out: &mut dyn Write,
    start: Instant,
    stop: &mut dyn FnMut() -> bool,
) -> Result<End, String> {
    if !kb.is_supported() {
        return Err(Error::UnsupportedProduct {
            product_code: kb.identity().product_code.clone(),
        }
        .to_string());
    }
    let setup = kb.travel_limits().and_then(|l| {
        let triggers = kb.read_triggers(&opts.keys)?;
        Ok((l, triggers))
    });
    let (limits, triggers) = match setup {
        Ok(v) => v,
        Err(e) => return Ok(End::NotReady(e)),
    };
    let hello =
        feed::hello(kb.identity(), &limits, &triggers, &opts.keys).map_err(|e| e.to_string())?;
    let mut keys: BTreeMap<usize, (String, KeyState)> = BTreeMap::new();
    for (&idx, t) in &triggers.0 {
        keys.insert(idx as usize, (key_name(idx as usize), KeyState::new(*t)));
    }
    let mut depth = DepthCoalescer::new(opts.depth_hz);
    let mut stream = match kb.travel_stream(&opts.keys) {
        Ok(s) => s,
        Err(e) => return Ok(End::NotReady(e)),
    };
    let ms = || start.elapsed().as_secs_f64() * 1000.0;
    let mut lost = false;
    let mut written = emit(out, &Line::Hello(&hello));
    while written.is_ok() && !stop() {
        let wait = depth
            .next_due(ms())
            .map_or(TICK, |d| Duration::from_secs_f64(d / 1000.0).min(TICK));
        match stream.next_sample(wait) {
            Err(_) => {
                lost = true;
                break;
            }
            Ok(Some(s)) => {
                let t = ms();
                let Some((name, state)) = keys.get_mut(&s.index) else {
                    continue;
                };
                if let Some(d) = depth.sample(s.index, t, s.depth) {
                    written = emit(out, &Line::depth(t, name, feed::mm(&limits, d)));
                }
                if let Some(edge) = state.update(s.depth) {
                    written = written.and_then(|_| emit(out, &Line::edge(t, name, edge)));
                }
            }
            Ok(None) => {}
        }
        let t = ms();
        for (idx, d) in depth.due(t) {
            if let Some((name, _)) = keys.get(&idx) {
                written =
                    written.and_then(|_| emit(out, &Line::depth(t, name, feed::mm(&limits, d))));
            }
        }
    }
    if lost {
        // the board is gone: the close cannot be acknowledged
        let _ = stream.close();
        return Ok(End::Lost);
    }
    if let Err(e) = written {
        let _ = stream.close();
        return Err(e);
    }
    stream
        .close()
        .map_err(|e| format!("closing the travel test: {e}"))?;
    Ok(End::Stopped)
}

/// Writes one line and flushes it.
fn emit(out: &mut dyn Write, line: &Line) -> Result<(), String> {
    put(out, &format!("{}\n", line.to_json()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use aula_core::frame::{row_col, Body, BODY_LEN};
    use aula_core::sim::SimKeyboard;

    fn idx(code: &str) -> usize {
        layout::by_code(code).unwrap().index as usize
    }

    fn sample(code: &str, depth: Option<u16>) -> Body {
        let (row, col) = row_col(idx(code));
        let mut b = [0u8; BODY_LEN];
        b[0] = 0x21;
        b[5] = 0x01;
        b[6] = row;
        b[7] = col;
        match depth {
            Some(d) => {
                b[4] = 5;
                b[8..10].copy_from_slice(&d.to_le_bytes());
            }
            None => {
                b[4] = 3;
                b[8..12].copy_from_slice(&[0x08, 0, 0xff, 0xff]);
            }
        }
        b
    }

    /// The owner's board with KeyZ pressed to the bottom and released.
    fn board(unplug: bool) -> Keyboard<SimKeyboard> {
        let mut sim = SimKeyboard::owner_board();
        sim.travel_script = [
            Some(50),
            Some(150),
            Some(340),
            Some(150),
            Some(30),
            None,
            None,
        ]
        .iter()
        .map(|d| sample("KeyZ", *d))
        .collect();
        sim.unplug_after_script = unplug;
        Keyboard::with_link(sim).unwrap()
    }

    fn opts(hz: u32) -> Options {
        Options {
            keys: vec![idx("KeyZ"), idx("KeyX")],
            depth_hz: hz,
        }
    }

    fn stop_after(n: usize) -> impl FnMut() -> bool {
        let mut calls = 0;
        move || {
            calls += 1;
            calls > n
        }
    }

    fn lines(out: &[u8]) -> Vec<serde_json::Value> {
        String::from_utf8(out.to_vec())
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    fn types(lines: &[serde_json::Value]) -> Vec<String> {
        lines
            .iter()
            .map(|l| l["type"].as_str().unwrap().to_string())
            .collect()
    }

    #[test]
    fn session_streams_hello_then_events_and_closes_last() {
        let mut kb = board(false);
        let mut out = Vec::new();
        let end = session(
            &mut kb,
            &opts(1000),
            &mut out,
            Instant::now(),
            &mut stop_after(20),
        )
        .unwrap();
        assert_eq!(end, End::Stopped);
        let l = lines(&out);
        assert_eq!(l[0]["type"], "hello");
        assert_eq!(l[0]["product"], "SI2825KZHEARGB");
        let keys = l[0]["keys"].as_array().unwrap();
        assert_eq!(keys.len(), 2);
        assert_eq!(keys[0]["code"], "KeyZ");
        assert_eq!(
            (keys[0]["mode"].as_str(), keys[0]["point"].as_f64()),
            (Some("rt"), Some(1.0))
        );
        let t = types(&l);
        assert_eq!(t.iter().filter(|t| *t == "press").count(), 1, "{t:?}");
        assert_eq!(t.iter().filter(|t| *t == "release").count(), 1, "{t:?}");
        let press = t.iter().position(|t| t == "press").unwrap();
        let release = t.iter().position(|t| t == "release").unwrap();
        assert!(press < release);
        assert!(l[1..]
            .iter()
            .all(|x| x["type"] == "hello" || x["key"] == "KeyZ"));
        let depths: Vec<f64> = l
            .iter()
            .filter(|x| x["type"] == "depth")
            .map(|x| x["mm"].as_f64().unwrap())
            .collect();
        assert_eq!(depths.first(), Some(&0.5));
        assert_eq!(depths.last(), Some(&0.0), "final rest depth: {depths:?}");
        assert_eq!(
            depths.iter().filter(|d| **d == 0.0).count(),
            1,
            "repeated rest frames send one 0"
        );
        let sent = &kb.link().sent;
        assert!(
            sent.iter()
                .any(|b| b[0] == 0x21 && b[4] == 0x18 && b[5] == 0x02),
            "opened"
        );
        assert_eq!(
            sent.last().map(|b| (b[0], b[4], b[5])),
            Some((0x21, 0x18, 0x03)),
            "closed last"
        );
        assert!(!kb.link().travel_open);
    }

    #[test]
    fn session_without_depth_lines() {
        let mut kb = board(false);
        let mut out = Vec::new();
        session(
            &mut kb,
            &opts(0),
            &mut out,
            Instant::now(),
            &mut stop_after(20),
        )
        .unwrap();
        assert_eq!(types(&lines(&out)), ["hello", "press", "release"]);
    }

    #[test]
    fn stopped_before_any_sample() {
        let mut kb = board(false);
        let mut out = Vec::new();
        assert_eq!(
            session(&mut kb, &opts(120), &mut out, Instant::now(), &mut || true).unwrap(),
            End::Stopped
        );
        assert_eq!(types(&lines(&out)), ["hello"]);
        assert!(!kb.link().travel_open);
    }

    #[test]
    fn unplug_ends_the_session_as_lost() {
        let mut kb = board(true);
        let mut out = Vec::new();
        let end = session(
            &mut kb,
            &opts(1000),
            &mut out,
            Instant::now(),
            &mut stop_after(1000),
        )
        .unwrap();
        assert_eq!(end, End::Lost);
        let l = lines(&out);
        let t = types(&l);
        assert_eq!(t.first().map(String::as_str), Some("hello"));
        assert!(t.iter().any(|t| t == "release"));
        assert_eq!(
            (
                l.last().unwrap()["type"].as_str(),
                l.last().unwrap()["mm"].as_f64()
            ),
            (Some("depth"), Some(0.0))
        );
    }

    #[test]
    fn a_board_that_does_not_answer_is_not_ready() {
        let mut sim = SimKeyboard::owner_board();
        sim.mute_cmd = Some(0x21);
        let mut kb = Keyboard::with_link(sim).unwrap();
        let mut out = Vec::new();
        let end = session(&mut kb, &opts(120), &mut out, Instant::now(), &mut || false).unwrap();
        assert!(matches!(end, End::NotReady(Error::Timeout(_))), "{end:?}");
        assert!(out.is_empty(), "no hello");
        assert!(
            !kb.link().sent.iter().any(|b| b[4] == 0x18 && b[5] == 0x02),
            "never opened"
        );
    }

    #[test]
    fn other_boards_are_refused() {
        let mut kb = Keyboard::with_link(SimKeyboard::other_board()).unwrap();
        let mut out = Vec::new();
        let e = session(&mut kb, &opts(120), &mut out, Instant::now(), &mut || false).unwrap_err();
        assert!(e.contains("unsupported board"), "{e}");
        assert!(out.is_empty());
    }

    struct ClosedPipe;

    impl Write for ClosedPipe {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::ErrorKind::BrokenPipe.into())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn closed_stdout_ends_quietly_and_closes_the_stream() {
        let mut kb = board(false);
        let e = session(
            &mut kb,
            &opts(120),
            &mut ClosedPipe,
            Instant::now(),
            &mut || false,
        )
        .unwrap_err();
        assert_eq!(e, crate::PIPE_CLOSED);
        assert_eq!(
            kb.link().sent.last().map(|b| (b[4], b[5])),
            Some((0x18, 0x03))
        );
        assert!(!kb.link().travel_open);
    }

    #[test]
    fn options() {
        assert_eq!(
            parse_options(&[]).unwrap(),
            Options {
                keys: layout::all_indices(),
                depth_hz: 120
            }
        );
        let o = parse_options(&["--keys", "KeyZ,KeyX", "--depth-hz", "0"]).unwrap();
        assert_eq!(
            o,
            Options {
                keys: layout::select("KeyX,KeyZ").unwrap(),
                depth_hz: 0
            }
        );
        assert_eq!(
            parse_options(&["--depth-hz", "1000"]).unwrap().depth_hz,
            1000
        );
        assert!(parse_options(&["--depth-hz", "1001"]).is_err());
        assert!(parse_options(&["--depth-hz", "-1"]).is_err());
        assert!(parse_options(&["--depth-hz"]).is_err());
        assert!(parse_options(&["--keys", ""]).is_err());
        assert!(parse_options(&["--keys", "nope"]).is_err());
        assert!(parse_options(&["KeyZ"]).is_err());
    }

    #[test]
    fn waiting_reasons() {
        assert_eq!(waiting_status(&Error::NotFound), Ok(Status::NoKeyboard));
        assert_eq!(
            waiting_status(&Error::Io("permission denied".into())),
            Ok(Status::NoKeyboard)
        );
        assert_eq!(
            waiting_status(&Error::Timeout("x".into())),
            Ok(Status::NoKeyboard)
        );
        assert_eq!(waiting_status(&Error::Busy("x".into())), Ok(Status::Busy));
        assert!(waiting_status(&Error::UnsupportedProduct {
            product_code: "X".into()
        })
        .is_err());
        assert!(waiting_status(&Error::Invalid("x".into())).is_err());
    }
}

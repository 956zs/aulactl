mod args;
mod device;
mod feed;
mod keys;
mod light;
mod macro_text;
mod render;
mod replay;
mod signal;
#[cfg(test)]
mod tests;
mod trigger;

use aula_core::hidraw::{self, DeviceInfo, Link};
use aula_core::store::Profiles;
use aula_core::Keyboard;
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

const USAGE: &str = "usage: aulactl [--device /dev/hidrawN] <command>

KEYS is a key selector: key codes or legends (KeyW, Esc, L-Shift, a), groups (wasd, letters, digits, mods,
all), row:0..4, idx:N, comma separated (e.g. wasd,Space). Lengths are in mm (1.20). Every write is read
back, and the first write of a run saves an automatic backup of all settings first.

device
  list                          detected keyboards and their hidraw nodes
  info                          product, firmware, travel range, poll rate, capabilities
  dump [FILE] [--force]         all settings as JSON (stdout, or a new FILE; --force replaces FILE)
  backup [FILE] [--force]       save all settings to a new FILE (default: the backups directory)
  diff FILE                     what restoring FILE would change
  restore FILE [--yes]          show what FILE changes; write it with --yes
  profile list                  saved profiles
  profile save NAME [--force]   save the keyboard's settings as profile NAME
  profile show NAME             summary of a profile
  profile diff NAME             what applying NAME would change
  profile apply NAME [--yes]    show what NAME changes; write it with --yes
  profile delete NAME --yes     delete a profile file

actuation
  actuation show [KEYS] [--table]
                                actuation point and rapid trigger per key
  actuation set KEYS [--point MM] [--rt PRESS[/RELEASE]] [--mode fixed|rt|rt-split]
                                --rt with one value selects rt, with two rt-split
  deadband show [KEYS]          dead band at the top and bottom of the travel
  deadband set KEYS [--top MM] [--bottom MM]
  switch show | switch list     switch type per key | types this board offers
  switch set KEYS TYPE          switch type by id or name (recalibrate afterwards)
  pollrate [1|2|4|8]            show or set the poll rate in kHz (the keyboard restarts)
  travel [KEYS] [--raw] [--seconds N]
                                live key depth on a keyboard map; Ctrl-C stops
  feed [--keys KEYS] [--depth-hz N] [--record [DIR] [--label TEXT]]
                                live key depth and press/release as JSON lines for other programs
                                (docs/feed.md); waits for the keyboard, SIGINT/SIGTERM stop;
                                --record also saves every raw sample to a session file
  calibrate --yes               any-key calibration: press every key fully, Enter finishes
  reset triggers --yes          every key's actuation and rapid trigger back to factory
  reset factory --yes           factory reset of the whole keyboard

keys
  keymap show [--layer base|fn] [--all]
  keymap set KEYS ACTION [--layer base|fn]
                                ACTION: KeyA, Esc, Fn, Default, VolumeUp, MouseLeft, Copy,
                                Ctrl+Shift+Esc, Macro0..9, Adv0..39
  keymap reset KEYS [--layer base|fn]
  keymap codes [FILTER]         every ACTION name
  macro list                    the 10 macro slots
  macro show SLOT
  macro set SLOT STEPS [--mode once|repeat|toggle|hold] [--repeat N] [--delay MS] [--bind KEYS]
                                STEPS: \"down KeyA, wait 20, up KeyA, tap b\"; --delay (default 10 ms)
                                is the wait after steps without one
  macro clear SLOT
  adv list                      advanced keys (40 slots)
  adv set SLOT dks ACTION:PHASES.. [--routes R1,R2,R3,R4 | --point MM] [--bind KEYS]
                                PHASES: 4 of - t p h (nothing, tap, press, held), e.g. KeyA:p--h
  adv set SLOT mt HOLD TAP [--threshold MS] [--bind KEYS]
  adv set SLOT tgl ACTION [--bind KEYS]
  adv clear SLOT
  socd [list]                   SOCD state and key pairs
  socd add KEY1 KEY2 [--mode last|key1|key2|neutral]
  socd remove N | socd remove KEY1 KEY2
  socd on|off
  winlock [win|shift-tab|alt-f4|alt-tab|none ..]
                                show or set what the on-keyboard win-lock blocks

lighting
  light [show]                  effect, brightness, speed, colours, custom slot
  light modes                   effects this board offers
  light effect MODE [--brightness N] [--speed N] [--color #rrggbb] [--bg #rrggbb]
                    [--direction D] [--rainbow on|off]
  light brightness N | speed N | direction D | rainbow on|off | on | off
  light color #rrggbb [#rrggbb] foreground and background colour
  light custom [show] [SLOT]    per-key colours of a custom slot
  light custom set KEYS #rrggbb [--slot 0|1]
  light slot [0|1]              show or select the custom slot the custom effect shows
  light music on|off            music-rhythm mode (needs the vendor desktop plugin to animate)

replay (osu! replays vs recorded feed sessions; no keyboard needed)
  replay sessions [--limit N]   recorded sessions, newest first
  replay match [--exports DIR] [--limit N]
                                exported .osr files that belong to each session
                                (default DIR ~/.local/share/osu/exports)
  replay report [SESSION|latest] [--osr FILE] [--exports DIR] [--open]
                                align a replay with a session and write an HTML report";

/// What `run` does after a command.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Done,
    /// The keyboard restarts (poll-rate change); the value is the poll rate to read back.
    Restarting(Option<u8>),
}

/// One command's environment: the keyboard, where output goes, and host-side settings.
pub struct Cli<'a, L: Link> {
    pub kb: &'a mut Keyboard<L>,
    pub out: &'a mut dyn Write,
    /// The hidraw node, for `info`.
    pub node: Option<PathBuf>,
    /// Profiles directory (None: the default under `$XDG_DATA_HOME`).
    pub profiles: Option<PathBuf>,
    /// Output is a terminal: colours and in-place redraws.
    pub tty: bool,
    backup_reported: bool,
}

impl<'a, L: Link> Cli<'a, L> {
    pub fn new(kb: &'a mut Keyboard<L>, out: &'a mut dyn Write) -> Self {
        Cli {
            kb,
            out,
            node: None,
            profiles: None,
            tty: false,
            backup_reported: false,
        }
    }

    /// Mentions the automatic backup once, after the first write.
    pub fn report_backup(&mut self) -> Result<(), String> {
        if self.backup_reported {
            return Ok(());
        }
        if let Some(p) = self.kb.last_backup() {
            let line = format!("automatic backup of the previous settings: {}", p.display());
            self.backup_reported = true;
            say(self.out, &line)?;
        }
        Ok(())
    }
}

/// Error text for output going to a closed pipe (`aulactl macro list | head -3`): exit quietly.
const PIPE_CLOSED: &str = "output: pipe closed";

fn output_error(e: std::io::Error) -> String {
    if e.kind() == std::io::ErrorKind::BrokenPipe {
        PIPE_CLOSED.into()
    } else {
        format!("output: {e}")
    }
}

/// Writes one line of output.
pub fn say(out: &mut dyn Write, line: &str) -> Result<(), String> {
    writeln!(out, "{line}").map_err(output_error)
}

/// Writes text as is (it carries its own newlines).
pub fn put(out: &mut dyn Write, text: &str) -> Result<(), String> {
    out.write_all(text.as_bytes())
        .and_then(|_| out.flush())
        .map_err(output_error)
}

pub fn profiles(dir: Option<&Path>) -> Result<Profiles, String> {
    match dir {
        Some(d) => Ok(Profiles::in_dir(d.to_path_buf())),
        None => Ok(Profiles::open_default()?),
    }
}

fn main() -> ExitCode {
    match run(std::env::args().skip(1).collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) if e == PIPE_CLOSED => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("aulactl: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(mut args: Vec<String>) -> Result<(), String> {
    let device = take_device_arg(&mut args)?;
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let mut stdout = std::io::stdout().lock();
    // commands that do not talk to the keyboard
    match args.as_slice() {
        [] | ["-h" | "--help" | "help"] => return say(&mut stdout, USAGE),
        ["list"] => return cmd_list(&mut stdout),
        ["keymap", "codes", rest @ ..] => return keys::codes(&mut stdout, rest),
        ["replay", rest @ ..] => {
            return replay::run(&mut stdout, &replay::Dirs::default(), rest)
        }
        ["feed", rest @ ..] => {
            drop(stdout);
            return feed::run(device, rest);
        }
        ["profile", "list" | "show" | "delete", ..] => {
            return device::profile_offline(&mut stdout, None, &args[1..])
        }
        _ => {}
    }
    if !is_device_command(args[0]) {
        return Err(format!(
            "unknown command '{}'; see 'aulactl --help'",
            args[0]
        ));
    }
    let info = select_device(hidraw::find_devices(), wanted_node(device).as_deref())?;
    let mut kb = Keyboard::open(&info)?;
    let tty = std::io::stdout().is_terminal();
    let mut cli = Cli::new(&mut kb, &mut stdout);
    cli.node = Some(info.node.clone());
    cli.tty = tty;
    match dispatch(&mut cli, &args)? {
        Outcome::Done => Ok(()),
        Outcome::Restarting(khz) => {
            drop(kb);
            reconnect(&mut stdout, &info, khz)
        }
    }
}

const DEVICE_COMMANDS: [&str; 19] = [
    "info",
    "dump",
    "backup",
    "diff",
    "restore",
    "profile",
    "actuation",
    "deadband",
    "switch",
    "pollrate",
    "travel",
    "calibrate",
    "reset",
    "keymap",
    "macro",
    "adv",
    "socd",
    "winlock",
    "light",
];

fn is_device_command(c: &str) -> bool {
    DEVICE_COMMANDS.contains(&c)
}

/// Runs a command that talks to the keyboard.
pub fn dispatch<L: Link>(c: &mut Cli<L>, args: &[&str]) -> Result<Outcome, String> {
    let done = |r: Result<(), String>| r.map(|_| Outcome::Done);
    match args {
        ["info"] => done(device::info(c)),
        ["dump", rest @ ..] => done(device::dump(c, rest)),
        ["backup", rest @ ..] => done(device::backup(c, rest)),
        ["diff", file] => done(device::diff(c, Path::new(file))),
        ["restore", rest @ ..] => device::restore(c, rest),
        ["profile", rest @ ..] => device::profile(c, rest),
        ["actuation", rest @ ..] => done(trigger::actuation(c, rest)),
        ["deadband", rest @ ..] => done(trigger::dead_band(c, rest)),
        ["switch", rest @ ..] => done(trigger::switch(c, rest)),
        ["pollrate", rest @ ..] => trigger::poll_rate(c, rest),
        ["travel", rest @ ..] => done(trigger::travel(c, rest)),
        ["calibrate", rest @ ..] => done(trigger::calibrate(c, rest)),
        ["reset", rest @ ..] => done(trigger::reset(c, rest)),
        ["keymap", rest @ ..] => done(keys::keymap(c, rest)),
        ["macro", rest @ ..] => done(keys::macros(c, rest)),
        ["adv", rest @ ..] => done(keys::advanced(c, rest)),
        ["socd", rest @ ..] => done(keys::socd(c, rest)),
        ["winlock", rest @ ..] => done(keys::win_lock(c, rest)),
        ["light", rest @ ..] => done(light::light(c, rest)),
        _ => Err(bad_args(args)),
    }
}

/// The error for arguments no command form matches.
pub fn bad_args(args: &[&str]) -> String {
    format!(
        "unknown arguments '{}'; see 'aulactl --help'",
        args.join(" ")
    )
}

/// After a poll-rate change: wait for the keyboard that was open (`was`) to leave and come back, then read
/// the rate back.
fn reconnect(out: &mut dyn Write, was: &DeviceInfo, khz: Option<u8>) -> Result<(), String> {
    say(out, "waiting for the keyboard to restart...")?;
    let gone_by = Instant::now() + Duration::from_secs(3);
    while same_board(hidraw::find_devices(), was).is_some() && Instant::now() < gone_by {
        std::thread::sleep(Duration::from_millis(100));
    }
    let back_by = Instant::now() + Duration::from_secs(15);
    let info = loop {
        if let Some(d) = same_board(hidraw::find_devices(), was) {
            break d;
        }
        if Instant::now() >= back_by {
            return Err(format!(
                "the keyboard at {} did not come back within 15 s",
                was.node.display()
            ));
        }
        std::thread::sleep(Duration::from_millis(200));
    };
    // udev may need a moment to grant access to the new node
    let mut tries = 0;
    let mut kb = loop {
        match Keyboard::open(&info) {
            Ok(kb) => break kb,
            Err(e) if tries >= 15 => return Err(e.into()),
            Err(_) => {
                tries += 1;
                std::thread::sleep(Duration::from_millis(200));
            }
        }
    };
    match khz {
        Some(k) => {
            let now = kb.read_poll_rate()?;
            if now != k {
                return Err(format!(
                    "the keyboard is back but reports {now} kHz instead of {k} kHz"
                ));
            }
            say(
                out,
                &format!("the keyboard is back; poll rate {now} kHz (read back)"),
            )
        }
        None => say(out, "the keyboard is back"),
    }
}

fn take_device_arg(args: &mut Vec<String>) -> Result<Option<PathBuf>, String> {
    let Some(i) = args.iter().position(|a| a == "--device" || a == "-d") else {
        return Ok(None);
    };
    if i + 1 >= args.len() {
        return Err("--device needs a path".into());
    }
    let path = args.remove(i + 1);
    args.remove(i);
    Ok(Some(PathBuf::from(path)))
}

/// `--device` resolved through symlinks (such as a udev alias) to the /dev/hidrawN that find_devices reports.
fn wanted_node(device: Option<PathBuf>) -> Option<PathBuf> {
    device.map(|p| std::fs::canonicalize(&p).unwrap_or(p))
}

/// The interface of the board that was open, after it re-enumerated: same USB port path (`HID_PHYS`, which
/// survives a new hidrawN number), or the same node when the kernel reports no port path.
fn same_board(devs: Vec<DeviceInfo>, was: &DeviceInfo) -> Option<DeviceInfo> {
    devs.into_iter().find(|d| {
        if was.phys.is_empty() {
            d.node == was.node
        } else {
            d.phys == was.phys
        }
    })
}

/// The interface to use: the one named by `--device`, which must be a detected vendor interface, or the
/// first one (WIN 60 HE product string first).
fn select_device(devs: Vec<DeviceInfo>, wanted: Option<&Path>) -> Result<DeviceInfo, String> {
    if let Some(node) = wanted {
        return devs.into_iter().find(|d| d.node == node).ok_or_else(|| {
            format!(
                "{} is not the vendor interface of a WIN 60 HE; see 'aulactl list'",
                node.display()
            )
        });
    }
    devs.into_iter()
        .next()
        .ok_or_else(|| aula_core::Error::NotFound.to_string())
}

fn cmd_list(out: &mut dyn Write) -> Result<(), String> {
    let devs = hidraw::find_devices();
    if devs.is_empty() {
        return Err(aula_core::Error::NotFound.to_string());
    }
    for d in devs {
        let note = if d.name_matches() {
            ""
        } else {
            "  (other board sharing the USB id)"
        };
        say(
            out,
            &format!(
                "{}  {:04x}:{:04x}  {}  {}{note}",
                d.node.display(),
                hidraw::VENDOR_ID,
                hidraw::PRODUCT_ID,
                d.name,
                d.phys
            ),
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod main_tests {
    use super::*;

    fn dev(node: &str, name: &str) -> DeviceInfo {
        DeviceInfo {
            node: PathBuf::from(node),
            name: name.into(),
            phys: String::new(),
        }
    }

    #[test]
    fn reconnect_follows_the_open_board() {
        let at = |node: &str, phys: &str| DeviceInfo {
            phys: phys.into(),
            ..dev(node, "WIN 60 HE")
        };
        let was = at("/dev/hidraw7", "usb-0000:00:14.0-2/input2");
        // renumbered, and another board listed first
        let devs = vec![
            at("/dev/hidraw3", "usb-0000:00:14.0-1/input2"),
            at("/dev/hidraw8", "usb-0000:00:14.0-2/input2"),
        ];
        assert_eq!(same_board(devs.clone(), &was).unwrap().node, PathBuf::from("/dev/hidraw8"));
        assert!(same_board(devs[..1].to_vec(), &was).is_none(), "only the other board: still gone");
        assert!(same_board(vec![], &was).is_none());
        // no port path: same node only
        let bare = dev("/dev/hidraw7", "WIN 60 HE");
        assert!(same_board(vec![dev("/dev/hidraw3", "WIN 60 HE")], &bare).is_none());
        assert!(same_board(vec![dev("/dev/hidraw7", "WIN 60 HE")], &bare).is_some());
    }

    #[test]
    fn device_selection() {
        let win = || dev("/dev/hidraw7", "WIN 60 HE");
        let other = || dev("/dev/hidraw9", "WIN68 HE");
        assert_eq!(
            select_device(vec![win(), other()], None).unwrap().node,
            win().node
        );
        assert_eq!(
            select_device(vec![win(), other()], Some(Path::new("/dev/hidraw9")))
                .unwrap()
                .node,
            other().node
        );
        let err = select_device(vec![win()], Some(Path::new("/dev/hidraw3"))).unwrap_err();
        assert!(
            err.contains("/dev/hidraw3 is not the vendor interface"),
            "{err}"
        );
        assert!(select_device(vec![], None)
            .unwrap_err()
            .contains("no AULA WIN 60 HE found"));
        assert_eq!(wanted_node(None), None);
        assert_eq!(
            wanted_node(Some(PathBuf::from("/nonexistent/x"))),
            Some(PathBuf::from("/nonexistent/x"))
        );
    }

    #[test]
    fn device_argument() {
        let mut a: Vec<String> = ["info", "--device", "/dev/hidraw7"]
            .map(String::from)
            .to_vec();
        assert_eq!(
            take_device_arg(&mut a),
            Ok(Some(PathBuf::from("/dev/hidraw7")))
        );
        assert_eq!(a, vec!["info"]);
        let mut a: Vec<String> = ["-d"].map(String::from).to_vec();
        assert!(take_device_arg(&mut a).is_err());
        let mut a: Vec<String> = vec![];
        assert_eq!(take_device_arg(&mut a), Ok(None));
    }

    #[test]
    fn command_names() {
        assert!(is_device_command("info") && is_device_command("light"));
        assert!(!is_device_command("list") && !is_device_command("bogus"));
        assert_eq!(
            bad_args(&["light", "x"]),
            "unknown arguments 'light x'; see 'aulactl --help'"
        );
    }
}

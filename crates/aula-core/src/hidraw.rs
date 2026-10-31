//! hidraw transport: find the WIN 60 HE vendor interface and move 63-byte bodies in and out.

use crate::error::{Error, Result};
use crate::frame::{hex, Body, BODY_LEN, REPORT_ID};
use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub const VENDOR_ID: u16 = 0x2e3c;
/// Shared by several boards (protocol 1.1): the product code decides.
pub const PRODUCT_ID: u16 = 0xc365;
/// USB product string of the WIN 60 HE.
pub const PRODUCT_NAME: &str = "WIN 60 HE";
/// Vendor collection: usage page 0xFF1B, usage 0x91.
const VENDOR_USAGE_PAGE: u32 = 0xff1b;
const VENDOR_USAGE: u32 = 0x91;
const HIDRAW_SYSFS: &str = "/sys/class/hidraw";

/// Set to print every frame on stderr.
pub const DEBUG_ENV: &str = "AULACTL_DEBUG";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceInfo {
    pub node: PathBuf,
    /// `HID_NAME`, the USB product string (`WIN 60 HE`).
    pub name: String,
    /// `HID_PHYS`, e.g. `usb-0000:00:14.0-1.2/input2`.
    pub phys: String,
}

impl DeviceInfo {
    /// Whether the USB product string is the WIN 60 HE's. Other boards share the PID.
    pub fn name_matches(&self) -> bool {
        self.name.trim() == PRODUCT_NAME
    }
}

/// Vendor interfaces of every `2e3c:c365` board, WIN 60 HE product string first, then by node.
pub fn find_devices() -> Vec<DeviceInfo> {
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir(HIDRAW_SYSFS) else { return out };
    for e in entries.flatten() {
        let sys = e.path();
        let Ok(uevent) = fs::read_to_string(sys.join("device/uevent")) else { continue };
        if parse_hid_id(&uevent) != Some((VENDOR_ID, PRODUCT_ID)) {
            continue;
        }
        let Ok(desc) = fs::read(sys.join("device/report_descriptor")) else { continue };
        if !is_vendor_collection(&desc) {
            continue;
        }
        let field = |key: &str| uevent.lines().find_map(|l| l.strip_prefix(key)).unwrap_or("").to_string();
        out.push(DeviceInfo { node: Path::new("/dev").join(e.file_name()), name: field("HID_NAME="), phys: field("HID_PHYS=") });
    }
    out.sort_by(|a, b| b.name_matches().cmp(&a.name_matches()).then_with(|| a.node.cmp(&b.node)));
    out
}

/// Waits until a vendor interface shows up again (after a poll-rate change re-enumerates the board).
pub fn wait_for_device(timeout: Duration) -> Result<DeviceInfo> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(d) = find_devices().into_iter().next() {
            return Ok(d);
        }
        if Instant::now() >= deadline {
            return Err(Error::Timeout(format!("the keyboard did not come back within {} s", timeout.as_secs())));
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// Parses `HID_ID=0003:00002E3C:0000C365` from a hidraw uevent.
pub fn parse_hid_id(uevent: &str) -> Option<(u16, u16)> {
    let id = uevent.lines().find_map(|l| l.strip_prefix("HID_ID="))?;
    let mut parts = id.split(':');
    let _bus = parts.next()?;
    let vid = u32::from_str_radix(parts.next()?, 16).ok()?;
    let pid = u32::from_str_radix(parts.next()?, 16).ok()?;
    Some((u16::try_from(vid).ok()?, u16::try_from(pid).ok()?))
}

/// Whether the first collection of a report descriptor is usage page 0xFF1B / usage 0x91.
pub fn is_vendor_collection(desc: &[u8]) -> bool {
    let (mut page, mut usage) = (None, None);
    let mut i = 0;
    while i < desc.len() {
        let prefix = desc[i];
        if prefix == 0xfe {
            // long item: 0xfe, size, tag, data
            let size = *desc.get(i + 1).unwrap_or(&0) as usize;
            i += 3 + size;
            continue;
        }
        let size = match prefix & 0x03 {
            3 => 4,
            n => n as usize,
        };
        let Some(data) = desc.get(i + 1..i + 1 + size) else { return false };
        let value = data.iter().rev().fold(0u32, |acc, b| (acc << 8) | *b as u32);
        match prefix & 0xfc {
            0x04 => page = page.or(Some(value)),   // Usage Page
            0x08 => usage = usage.or(Some(value)), // Usage
            0xa0 => break,                         // Collection
            _ => {}
        }
        i += 1 + size;
    }
    page == Some(VENDOR_USAGE_PAGE) && usage == Some(VENDOR_USAGE)
}

/// Moves frames to and from a keyboard. `Device` talks to hidraw; the `sim` feature provides an in-memory
/// keyboard for tests.
pub trait Link {
    /// Sends one Output report body.
    fn send(&mut self, body: &Body) -> Result<()>;
    /// Next Input report body, or None when nothing arrived within `timeout`.
    fn recv(&mut self, timeout: Duration) -> Result<Option<Body>>;
    /// Discards queued input.
    fn drain(&mut self) -> Result<()>;
    /// Exclusive access across processes. Re-entrant: every `lock` needs one `unlock`.
    fn lock(&mut self) -> Result<()> {
        Ok(())
    }
    fn unlock(&mut self) {}
}

/// `drain` stops after this many stale frames or this long, whichever comes first.
const DRAIN_MAX_FRAMES: usize = 256;
const DRAIN_MAX_TIME: Duration = Duration::from_millis(100);

/// How long an operation waits for another process's operation to finish.
const LOCK_WAIT: Duration = Duration::from_secs(10);
const LOCK_POLL: Duration = Duration::from_millis(20);

/// `flock(LOCK_EX | LOCK_NB)`, retried until `wait` has passed.
fn lock_file(file: &File, wait: Duration) -> Result<()> {
    let deadline = Instant::now() + wait;
    loop {
        // SAFETY: the descriptor stays open for the duration of the call.
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
            return Ok(());
        }
        let e = std::io::Error::last_os_error();
        match e.raw_os_error() {
            Some(libc::EINTR) => continue,
            Some(libc::EWOULDBLOCK) => {}
            _ => return Err(Error::Io(format!("lock: {e}"))),
        }
        if Instant::now() >= deadline {
            return Err(Error::Busy(format!(
                "device busy: another aulactl or aula-gui operation is running (waited {} s)",
                wait.as_secs()
            )));
        }
        std::thread::sleep(LOCK_POLL);
    }
}

fn unlock_file(file: &File) {
    // SAFETY: as above. Closing the descriptor releases the lock anyway.
    unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_UN) };
}

pub struct Device {
    file: File,
    node: PathBuf,
    debug: bool,
    lock_depth: u32,
    lock_wait: Duration,
}

impl Device {
    pub fn open(node: &Path) -> Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(node)
            .map_err(|e| match e.kind() {
                ErrorKind::PermissionDenied => Error::Io(format!(
                    "{}: permission denied (install udev/70-aulactl.rules, then replug the keyboard)",
                    node.display()
                )),
                _ => Error::Io(format!("{}: {e}", node.display())),
            })?;
        Ok(Self {
            file,
            node: node.to_path_buf(),
            debug: std::env::var_os(DEBUG_ENV).is_some(),
            lock_depth: 0,
            lock_wait: LOCK_WAIT,
        })
    }

    pub fn node(&self) -> &Path {
        &self.node
    }

    fn poll_readable(&self, timeout: Duration) -> Result<bool> {
        let mut pfd = libc::pollfd { fd: self.file.as_raw_fd(), events: libc::POLLIN, revents: 0 };
        let ms = timeout.as_millis().clamp(1, i32::MAX as u128) as i32;
        // SAFETY: pfd is a valid pollfd for the duration of the call.
        let r = unsafe { libc::poll(&mut pfd, 1, ms) };
        if r < 0 {
            let e = std::io::Error::last_os_error();
            if e.kind() == ErrorKind::Interrupted {
                return Ok(false);
            }
            return Err(Error::Io(format!("poll: {e}")));
        }
        if pfd.revents & (libc::POLLHUP | libc::POLLERR) != 0 {
            return Err(Error::Io(format!("{}: the keyboard was disconnected", self.node.display())));
        }
        Ok(r > 0 && pfd.revents & libc::POLLIN != 0)
    }
}

impl Link for Device {
    fn send(&mut self, body: &Body) -> Result<()> {
        if self.debug {
            eprintln!("tx {}", hex(body));
        }
        let mut report = [0u8; BODY_LEN + 1];
        report[0] = REPORT_ID;
        report[1..].copy_from_slice(body);
        self.file.write_all(&report).map_err(|e| Error::Io(format!("write: {e}")))
    }

    fn recv(&mut self, timeout: Duration) -> Result<Option<Body>> {
        let deadline = Instant::now() + timeout;
        let mut buf = [0u8; 64];
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() || !self.poll_readable(left)? {
                return Ok(None);
            }
            match self.file.read(&mut buf) {
                Ok(n) if n == BODY_LEN + 1 && buf[0] == REPORT_ID => {
                    let mut b = [0u8; BODY_LEN];
                    b.copy_from_slice(&buf[1..]);
                    if self.debug {
                        eprintln!("rx {}", hex(&b));
                    }
                    return Ok(Some(b));
                }
                Ok(_) => {}
                Err(e) if e.kind() == ErrorKind::WouldBlock => {}
                Err(e) => return Err(Error::Io(format!("read: {e}"))),
            }
        }
    }

    fn drain(&mut self) -> Result<()> {
        let mut buf = [0u8; 64];
        let deadline = Instant::now() + DRAIN_MAX_TIME;
        for _ in 0..DRAIN_MAX_FRAMES {
            if Instant::now() >= deadline {
                break;
            }
            match self.file.read(&mut buf) {
                Ok(_) => {
                    if self.debug {
                        eprintln!("rx (stale) {}", hex(&buf));
                    }
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => return Ok(()),
                Err(e) => return Err(Error::Io(format!("read: {e}"))),
            }
        }
        // A board that keeps sending (a travel stream left open) cannot hold the caller up: replies are
        // matched by command anyway.
        Ok(())
    }

    fn lock(&mut self) -> Result<()> {
        if self.lock_depth == 0 {
            lock_file(&self.file, self.lock_wait)?;
        }
        self.lock_depth += 1;
        Ok(())
    }

    fn unlock(&mut self) {
        match self.lock_depth {
            0 => {}
            1 => {
                self.lock_depth = 0;
                unlock_file(&self.file);
            }
            n => self.lock_depth = n - 1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Vendor report descriptor of the WIN 60 HE (protocol 1).
    const VENDOR_DESC: [u8; 29] = [
        0x06, 0x1b, 0xff, 0x09, 0x91, 0xa1, 0x01, 0x85, 0x01, 0x19, 0x00, 0x2a, 0xff, 0x00, 0x15, 0x00, 0x26, 0xff,
        0x00, 0x75, 0x08, 0x95, 0x3f, 0x81, 0x02, 0x19, 0x00, 0x29, 0xff,
    ];
    /// Boot keyboard (interface 0).
    const KEYBOARD_DESC: [u8; 8] = [0x05, 0x01, 0x09, 0x06, 0xa1, 0x01, 0x05, 0x07];

    #[test]
    fn recognises_the_vendor_collection() {
        assert!(is_vendor_collection(&VENDOR_DESC));
        assert!(!is_vendor_collection(&KEYBOARD_DESC));
        assert!(!is_vendor_collection(&[]));
        assert!(!is_vendor_collection(&[0x06, 0x1b]), "truncated");
        assert!(!is_vendor_collection(&[0x06, 0x1b, 0xff, 0x09, 0x92, 0xa1, 0x01]), "other usage");
        // a later vendor collection does not count
        let mut later = KEYBOARD_DESC.to_vec();
        later.extend_from_slice(&VENDOR_DESC);
        assert!(!is_vendor_collection(&later));
    }

    #[test]
    fn parses_hid_id() {
        let ue = "DRIVER=hid-generic\nHID_ID=0003:00002E3C:0000C365\nHID_NAME=WIN 60 HE\n";
        assert_eq!(parse_hid_id(ue), Some((0x2e3c, 0xc365)));
        assert_eq!(parse_hid_id(""), None);
        assert_eq!(parse_hid_id("HID_ID=0003:zz:0000C365"), None);
        assert_eq!(parse_hid_id("HID_ID=0003:0001000000:0000C365"), None);
    }

    #[test]
    fn product_name_match() {
        let d = |name: &str| DeviceInfo { node: PathBuf::from("/dev/hidraw7"), name: name.into(), phys: String::new() };
        assert!(d("WIN 60 HE").name_matches());
        assert!(!d("WIN 68 HE").name_matches());
        assert!(!d("").name_matches());
    }

    fn temp_file(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let path = std::env::temp_dir().join(format!("aulactl-lock-{tag}-{}-{nanos}", std::process::id()));
        fs::write(&path, b"").unwrap();
        path
    }

    #[test]
    fn device_lock_is_reentrant() {
        let path = temp_file("dev");
        let (mut a, mut b) = (Device::open(&path).unwrap(), Device::open(&path).unwrap());
        b.lock_wait = Duration::from_millis(30);
        a.lock().unwrap();
        a.lock().unwrap();
        assert!(matches!(b.lock(), Err(Error::Busy(_))));
        a.unlock();
        assert!(b.lock().is_err(), "inner unlock keeps the lock");
        a.unlock();
        assert!(b.lock().is_ok());
        b.unlock();
        a.unlock(); // unmatched: ignored
        fs::remove_file(&path).unwrap();
    }

    #[test]
    fn open_missing_node_fails() {
        assert!(matches!(Device::open(Path::new("/nonexistent/hidraw99")), Err(Error::Io(_))));
    }

    #[test]
    fn drain_is_bounded() {
        // a regular file never reports WouldBlock: an unbounded drain would spin forever
        let path = temp_file("drain");
        fs::write(&path, vec![0u8; 64 * 1024]).unwrap();
        let mut d = Device::open(&path).unwrap();
        let t = Instant::now();
        d.drain().unwrap();
        assert!(t.elapsed() < Duration::from_secs(1));
        fs::remove_file(&path).unwrap();
    }
}

//! Host-side files: automatic backups and named profiles under `$XDG_DATA_HOME/aulactl`.

use crate::error::{invalid, Error, Result};
use crate::snapshot::Snapshot;
use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Read, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

/// Automatic backups kept (oldest deleted first).
pub const KEEP_AUTO_BACKUPS: usize = 20;
const AUTO_PREFIX: &str = "auto-";
const JSON_EXT: &str = ".json";
const MAX_PROFILE_NAME: usize = 32;
/// Largest snapshot file `load_snapshot` reads (a real one is about 60 KiB).
pub const MAX_SNAPSHOT_BYTES: u64 = 4 * 1024 * 1024;
/// Backups and profiles hold the whole keyboard configuration: owner-only.
const DIR_MODE: u32 = 0o700;
const FILE_MODE: u32 = 0o600;

/// `$XDG_DATA_HOME/aulactl`, or `~/.local/share/aulactl`. Relative XDG paths are ignored (XDG spec).
pub fn data_dir() -> Result<PathBuf> {
    data_dir_from(std::env::var_os("XDG_DATA_HOME").map(PathBuf::from), std::env::var_os("HOME").map(PathBuf::from))
}

fn data_dir_from(xdg: Option<PathBuf>, home: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(x) = xdg.filter(|p| p.is_absolute()) {
        return Ok(x.join("aulactl"));
    }
    match home.filter(|p| p.is_absolute()) {
        Some(h) => Ok(h.join(".local/share/aulactl")),
        None => Err(Error::Io("neither XDG_DATA_HOME nor HOME is set".into())),
    }
}

pub fn backups_dir() -> Result<PathBuf> {
    Ok(data_dir()?.join("backups"))
}

pub fn profiles_dir() -> Result<PathBuf> {
    Ok(data_dir()?.join("profiles"))
}

fn io_err(path: &Path, e: std::io::Error) -> Error {
    Error::Io(format!("{}: {e}", path.display()))
}

/// Creates `dir` (and missing parents) owner-only (0700); an existing `dir` is narrowed to 0700. For aulactl's
/// own directories (backups, profiles), not for directories the user names.
pub fn private_dir(dir: &Path) -> Result<()> {
    fs::DirBuilder::new().recursive(true).mode(DIR_MODE).create(dir).map_err(|e| io_err(dir, e))?;
    let meta = fs::metadata(dir).map_err(|e| io_err(dir, e))?;
    if meta.permissions().mode() & 0o777 != DIR_MODE {
        fs::set_permissions(dir, fs::Permissions::from_mode(DIR_MODE)).map_err(|e| io_err(dir, e))?;
    }
    Ok(())
}

/// Writes `text` to a new owner-only (0600) temporary file next to `path` and returns its name.
fn write_temp(path: &Path, text: &str) -> Result<PathBuf> {
    let dir = path.parent().ok_or_else(|| Error::Io(format!("{}: no parent directory", path.display())))?;
    let dir = if dir.as_os_str().is_empty() { Path::new(".") } else { dir };
    fs::DirBuilder::new().recursive(true).mode(DIR_MODE).create(dir).map_err(|e| io_err(dir, e))?;
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let tmp = dir.join(format!(".{name}.tmp-{}", std::process::id()));
    let open = || OpenOptions::new().write(true).create_new(true).mode(FILE_MODE).open(&tmp);
    let mut f = match open() {
        Err(e) if e.kind() == ErrorKind::AlreadyExists => {
            // left over by a crashed process with the same pid
            fs::remove_file(&tmp).map_err(|e| io_err(&tmp, e))?;
            open()
        }
        r => r,
    }
    .map_err(|e| io_err(&tmp, e))?;
    if let Err(e) = f.write_all(text.as_bytes()).and_then(|_| f.sync_all()) {
        let _ = fs::remove_file(&tmp);
        return Err(io_err(&tmp, e));
    }
    Ok(tmp)
}

/// Writes `text` to `path` through a temporary file and a rename, so a crash never leaves half a file.
/// Replaces an existing `path`. The file is owner-only (0600).
pub fn write_atomic(path: &Path, text: &str) -> Result<()> {
    let tmp = write_temp(path, text)?;
    fs::rename(&tmp, path).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        io_err(path, e)
    })
}

/// Like `write_atomic`, but fails when `path` exists, also when it appears meanwhile (the temporary file is
/// hard-linked into place, which never replaces anything).
pub fn write_new(path: &Path, text: &str) -> Result<()> {
    let tmp = write_temp(path, text)?;
    let r = fs::hard_link(&tmp, path);
    let _ = fs::remove_file(&tmp);
    match r {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == ErrorKind::AlreadyExists => {
            invalid(format!("{} already exists; pick a new file name", path.display()))
        }
        Err(e) => Err(io_err(path, e)),
    }
}

/// Saves a snapshot, replacing `path` if it exists.
pub fn save_snapshot(path: &Path, snap: &Snapshot) -> Result<()> {
    write_atomic(path, &(snap.to_json()? + "\n"))
}

/// Saves a snapshot to a new file; fails when `path` exists.
pub fn save_snapshot_new(path: &Path, snap: &Snapshot) -> Result<()> {
    write_new(path, &(snap.to_json()? + "\n"))
}

/// Reads and checks a snapshot file (at most `MAX_SNAPSHOT_BYTES`).
pub fn load_snapshot(path: &Path) -> Result<Snapshot> {
    let f = fs::File::open(path).map_err(|e| io_err(path, e))?;
    let mut text = String::new();
    f.take(MAX_SNAPSHOT_BYTES + 1).read_to_string(&mut text).map_err(|e| io_err(path, e))?;
    if text.len() as u64 > MAX_SNAPSHOT_BYTES {
        return invalid(format!("{}: larger than {} MiB, not a snapshot", path.display(), MAX_SNAPSHOT_BYTES >> 20));
    }
    Snapshot::from_json(&text).map_err(|e| Error::Invalid(format!("{}: {e}", path.display())))
}

/// UTC `YYYYmmdd-HHMMSS` for unix seconds.
pub fn timestamp(unix: u64) -> String {
    let days = (unix / 86_400) as i64;
    let secs = unix % 86_400;
    // civil-from-days (Howard Hinnant)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}{month:02}{day:02}-{:02}{:02}{:02}", secs / 3600, secs / 60 % 60, secs % 60)
}

/// Saves `snap` as `<dir>/auto-<timestamp>.json` and deletes the oldest automatic backups beyond 20.
pub fn save_auto_backup(dir: &Path, snap: &Snapshot) -> Result<PathBuf> {
    private_dir(dir)?;
    let stamp = timestamp(snap.created_unix);
    let mut path = dir.join(format!("{AUTO_PREFIX}{stamp}{JSON_EXT}"));
    let mut n = 1;
    while path.exists() {
        path = dir.join(format!("{AUTO_PREFIX}{stamp}-{n}{JSON_EXT}"));
        n += 1;
    }
    save_snapshot_new(&path, snap)?;
    prune_auto_backups(dir, KEEP_AUTO_BACKUPS)?;
    Ok(path)
}

/// Automatic backups in `dir`, oldest first (names sort by time).
pub fn list_auto_backups(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut v: Vec<PathBuf> = match fs::read_dir(dir) {
        Ok(rd) => rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with(AUTO_PREFIX) && n.ends_with(JSON_EXT))
            })
            .collect(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(e) => return Err(Error::Io(format!("{}: {e}", dir.display()))),
    };
    v.sort();
    Ok(v)
}

pub fn prune_auto_backups(dir: &Path, keep: usize) -> Result<()> {
    let all = list_auto_backups(dir)?;
    for p in &all[..all.len().saturating_sub(keep)] {
        fs::remove_file(p).map_err(|e| Error::Io(format!("{}: {e}", p.display())))?;
    }
    Ok(())
}

/// Profile names: 1..=32 of `[A-Za-z0-9_-]` (no path separators, no dots).
pub fn validate_profile_name(name: &str) -> Result<()> {
    if name.is_empty() || name.len() > MAX_PROFILE_NAME {
        return invalid(format!("profile name must be 1 to {MAX_PROFILE_NAME} characters"));
    }
    if !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-') {
        return invalid(format!("profile name {name:?} may only contain A-Z, a-z, 0-9, _ and -"));
    }
    Ok(())
}

/// Host-side profiles in a directory.
#[derive(Debug, Clone)]
pub struct Profiles {
    dir: PathBuf,
}

impl Profiles {
    /// `$XDG_DATA_HOME/aulactl/profiles`.
    pub fn open_default() -> Result<Self> {
        Ok(Profiles { dir: profiles_dir()? })
    }

    pub fn in_dir(dir: PathBuf) -> Self {
        Profiles { dir }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn path(&self, name: &str) -> Result<PathBuf> {
        validate_profile_name(name)?;
        Ok(self.dir.join(format!("{name}{JSON_EXT}")))
    }

    /// Profile names, sorted.
    pub fn list(&self) -> Result<Vec<String>> {
        let mut v: Vec<String> = match fs::read_dir(&self.dir) {
            Ok(rd) => rd
                .flatten()
                .filter_map(|e| e.file_name().to_str()?.strip_suffix(JSON_EXT).map(str::to_string))
                .filter(|n| validate_profile_name(n).is_ok())
                .collect(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(Error::Io(format!("{}: {e}", self.dir.display()))),
        };
        v.sort();
        Ok(v)
    }

    pub fn exists(&self, name: &str) -> Result<bool> {
        Ok(self.path(name)?.exists())
    }

    /// Saves a profile. Refuses to replace an existing one unless `overwrite`.
    pub fn save(&self, name: &str, snap: &Snapshot, overwrite: bool) -> Result<PathBuf> {
        let path = self.path(name)?;
        private_dir(&self.dir)?;
        if overwrite {
            save_snapshot(&path, snap)?;
        } else {
            save_snapshot_new(&path, snap).map_err(|e| match e {
                Error::Invalid(_) if path.exists() => Error::Invalid(format!("profile {name:?} exists (overwrite it explicitly)")),
                e => e,
            })?;
        }
        Ok(path)
    }

    pub fn load(&self, name: &str) -> Result<Snapshot> {
        let path = self.path(name)?;
        if !path.exists() {
            return invalid(format!("no profile named {name:?}"));
        }
        load_snapshot(&path)
    }

    pub fn delete(&self, name: &str) -> Result<()> {
        let path = self.path(name)?;
        fs::remove_file(&path).map_err(|e| Error::Io(format!("{}: {e}", path.display())))
    }
}

#[cfg(test)]
pub(crate) fn temp_dir(tag: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let d = std::env::temp_dir().join(format!("aulactl-test-{tag}-{}-{nanos}", std::process::id()));
    fs::create_dir_all(&d).unwrap();
    d
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamps() {
        assert_eq!(timestamp(0), "19700101-000000");
        assert_eq!(timestamp(951_782_400), "20000229-000000", "leap day");
        assert_eq!(timestamp(1_793_491_199), "20261031-235959");
        assert_eq!(timestamp(1_793_491_200), "20261101-000000");
        assert_eq!(timestamp(4_102_444_800), "21000101-000000");
    }

    #[test]
    fn data_dir_rules() {
        let p = |s: &str| Some(PathBuf::from(s));
        assert_eq!(data_dir_from(p("/x"), p("/home/u")).unwrap(), PathBuf::from("/x/aulactl"));
        assert_eq!(data_dir_from(p("rel"), p("/home/u")).unwrap(), PathBuf::from("/home/u/.local/share/aulactl"));
        assert_eq!(data_dir_from(None, p("/home/u")).unwrap(), PathBuf::from("/home/u/.local/share/aulactl"));
        assert!(data_dir_from(None, None).is_err());
        assert!(data_dir_from(p(""), p("")).is_err());
    }

    #[test]
    fn profile_names() {
        for ok in ["a", "default", "game_1", "FPS-low", &"x".repeat(32)] {
            assert!(validate_profile_name(ok).is_ok(), "{ok}");
        }
        for bad in ["", "../x", "a/b", ".", "..", "a.b", "a b", "é", &"x".repeat(33), "x\0", "-\n"] {
            assert!(validate_profile_name(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn atomic_write_and_prune() {
        let d = temp_dir("prune");
        for i in 0..25 {
            write_atomic(&d.join(format!("auto-2026010{}-0000{i:02}.json", i / 10)), "{}").unwrap();
        }
        write_atomic(&d.join("manual.json"), "{}").unwrap();
        assert_eq!(list_auto_backups(&d).unwrap().len(), 25);
        prune_auto_backups(&d, KEEP_AUTO_BACKUPS).unwrap();
        let left = list_auto_backups(&d).unwrap();
        assert_eq!(left.len(), 20);
        assert!(left[0].ends_with("auto-20260100-000005.json"), "oldest five removed: {left:?}");
        assert!(d.join("manual.json").exists());
        prune_auto_backups(&d, 0).unwrap();
        assert!(list_auto_backups(&d).unwrap().is_empty());
        assert!(list_auto_backups(&d.join("missing")).unwrap().is_empty());
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn profiles_list_and_errors() {
        let d = temp_dir("profiles");
        let p = Profiles::in_dir(d.join("profiles"));
        assert!(p.list().unwrap().is_empty(), "missing dir is empty");
        assert!(p.load("nope").is_err());
        assert!(p.path("../etc").is_err());
        fs::create_dir_all(p.dir()).unwrap();
        fs::write(p.dir().join("ok.json"), "{}").unwrap();
        fs::write(p.dir().join("bad name.json"), "{}").unwrap();
        fs::write(p.dir().join("notjson.txt"), "{}").unwrap();
        assert_eq!(p.list().unwrap(), vec!["ok".to_string()]);
        assert!(p.load("ok").is_err(), "not a snapshot");
        p.delete("ok").unwrap();
        assert!(p.delete("ok").is_err());
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn write_new_never_replaces() {
        let d = temp_dir("new");
        let p = d.join("a.json");
        write_new(&p, "one").unwrap();
        assert!(write_new(&p, "two").unwrap_err().to_string().contains("already exists"));
        assert_eq!(fs::read_to_string(&p).unwrap(), "one");
        write_atomic(&p, "three").unwrap();
        assert_eq!(fs::read_to_string(&p).unwrap(), "three");
        // no temporary files left behind
        assert_eq!(fs::read_dir(&d).unwrap().count(), 1);
        // a missing parent is created owner-only
        write_new(&d.join("sub/b.json"), "x").unwrap();
        assert_eq!(fs::metadata(d.join("sub")).unwrap().permissions().mode() & 0o777, 0o700);
        assert_eq!(fs::metadata(d.join("sub/b.json")).unwrap().permissions().mode() & 0o777, 0o600);
        assert!(write_new(Path::new("/"), "x").is_err());
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn load_snapshot_size_cap() {
        let d = temp_dir("big");
        let big = d.join("big.json");
        let f = fs::File::create(&big).unwrap();
        f.set_len(MAX_SNAPSHOT_BYTES + 1).unwrap();
        assert!(load_snapshot(&big).unwrap_err().to_string().contains("larger than 4 MiB"));
        fs::write(&big, "{}").unwrap();
        assert!(matches!(load_snapshot(&big), Err(Error::Invalid(_))), "small but not a snapshot");
        assert!(matches!(load_snapshot(&d.join("missing.json")), Err(Error::Io(_))));
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn private_dir_narrows_existing() {
        let d = temp_dir("narrow");
        fs::set_permissions(&d, fs::Permissions::from_mode(0o755)).unwrap();
        private_dir(&d).unwrap();
        assert_eq!(fs::metadata(&d).unwrap().permissions().mode() & 0o777, 0o700);
        private_dir(&d.join("a/b")).unwrap();
        assert_eq!(fs::metadata(d.join("a")).unwrap().permissions().mode() & 0o777, 0o700);
        fs::write(d.join("file"), "").unwrap();
        assert!(private_dir(&d.join("file")).is_err());
        fs::remove_dir_all(&d).unwrap();
    }
}

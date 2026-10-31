//! Where backups and profiles live (the core store directories, or a scratch directory with `--sim` so the
//! simulator never touches real files), and the backup list shown on the Overview page.

use aula_core::store;
use serde_json::{json, Value};
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

const MANUAL_PREFIX: &str = "manual-";
const AUTO_PREFIX: &str = "auto-";
const JSON_EXT: &str = ".json";

/// Backup and profile directories for this run.
#[derive(Debug, Clone, PartialEq)]
pub struct Dirs {
    pub backups: PathBuf,
    pub profiles: PathBuf,
}

/// The `--sim` scratch directory, created once per process (removed again by `remove_sim_dir`).
static SIM_DIR: OnceLock<Result<PathBuf, String>> = OnceLock::new();

impl Dirs {
    pub fn for_run(sim: bool) -> Result<Dirs, String> {
        if sim {
            let base = SIM_DIR
                .get_or_init(|| {
                    let base = sim_base(std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from));
                    create_scratch(&base).map(|_| base)
                })
                .clone()?;
            return Ok(Dirs { backups: base.join("backups"), profiles: base.join("profiles") });
        }
        Ok(Dirs { backups: store::backups_dir()?, profiles: store::profiles_dir()? })
    }
}

/// `$XDG_RUNTIME_DIR/aula-gui-sim-<pid>` (owner-only by the XDG spec), else the same under the temp dir.
fn sim_base(runtime: Option<PathBuf>) -> PathBuf {
    let parent = runtime.filter(|p| p.is_absolute()).unwrap_or_else(std::env::temp_dir);
    parent.join(format!("aula-gui-sim-{}", std::process::id()))
}

/// Creates a fresh owner-only (0700) directory. An existing one (someone else's, or a symlink planted in a
/// shared temp dir) is refused rather than used.
fn create_scratch(dir: &Path) -> Result<(), String> {
    std::fs::DirBuilder::new().mode(0o700).create(dir).map_err(|e| match e.kind() {
        std::io::ErrorKind::AlreadyExists => format!("{} already exists; refusing to use it", dir.display()),
        _ => format!("{}: {e}", dir.display()),
    })
}

/// Deletes the `--sim` scratch directory, if this process created it.
pub fn remove_sim_dir() {
    if let Some(Ok(dir)) = SIM_DIR.get() {
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Backup {
    pub path: PathBuf,
    /// File name without `.json`.
    pub name: String,
    /// Written automatically before the first write of a session.
    pub auto: bool,
}

/// Backups in `dir` (automatic and manual), newest first. A missing directory is an empty list.
pub fn list_backups(dir: &Path) -> Result<Vec<Backup>, String> {
    let rd = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("{}: {e}", dir.display())),
    };
    let mut v: Vec<(String, Backup)> = rd
        .flatten()
        .filter_map(|e| {
            let file = e.file_name().to_str()?.to_string();
            let name = file.strip_suffix(JSON_EXT)?.to_string();
            let (auto, stamp) = if let Some(s) = name.strip_prefix(AUTO_PREFIX) {
                (true, s.to_string())
            } else {
                (false, name.strip_prefix(MANUAL_PREFIX)?.to_string())
            };
            Some((stamp, Backup { path: e.path(), name, auto }))
        })
        .collect();
    // Timestamps sort by time; newest first.
    v.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.name.cmp(&a.1.name)));
    Ok(v.into_iter().map(|(_, b)| b).collect())
}

/// A free `manual-<timestamp>.json` path in `dir`.
pub fn manual_backup_path(dir: &Path, unix: u64) -> PathBuf {
    let stamp = store::timestamp(unix);
    let mut path = dir.join(format!("{MANUAL_PREFIX}{stamp}{JSON_EXT}"));
    let mut n = 1;
    while path.exists() {
        path = dir.join(format!("{MANUAL_PREFIX}{stamp}-{n}{JSON_EXT}"));
        n += 1;
    }
    path
}

pub fn backups_json(list: &[Backup]) -> Value {
    Value::Array(
        list.iter().map(|b| json!({ "name": b.name, "path": b.path.to_string_lossy(), "auto": b.auto })).collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("aula-gui-files-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn lists_newest_first_and_ignores_other_files() {
        let d = scratch("list");
        for f in ["auto-20261101-100000.json", "manual-20261101-120000.json", "auto-20261102-090000.json", "notes.txt", "x.json", "auto-.tmp"] {
            std::fs::write(d.join(f), "{}").unwrap();
        }
        let names: Vec<String> = list_backups(&d).unwrap().into_iter().map(|b| b.name).collect();
        assert_eq!(names, ["auto-20261102-090000", "manual-20261101-120000", "auto-20261101-100000"]);
        let all = list_backups(&d).unwrap();
        assert!(all[0].auto && !all[1].auto);
        let j = backups_json(&all);
        assert_eq!(j[1]["name"], "manual-20261101-120000");
        assert_eq!(j[1]["auto"], false);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn missing_or_empty_dir() {
        assert_eq!(list_backups(Path::new("/nonexistent/aula-gui/backups")).unwrap(), vec![]);
        let d = scratch("empty");
        assert_eq!(list_backups(&d).unwrap(), vec![]);
        assert_eq!(backups_json(&[]), json!([]));
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn not_a_directory_is_an_error() {
        let d = scratch("file");
        let f = d.join("plain");
        std::fs::write(&f, "").unwrap();
        assert!(list_backups(&f).is_err());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn manual_paths_never_collide() {
        let d = scratch("manual");
        let a = manual_backup_path(&d, 0);
        assert_eq!(a.file_name().unwrap(), "manual-19700101-000000.json");
        std::fs::write(&a, "{}").unwrap();
        let b = manual_backup_path(&d, 0);
        assert_eq!(b.file_name().unwrap(), "manual-19700101-000000-1.json");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn sim_dirs_are_private_scratch() {
        use std::os::unix::fs::PermissionsExt;
        let d = Dirs::for_run(true).unwrap();
        let base = d.backups.parent().unwrap().to_path_buf();
        assert_eq!(d.profiles.parent(), Some(base.as_path()));
        assert_ne!(d.backups, d.profiles);
        assert_eq!(std::fs::metadata(&base).unwrap().permissions().mode() & 0o777, 0o700);
        assert_eq!(Dirs::for_run(true).unwrap(), d, "created once per process");
    }

    #[test]
    fn sim_base_prefers_the_runtime_dir() {
        let pid = std::process::id();
        assert_eq!(sim_base(Some("/run/user/1000".into())), PathBuf::from(format!("/run/user/1000/aula-gui-sim-{pid}")));
        assert!(sim_base(Some("relative".into())).starts_with(std::env::temp_dir()));
        assert!(sim_base(None).starts_with(std::env::temp_dir()));
    }

    #[test]
    fn scratch_refuses_existing_paths() {
        let d = scratch("exist");
        assert!(create_scratch(&d).unwrap_err().contains("refusing"));
        let link = d.join("link");
        std::os::unix::fs::symlink("/", &link).unwrap();
        assert!(create_scratch(&link).is_err(), "a planted symlink is not followed");
        assert!(create_scratch(&d.join("missing/deeper")).is_err());
        let fresh = d.join("fresh");
        create_scratch(&fresh).unwrap();
        std::fs::remove_dir_all(&d).unwrap();
    }
}

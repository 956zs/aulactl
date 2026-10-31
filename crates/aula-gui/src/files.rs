//! Where backups and profiles live (the core store directories, or a scratch directory with `--sim` so the
//! simulator never touches real files), and the backup list shown on the Overview page.

use aula_core::store;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

const MANUAL_PREFIX: &str = "manual-";
const AUTO_PREFIX: &str = "auto-";
const JSON_EXT: &str = ".json";

/// Backup and profile directories for this run.
#[derive(Debug, Clone, PartialEq)]
pub struct Dirs {
    pub backups: PathBuf,
    pub profiles: PathBuf,
}

impl Dirs {
    pub fn for_run(sim: bool) -> Result<Dirs, String> {
        if sim {
            let base = std::env::temp_dir().join(format!("aula-gui-sim-{}", std::process::id()));
            return Ok(Dirs { backups: base.join("backups"), profiles: base.join("profiles") });
        }
        Ok(Dirs { backups: store::backups_dir()?, profiles: store::profiles_dir()? })
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
    fn sim_dirs_are_scratch() {
        let d = Dirs::for_run(true).unwrap();
        assert!(d.backups.starts_with(std::env::temp_dir()));
        assert_ne!(d.backups, d.profiles);
    }
}

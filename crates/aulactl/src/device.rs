//! info, dump, backup, diff, restore, profiles.

use crate::args::{win_lock_text, Args};
use crate::{bad_args, keys, light, profiles, put, say, trigger, Cli, Outcome};
use aula_core::hidraw::Link;
use aula_core::keymap::Layer;
use aula_core::snapshot::{self, Snapshot};
use aula_core::store;
use std::io::Write;
use std::path::{Path, PathBuf};

pub fn info<L: Link>(c: &mut Cli<L>) -> Result<(), String> {
    let id = c.kb.identity().clone();
    let mut lines = vec![
        format!("product:    {}", id.product_code),
        format!("firmware:   {} (built {})", id.firmware, id.build_date),
    ];
    if let Some(node) = &c.node {
        lines.push(format!("device:     {}", node.display()));
    }
    if !c.kb.is_supported() {
        lines.push(
            "this board is not a WIN 60 HE; aulactl only identifies it and sends nothing else"
                .into(),
        );
        return say(c.out, &lines.join("\n"));
    }
    let caps = *c.kb.capabilities()?;
    let l = c.kb.travel_limits()?;
    lines.push(format!(
        "travel:     {}..{} mm in {:.2} mm steps",
        crate::args::mm(&l, l.min),
        crate::args::mm(&l, l.max),
        l.unit_mm()
    ));
    if caps.poll_rate() {
        lines.push(format!("poll rate:  {} kHz", c.kb.read_poll_rate()?));
    }
    lines.push(format!(
        "rollover:   {}",
        if caps.six_key_rollover {
            "6KRO"
        } else {
            "NKRO"
        }
    ));
    lines.push(format!(
        "win-lock:   blocks {}",
        win_lock_text(&caps.win_lock())
    ));
    let (has, lacks): (Vec<_>, Vec<_>) = caps.features().into_iter().partition(|(_, on)| *on);
    let names = |v: Vec<(&str, bool)>| v.into_iter().map(|(n, _)| n).collect::<Vec<_>>().join(", ");
    lines.push(format!("features:   {}", names(has)));
    lines.push(format!("missing:    {}", names(lacks)));
    say(c.out, &lines.join("\n"))
}

/// Refuses to replace an existing file unless `force` (checked again when writing).
fn new_file(path: &Path, force: bool) -> Result<(), String> {
    if !force && path.exists() {
        return Err(format!(
            "{} already exists; pick a new file name or add --force",
            path.display()
        ));
    }
    Ok(())
}

/// Saves `snap` to `path`: a new file, or any file with `force`.
fn save(path: &Path, snap: &Snapshot, force: bool) -> Result<(), String> {
    if force {
        store::save_snapshot(path, snap)?;
    } else {
        store::save_snapshot_new(path, snap)
            .map_err(|e| format!("{e} (or add --force)"))?;
    }
    Ok(())
}

pub fn dump<L: Link>(c: &mut Cli<L>, rest: &[&str]) -> Result<(), String> {
    let mut a = Args::new(rest);
    let force = a.flag("--force");
    match a.positional()?[..] {
        [] => {
            let snap = c.kb.read_snapshot()?;
            say(c.out, &snap.to_json()?)
        }
        [file] => {
            let path = Path::new(file);
            new_file(path, force)?;
            let snap = c.kb.read_snapshot()?;
            save(path, &snap, force)?;
            say(c.out, &format!("saved to {}", path.display()))
        }
        _ => Err(bad_args(&[&["dump"], rest].concat())),
    }
}

pub fn backup<L: Link>(c: &mut Cli<L>, rest: &[&str]) -> Result<(), String> {
    let mut a = Args::new(rest);
    let force = a.flag("--force");
    let path = match a.positional()?[..] {
        [] => None,
        [file] => Some(PathBuf::from(file)),
        _ => return Err(bad_args(&[&["backup"], rest].concat())),
    };
    if let Some(p) = &path {
        new_file(p, force)?;
    }
    let snap = c.kb.read_snapshot()?;
    let path = match path {
        Some(p) => p,
        None => {
            let dir = store::backups_dir()?;
            store::private_dir(&dir)?;
            dir.join(format!("backup-{}.json", store::timestamp(snap.created_unix)))
        }
    };
    save(&path, &snap, force)?;
    put(c.out, &summary(&snap))?;
    say(c.out, &format!("saved to {}", path.display()))
}

pub fn diff<L: Link>(c: &mut Cli<L>, file: &Path) -> Result<(), String> {
    let want = store::load_snapshot(file)?;
    let now = c.kb.read_snapshot()?;
    let changes = snapshot::diff(&now, &want);
    if changes.is_empty() {
        return say(c.out, &format!("the keyboard matches {}", file.display()));
    }
    for ch in &changes {
        say(c.out, &ch.to_string())?;
    }
    say(c.out, &format!("{} differences", changes.len()))
}

pub fn restore<L: Link>(c: &mut Cli<L>, rest: &[&str]) -> Result<Outcome, String> {
    let mut a = Args::new(rest);
    let yes = a.flag("--yes");
    let [file] = a.positional()?[..] else {
        return Err(bad_args(&[&["restore"], rest].concat()));
    };
    let want = store::load_snapshot(Path::new(file))?;
    apply(c, &want, yes, &format!("aulactl restore {file} --yes"))
}

/// Shows what `want` changes, and writes it when `yes`. The board is read and written under one device lock,
/// so what is compared is what is overwritten.
fn apply<L: Link>(
    c: &mut Cli<L>,
    want: &Snapshot,
    yes: bool,
    again: &str,
) -> Result<Outcome, String> {
    let mut lines = Vec::new();
    let written = c.kb.with_lock(|kb| {
        let now = kb.read_snapshot()?;
        if want.device.firmware != now.device.firmware {
            lines.push(format!(
                "note: saved on firmware {}, the keyboard runs {}",
                want.device.firmware, now.device.firmware
            ));
        }
        let changes = snapshot::diff(&now, want);
        if changes.is_empty() {
            lines.push("nothing to change: the keyboard already has these settings".into());
            return Ok(None);
        }
        lines.extend(changes.iter().map(|ch| ch.to_string()));
        if !yes {
            lines.push(format!(
                "{} differences; nothing was written. Run '{again}' to write them.",
                changes.len()
            ));
            return Ok(None);
        }
        // a failed write is reported after the differences
        Ok(Some(kb.apply_snapshot(want, &now)))
    });
    for l in &lines {
        say(c.out, l)?;
    }
    let Some(report) = written? else {
        return Ok(Outcome::Done);
    };
    let report = report?;
    c.report_backup()?;
    say(
        c.out,
        &format!("written and read back: {}", report.written.join(", ")),
    )?;
    if report.reconnect_needed {
        return Ok(Outcome::Restarting(want.poll_rate_khz));
    }
    Ok(Outcome::Done)
}

pub fn profile<L: Link>(c: &mut Cli<L>, rest: &[&str]) -> Result<Outcome, String> {
    let mut a = Args::new(rest);
    let force = a.flag("--force");
    let yes = a.flag("--yes");
    let pos = a.positional()?;
    let p = profiles(c.profiles.as_deref())?;
    match pos[..] {
        ["save", name] => {
            store::validate_profile_name(name)?;
            if !force && p.exists(name)? {
                return Err(format!("profile {name} exists; add --force to replace it"));
            }
            let snap = c.kb.read_snapshot()?;
            let path = p.save(name, &snap, force)?;
            say(c.out, &format!("saved profile {name} ({})", path.display()))?;
            Ok(Outcome::Done)
        }
        ["diff", name] => {
            let want = p.load(name)?;
            let now = c.kb.read_snapshot()?;
            let changes = snapshot::diff(&now, &want);
            if changes.is_empty() {
                say(c.out, &format!("the keyboard matches profile {name}"))?;
            }
            for ch in &changes {
                say(c.out, &ch.to_string())?;
            }
            Ok(Outcome::Done)
        }
        ["apply", name] => {
            let want = p.load(name)?;
            apply(
                c,
                &want,
                yes,
                &format!("aulactl profile apply {name} --yes"),
            )
        }
        _ => profile_offline(c.out, c.profiles.as_deref(), rest).map(|_| Outcome::Done),
    }
}

/// Profile commands that only touch files: list, show, delete.
pub fn profile_offline(
    out: &mut dyn Write,
    dir: Option<&Path>,
    rest: &[&str],
) -> Result<(), String> {
    let mut a = Args::new(rest);
    let yes = a.flag("--yes");
    let pos = a.positional()?;
    let p = profiles(dir)?;
    match pos[..] {
        ["list"] => {
            let names = p.list()?;
            if names.is_empty() {
                return say(
                    out,
                    "no profiles yet; save one with 'aulactl profile save NAME'",
                );
            }
            say(out, &names.join("\n"))
        }
        ["show", name] => put(out, &summary(&p.load(name)?)),
        ["delete", name] if yes => {
            p.delete(name)?;
            say(out, &format!("deleted profile {name}"))
        }
        ["delete", name] => {
            p.load(name)?;
            Err(format!("deleting profile {name} cannot be undone; run 'aulactl profile delete {name} --yes'"))
        }
        _ => Err(bad_args(&[&["profile"], rest].concat())),
    }
}

/// A few lines describing a snapshot.
pub fn summary(s: &Snapshot) -> String {
    let l = &s.device.travel;
    let mut out = vec![
        format!(
            "{} firmware {}, taken {} UTC",
            s.device.product_code,
            s.device.firmware,
            store::timestamp(s.created_unix)
        ),
        format!("lighting: {}", light::effect_text(&s.lighting.effect)),
        format!(
            "keymap: {} keys remapped on the base layer, {} set on the fn layer",
            keys::remapped(Layer::Base, &s.keymap.base).len(),
            keys::remapped(Layer::Fn, &s.keymap.fn_layer).len()
        ),
        format!(
            "macros: {} of {} slots used; advanced keys: {} of {} used",
            s.macros.iter().flatten().count(),
            s.macros.len(),
            s.advanced
                .iter()
                .filter(|a| **a != aula_core::advanced::AdvancedKey::Empty)
                .count(),
            s.advanced.len()
        ),
    ];
    for (text, keys) in trigger::groups(l, &s.trigger) {
        out.push(format!(
            "trigger: {text}: {}",
            crate::render::key_list(&keys)
        ));
    }
    if let Some(socd) = &s.socd {
        out.push(format!(
            "SOCD: {}, {} pairs",
            if socd.enabled { "on" } else { "off" },
            socd.pairs.len()
        ));
    }
    if let Some(k) = s.poll_rate_khz {
        out.push(format!("poll rate: {k} kHz"));
    }
    out.push(format!("win-lock blocks: {}", win_lock_text(&s.win_lock)));
    out.join("\n") + "\n"
}

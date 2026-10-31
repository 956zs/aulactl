//! Config snapshot: everything readable, as JSON. Used for backups, host-side profiles, diff and restore.

use crate::advanced::AdvancedKey;
use crate::error::{invalid, Error, Result};
use crate::hidraw::Link;
use crate::info::{WinLock, SUPPORTED_PRODUCT_CODES};
use crate::keyboard::Keyboard;
use crate::keymap::{Keymap, Layer};
use crate::layout;
use crate::lighting::{CustomColors, Effect};
use crate::macros::{self, Macro};
use crate::perkey::{key_name, PerKey};
use crate::socd::SocdPair;
use crate::trigger::{DeadBand, KeyTrigger, TravelLimits};
use serde::{Deserialize, Serialize};
use std::fmt;

/// Bump when the JSON layout changes incompatibly.
pub const SCHEMA: u32 = 1;
const FORMAT_NAME: &str = "aulactl snapshot";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    /// Always `"aulactl snapshot"`.
    pub format: String,
    pub schema: u32,
    /// Unix time the snapshot was taken.
    pub created_unix: u64,
    /// Information only; not written back.
    pub device: DeviceInfo,
    pub lighting: Lighting,
    pub keymap: Keymaps,
    /// 10 slots, `null` = empty.
    pub macros: Vec<Option<Macro>>,
    /// 40 slots.
    pub advanced: Vec<AdvancedKey>,
    /// Per-key trigger config in trigger units (0.01 mm, see `device.travel`).
    pub trigger: PerKey<KeyTrigger>,
    /// Per-key dead band in trigger units; absent when the board has no dead band.
    pub dead_band: Option<PerKey<DeadBand>>,
    /// Per-key switch id; absent when the board has no switch-type table.
    pub switch_types: Option<PerKey<u8>>,
    /// kHz; absent when the board has no poll-rate setting.
    pub poll_rate_khz: Option<u8>,
    /// Absent when the board has no SOCD.
    pub socd: Option<Socd>,
    pub win_lock: WinLock,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceInfo {
    pub product_code: String,
    pub firmware: String,
    pub travel: TravelLimits,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Lighting {
    pub effect: Effect,
    /// Slot shown by Custom mode (`09 20`). Changing it here rewrites that slot's table, which is how the
    /// official driver selects it.
    pub active_custom_slot: u8,
    /// Both custom slots; absent when the board has no per-key RGB.
    pub custom: Option<[CustomColors; 2]>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Keymaps {
    pub base: Keymap,
    #[serde(rename = "fn")]
    pub fn_layer: Keymap,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Socd {
    pub enabled: bool,
    pub pairs: Vec<SocdPair>,
}

impl Snapshot {
    pub fn to_json(&self) -> Result<String> {
        Ok(serde_json::to_string_pretty(self)?)
    }

    /// Parses and checks the format name, schema and table sizes.
    pub fn from_json(text: &str) -> Result<Snapshot> {
        let probe: serde_json::Value = serde_json::from_str(text)?;
        if probe.get("format").and_then(|v| v.as_str()) != Some(FORMAT_NAME) {
            return invalid(format!("not an aulactl snapshot (\"format\" must be \"{FORMAT_NAME}\")"));
        }
        match probe.get("schema").and_then(|v| v.as_u64()) {
            Some(s) if s == SCHEMA as u64 => {}
            Some(s) => return invalid(format!("snapshot schema {s} is not supported (this build reads {SCHEMA})")),
            None => return invalid("snapshot has no \"schema\""),
        }
        let snap: Snapshot = serde_json::from_value(probe)?;
        snap.check_shape()?;
        Ok(snap)
    }

    /// Structural checks that do not need the board.
    pub fn check_shape(&self) -> Result<()> {
        if !SUPPORTED_PRODUCT_CODES.contains(&self.device.product_code.as_str()) {
            return invalid(format!("snapshot is from another board ({})", self.device.product_code));
        }
        if self.macros.len() != macros::SLOTS as usize {
            return invalid(format!("snapshot has {} macro slots, expected {}", self.macros.len(), macros::SLOTS));
        }
        if self.advanced.len() != crate::advanced::SLOTS as usize {
            return invalid(format!("snapshot has {} advanced-key slots, expected {}", self.advanced.len(), crate::advanced::SLOTS));
        }
        crate::lighting::check_slot(self.lighting.active_custom_slot)?;
        if self.trigger.len() != layout::KEYS.len() {
            return invalid(format!("snapshot has trigger settings for {} keys, expected {}", self.trigger.len(), layout::KEYS.len()));
        }
        Ok(())
    }
}

fn now_unix() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// Reads every table (in the official driver's connect order) under one device lock.
pub(crate) fn read<L: Link>(kb: &mut Keyboard<L>) -> Result<Snapshot> {
    kb.with_lock(|kb| {
        let caps = *kb.capabilities()?;
        let travel = kb.travel_limits()?;
        let macros = kb.read_macros()?;
        let advanced = kb.read_advanced()?;
        let socd = if caps.socd {
            Some(Socd { enabled: kb.read_socd_power()?.enabled, pairs: kb.read_socd_pairs()? })
        } else {
            None
        };
        let keymap = Keymaps { base: kb.read_keymap(Layer::Base)?, fn_layer: kb.read_keymap(Layer::Fn)? };
        let effect = kb.read_effect()?;
        let active_custom_slot = kb.read_active_custom_slot()?;
        let custom = if caps.custom_light() { Some([kb.read_custom_colors(0)?, kb.read_custom_colors(1)?]) } else { None };
        let trigger = kb.read_triggers(&layout::all_indices())?;
        let poll_rate_khz = if caps.poll_rate() { Some(kb.read_poll_rate()?) } else { None };
        let switch_types = if caps.switch_types() { Some(kb.read_switch_types()?) } else { None };
        let dead_band = if caps.dead_band() { Some(kb.read_dead_band()?) } else { None };
        let id = kb.identity().clone();
        Ok(Snapshot {
            format: FORMAT_NAME.into(),
            schema: SCHEMA,
            created_unix: now_unix(),
            device: DeviceInfo { product_code: id.product_code, firmware: id.firmware, travel },
            lighting: crate::snapshot::Lighting { effect, active_custom_slot, custom },
            keymap,
            macros,
            advanced,
            trigger,
            dead_band,
            switch_types,
            poll_rate_khz,
            socd,
            win_lock: caps.win_lock(),
        })
    })
}

/// What `apply` wrote.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ApplyReport {
    /// Tables written (and verified), in order.
    pub written: Vec<String>,
    /// The poll rate was changed: the keyboard restarts and must be opened again.
    pub reconnect_needed: bool,
}

/// Tables of `new` that differ from `old`, by name (in apply order).
pub fn changed_tables(new: &Snapshot, old: &Snapshot) -> Vec<String> {
    let mut v = Vec::new();
    if new.lighting.effect != old.lighting.effect {
        v.push("lighting effect".to_string());
    }
    if let (Some(n), Some(o)) = (&new.lighting.custom, &old.lighting.custom) {
        for s in 0..2 {
            let reselect = s as u8 == new.lighting.active_custom_slot
                && new.lighting.active_custom_slot != old.lighting.active_custom_slot;
            if n[s] != o[s] || reselect {
                v.push(format!("custom colours slot {s}"));
            }
        }
    }
    for s in 0..macros::SLOTS as usize {
        if new.macros.get(s) != old.macros.get(s) {
            v.push(format!("macro slot {s}"));
        }
    }
    if new.advanced != old.advanced {
        v.push("advanced keys".into());
    }
    if new.keymap.base != old.keymap.base {
        v.push("base layer".into());
    }
    if new.keymap.fn_layer != old.keymap.fn_layer {
        v.push("fn layer".into());
    }
    if !new.trigger.changed_keys(&old.trigger).is_empty() {
        v.push("trigger".into());
    }
    if new.dead_band.is_some() && new.dead_band != old.dead_band {
        v.push("dead band".into());
    }
    if new.switch_types.is_some() && new.switch_types != old.switch_types {
        v.push("switch types".into());
    }
    if let (Some(n), Some(o)) = (&new.socd, &old.socd) {
        if n.pairs != o.pairs {
            v.push("SOCD pairs".into());
        }
        if n.enabled != o.enabled {
            v.push("SOCD power".into());
        }
    }
    if new.win_lock != old.win_lock {
        v.push("win-lock".into());
    }
    if new.poll_rate_khz.is_some() && new.poll_rate_khz != old.poll_rate_khz {
        v.push("poll rate".into());
    }
    v
}

pub(crate) fn apply<L: Link>(kb: &mut Keyboard<L>, new: &Snapshot, old: &Snapshot) -> Result<ApplyReport> {
    new.check_shape()?;
    let caps = *kb.capabilities()?;
    // optional sections must match what the board has
    let has = |present: bool, cap: bool, what: &str| -> Result<()> {
        if present && !cap {
            return Err(Error::Unsupported(format!("{what} (present in the snapshot)")));
        }
        Ok(())
    };
    has(new.lighting.custom.is_some(), caps.custom_light(), "per-key RGB")?;
    has(new.dead_band.is_some(), caps.dead_band(), "dead band")?;
    has(new.switch_types.is_some(), caps.switch_types(), "switch types")?;
    has(new.poll_rate_khz.is_some(), caps.poll_rate(), "poll rate")?;
    has(new.socd.is_some(), caps.socd, "SOCD")?;
    // validate before the first write
    let limits = kb.travel_limits()?;
    for (idx, t) in new.trigger.iter() {
        t.validate(&limits).map_err(|e| Error::Invalid(format!("trigger {}: {e}", key_name(idx))))?;
    }
    crate::keymap::encode_layer(Layer::Base, &new.keymap.base)?;
    crate::keymap::encode_layer(Layer::Fn, &new.keymap.fn_layer)?;
    for m in new.macros.iter() {
        macros::encode(m.as_ref())?;
    }
    for a in &new.advanced {
        crate::advanced::encode(a)?;
    }
    if new.lighting.effect != old.lighting.effect {
        new.lighting.effect.validate(&kb.read_mode_list()?)?;
    }

    let tables = changed_tables(new, old);
    let mut report = ApplyReport::default();
    kb.with_lock(|kb| {
        for t in &tables {
            match t.as_str() {
                "lighting effect" => kb.set_effect(&new.lighting.effect)?,
                "advanced keys" => {
                    let slots: Vec<(u8, AdvancedKey)> = (0..new.advanced.len())
                        .filter(|s| old.advanced.get(*s) != new.advanced.get(*s))
                        .map(|s| (s as u8, new.advanced[s].clone()))
                        .collect();
                    kb.write_advanced(&slots)?
                }
                "base layer" => kb.write_keymap(Layer::Base, &new.keymap.base)?,
                "fn layer" => kb.write_keymap(Layer::Fn, &new.keymap.fn_layer)?,
                "trigger" => {
                    let mut changes = PerKey::new();
                    for idx in new.trigger.changed_keys(&old.trigger) {
                        if let Some(t) = new.trigger.get(idx) {
                            changes.0.insert(idx as u8, *t);
                        }
                    }
                    kb.write_triggers(&changes)?
                }
                "dead band" => kb.write_dead_band(new.dead_band.as_ref().expect("checked"))?,
                "switch types" => kb.write_switch_types(new.switch_types.as_ref().expect("checked"))?,
                "SOCD pairs" => kb.write_socd_pairs(&new.socd.as_ref().expect("checked").pairs)?,
                "SOCD power" => kb.set_socd_enabled(new.socd.as_ref().expect("checked").enabled)?,
                "win-lock" => kb.set_win_lock(new.win_lock)?,
                "poll rate" => {
                    kb.set_poll_rate(new.poll_rate_khz.expect("checked"))?;
                    report.reconnect_needed = true;
                }
                other => {
                    if let Some(s) = other.strip_prefix("custom colours slot ") {
                        let s: u8 = s.parse().expect("own label");
                        kb.write_custom_colors(s, &new.lighting.custom.as_ref().expect("checked")[s as usize])?;
                    } else if let Some(s) = other.strip_prefix("macro slot ") {
                        let s: u8 = s.parse().expect("own label");
                        kb.write_macro(s, new.macros[s as usize].as_ref())?;
                    } else {
                        unreachable!("unknown table {other}");
                    }
                }
            }
            report.written.push(t.clone());
        }
        Ok(())
    })?;
    Ok(report)
}

/// One human-readable difference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub section: String,
    pub detail: String,
}

impl fmt::Display for Change {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.section, self.detail)
    }
}

fn change(out: &mut Vec<Change>, section: &str, detail: String) {
    out.push(Change { section: section.to_string(), detail });
}

fn per_key<T: PartialEq>(out: &mut Vec<Change>, section: &str, old: &PerKey<T>, new: &PerKey<T>, show: impl Fn(Option<&T>) -> String) {
    for idx in old.changed_keys(new) {
        change(out, section, format!("{}: {} -> {}", key_name(idx), show(old.get(idx)), show(new.get(idx))));
    }
}

fn opt<T>(v: Option<&T>, f: impl Fn(&T) -> String) -> String {
    v.map_or_else(|| "-".to_string(), f)
}

fn effect_text(e: &Effect) -> String {
    let mode = crate::lighting::mode_info(e.mode).map_or_else(|| e.mode.to_string(), |m| m.name.to_string());
    format!(
        "{mode} brightness {} speed {} fg {} bg {} direction {}{}{}",
        e.brightness,
        e.speed,
        e.fg,
        e.bg,
        e.direction,
        if e.full_color { " full-colour" } else { "" },
        if e.on { "" } else { " (off)" }
    )
}

fn trigger_text(t: &KeyTrigger) -> String {
    let mode = match t.mode {
        crate::trigger::TriggerMode::Fixed => "fixed",
        crate::trigger::TriggerMode::Rapid => "rapid",
        crate::trigger::TriggerMode::RapidSplit => "rapid-split",
    };
    format!("{mode} actuation {} press {} release {}", t.actuation, t.press, t.release)
}

fn macro_text(m: Option<&Macro>) -> String {
    match m {
        None => "empty".into(),
        Some(m) => format!("{} events, {:?}, repeat {}", m.events.len(), m.mode, m.repeat),
    }
}

/// Human-readable differences from `old` to `new` (what applying `new` would change).
pub fn diff(old: &Snapshot, new: &Snapshot) -> Vec<Change> {
    let mut out = Vec::new();
    let (o, n) = (&old.lighting, &new.lighting);
    if o.effect != n.effect {
        change(&mut out, "lighting", format!("{} -> {}", effect_text(&o.effect), effect_text(&n.effect)));
    }
    if o.active_custom_slot != n.active_custom_slot {
        change(&mut out, "lighting", format!("active custom slot {} -> {}", o.active_custom_slot, n.active_custom_slot));
    }
    if let (Some(oc), Some(nc)) = (&o.custom, &n.custom) {
        for s in 0..2 {
            per_key(&mut out, &format!("custom colours {s}"), &oc[s], &nc[s], |c| opt(c, |c| c.to_string()));
        }
    }
    per_key(&mut out, "base layer", &old.keymap.base, &new.keymap.base, |a| opt(a, |a| a.to_string()));
    per_key(&mut out, "fn layer", &old.keymap.fn_layer, &new.keymap.fn_layer, |a| opt(a, |a| a.to_string()));
    for s in 0..macros::SLOTS as usize {
        let (a, b) = (old.macros.get(s).and_then(|m| m.as_ref()), new.macros.get(s).and_then(|m| m.as_ref()));
        if a != b {
            change(&mut out, "macros", format!("slot {s}: {} -> {}", macro_text(a), macro_text(b)));
        }
    }
    for s in 0..old.advanced.len().max(new.advanced.len()) {
        let (a, b) = (old.advanced.get(s), new.advanced.get(s));
        if a != b {
            let show = |k: Option<&AdvancedKey>| opt(k, |k| serde_json::to_string(k).unwrap_or_default());
            change(&mut out, "advanced keys", format!("slot {s}: {} -> {}", show(a), show(b)));
        }
    }
    per_key(&mut out, "trigger", &old.trigger, &new.trigger, |t| opt(t, trigger_text));
    if let (Some(a), Some(b)) = (&old.dead_band, &new.dead_band) {
        per_key(&mut out, "dead band", a, b, |d| opt(d, |d| format!("top {} bottom {}", d.top, d.bottom)));
    }
    if let (Some(a), Some(b)) = (&old.switch_types, &new.switch_types) {
        per_key(&mut out, "switch types", a, b, |s| {
            opt(s, |s| crate::trigger::switch_name(*s).map_or_else(|| s.to_string(), |n| format!("{s} {n}")))
        });
    }
    if new.poll_rate_khz.is_some() && old.poll_rate_khz != new.poll_rate_khz {
        let k = |v: Option<u8>| opt(v.as_ref(), |v| format!("{v} kHz"));
        change(&mut out, "poll rate", format!("{} -> {} (the keyboard restarts)", k(old.poll_rate_khz), k(new.poll_rate_khz)));
    }
    if let (Some(a), Some(b)) = (&old.socd, &new.socd) {
        if a.enabled != b.enabled {
            change(&mut out, "SOCD", format!("{} -> {}", on_off(a.enabled), on_off(b.enabled)));
        }
        if a.pairs != b.pairs {
            let show = |p: &[SocdPair]| {
                if p.is_empty() {
                    return "none".to_string();
                }
                p.iter().map(|p| format!("{}/{} {:?}", p.key1, p.key2, p.mode)).collect::<Vec<_>>().join(", ")
            };
            change(&mut out, "SOCD", format!("pairs {} -> {}", show(&a.pairs), show(&b.pairs)));
        }
    }
    if old.win_lock != new.win_lock {
        change(&mut out, "win-lock", format!("{:?} -> {:?}", old.win_lock, new.win_lock));
    }
    out
}

fn on_off(b: bool) -> &'static str {
    if b {
        "on"
    } else {
        "off"
    }
}

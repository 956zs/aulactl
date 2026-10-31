//! Argument helpers: options taken out of a command's arguments by name, and value parsers.

use aula_core::advanced::{DksKey, PHASE_HOLD, PHASE_NONE, PHASE_PRESS, PHASE_TAP};
use aula_core::info::WinLock;
use aula_core::keymap::{KeyAction, Layer};
use aula_core::lighting::ModeInfo;
use aula_core::socd::SocdMode;
use aula_core::trigger::{TravelLimits, TriggerMode, POLL_RATES};

/// One command's arguments. Options (`--name VALUE`, `--name=VALUE`, `--flag`) are taken out first, then
/// `positional` returns the rest and rejects unknown options.
pub struct Args<'a> {
    items: Vec<&'a str>,
}

impl<'a> Args<'a> {
    pub fn new(items: &[&'a str]) -> Self {
        Args {
            items: items.to_vec(),
        }
    }

    /// Whether `--name` was given (every occurrence is removed).
    pub fn flag(&mut self, name: &str) -> bool {
        let before = self.items.len();
        self.items.retain(|a| *a != name);
        self.items.len() != before
    }

    /// Value of `--name VALUE` or `--name=VALUE`. Given twice is an error.
    pub fn opt(&mut self, name: &str) -> Result<Option<&'a str>, String> {
        let mut found = None;
        let mut i = 0;
        while i < self.items.len() {
            let a = self.items[i];
            let value = if a == name {
                if i + 1 >= self.items.len() {
                    return Err(format!("{name} needs a value"));
                }
                let v = self.items.remove(i + 1);
                self.items.remove(i);
                v
            } else if let Some(v) = a.strip_prefix(name).and_then(|r| r.strip_prefix('=')) {
                self.items.remove(i);
                v
            } else {
                i += 1;
                continue;
            };
            if found.replace(value).is_some() {
                return Err(format!("{name} given twice"));
            }
        }
        Ok(found)
    }

    /// The remaining arguments; an option nobody took is an error.
    pub fn positional(self) -> Result<Vec<&'a str>, String> {
        if let Some(a) = self.items.iter().find(|a| a.starts_with("--")) {
            return Err(format!("unknown option '{a}'"));
        }
        Ok(self.items)
    }
}

/// A length in mm: `1.2`, `1.20mm`, `0.5 mm`. Not negative.
pub fn parse_mm(s: &str) -> Result<f64, String> {
    let t = s.trim();
    let t = t.strip_suffix("mm").unwrap_or(t).trim();
    match t.parse::<f64>() {
        Ok(v) if v.is_finite() && v >= 0.0 && !t.is_empty() => Ok(v),
        _ => Err(format!("'{s}' is not a length in mm (e.g. 1.20)")),
    }
}

/// mm to the nearest raw trigger unit.
pub fn mm_to_raw(limits: &TravelLimits, mm: f64) -> Result<u16, String> {
    Ok(limits.from_mm(mm)?)
}

/// Raw trigger units as mm with 2 decimals.
pub fn mm(limits: &TravelLimits, raw: u16) -> String {
    format!("{:.2}", limits.to_mm(raw))
}

pub fn parse_on_off(v: &str) -> Result<bool, String> {
    match v {
        "on" => Ok(true),
        "off" => Ok(false),
        _ => Err(format!("'{v}' is not on or off")),
    }
}

pub fn parse_layer(v: Option<&str>) -> Result<Layer, String> {
    match v {
        None | Some("base") => Ok(Layer::Base),
        Some("fn") => Ok(Layer::Fn),
        Some(v) => Err(format!("'{v}' is not a layer (base or fn)")),
    }
}

/// A slot number below `count`.
pub fn parse_slot(v: &str, count: u8, what: &str) -> Result<u8, String> {
    match v.parse::<u8>() {
        Ok(n) if n < count => Ok(n),
        _ => Err(format!("'{v}' is not a {what} slot (0..{})", count - 1)),
    }
}

/// A number in `range`.
pub fn parse_num(v: &str, range: std::ops::RangeInclusive<u32>, what: &str) -> Result<u32, String> {
    match v.parse::<u32>() {
        Ok(n) if range.contains(&n) => Ok(n),
        _ => Err(format!(
            "'{v}' is not a {what} ({}..{})",
            range.start(),
            range.end()
        )),
    }
}

/// Poll rate in kHz: `8`, `8k`, `8khz`, `8000`, `8000hz`.
pub fn parse_poll_rate(v: &str) -> Result<u8, String> {
    let lower = v.to_ascii_lowercase();
    let khz = if let Some(n) = lower
        .strip_suffix("khz")
        .or_else(|| lower.strip_suffix('k'))
    {
        n.parse::<u32>().ok()
    } else {
        let n = lower.strip_suffix("hz").unwrap_or(&lower);
        n.parse::<u32>().ok().map(|n| {
            if n >= 1000 && n % 1000 == 0 {
                n / 1000
            } else {
                n
            }
        })
    };
    match khz.and_then(|k| u8::try_from(k).ok()) {
        Some(k) if POLL_RATES.contains(&k) => Ok(k),
        _ => Err(format!("'{v}' is not a poll rate (1, 2, 4 or 8 kHz)")),
    }
}

pub fn parse_trigger_mode(v: &str) -> Result<TriggerMode, String> {
    match v {
        "fixed" => Ok(TriggerMode::Fixed),
        "rt" | "rapid" => Ok(TriggerMode::Rapid),
        "rt-split" | "rapid-split" | "split" => Ok(TriggerMode::RapidSplit),
        _ => Err(format!(
            "'{v}' is not a trigger mode (fixed, rt or rt-split)"
        )),
    }
}

pub fn trigger_mode_name(m: TriggerMode) -> &'static str {
    match m {
        TriggerMode::Fixed => "fixed",
        TriggerMode::Rapid => "rt",
        TriggerMode::RapidSplit => "rt-split",
    }
}

/// `--rt PRESS[/RELEASE]` in mm.
pub fn parse_rt(v: &str) -> Result<(f64, Option<f64>), String> {
    match v.split_once('/') {
        None => Ok((parse_mm(v)?, None)),
        Some((p, r)) => Ok((parse_mm(p)?, Some(parse_mm(r)?))),
    }
}

/// Win-lock options: `none` or a list of `win`, `shift-tab`, `alt-f4`, `alt-tab` (comma or space separated).
pub fn parse_win_lock(items: &[&str]) -> Result<WinLock, String> {
    let names: Vec<&str> = items
        .iter()
        .flat_map(|i| i.split(','))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    if names.is_empty() {
        return Err("name what win-lock blocks: win, shift-tab, alt-f4, alt-tab, or none".into());
    }
    let mut w = WinLock::default();
    for n in &names {
        match *n {
            "none" if names.len() == 1 => {}
            "win" => w.win = true,
            "shift-tab" => w.shift_tab = true,
            "alt-f4" => w.alt_f4 = true,
            "alt-tab" => w.alt_tab = true,
            _ => {
                return Err(format!(
                    "'{n}' is not a win-lock option (win, shift-tab, alt-f4, alt-tab, none)"
                ))
            }
        }
    }
    Ok(w)
}

pub fn win_lock_text(w: &WinLock) -> String {
    let names: Vec<&str> = [
        (w.win, "win"),
        (w.shift_tab, "shift-tab"),
        (w.alt_f4, "alt-f4"),
        (w.alt_tab, "alt-tab"),
    ]
    .iter()
    .filter(|(on, _)| *on)
    .map(|(_, n)| *n)
    .collect();
    if names.is_empty() {
        "none".into()
    } else {
        names.join(", ")
    }
}

pub fn parse_socd_mode(v: &str) -> Result<SocdMode, String> {
    match v {
        "last" => Ok(SocdMode::LastWins),
        "key1" => Ok(SocdMode::Key1Wins),
        "key2" => Ok(SocdMode::Key2Wins),
        "neutral" => Ok(SocdMode::Neutral),
        _ => Err(format!(
            "'{v}' is not a SOCD mode (last, key1, key2 or neutral)"
        )),
    }
}

pub fn socd_mode_name(m: SocdMode) -> &'static str {
    match m {
        SocdMode::LastWins => "last",
        SocdMode::Key1Wins => "key1",
        SocdMode::Key2Wins => "key2",
        SocdMode::Neutral => "neutral",
    }
}

/// A direction of `mode` by name (`left`, `spread`) or number.
pub fn parse_direction(mode: &ModeInfo, v: &str) -> Result<u8, String> {
    if mode.directions.is_empty() {
        return Err(format!("{} has no direction", mode.name));
    }
    let names: Vec<&str> = mode.directions.iter().map(|(_, n)| *n).collect();
    mode.directions
        .iter()
        .find(|(d, n)| n.eq_ignore_ascii_case(v) || v.parse::<u8>() == Ok(*d))
        .map(|(d, _)| *d)
        .ok_or_else(|| {
            format!(
                "'{v}' is not a direction of {} ({})",
                mode.name,
                names.join(", ")
            )
        })
}

pub fn direction_name(mode: Option<&ModeInfo>, d: u8) -> String {
    match mode {
        Some(m) if m.directions.is_empty() => "-".into(),
        Some(m) => m
            .directions
            .iter()
            .find(|(v, _)| *v == d)
            .map_or_else(|| d.to_string(), |(_, n)| n.to_string()),
        None => d.to_string(),
    }
}

/// DKS phase letters: `-` nothing, `t` tap, `p` press and hold, `h` still held (or digits 0..3).
const PHASE_LETTERS: [(u8, char); 4] = [
    (PHASE_NONE, '-'),
    (PHASE_TAP, 't'),
    (PHASE_PRESS, 'p'),
    (PHASE_HOLD, 'h'),
];

/// A DKS output key: `ACTION:PPPP`, e.g. `KeyA:p--h`, `Space:t---`.
pub fn parse_dks_key(v: &str) -> Result<DksKey, String> {
    let Some((action, phases)) = v.rsplit_once(':') else {
        return Err(format!(
            "'{v}' is not a DKS key (ACTION:PHASES, e.g. KeyA:p--h)"
        ));
    };
    let action: KeyAction = action.parse()?;
    let chars: Vec<char> = phases.chars().collect();
    if chars.len() != 4 {
        return Err(format!("'{phases}' needs 4 phases (- t p h, e.g. p--h)"));
    }
    let mut out = [0u8; 4];
    for (slot, c) in out.iter_mut().zip(&chars) {
        *slot = PHASE_LETTERS
            .iter()
            .find(|(n, l)| *l == c.to_ascii_lowercase() || c.to_digit(10) == Some(u32::from(*n)))
            .map(|(n, _)| *n)
            .ok_or_else(|| format!("'{c}' is not a DKS phase (- t p h)"))?;
    }
    Ok(DksKey {
        action,
        phases: out,
    })
}

pub fn phases_text(phases: &[u8; 4]) -> String {
    phases
        .iter()
        .map(|p| {
            PHASE_LETTERS
                .iter()
                .find(|(n, _)| n == p)
                .map_or('?', |(_, l)| *l)
        })
        .collect()
}

/// DKS routes: four mm values `R1,R2,R3,R4`, stored in 0.1 mm.
pub fn parse_routes(v: &str) -> Result<[u8; 4], String> {
    let parts: Vec<&str> = v.split(',').collect();
    if parts.len() != 4 {
        return Err(format!(
            "'{v}' needs four travel points in mm, e.g. 1.5,3.0,3.0,1.5"
        ));
    }
    let mut out = [0u8; 4];
    for (slot, p) in out.iter_mut().zip(parts) {
        let tenths = (parse_mm(p)? * 10.0).round();
        if !(1.0..=255.0).contains(&tenths) {
            return Err(format!("DKS point '{p}' is outside 0.1..25.5 mm"));
        }
        *slot = tenths as u8;
    }
    Ok(out)
}

pub fn routes_text(r: &[u8; 4]) -> String {
    r.iter()
        .map(|t| format!("{:.1}", f64::from(*t) / 10.0))
        .collect::<Vec<_>>()
        .join("/")
}

/// Mod-tap hold threshold: `200` or `200ms`, stored in 10 ms units.
pub fn parse_threshold(v: &str) -> Result<u8, String> {
    let n = v.strip_suffix("ms").unwrap_or(v);
    match n.parse::<u32>() {
        Ok(ms) if ms % 10 == 0 && (150..=1000).contains(&ms) => Ok((ms / 10) as u8),
        _ => Err(format!(
            "'{v}' is not a hold threshold (150..1000 ms in steps of 10)"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limits() -> TravelLimits {
        TravelLimits {
            max: 340,
            min: 8,
            unit_num: 1,
            high_precision: false,
        }
    }

    #[test]
    fn options_and_flags() {
        let mut a = Args::new(&["wasd", "--point", "1.2", "--yes", "--rt=0.3/0.5"]);
        assert!(a.flag("--yes"));
        assert!(!a.flag("--yes"));
        assert_eq!(a.opt("--point"), Ok(Some("1.2")));
        assert_eq!(a.opt("--rt"), Ok(Some("0.3/0.5")));
        assert_eq!(a.opt("--mode"), Ok(None));
        assert_eq!(a.positional(), Ok(vec!["wasd"]));
    }

    #[test]
    fn option_errors() {
        assert!(Args::new(&["--point"])
            .opt("--point")
            .unwrap_err()
            .contains("needs a value"));
        assert!(Args::new(&["--point", "1", "--point=2"])
            .opt("--point")
            .unwrap_err()
            .contains("twice"));
        assert_eq!(
            Args::new(&["a", "--bogus"]).positional().unwrap_err(),
            "unknown option '--bogus'"
        );
        assert_eq!(Args::new(&[]).positional(), Ok(vec![]));
        // a prefix of another option is not that option
        assert_eq!(Args::new(&["--pointy=1"]).opt("--point"), Ok(None));
    }

    #[test]
    fn mm_values() {
        assert_eq!(parse_mm("1.2"), Ok(1.2));
        assert_eq!(parse_mm("1.20mm"), Ok(1.2));
        assert_eq!(parse_mm(" 0.5 mm"), Ok(0.5));
        assert_eq!(parse_mm("0"), Ok(0.0));
        assert_eq!(parse_mm("3.40"), Ok(3.4));
        for bad in ["", "mm", "-0.1", "abc", "1,2", "NaN", "inf"] {
            assert!(parse_mm(bad).is_err(), "{bad}");
        }
        assert_eq!(mm_to_raw(&limits(), 1.2), Ok(120));
        assert_eq!(mm_to_raw(&limits(), 0.005), Ok(1));
        assert_eq!(mm_to_raw(&limits(), 3.4), Ok(340));
        assert!(mm_to_raw(&limits(), 1e9).is_err());
        assert_eq!(mm(&limits(), 0), "0.00");
        assert_eq!(mm(&limits(), 8), "0.08");
        assert_eq!(mm(&limits(), 340), "3.40");
    }

    #[test]
    fn small_values() {
        assert_eq!(parse_on_off("on"), Ok(true));
        assert_eq!(parse_on_off("off"), Ok(false));
        assert!(parse_on_off("").is_err() && parse_on_off("ON").is_err());
        assert_eq!(parse_layer(None), Ok(Layer::Base));
        assert_eq!(parse_layer(Some("fn")), Ok(Layer::Fn));
        assert!(parse_layer(Some("Fn2")).is_err());
        assert_eq!(parse_slot("0", 10, "macro"), Ok(0));
        assert_eq!(parse_slot("9", 10, "macro"), Ok(9));
        assert_eq!(
            parse_slot("10", 10, "macro").unwrap_err(),
            "'10' is not a macro slot (0..9)"
        );
        assert!(parse_slot("-1", 10, "macro").is_err() && parse_slot("", 10, "macro").is_err());
        assert_eq!(parse_num("5", 0..=5, "brightness"), Ok(5));
        assert!(parse_num("6", 0..=5, "brightness").is_err());
    }

    #[test]
    fn poll_rates() {
        for (s, k) in [
            ("8", 8),
            ("1", 1),
            ("8k", 8),
            ("4kHz", 4),
            ("2000", 2),
            ("1000hz", 1),
        ] {
            assert_eq!(parse_poll_rate(s), Ok(k), "{s}");
        }
        for bad in ["", "3", "16", "500", "8000k", "fast", "0"] {
            assert!(parse_poll_rate(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn trigger_modes_and_rt() {
        assert_eq!(parse_trigger_mode("fixed"), Ok(TriggerMode::Fixed));
        assert_eq!(parse_trigger_mode("rt"), Ok(TriggerMode::Rapid));
        assert_eq!(parse_trigger_mode("rt-split"), Ok(TriggerMode::RapidSplit));
        assert!(parse_trigger_mode("RT").is_err() && parse_trigger_mode("").is_err());
        for m in [
            TriggerMode::Fixed,
            TriggerMode::Rapid,
            TriggerMode::RapidSplit,
        ] {
            assert_eq!(parse_trigger_mode(trigger_mode_name(m)), Ok(m));
        }
        assert_eq!(parse_rt("0.3"), Ok((0.3, None)));
        assert_eq!(parse_rt("0.3/0.5mm"), Ok((0.3, Some(0.5))));
        assert!(parse_rt("0.3/").is_err() && parse_rt("/0.3").is_err() && parse_rt("").is_err());
    }

    #[test]
    fn win_lock_options() {
        assert_eq!(parse_win_lock(&["none"]), Ok(WinLock::default()));
        let w = parse_win_lock(&["win,alt-tab"]).unwrap();
        assert!(w.win && w.alt_tab && !w.alt_f4 && !w.shift_tab);
        assert_eq!(
            parse_win_lock(&["win", "shift-tab", "alt-f4", "alt-tab"])
                .unwrap()
                .bits(),
            0x0f
        );
        assert!(parse_win_lock(&[]).is_err());
        assert!(parse_win_lock(&["none", "win"]).is_err());
        assert!(parse_win_lock(&["ctrl"]).is_err());
        assert_eq!(win_lock_text(&WinLock::default()), "none");
        assert_eq!(win_lock_text(&WinLock::from_bits(0x09)), "win, alt-tab");
    }

    #[test]
    fn socd_modes() {
        for m in [
            SocdMode::LastWins,
            SocdMode::Key1Wins,
            SocdMode::Key2Wins,
            SocdMode::Neutral,
        ] {
            assert_eq!(parse_socd_mode(socd_mode_name(m)), Ok(m));
        }
        assert!(parse_socd_mode("first").is_err());
    }

    #[test]
    fn directions() {
        let wave = aula_core::lighting::mode_by_name("wave").unwrap();
        let static_ = aula_core::lighting::mode_by_name("static").unwrap();
        assert_eq!(parse_direction(wave, "left"), Ok(1));
        assert_eq!(parse_direction(wave, "Gather"), Ok(5));
        assert_eq!(parse_direction(wave, "3"), Ok(3));
        assert!(parse_direction(wave, "9")
            .unwrap_err()
            .contains("right, left"));
        assert!(parse_direction(static_, "left")
            .unwrap_err()
            .contains("no direction"));
        assert_eq!(direction_name(Some(wave), 4), "spread");
        assert_eq!(direction_name(Some(static_), 0), "-");
        assert_eq!(direction_name(None, 7), "7");
    }

    #[test]
    fn dks_keys_and_routes() {
        let k = parse_dks_key("KeyA:p--h").unwrap();
        assert_eq!(k.action, KeyAction::Key(0x04));
        assert_eq!(k.phases, [PHASE_PRESS, PHASE_NONE, PHASE_NONE, PHASE_HOLD]);
        assert_eq!(phases_text(&k.phases), "p--h");
        assert_eq!(parse_dks_key("Space:1023").unwrap().phases, [1, 0, 2, 3]);
        assert_eq!(
            parse_dks_key("raw:00040000:t---").unwrap().action,
            KeyAction::Raw([0, 4, 0, 0])
        );
        assert!(parse_dks_key("KeyA").is_err());
        assert!(parse_dks_key("KeyA:p-h").is_err());
        assert!(parse_dks_key("KeyA:x---").is_err());
        assert!(parse_dks_key("Nope:t---").is_err());
        assert_eq!(phases_text(&[9, 0, 0, 0]), "?---");
        assert_eq!(parse_routes("1.5,3.0,3.0,1.5"), Ok([15, 30, 30, 15]));
        assert_eq!(routes_text(&[15, 30, 30, 15]), "1.5/3.0/3.0/1.5");
        assert!(parse_routes("1.5,3.0,3.0").is_err());
        assert!(parse_routes("0,3.0,3.0,1.5").is_err());
        assert!(parse_routes("1.5,30,3.0,1.5").is_err());
    }

    #[test]
    fn thresholds() {
        assert_eq!(parse_threshold("150"), Ok(15));
        assert_eq!(parse_threshold("200ms"), Ok(20));
        assert_eq!(parse_threshold("1000"), Ok(100));
        for bad in ["140", "1010", "155", "", "fast"] {
            assert!(parse_threshold(bad).is_err(), "{bad}");
        }
    }
}

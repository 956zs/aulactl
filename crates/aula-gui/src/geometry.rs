//! Keyboard drawing data and key selection for the pages: the core layout table (protocol section 14) in
//! key units, rectangle hit tests for drag selection, and the quick groups.

use aula_core::layout::{self, Key};
use serde_json::{json, Value};

/// Space between drawn keys, in u (the official 3 px of a 38 px pitch).
pub const GAP_U: f64 = 3.0 / 38.0;

fn round3(v: f32) -> f64 {
    (f64::from(v) * 1000.0).round() / 1000.0
}

/// One key for the keyboard view: position and width in u (the gap included), height 1 u.
fn key_json(k: &Key) -> Value {
    json!({
        "index": k.index,
        "code": k.code,
        "label": k.label,
        "x": round3(k.x()),
        "y": round3(k.y()),
        "w": round3(k.width()),
        "row": k.visual_row,
        "fnFixed": k.fn_fixed,
    })
}

/// `{ keys: [...], width, height }` in u.
pub fn layout_json() -> Value {
    let keys: Vec<Value> = layout::KEYS.iter().map(key_json).collect();
    let (w, h) = size_u();
    json!({ "keys": keys, "width": w, "height": h })
}

/// Width and height of the whole board in u.
pub fn size_u() -> (f64, f64) {
    let w = layout::KEYS.iter().map(|k| round3(k.x() + k.width())).fold(0.0, f64::max);
    let h = layout::KEYS.iter().map(|k| round3(k.y() + 1.0)).fold(0.0, f64::max);
    (w, h)
}

/// Keys whose drawn rectangle intersects the rectangle spanned by two corners (u, any order), ascending.
/// A rectangle of zero size still hits the key under it.
pub fn keys_in_rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Vec<usize> {
    if ![x0, y0, x1, y1].iter().all(|v| v.is_finite()) {
        return Vec::new();
    }
    let (left, right) = (x0.min(x1), x0.max(x1));
    let (top, bottom) = (y0.min(y1), y0.max(y1));
    let mut out: Vec<usize> = layout::KEYS
        .iter()
        .filter(|k| {
            let kx = f64::from(k.x());
            let ky = f64::from(k.y());
            let kr = kx + f64::from(k.width()) - GAP_U;
            let kb = ky + 1.0 - GAP_U;
            left <= kr && right >= kx && top <= kb && bottom >= ky
        })
        .map(|k| k.index as usize)
        .collect();
    out.sort_unstable();
    out
}

/// Resolves a core key selector (`all`, `wasd`, `letters`, `row:2`, ...) to indices.
pub fn select(selector: &str) -> Result<Vec<usize>, String> {
    layout::select(selector).map_err(String::from)
}

/// How a click changes the selection: replace it, or toggle the key (Ctrl / Shift held).
pub fn click(selection: &[usize], key: usize, toggle: bool) -> Vec<usize> {
    if !layout::is_key(key) {
        return selection.to_vec();
    }
    if !toggle {
        return vec![key];
    }
    let mut out: Vec<usize> = selection.iter().copied().filter(|k| *k != key).collect();
    if out.len() == selection.len() {
        out.push(key);
    }
    out.sort_unstable();
    out.dedup();
    out
}

/// Adds `keys` to `selection` (drag with a modifier) or replaces it.
pub fn merge(selection: &[usize], keys: &[usize], add: bool) -> Vec<usize> {
    let mut out: Vec<usize> = if add { selection.to_vec() } else { Vec::new() };
    out.extend(keys.iter().copied().filter(|k| layout::is_key(*k)));
    out.sort_unstable();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn idx(code: &str) -> usize {
        layout::by_code(code).unwrap().index as usize
    }

    #[test]
    fn layout_has_every_key_in_u() {
        let v = layout_json();
        let keys = v["keys"].as_array().unwrap();
        assert_eq!(keys.len(), 61);
        let esc = &keys[0];
        assert_eq!(esc["code"], "Escape");
        assert_eq!((esc["x"].as_f64().unwrap(), esc["y"].as_f64().unwrap(), esc["w"].as_f64().unwrap()), (0.0, 0.0, 1.0));
        let space = keys.iter().find(|k| k["code"] == "Space").unwrap();
        assert_eq!(space["row"], 4);
        assert!(space["w"].as_f64().unwrap() > 5.0);
        let (w, h) = size_u();
        assert!((14.9..=15.1).contains(&w), "{w}");
        assert_eq!(h, 5.0);
    }

    #[test]
    fn point_rect_hits_one_key() {
        assert_eq!(keys_in_rect(0.5, 0.5, 0.5, 0.5), vec![idx("Escape")]);
        // Space spans the middle of the bottom row.
        assert_eq!(keys_in_rect(7.0, 4.5, 7.0, 4.5), vec![idx("Space")]);
    }

    #[test]
    fn drag_rect_in_any_direction() {
        // Q..E on row 1, dragged right to left and bottom to top.
        let a = keys_in_rect(3.6, 1.6, 1.8, 1.4);
        assert_eq!(a, vec![idx("KeyQ"), idx("KeyW"), idx("KeyE")]);
        assert_eq!(keys_in_rect(1.8, 1.4, 3.6, 1.6), a);
    }

    #[test]
    fn gaps_and_outside_hit_nothing() {
        // The gap after Esc (x 0.921 .. 1.0) on row 0.
        assert!(keys_in_rect(0.96, 0.5, 0.96, 0.5).is_empty());
        assert!(keys_in_rect(-5.0, -5.0, -1.0, -1.0).is_empty());
        assert!(keys_in_rect(20.0, 0.0, 30.0, 4.0).is_empty());
        assert!(keys_in_rect(f64::NAN, 0.0, 1.0, 1.0).is_empty());
    }

    #[test]
    fn whole_board_rect_is_all_keys() {
        assert_eq!(keys_in_rect(0.0, 0.0, 16.0, 5.0), layout::all_indices());
    }

    #[test]
    fn selectors() {
        assert_eq!(select("wasd").unwrap().len(), 4);
        assert_eq!(select("all").unwrap().len(), 61);
        assert!(select("").is_err());
        assert!(select("row:9").is_err());
    }

    #[test]
    fn click_replaces_or_toggles() {
        let (q, w) = (idx("KeyQ"), idx("KeyW"));
        assert_eq!(click(&[], q, false), vec![q]);
        assert_eq!(click(&[q, w], w, false), vec![w]);
        assert_eq!(click(&[q], w, true), vec![q, w]);
        assert_eq!(click(&[q, w], q, true), vec![w]);
        assert_eq!(click(&[q], q, true), Vec::<usize>::new());
        // Not a key: unchanged.
        assert_eq!(click(&[q], 0, false), vec![q]);
    }

    #[test]
    fn merge_adds_or_replaces() {
        let (q, w, e) = (idx("KeyQ"), idx("KeyW"), idx("KeyE"));
        assert_eq!(merge(&[q], &[w, e], true), vec![q, w, e]);
        assert_eq!(merge(&[q], &[w, e], false), vec![w, e]);
        assert_eq!(merge(&[q], &[q, 0, 131], true), vec![q]);
        assert_eq!(merge(&[], &[], false), Vec::<usize>::new());
    }
}

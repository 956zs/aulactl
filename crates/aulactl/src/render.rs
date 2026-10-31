//! Text rendering: keyboard-shaped views, aligned tables, key lists.

use aula_core::layout::{self, Key, VISUAL_ROWS};
use std::fmt::Write as _;

/// The board drawn as five rows of `|cell|cell|` boxes, `unit_cols` characters per 1 u. `text` fills a cell
/// (cut to fit), `style` wraps it in an ANSI SGR sequence (e.g. `"7"` for inverse) when it returns one.
pub fn board(
    unit_cols: usize,
    text: impl Fn(&Key) -> String,
    style: impl Fn(&Key) -> Option<String>,
) -> String {
    let col = |u: f32| (u * unit_cols as f32).round() as usize;
    let mut out = String::new();
    for row in 0..VISUAL_ROWS {
        let mut keys: Vec<&Key> = layout::KEYS
            .iter()
            .filter(|k| k.visual_row == row)
            .collect();
        keys.sort_by(|a, b| a.x().total_cmp(&b.x()));
        let mut line = String::new();
        let mut at = 0;
        for k in keys {
            let (start, end) = (col(k.x()), col(k.x() + k.width()));
            if start > at {
                line.push_str(&" ".repeat(start - at));
            }
            let inner = end.saturating_sub(start + 1).max(1);
            let t: String = text(k).chars().take(inner).collect();
            let cell = format!("{t:^inner$}");
            match style(k) {
                Some(sgr) => write!(line, "|\x1b[{sgr}m{cell}\x1b[0m").expect("write to String"),
                None => write!(line, "|{cell}").expect("write to String"),
            }
            at = start.max(at) + 1 + inner;
        }
        line.push('|');
        out.push_str(&line);
        out.push('\n');
    }
    out
}

/// Rows with columns padded to the widest cell (2 spaces apart), trailing spaces trimmed.
pub fn table(rows: &[Vec<String>]) -> String {
    let cols = rows.iter().map(Vec::len).max().unwrap_or(0);
    let widths: Vec<usize> = (0..cols)
        .map(|c| {
            rows.iter()
                .filter_map(|r| r.get(c))
                .map(|s| s.chars().count())
                .max()
                .unwrap_or(0)
        })
        .collect();
    let mut out = String::new();
    for r in rows {
        let mut line = String::new();
        for (c, cell) in r.iter().enumerate() {
            if c > 0 {
                line.push_str("  ");
            }
            write!(line, "{cell:<w$}", w = widths[c]).expect("write to String");
        }
        out.push_str(line.trim_end());
        out.push('\n');
    }
    out
}

/// Key codes of `keys` (indices), space separated; every key of the board is `all keys`.
pub fn key_list(keys: &[usize]) -> String {
    if keys.len() == layout::KEYS.len() {
        return "all keys".into();
    }
    keys.iter()
        .map(|i| aula_core::perkey::key_name(*i))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn board_shape() {
        let text = board(5, |k| k.label.to_string(), |_| None);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 5);
        // 15 u wide: every row ends at the same column
        for l in &lines {
            assert_eq!(l.chars().count(), 15 * 5 + 1, "{l:?}");
            assert!(l.starts_with('|') && l.ends_with('|'));
        }
        assert!(lines[0].starts_with("|Esc | 1! | 2@ |"), "{}", lines[0]);
        assert!(lines[0].ends_with("|  Back   |"), "{}", lines[0]);
        assert!(
            lines[4].contains("|           Space            |"),
            "{}",
            lines[4]
        );
        assert_eq!(lines[2].matches('|').count(), 14);
    }

    #[test]
    fn board_cuts_long_text_and_styles() {
        let text = board(
            5,
            |_| "BrightnessUp".into(),
            |k| (k.code == "KeyW").then(|| "7".to_string()),
        );
        let first = text.lines().next().unwrap();
        assert!(first.starts_with("|Brig|Brig|"), "{first}");
        let second = text.lines().nth(1).unwrap();
        assert!(second.contains("|\x1b[7mBrig\x1b[0m|"), "{second}");
        assert_eq!(text.matches("\x1b[7m").count(), 1);
        let empty = board(5, |_| String::new(), |_| None);
        assert!(empty.lines().all(|l| l.chars().count() == 76));
    }

    #[test]
    fn tables() {
        let rows = vec![
            vec!["key".to_string(), "mode".into(), "mm".into()],
            vec!["Escape".to_string(), "rt".into(), "1.00".into()],
            vec!["W".to_string(), "fixed".into()],
        ];
        assert_eq!(
            table(&rows),
            "key     mode   mm\nEscape  rt     1.00\nW       fixed\n"
        );
        assert_eq!(table(&[]), "");
        assert_eq!(table(&[vec![]]), "\n");
    }

    #[test]
    fn key_lists() {
        assert_eq!(key_list(&[]), "");
        assert_eq!(key_list(&[46, 68]), "KeyW KeyA");
        assert_eq!(key_list(&layout::all_indices()), "all keys");
        assert_eq!(key_list(&[3]), "idx:3");
    }
}

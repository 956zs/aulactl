//! Text for the key overlay: a TrueType/OpenType font through `fontdue` (rasterized glyphs cached per size), or
//! the built-in 5×7 pixel font when no font file is found (tests use it, so their output does not depend on
//! the fonts installed).

use std::collections::HashMap;
use std::path::Path;
use tiny_skia::{Paint, Pixmap, Rect, Transform};

/// Fonts tried when `--font` is not given and fontconfig has no answer.
const FALLBACK_FONTS: [&str; 6] = [
    "/usr/share/fonts/noto/NotoSans-Bold.ttf",
    "/usr/share/fonts/TTF/DejaVuSans-Bold.ttf",
    "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
    "/usr/share/fonts/liberation/LiberationSans-Bold.ttf",
    "/usr/share/fonts/OpenSans/OpenSans-Bold.ttf",
    "/usr/share/fonts/Adwaita/AdwaitaSans-Regular.ttf",
];
/// Largest font file read.
const MAX_FONT_BYTES: u64 = 64 << 20;

/// Straight (not premultiplied) RGBA.
pub type Rgba = [u8; 4];

type Glyph = (fontdue::Metrics, Vec<u8>);

pub enum Font {
    Ttf {
        font: Box<fontdue::Font>,
        cache: HashMap<(char, u32), Glyph>,
    },
    Pixel,
}

impl Font {
    /// Loads a font file.
    pub fn load(path: &Path) -> Result<Font, String> {
        let meta = std::fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
        if meta.len() > MAX_FONT_BYTES {
            return Err(format!("{}: not a font (too large)", path.display()));
        }
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let font = fontdue::Font::from_bytes(bytes, fontdue::FontSettings::default())
            .map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(Font::Ttf {
            font: Box::new(font),
            cache: HashMap::new(),
        })
    }

    /// A bold sans-serif font from fontconfig (`fc-match`) or a few usual paths; the pixel font when none loads.
    pub fn system() -> Font {
        let fc = std::process::Command::new("fc-match")
            .args(["-f", "%{file}", "sans-serif:bold"])
            .stderr(std::process::Stdio::null())
            .output()
            .ok()
            .filter(|o| o.status.success())
            .and_then(|o| String::from_utf8(o.stdout).ok());
        fc.into_iter()
            .chain(FALLBACK_FONTS.iter().map(|s| s.to_string()))
            .find_map(|p| Font::load(Path::new(&p)).ok())
            .unwrap_or(Font::Pixel)
    }

    /// Width of `text` at `size` px (cap height ≈ 0.7 size).
    pub fn width(&mut self, text: &str, size: f32) -> f32 {
        match self {
            Font::Pixel => {
                let n = text.chars().count() as f32;
                if n == 0.0 {
                    0.0
                } else {
                    (n * 6.0 - 1.0) * pixel_unit(size)
                }
            }
            Font::Ttf { font, cache } => {
                let px = size.round().max(1.0) as u32;
                text.chars()
                    .map(|c| glyph(font, cache, c, px).0.advance_width)
                    .sum()
            }
        }
    }

    /// Draws `text` centred on (`cx`, `cy`) (the middle of its capital letters).
    pub fn draw(&mut self, pm: &mut Pixmap, text: &str, cx: f32, cy: f32, size: f32, color: Rgba) {
        if text.is_empty() || size < 1.0 || color[3] == 0 {
            return;
        }
        let w = self.width(text, size);
        let mut x = cx - w / 2.0;
        match self {
            Font::Pixel => {
                let u = pixel_unit(size);
                let top = cy - 3.5 * u;
                let mut paint = Paint::default();
                paint.set_color_rgba8(color[0], color[1], color[2], color[3]);
                paint.anti_alias = false;
                for c in text.chars() {
                    let rows = pixel_glyph(c);
                    for (r, bits) in rows.iter().enumerate() {
                        for col in 0..5 {
                            if bits & (0x10 >> col) != 0 {
                                let rect = Rect::from_xywh(
                                    (x + col as f32 * u).round(),
                                    (top + r as f32 * u).round(),
                                    u.round().max(1.0),
                                    u.round().max(1.0),
                                );
                                if let Some(rect) = rect {
                                    pm.fill_rect(rect, &paint, Transform::identity(), None);
                                }
                            }
                        }
                    }
                    x += 6.0 * u;
                }
            }
            Font::Ttf { font, cache } => {
                let px = size.round().max(1.0) as u32;
                // baseline so the capitals' middle lands on cy (cap height ~0.7 em)
                let baseline = cy + 0.35 * px as f32;
                for c in text.chars() {
                    let (m, bitmap) = glyph(font, cache, c, px);
                    let gx = (x + m.xmin as f32).round() as i32;
                    let gy = (baseline - m.height as f32 - m.ymin as f32).round() as i32;
                    blit_coverage(pm, gx, gy, m.width, m.height, bitmap, color);
                    x += m.advance_width;
                }
            }
        }
    }
}

fn glyph<'a>(
    font: &fontdue::Font,
    cache: &'a mut HashMap<(char, u32), Glyph>,
    c: char,
    px: u32,
) -> &'a Glyph {
    cache
        .entry((c, px))
        .or_insert_with(|| font.rasterize(c, px as f32))
}

/// One pixel-font cell: 9 cells per `size` (7 rows + spacing).
fn pixel_unit(size: f32) -> f32 {
    (size / 9.0).max(1.0)
}

/// Source-over of a coverage bitmap in `color` onto a premultiplied pixmap.
fn blit_coverage(pm: &mut Pixmap, x: i32, y: i32, w: usize, h: usize, cov: &[u8], color: Rgba) {
    let (pw, ph) = (pm.width() as i32, pm.height() as i32);
    let data = pm.data_mut();
    for row in 0..h as i32 {
        let py = y + row;
        if py < 0 || py >= ph {
            continue;
        }
        for col in 0..w as i32 {
            let px = x + col;
            if px < 0 || px >= pw {
                continue;
            }
            let c = cov[(row * w as i32 + col) as usize] as u32;
            let a = c * color[3] as u32 / 255;
            if a == 0 {
                continue;
            }
            let i = ((py * pw + px) * 4) as usize;
            let inv = 255 - a;
            for (k, &ch) in color[..3].iter().enumerate() {
                let src = ch as u32 * a / 255;
                data[i + k] = (src + data[i + k] as u32 * inv / 255).min(255) as u8;
            }
            data[i + 3] = (a + data[i + 3] as u32 * inv / 255).min(255) as u8;
        }
    }
}

/// 5×7 glyph rows (bit 4 = left column). Lowercase letters use the capitals; unknown characters are `?`.
fn pixel_glyph(c: char) -> [u8; 7] {
    match c.to_ascii_uppercase() {
        '0' => [0x0E, 0x11, 0x13, 0x15, 0x19, 0x11, 0x0E],
        '1' => [0x04, 0x0C, 0x04, 0x04, 0x04, 0x04, 0x0E],
        '2' => [0x0E, 0x11, 0x01, 0x02, 0x04, 0x08, 0x1F],
        '3' => [0x1F, 0x02, 0x04, 0x02, 0x01, 0x11, 0x0E],
        '4' => [0x02, 0x06, 0x0A, 0x12, 0x1F, 0x02, 0x02],
        '5' => [0x1F, 0x10, 0x1E, 0x01, 0x01, 0x11, 0x0E],
        '6' => [0x06, 0x08, 0x10, 0x1E, 0x11, 0x11, 0x0E],
        '7' => [0x1F, 0x01, 0x02, 0x04, 0x08, 0x08, 0x08],
        '8' => [0x0E, 0x11, 0x11, 0x0E, 0x11, 0x11, 0x0E],
        '9' => [0x0E, 0x11, 0x11, 0x0F, 0x01, 0x02, 0x0C],
        'A' => [0x0E, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11],
        'B' => [0x1E, 0x11, 0x11, 0x1E, 0x11, 0x11, 0x1E],
        'C' => [0x0E, 0x11, 0x10, 0x10, 0x10, 0x11, 0x0E],
        'D' => [0x1C, 0x12, 0x11, 0x11, 0x11, 0x12, 0x1C],
        'E' => [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x1F],
        'F' => [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x10],
        'G' => [0x0E, 0x11, 0x10, 0x17, 0x11, 0x11, 0x0F],
        'H' => [0x11, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11],
        'I' => [0x0E, 0x04, 0x04, 0x04, 0x04, 0x04, 0x0E],
        'J' => [0x07, 0x02, 0x02, 0x02, 0x02, 0x12, 0x0C],
        'K' => [0x11, 0x12, 0x14, 0x18, 0x14, 0x12, 0x11],
        'L' => [0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x1F],
        'M' => [0x11, 0x1B, 0x15, 0x15, 0x11, 0x11, 0x11],
        'N' => [0x11, 0x11, 0x19, 0x15, 0x13, 0x11, 0x11],
        'O' => [0x0E, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E],
        'P' => [0x1E, 0x11, 0x11, 0x1E, 0x10, 0x10, 0x10],
        'Q' => [0x0E, 0x11, 0x11, 0x11, 0x15, 0x12, 0x0D],
        'R' => [0x1E, 0x11, 0x11, 0x1E, 0x14, 0x12, 0x11],
        'S' => [0x0F, 0x10, 0x10, 0x0E, 0x01, 0x01, 0x1E],
        'T' => [0x1F, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04],
        'U' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E],
        'V' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x0A, 0x04],
        'W' => [0x11, 0x11, 0x11, 0x15, 0x15, 0x15, 0x0A],
        'X' => [0x11, 0x11, 0x0A, 0x04, 0x0A, 0x11, 0x11],
        'Y' => [0x11, 0x11, 0x11, 0x0A, 0x04, 0x04, 0x04],
        'Z' => [0x1F, 0x01, 0x02, 0x04, 0x08, 0x10, 0x1F],
        ' ' => [0; 7],
        '.' => [0, 0, 0, 0, 0, 0x0C, 0x0C],
        ',' => [0, 0, 0, 0, 0x0C, 0x04, 0x08],
        '-' => [0, 0, 0, 0x1F, 0, 0, 0],
        '/' => [0x01, 0x02, 0x02, 0x04, 0x08, 0x08, 0x10],
        '\\' => [0x10, 0x08, 0x08, 0x04, 0x02, 0x02, 0x01],
        ':' => [0, 0x0C, 0x0C, 0, 0x0C, 0x0C, 0],
        ';' => [0, 0x0C, 0x0C, 0, 0x0C, 0x04, 0x08],
        '\'' => [0x0C, 0x04, 0x08, 0, 0, 0, 0],
        '`' => [0x08, 0x04, 0x02, 0, 0, 0, 0],
        '[' => [0x0E, 0x08, 0x08, 0x08, 0x08, 0x08, 0x0E],
        ']' => [0x0E, 0x02, 0x02, 0x02, 0x02, 0x02, 0x0E],
        '(' => [0x02, 0x04, 0x08, 0x08, 0x08, 0x04, 0x02],
        ')' => [0x08, 0x04, 0x02, 0x02, 0x02, 0x04, 0x08],
        '=' => [0, 0, 0x1F, 0, 0x1F, 0, 0],
        '+' => [0, 0x04, 0x04, 0x1F, 0x04, 0x04, 0],
        '!' => [0x04, 0x04, 0x04, 0x04, 0x04, 0, 0x04],
        '%' => [0x18, 0x19, 0x02, 0x04, 0x08, 0x13, 0x03],
        '<' => [0x02, 0x04, 0x08, 0x10, 0x08, 0x04, 0x02],
        '>' => [0x08, 0x04, 0x02, 0x01, 0x02, 0x04, 0x08],
        _ => [0x0E, 0x11, 0x01, 0x02, 0x04, 0, 0x04],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alpha_sum(pm: &Pixmap) -> u64 {
        pm.pixels().iter().map(|p| p.alpha() as u64).sum()
    }

    #[test]
    fn pixel_font_draws_and_measures() {
        let mut f = Font::Pixel;
        assert_eq!(f.width("", 18.0), 0.0);
        // 2 cells per unit at size 18: "AB" = 11 cells
        assert_eq!(f.width("AB", 18.0), 22.0);
        assert_eq!(f.width("ab", 18.0), f.width("AB", 18.0));
        let mut pm = Pixmap::new(64, 32).unwrap();
        f.draw(&mut pm, "1", 32.0, 16.0, 18.0, [255, 255, 255, 255]);
        // "1" has 10 set cells of 2×2 px
        assert_eq!(alpha_sum(&pm), 10 * 4 * 255);
    }

    #[test]
    fn nothing_drawn_for_empty_or_invisible_text() {
        let mut f = Font::Pixel;
        let mut pm = Pixmap::new(16, 16).unwrap();
        f.draw(&mut pm, "", 8.0, 8.0, 18.0, [255; 4]);
        f.draw(&mut pm, "X", 8.0, 8.0, 18.0, [255, 255, 255, 0]);
        f.draw(&mut pm, "X", 8.0, 8.0, 0.5, [255; 4]);
        assert_eq!(alpha_sum(&pm), 0);
        // off the canvas: clipped, no panic
        f.draw(&mut pm, "WWW", -100.0, -100.0, 18.0, [255; 4]);
        assert_eq!(alpha_sum(&pm), 0);
    }

    #[test]
    fn unknown_glyphs_fall_back() {
        assert_eq!(pixel_glyph('€'), pixel_glyph('?'));
        assert_eq!(pixel_glyph('z'), pixel_glyph('Z'));
    }

    #[test]
    fn coverage_blend_is_premultiplied() {
        let mut pm = Pixmap::new(2, 1).unwrap();
        blit_coverage(&mut pm, 0, 0, 2, 1, &[255, 128], [200, 100, 0, 255]);
        let p = pm.pixels();
        assert_eq!((p[0].red(), p[0].green(), p[0].alpha()), (200, 100, 255));
        assert_eq!(p[1].alpha(), 128);
        assert!(p[1].red() <= p[1].alpha());
        // clipped writes
        blit_coverage(&mut pm, 5, 5, 2, 1, &[255, 255], [255; 4]);
        blit_coverage(&mut pm, -2, 0, 2, 1, &[255, 255], [255; 4]);
    }

    #[test]
    fn missing_font_file_is_an_error() {
        assert!(Font::load(Path::new("/nonexistent/font.ttf")).is_err());
    }
}

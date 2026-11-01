//! Key overlay frames: the overlay at one song time drawn into an RGBA pixmap (pure: same scene, style and
//! time give the same pixels). Styled after osu! "KeyOverlay": a keycap per recorded key with its live travel
//! (a fill rising from the bottom that fades in at its top edge), the actuation point as notches on the
//! cap's side edges and its press count; a stats line (KPS, peak KPS, tapping BPM);
//! and a bar per press rising from its key (length = hold time, scrolling up at a constant speed, fading out
//! at the top), wider and brighter the deeper the press went.

use super::font::{Font, Rgba};
use aula_core::overlay::{Data, Press};
use aula_core::report::depth_at;
use tiny_skia::{
    Color, FillRule, GradientStop, LinearGradient, Mask, Paint, Path, PathBuilder, Pixmap, Point,
    Rect, SpreadMode, Stroke, Transform,
};

/// KPS window (real ms).
const KPS_WINDOW_MS: f64 = 1000.0;
/// Presses considered for the tapping BPM: within this window (real ms) ...
const BPM_WINDOW_MS: f64 = 2000.0;
/// ... the last this many ...
const BPM_PRESSES: usize = 9;
/// ... and at least this many.
const BPM_MIN_PRESSES: usize = 4;
/// Stats line colour.
const STATS_COLOUR: Rgba = [240, 240, 245, 235];
/// Key colours, cycled.
const PALETTE: [Rgba; 6] = [
    [255, 102, 170, 255],
    [102, 204, 255, 255],
    [255, 210, 95, 255],
    [140, 255, 110, 255],
    [190, 140, 255, 255],
    [255, 150, 90, 255],
];

/// Canvas and motion settings.
#[derive(Debug, Clone, PartialEq)]
pub struct Style {
    pub width: u32,
    pub height: u32,
    /// Bar scroll speed in px per real second; None = the bar area's height in 1.2 s.
    pub bar_speed: Option<f32>,
    /// Draw the travel in mm above the legend.
    pub show_depth: bool,
}

/// Overlay data with the lookups the frames need (sorted press times, running peak KPS).
pub struct Scene {
    pub data: Data,
    /// Every key's press times (song ms) from the start of the play on, sorted.
    downs: Vec<f64>,
    /// KPS reached at each of `downs`, running maximum.
    peak_kps: Vec<u32>,
    /// Longest hold (song ms), for the bar search.
    max_hold: f64,
}

impl Scene {
    pub fn new(data: Data) -> Result<Scene, String> {
        if !(data.rate.is_finite() && data.rate > 0.0) {
            return Err(format!("bad song rate {}", data.rate));
        }
        if !(data.max_mm.is_finite() && data.max_mm > 0.0) {
            return Err(format!("bad travel range {} mm", data.max_mm));
        }
        let mut downs: Vec<f64> = data
            .keys
            .iter()
            .flat_map(|k| k.presses.iter().map(|p| p.down))
            .filter(|t| t.is_finite() && *t >= data.replay_range.0)
            .collect();
        downs.sort_by(f64::total_cmp);
        let window = KPS_WINDOW_MS * data.rate;
        let mut peak = 0;
        let mut from = 0;
        let peak_kps = downs
            .iter()
            .enumerate()
            .map(|(i, &t)| {
                while downs[from] <= t - window {
                    from += 1;
                }
                peak = peak.max((i + 1 - from) as u32);
                peak
            })
            .collect();
        let max_hold = data
            .keys
            .iter()
            .flat_map(|k| &k.presses)
            .filter_map(|p| p.up.map(|u| u - p.down))
            .filter(|h| h.is_finite())
            .fold(0.0, f64::max);
        Ok(Scene {
            data,
            downs,
            peak_kps,
            max_hold,
        })
    }

    /// Presses in the last second (real time) up to song time `t`.
    pub fn kps(&self, t: f64) -> u32 {
        let hi = self.downs.partition_point(|&d| d <= t);
        let lo = self
            .downs
            .partition_point(|&d| d <= t - KPS_WINDOW_MS * self.data.rate);
        (hi - lo) as u32
    }

    /// Highest KPS reached up to song time `t`.
    pub fn peak_kps(&self, t: f64) -> u32 {
        match self.downs.partition_point(|&d| d <= t) {
            0 => 0,
            i => self.peak_kps[i - 1],
        }
    }

    /// Tapping speed as 1/4-note BPM (15000 / median interval between the recent presses of all keys, real
    /// ms); None when fewer than 4 presses came in the last 2 s.
    pub fn bpm(&self, t: f64) -> Option<f64> {
        let hi = self.downs.partition_point(|&d| d <= t);
        let lo = self
            .downs
            .partition_point(|&d| d <= t - BPM_WINDOW_MS * self.data.rate)
            .max(hi.saturating_sub(BPM_PRESSES));
        if hi - lo < BPM_MIN_PRESSES {
            return None;
        }
        let mut gaps: Vec<f64> = self.downs[lo..hi]
            .windows(2)
            .map(|w| (w[1] - w[0]) / self.data.rate)
            .collect();
        gaps.sort_by(f64::total_cmp);
        let median = gaps[gaps.len() / 2];
        (median > 0.0).then(|| 15_000.0 / median)
    }

    /// Travel (mm) of key `k` at song time `t`.
    pub fn depth(&self, k: usize, t: f64) -> f64 {
        self.data
            .keys
            .get(k)
            .map_or(0.0, |key| depth_at(&key.depth, t))
    }

    /// Presses of key `k` from the start of the play (the first replay frame) up to song time `t`; taps
    /// before it (menus, retries) do not count, like the game's key counter.
    pub fn count(&self, k: usize, t: f64) -> usize {
        let start = self.data.replay_range.0;
        self.data.keys.get(k).map_or(0, |key| {
            let hi = key.presses.partition_point(|p| p.down <= t);
            hi.saturating_sub(key.presses.partition_point(|p| p.down < start))
        })
    }

    /// Whether key `k` is held at song time `t`.
    pub fn held(&self, k: usize, t: f64) -> bool {
        let Some(key) = self.data.keys.get(k) else {
            return false;
        };
        let i = key.presses.partition_point(|p| p.down <= t);
        i > 0 && key.presses[i - 1].up.is_none_or(|u| u > t)
    }

    /// Deepest travel of a press up to song time `t` (the whole press once it is released).
    fn peak_so_far(&self, k: usize, p: &Press, t: f64) -> f64 {
        if p.up.is_some_and(|u| u <= t) {
            return p.peak;
        }
        let d = &self.data.keys[k].depth;
        let lo = d.partition_point(|s| s.0 <= p.down);
        let hi = d.partition_point(|s| s.0 <= t);
        d[lo..hi.max(lo)]
            .iter()
            .map(|s| s.1)
            .fold(depth_at(d, p.down), f64::max)
    }
}

/// Where things go on the canvas.
#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    /// Keycap size (square).
    pub key: f32,
    pub gap: f32,
    /// Left edge of the first keycap.
    pub x0: f32,
    /// Top of the keycaps.
    pub key_top: f32,
    /// Bars live between these (bars start at `bar_base`).
    pub bar_top: f32,
    pub bar_base: f32,
    /// Bars are fully faded at `bar_top` and opaque below `fade_end`.
    pub fade_end: f32,
    /// Middle of the stats line and its text size.
    pub stats_y: f32,
    pub stats_size: f32,
    /// Bar speed, px per real ms.
    pub speed: f32,
}

impl Layout {
    pub fn new(style: &Style, keys: usize) -> Layout {
        let (w, h) = (style.width as f32, style.height as f32);
        let n = keys.max(1) as f32;
        let margin = (w.min(h) * 0.04).max(2.0);
        let gap = margin * 0.6;
        let key = ((w - 2.0 * margin - (n - 1.0) * gap) / n)
            .min(h * 0.22)
            .max(4.0);
        let x0 = (w - (n * key + (n - 1.0) * gap)) / 2.0;
        let stats_size = (key * 0.2).clamp(8.0, 64.0);
        let stats_h = stats_size * 1.8;
        let key_top = h - margin - stats_h - key;
        let bar_base = key_top - gap;
        let bar_top = margin.min(bar_base);
        let fade_end = bar_top + (bar_base - bar_top) * 0.35;
        let speed = style
            .bar_speed
            .map_or((bar_base - bar_top) / 1200.0, |s| s / 1000.0);
        Layout {
            key,
            gap,
            x0,
            key_top,
            bar_top,
            bar_base,
            fade_end,
            stats_y: h - margin - stats_h / 2.0,
            stats_size,
            speed,
        }
    }

    /// Left edge of keycap `i`.
    pub fn key_x(&self, i: usize) -> f32 {
        self.x0 + i as f32 * (self.key + self.gap)
    }
}

fn paint(c: Rgba) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color_rgba8(c[0], c[1], c[2], c[3]);
    p.anti_alias = true;
    p
}

fn with_alpha(c: Rgba, a: f64) -> Rgba {
    [
        c[0],
        c[1],
        c[2],
        (c[3] as f64 * a.clamp(0.0, 1.0)).round() as u8,
    ]
}

/// `c` moved towards white by `f` (0..1).
fn lighten(c: Rgba, f: f64) -> Rgba {
    let f = f.clamp(0.0, 1.0);
    let l = |v: u8| (v as f64 + (255.0 - v as f64) * f).round() as u8;
    [l(c[0]), l(c[1]), l(c[2]), c[3]]
}

fn rounded_rect(x: f32, y: f32, w: f32, h: f32, r: f32) -> Option<Path> {
    let r = r.min(w / 2.0).min(h / 2.0).max(0.0);
    let mut pb = PathBuilder::new();
    pb.move_to(x + r, y);
    pb.line_to(x + w - r, y);
    pb.quad_to(x + w, y, x + w, y + r);
    pb.line_to(x + w, y + h - r);
    pb.quad_to(x + w, y + h, x + w - r, y + h);
    pb.line_to(x + r, y + h);
    pb.quad_to(x, y + h, x, y + h - r);
    pb.line_to(x, y + r);
    pb.quad_to(x, y, x + r, y);
    pb.close();
    pb.finish()
}

fn colour_of(i: usize) -> Rgba {
    PALETTE[i % PALETTE.len()]
}

/// Draws the overlay at song time `t` into `pm` (cleared first). `pm` must be `style.width` × `style.height`.
pub fn render(scene: &Scene, style: &Style, font: &mut Font, t: f64, pm: &mut Pixmap) {
    pm.fill(Color::TRANSPARENT);
    let l = Layout::new(style, scene.data.keys.len());
    for k in 0..scene.data.keys.len() {
        draw_bars(scene, &l, k, t, pm);
    }
    for k in 0..scene.data.keys.len() {
        draw_key(scene, &l, style, font, k, t, pm);
    }
    // "BPM -" (not enough presses yet) is drawn dimmer than the numbers
    let bpm = scene.bpm(t);
    let lead = format!("KPS {}   MAX {}   ", scene.kps(t), scene.peak_kps(t));
    let tail = bpm.map_or_else(|| "BPM -".to_string(), |b| format!("BPM {b:.0}"));
    let mut size = l.stats_size;
    let room = style.width as f32 * 0.96;
    let (lw, tw) = (font.width(&lead, size), font.width(&tail, size));
    if lw + tw > room {
        size *= room / (lw + tw);
    }
    let (lw, tw) = (font.width(&lead, size), font.width(&tail, size));
    let left = (style.width as f32 - lw - tw) / 2.0;
    font.draw(pm, &lead, left + lw / 2.0, l.stats_y, size, STATS_COLOUR);
    let tail_colour = if bpm.is_some() {
        STATS_COLOUR
    } else {
        with_alpha(STATS_COLOUR, 0.45)
    };
    font.draw(
        pm,
        &tail,
        left + lw + tw / 2.0,
        l.stats_y,
        size,
        tail_colour,
    );
}

fn draw_bars(scene: &Scene, l: &Layout, k: usize, t: f64, pm: &mut Pixmap) {
    let key = &scene.data.keys[k];
    let rate = scene.data.rate;
    let colour = colour_of(k);
    let cx = l.key_x(k) + l.key / 2.0;
    // a bar has scrolled away when its bottom passed the top
    let travel_ms = ((l.bar_base - l.bar_top) / l.speed.max(1e-6)) as f64 * rate;
    let first = key
        .presses
        .partition_point(|p| p.down < t - travel_ms - scene.max_hold);
    let last = key.presses.partition_point(|p| p.down <= t);
    for p in &key.presses[first..last] {
        let y_top = l.bar_base - ((t - p.down) / rate) as f32 * l.speed;
        let y_bottom = match p.up {
            Some(u) if u <= t => l.bar_base - ((t - u) / rate) as f32 * l.speed,
            _ => l.bar_base,
        };
        if y_bottom <= l.bar_top {
            continue;
        }
        let y_top = y_top.max(l.bar_top);
        let height = (y_bottom - y_top).max(2.0);
        let f = (scene.peak_so_far(k, p, t) / scene.data.max_mm).clamp(0.0, 1.0);
        let width = l.key * (0.3 + 0.55 * f as f32);
        let c = with_alpha(lighten(colour, f * 0.35), 0.45 + 0.55 * f);
        let Some(rect) = Rect::from_xywh(cx - width / 2.0, y_bottom - height, width, height) else {
            continue;
        };
        let mut paint = paint(c);
        if y_top < l.fade_end {
            let clear = Color::from_rgba8(c[0], c[1], c[2], 0);
            let solid = Color::from_rgba8(c[0], c[1], c[2], c[3]);
            if let Some(shader) = LinearGradient::new(
                Point::from_xy(0.0, l.bar_top),
                Point::from_xy(0.0, l.fade_end),
                vec![GradientStop::new(0.0, clear), GradientStop::new(1.0, solid)],
                SpreadMode::Pad,
                Transform::identity(),
            ) {
                paint.shader = shader;
            }
        }
        if let Some(path) = rounded_rect(
            rect.x(),
            rect.y(),
            rect.width(),
            rect.height(),
            width * 0.12,
        ) {
            pm.fill_path(
                &path,
                &paint,
                FillRule::Winding,
                Transform::identity(),
                None,
            );
        }
    }
}

/// Cap colours: the body, idle and pressed (brighter, tinted towards the key's colour).
const CAP: Rgba = [24, 24, 32, 200];
/// Legend tones; the one that contrasts more with what is behind the legend is used.
const LEGEND_LIGHT: Rgba = [245, 245, 250, 255];
const LEGEND_DARK: Rgba = [18, 18, 26, 255];
/// Travel fill opacity at its top edge and at the bottom of the cap; pressed caps are this much stronger.
const FILL_ALPHA_TOP: f64 = 0.05;
const FILL_ALPHA_BOTTOM: f64 = 0.7;
const PRESSED_BOOST: f64 = 1.3;

/// Fill opacity at `position` (0 = the fill's top edge, 1 = the bottom of the cap).
pub fn fill_alpha(position: f64, pressed: bool) -> f64 {
    let a = FILL_ALPHA_TOP + (FILL_ALPHA_BOTTOM - FILL_ALPHA_TOP) * position.clamp(0.0, 1.0);
    (if pressed { a * PRESSED_BOOST } else { a }).min(1.0)
}

/// `a` over `b` by `alpha` (colour channels only).
fn mix(b: Rgba, a: Rgba, alpha: f64) -> Rgba {
    let m = |x: u8, y: u8| (x as f64 + (y as f64 - x as f64) * alpha.clamp(0.0, 1.0)).round() as u8;
    [m(b[0], a[0]), m(b[1], a[1]), m(b[2], a[2]), b[3]]
}

/// WCAG relative luminance.
fn luminance(c: Rgba) -> f64 {
    let lin = |v: u8| {
        let v = v as f64 / 255.0;
        if v <= 0.03928 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * lin(c[0]) + 0.7152 * lin(c[1]) + 0.0722 * lin(c[2])
}

fn contrast(a: Rgba, b: Rgba) -> f64 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

/// The cap colour: brighter and tinted towards `accent` while pressed.
fn cap_colour(accent: Rgba, pressed: bool) -> Rgba {
    if pressed {
        lighten(mix(CAP, accent, 0.18), 0.06)
    } else {
        CAP
    }
}

/// Legend colour over a cap filled to `fraction` (the fill reaches the legend at the centre).
pub fn legend_colour(accent: Rgba, fraction: f64, pressed: bool) -> Rgba {
    let cap = cap_colour(accent, pressed);
    let backdrop = if fraction < 0.5 {
        cap
    } else {
        // where the cap's centre sits in the fill's gradient
        mix(cap, accent, fill_alpha(1.0 - 0.5 / fraction, pressed))
    };
    if contrast(backdrop, LEGEND_DARK) > contrast(backdrop, LEGEND_LIGHT) {
        LEGEND_DARK
    } else {
        LEGEND_LIGHT
    }
}

fn draw_key(
    scene: &Scene,
    l: &Layout,
    style: &Style,
    font: &mut Font,
    k: usize,
    t: f64,
    pm: &mut Pixmap,
) {
    let key = &scene.data.keys[k];
    let max_mm = scene.data.max_mm;
    let accent = colour_of(k);
    let (x, y, s) = (l.key_x(k), l.key_top, l.key);
    let held = scene.held(k, t);
    let depth = scene.depth(k, t).clamp(0.0, max_mm);
    let fraction = depth / max_mm;
    let radius = s * 0.14;
    let border = (s * if held { 0.045 } else { 0.025 }).max(1.0);
    let Some(body) = rounded_rect(x, y, s, s, radius) else {
        return;
    };
    pm.fill_path(
        &body,
        &paint(cap_colour(accent, held)),
        FillRule::Winding,
        Transform::identity(),
        None,
    );
    // travel: a cap-shaped gradient revealed from the bottom, fading in at its top edge (no edge line)
    let (iy, ih) = (y + border, s - 2.0 * border);
    let fill_h = ih * fraction as f32;
    if fill_h > 0.5 {
        let top = y + border + ih - fill_h;
        let shape = rounded_rect(
            x + border,
            iy,
            s - 2.0 * border,
            ih,
            (radius - border).max(0.0),
        );
        let reveal = Rect::from_xywh(x, top, s, y + s - top).map(PathBuilder::from_rect);
        let mask = Mask::new(pm.width(), pm.height())
            .zip(reveal)
            .map(|(mut m, r)| {
                m.fill_path(&r, FillRule::Winding, false, Transform::identity());
                m
            });
        let alpha = |p: f64| (fill_alpha(p, held) * 255.0).round() as u8;
        let shader = LinearGradient::new(
            Point::from_xy(0.0, top),
            Point::from_xy(0.0, y + s - border),
            vec![
                GradientStop::new(
                    0.0,
                    Color::from_rgba8(accent[0], accent[1], accent[2], alpha(0.0)),
                ),
                GradientStop::new(
                    1.0,
                    Color::from_rgba8(accent[0], accent[1], accent[2], alpha(1.0)),
                ),
            ],
            SpreadMode::Pad,
            Transform::identity(),
        );
        if let (Some(shape), Some(mask), Some(shader)) = (shape, mask, shader) {
            let p = Paint {
                shader,
                anti_alias: true,
                ..Paint::default()
            };
            pm.fill_path(
                &shape,
                &p,
                FillRule::Winding,
                Transform::identity(),
                Some(&mask),
            );
        }
    }
    // actuation point: short low-contrast notches on both side edges, never across the legend
    let ay = iy + ih * (1.0 - (key.point / max_mm).clamp(0.0, 1.0) as f32);
    let (nw, nh) = ((s * 0.07).max(3.0), (s * 0.022).max(1.0));
    for nx in [x + border, x + s - border - nw] {
        if let Some(r) = rounded_rect(nx, ay - nh / 2.0, nw, nh, nh / 2.0) {
            pm.fill_path(
                &r,
                &paint([200, 200, 210, 120]),
                FillRule::Winding,
                Transform::identity(),
                None,
            );
        }
    }
    let stroke = Stroke {
        width: border,
        ..Stroke::default()
    };
    let edge = if held { accent } else { [255, 255, 255, 70] };
    if let Some(edge_path) = rounded_rect(
        x + border / 2.0,
        y + border / 2.0,
        s - border,
        s - border,
        radius,
    ) {
        pm.stroke_path(
            &edge_path,
            &paint(edge),
            &stroke,
            Transform::identity(),
            None,
        );
    }
    let legend = legend_colour(accent, fraction, held);
    let label = if key.label.is_empty() {
        &key.code
    } else {
        &key.label
    };
    let mut size = s * 0.34;
    let w = font.width(label, size);
    if w > s * 0.8 {
        size *= s * 0.8 / w;
    }
    font.draw(pm, label, x + s / 2.0, y + s * 0.45, size, legend);
    let count_colour = with_alpha(legend, 0.8);
    font.draw(
        pm,
        &scene.count(k, t).to_string(),
        x + s / 2.0,
        y + s * 0.78,
        s * 0.16,
        count_colour,
    );
    if style.show_depth {
        font.draw(
            pm,
            &format!("{depth:.2}"),
            x + s / 2.0,
            y + s * 0.2,
            s * 0.12,
            with_alpha(legend, 0.65),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aula_core::overlay::{Key, SCHEMA};

    fn press(down: f64, up: f64, peak: f64) -> Press {
        Press {
            down,
            up: Some(up),
            peak,
        }
    }

    fn key(code: &str, presses: Vec<Press>) -> Key {
        let mut depth = Vec::new();
        for p in &presses {
            depth.push((p.down, 1.2));
            depth.push((p.down + 5.0, p.peak));
            depth.push((p.up.unwrap(), 0.0));
        }
        Key {
            code: code.into(),
            label: code[3..].into(),
            input: "K1/M1".into(),
            mode: "rt".into(),
            point: 1.0,
            presses,
            depth,
        }
    }

    fn data(keys: Vec<Key>, rate: f64) -> Data {
        Data {
            schema: SCHEMA,
            session_file: "s".into(),
            replay_file: "r".into(),
            rate,
            offset_ms: 0.0,
            matched: 0,
            replay_downs: 0,
            match_rate: 1.0,
            residual_median_ms: 0.0,
            max_mm: 3.4,
            replay_range: (0.0, 10_000.0),
            keys,
        }
    }

    /// Z and X alternate from 1000 ms, one press every 125 ms song time (Z 3.4 mm deep, X 1.7 mm).
    fn stream(rate: f64) -> Scene {
        let z = (0..10)
            .map(|i| press(1000.0 + i as f64 * 250.0, 1040.0 + i as f64 * 250.0, 3.4))
            .collect();
        let x = (0..10)
            .map(|i| press(1125.0 + i as f64 * 250.0, 1160.0 + i as f64 * 250.0, 1.7))
            .collect();
        Scene::new(data(vec![key("KeyZ", z), key("KeyX", x)], rate)).unwrap()
    }

    fn style() -> Style {
        Style {
            width: 200,
            height: 300,
            bar_speed: None,
            show_depth: false,
        }
    }

    fn render_at(scene: &Scene, t: f64) -> Pixmap {
        let st = style();
        let mut pm = Pixmap::new(st.width, st.height).unwrap();
        render(scene, &st, &mut Font::Pixel, t, &mut pm);
        pm
    }

    fn alpha_in(pm: &Pixmap, x0: u32, y0: u32, x1: u32, y1: u32) -> u64 {
        let mut s = 0;
        for y in y0..y1 {
            for x in x0..x1 {
                s += pm.pixel(x, y).unwrap().alpha() as u64;
            }
        }
        s
    }

    #[test]
    fn stats_follow_the_presses() {
        let s = stream(1.0);
        assert_eq!(s.kps(0.0), 0);
        assert_eq!(s.bpm(0.0), None);
        // presses every 125 ms: 8 in a second, 1/4 BPM = 15000 / 125 = 120
        assert_eq!(s.kps(2400.0), 8);
        assert_eq!(s.bpm(2400.0).map(f64::round), Some(120.0));
        assert_eq!(s.peak_kps(2400.0), 8);
        // after the stream: KPS drops, the peak stays, BPM gone after 2 s
        assert_eq!(s.kps(10_000.0), 0);
        assert_eq!(s.peak_kps(10_000.0), 8);
        assert_eq!(s.bpm(10_000.0), None);
        assert_eq!(s.count(0, 1000.0), 1);
        assert_eq!(s.count(0, 999.9), 0);
        assert_eq!(s.count(1, 100_000.0), 10);
        assert!(s.held(0, 1020.0) && !s.held(0, 1040.0) && !s.held(0, 999.0));
        assert_eq!(s.depth(0, 1010.0), 3.4);
        assert_eq!(s.depth(0, 1100.0), 0.0);
        assert_eq!(s.depth(7, 1100.0), 0.0, "no such key");
    }

    /// One idle Z cap (point 1.0 of 3.4 mm) at rest and a canvas just for it.
    fn idle_cap() -> (Pixmap, Layout) {
        let s = Scene::new(data(
            vec![key("KeyZ", vec![press(5000.0, 5100.0, 3.0)])],
            1.0,
        ))
        .unwrap();
        let pm = render_at(&s, 0.0);
        (pm, Layout::new(&style(), 1))
    }

    #[test]
    fn actuation_point_is_edge_notches_not_a_line() {
        let (pm, l) = idle_cap();
        let (x, s) = (l.key_x(0), l.key);
        let border = (s * 0.025).max(1.0);
        let ay = (l.key_top + border + (s - 2.0 * border) * (1.0 - 1.0 / 3.4) as f32) as u32;
        let px = |x: f32| pm.pixel(x as u32, ay).unwrap().demultiply();
        // a notch at both side edges ...
        assert!(
            px(x + border + 1.0).red() > 80,
            "{:?}",
            px(x + border + 1.0)
        );
        assert!(px(x + s - border - 2.0).red() > 80);
        // ... and only the dark cap between them (a quarter in, away from the legend)
        let mid = px(x + s * 0.25);
        assert!(mid.red() < 60 && mid.green() < 60, "{mid:?}");
    }

    #[test]
    fn fill_rises_from_the_bottom_and_fades_in() {
        // released at 1200 but resting at half travel: the fill shows travel, not the press
        let mut z = key("KeyZ", vec![press(1000.0, 1200.0, 1.7)]);
        z.depth = vec![(1000.0, 1.7)];
        let s = Scene::new(data(vec![z], 1.0)).unwrap();
        let l = Layout::new(&style(), 1);
        let pm = render_at(&s, 1500.0);
        let cx = (l.key_x(0) + l.key * 0.2) as u32;
        let at = |f: f32| {
            pm.pixel(cx, (l.key_top + l.key * f) as u32)
                .unwrap()
                .demultiply()
        };
        // half travel: the bottom is tinted pink, the top half only the cap
        let (bottom, top) = (at(0.92), at(0.2));
        assert!(bottom.red() > bottom.green() + 40, "{bottom:?}");
        assert!(top.red() < 60, "{top:?}");
        // the fill gets stronger towards the bottom (no hard line at its top edge)
        let (upper, lower) = (at(0.56), at(0.85));
        assert!(lower.red() > upper.red(), "{upper:?} {lower:?}");
    }

    #[test]
    fn pressed_caps_get_a_thicker_accent_border() {
        let s = stream(1.0);
        let l = Layout::new(&style(), 2);
        let pm = render_at(&s, 1020.0); // Z held, X idle
        let y = (l.key_top + l.key / 2.0) as u32;
        let edge = |k: usize| {
            (0..6)
                .filter(|d| {
                    let p = pm.pixel(l.key_x(k) as u32 + d, y).unwrap().demultiply();
                    p.alpha() > 0 && p.red() > 200 && p.green() < 150
                })
                .count()
        };
        assert!(edge(0) >= 2, "{}", edge(0));
        assert_eq!(edge(1), 0);
    }

    #[test]
    fn legend_picks_the_readable_tone() {
        let (pink, white) = (PALETTE[0], [255, 255, 255, 255]);
        // empty cap: light legend
        assert_eq!(legend_colour(pink, 0.0, false), LEGEND_LIGHT);
        assert_eq!(
            legend_colour(white, 0.4, true),
            LEGEND_LIGHT,
            "fill below the legend"
        );
        // a bright fill behind the legend: dark reads better
        assert_eq!(legend_colour(white, 1.0, true), LEGEND_DARK);
        assert!(fill_alpha(1.0, true) > fill_alpha(1.0, false));
        assert!(fill_alpha(0.0, false) < fill_alpha(1.0, false));
        assert!(fill_alpha(5.0, true) <= 1.0);
        assert!(contrast([0, 0, 0, 255], [255, 255, 255, 255]) > 20.0);
    }

    #[test]
    fn depth_number_only_on_request() {
        let mut z = key("KeyZ", vec![press(1000.0, 2000.0, 2.0)]);
        z.depth = vec![(1000.0, 2.0)];
        let s = Scene::new(data(vec![z], 1.0)).unwrap();
        let l = Layout::new(&style(), 1);
        let band = |pm: &Pixmap| {
            let (x, y) = (l.key_x(0) + l.key * 0.3, l.key_top + l.key * 0.15);
            alpha_in(
                pm,
                x as u32,
                y as u32,
                (x + l.key * 0.4) as u32,
                (y + l.key * 0.1) as u32,
            )
        };
        let plain = render_at(&s, 1500.0);
        let mut st = style();
        st.show_depth = true;
        let mut with = Pixmap::new(st.width, st.height).unwrap();
        render(&s, &st, &mut Font::Pixel, 1500.0, &mut with);
        assert_ne!(band(&plain), band(&with));
    }

    #[test]
    fn unknown_bpm_is_dimmer() {
        let s = stream(1.0);
        let l = Layout::new(&style(), 2);
        let brightest = |pm: &Pixmap| {
            (160..195u32)
                .flat_map(|x| (0..6).map(move |dy| (x, dy)))
                .map(|(x, dy)| pm.pixel(x, l.stats_y as u32 - 3 + dy).unwrap().alpha())
                .max()
                .unwrap()
        };
        // the right end of the line: "BPM -" before the stream, "BPM 120" during it
        let (idle, busy) = (render_at(&s, 0.0), render_at(&s, 2400.0));
        assert!(
            brightest(&idle) < brightest(&busy),
            "{} {}",
            brightest(&idle),
            brightest(&busy)
        );
    }

    #[test]
    fn taps_before_the_play_are_not_counted() {
        // the play starts at song 0 (first replay frame); taps in the menu before it only draw
        let early: Vec<Press> = (0..6)
            .map(|i| press(-900.0 + i as f64 * 100.0, -880.0 + i as f64 * 100.0, 2.0))
            .collect();
        let mut z = early.clone();
        z.push(press(500.0, 540.0, 3.0));
        let s = Scene::new(data(vec![key("KeyZ", z)], 1.0)).unwrap();
        assert_eq!(s.count(0, -100.0), 0);
        assert_eq!(s.count(0, 1000.0), 1);
        assert_eq!(
            (s.kps(100.0), s.peak_kps(1000.0), s.bpm(100.0)),
            (0, 1, None)
        );
    }

    #[test]
    fn rate_mods_count_real_time() {
        // DT: song ms run 1.5x, so a 1 s real window holds 1.5x the song presses
        let s = stream(1.5);
        assert_eq!(s.kps(2400.0), 12);
        assert_eq!(s.bpm(2400.0).map(f64::round), Some(180.0));
    }

    #[test]
    fn frame_shows_keys_bars_and_stats() {
        let s = stream(1.0);
        let l = Layout::new(&style(), 2);
        let pm = render_at(&s, 1020.0);
        let ky = l.key_top as u32;
        let ks = l.key as u32;
        let (zx, xx) = (l.key_x(0) as u32, l.key_x(1) as u32);
        // both keycaps are drawn
        assert!(alpha_in(&pm, zx, ky, zx + ks, ky + ks) > 0);
        assert!(alpha_in(&pm, xx, ky, xx + ks, ky + ks) > 0);
        // Z is held and pressed to the bottom: its keycap shows the fill in its colour
        let mid = pm
            .pixel(zx + ks / 2, ky + ks - (l.key * 0.12) as u32)
            .unwrap()
            .demultiply();
        assert!(
            mid.red() > 200 && mid.green() < 130 && mid.blue() > 120,
            "{mid:?}"
        );
        // a bar rises from Z (20 ms of hold), nothing above X yet
        let base = l.bar_base as u32;
        assert!(alpha_in(&pm, zx, base - 6, zx + ks, base) > 0);
        assert_eq!(alpha_in(&pm, xx, l.bar_top as u32, xx + ks, base), 0);
        // stats line
        assert!(alpha_in(&pm, 0, ky + ks, 200, 300) > 0);
    }

    #[test]
    fn bars_scroll_up_and_fade_out() {
        let s = stream(1.0);
        let l = Layout::new(&style(), 2);
        let (zx, ks) = (l.key_x(0) as u32, l.key as u32);
        let column =
            |pm: &Pixmap, y0: f32, y1: f32| alpha_in(pm, zx, y0 as u32, zx + ks, y1 as u32);
        // 0.6 s after the last Z press (3250) its bar sits in the middle of the area
        let pm = render_at(&s, 3850.0);
        let mid = (l.bar_top + l.bar_base) / 2.0;
        assert!(column(&pm, mid - 25.0, mid + 25.0) > 0);
        // at the very top bars are (almost) transparent
        let pm = render_at(&s, 4470.0);
        let top = pm.pixel(zx + ks / 2, l.bar_top as u32 + 1).unwrap().alpha();
        assert!(top < 30, "{top}");
        // long after: no bars at all
        let pm = render_at(&s, 20_000.0);
        assert_eq!(column(&pm, l.bar_top, l.bar_base), 0);
    }

    #[test]
    fn deeper_presses_draw_wider_bars() {
        let s = stream(1.0);
        let l = Layout::new(&style(), 2);
        let pm = render_at(&s, 2300.0);
        let width = |k: usize| {
            let y = l.bar_base as u32 - 3;
            (l.key_x(k) as u32..(l.key_x(k) + l.key) as u32)
                .filter(|&x| pm.pixel(x, y).unwrap().alpha() > 0)
                .count()
        };
        // Z (3.4 mm, pressed at 2250) is held at the base: a full-depth bar
        assert!(width(0) as f32 >= l.key * 0.8, "{}", width(0));
        // X (1.7 mm so far) 15 ms after its press: a thin bar at the base
        let pm = render_at(&s, 2140.0);
        let y = (l.bar_base - 1.0) as u32;
        let xw = (l.key_x(1) as u32..(l.key_x(1) + l.key) as u32)
            .filter(|&x| pm.pixel(x, y).unwrap().alpha() > 0)
            .count();
        assert!(xw > 0 && (xw as f32) < l.key * 0.7, "{xw}");
    }

    #[test]
    fn empty_scene_draws_only_stats() {
        let s = Scene::new(data(Vec::new(), 1.0)).unwrap();
        assert_eq!((s.kps(5.0), s.peak_kps(5.0), s.bpm(5.0)), (0, 0, None));
        let pm = render_at(&s, 0.0);
        let l = Layout::new(&style(), 0);
        assert_eq!(alpha_in(&pm, 0, 0, 200, l.key_top as u32), 0);
        assert!(alpha_in(&pm, 0, 0, 200, 300) > 0);
    }

    #[test]
    fn same_time_same_pixels() {
        let s = stream(1.0);
        let a = render_at(&s, 2222.0);
        let b = render_at(&s, 2222.0);
        assert_eq!(a.data(), b.data());
        // and the frame is cleared between calls
        let st = style();
        let mut pm = Pixmap::new(st.width, st.height).unwrap();
        render(&s, &st, &mut Font::Pixel, 1020.0, &mut pm);
        render(&s, &st, &mut Font::Pixel, 2222.0, &mut pm);
        assert_eq!(pm.data(), a.data());
    }

    #[test]
    fn bad_scenes_are_refused() {
        assert!(Scene::new(data(Vec::new(), 0.0)).is_err());
        assert!(Scene::new(data(Vec::new(), f64::NAN)).is_err());
        let mut d = data(Vec::new(), 1.0);
        d.max_mm = 0.0;
        assert!(Scene::new(d).is_err());
    }

    #[test]
    fn tiny_canvas_and_held_to_the_end() {
        let mut z = key("KeyZ", vec![press(100.0, 200.0, 2.0)]);
        z.presses[0].up = None;
        let s = Scene::new(data(vec![z], 1.0)).unwrap();
        assert!(s.held(0, 1e9));
        let st = Style {
            width: 8,
            height: 8,
            bar_speed: Some(100.0),
            show_depth: true,
        };
        let mut pm = Pixmap::new(8, 8).unwrap();
        render(&s, &st, &mut Font::Pixel, 150.0, &mut pm);
    }

    #[test]
    fn colour_helpers() {
        assert_eq!(lighten([0, 100, 255, 7], 1.0), [255, 255, 255, 7]);
        assert_eq!(lighten([10, 20, 30, 40], 0.0), [10, 20, 30, 40]);
        assert_eq!(with_alpha([1, 2, 3, 200], 0.5), [1, 2, 3, 100]);
        assert_eq!(with_alpha([1, 2, 3, 200], 9.0)[3], 200);
        assert_eq!(colour_of(6), colour_of(0));
    }
}

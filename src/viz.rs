use ratatui::prelude::*;
use std::f64::consts::PI;
use std::sync::OnceLock;

/// Only the ribbons are shown. `plasma` and `spectrum` are kept (parked) so they can be
/// brought back as a setting; `render()` still dispatches to them.
#[allow(dead_code)]
pub const MODES: [&str; 3] = ["ribbons", "plasma", "spectrum"];

pub fn is_truecolor() -> bool {
    static T: OnceLock<bool> = OnceLock::new();
    *T.get_or_init(|| {
        std::env::var("ROLA_TRUECOLOR").is_ok() || matches!(std::env::var("COLORTERM").as_deref(), Ok("truecolor") | Ok("24bit"))
    })
}

/// Truecolor where supported (iTerm2, Ghostty, kitty, WezTerm...), nearest xterm-256 otherwise
/// (Apple's Terminal.app has no truecolor and turns RGB gradients into mush).
pub fn rgb(r: u8, g: u8, b: u8) -> Color {
    if is_truecolor() {
        Color::Rgb(r, g, b)
    } else {
        Color::Indexed(to256(r, g, b))
    }
}

fn to256(r: u8, g: u8, b: u8) -> u8 {
    const LV: [i32; 6] = [0, 95, 135, 175, 215, 255];
    let near = |v: u8| (0..6).min_by_key(|&i| (LV[i] - v as i32).abs()).unwrap();
    let (ri, gi, bi) = (near(r), near(g), near(b));
    let cube = (LV[ri], LV[gi], LV[bi]);
    let avg = (r as i32 + g as i32 + b as i32) / 3;
    let step = (((avg - 8) as f64 / 10.0).round().clamp(0.0, 23.0)) as i32;
    let gv = 8 + 10 * step;
    let d = |c: (i32, i32, i32)| (c.0 - r as i32).pow(2) + (c.1 - g as i32).pow(2) + (c.2 - b as i32).pow(2);
    if d((gv, gv, gv)) < d(cube) {
        (232 + step) as u8
    } else {
        (16 + 36 * ri + 6 * gi + bi) as u8
    }
}

pub fn seed_of(s: &str) -> u64 {
    let mut x: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        x ^= b as u64;
        x = x.wrapping_mul(0x100000001b3);
    }
    x
}

pub fn hsl(h: f64, s: f64, l: f64) -> Color {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let hp = h.rem_euclid(360.0) / 60.0;
    let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
    let (r, g, b) = match hp as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    rgb(((r + m) * 255.0) as u8, ((g + m) * 255.0) as u8, ((b + m) * 255.0) as u8)
}

pub fn base_hue(seed: u64) -> f64 {
    (seed % 360) as f64
}

pub struct Ctx {
    pub w: usize,
    pub h: usize,
    pub seed: u64,
    pub t: f64,
    pub progress: f64,
    pub energy: f64,
    /// Ribbon gradient runs from hue_a (left) to hue_b (right), in degrees.
    pub hue_a: f64,
    pub hue_b: f64,
}

impl Ctx {
    /// Per-track constants in 0..1, all derived from the track id.
    fn f(&self, n: u32) -> f64 {
        ((self.seed >> (n * 8)) & 0xff) as f64 / 255.0
    }
}

pub fn render(mode: usize, c: &Ctx) -> Vec<Line<'static>> {
    if c.w == 0 || c.h == 0 {
        return vec![]; // terminal too small to draw into (and the math below assumes >= 1 cell)
    }
    match mode % MODES.len() {
        0 => ribbons(c),
        1 => plasma(c),
        _ => spectrum(c),
    }
}

/// Joy-Division-ish layered sine ribbons drawn on a braille sub-pixel canvas (2x4 dots per cell),
/// each with a fading echo trail. Harmonics, speeds and hue come from the track.
fn ribbons(c: &Ctx) -> Vec<Line<'static>> {
    let (w, h) = (c.w, c.h);
    let (dw, dh) = (w * 2, h * 4);
    let mut mask = vec![0u8; w * h];
    let mut col: Vec<Option<Color>> = vec![None; w * h];
    const BIT: [[u8; 4]; 2] = [[0, 1, 2, 6], [3, 4, 5, 7]];
    let (k1, k2, k3) = (1.0 + c.f(0) * 2.0, 2.0 + c.f(1) * 3.0, 4.0 + c.f(2) * 4.0);
    let (s1, s2, s3) = (0.7 + c.f(3), 0.5 + c.f(4) * 1.2, 1.2 + c.f(5) * 1.5);
    let (p1, p2, p3) = (c.f(6) * 6.28, c.f(7) * 6.28, c.f(0) * 3.0 + c.progress * 8.0);

    for ribbon in 0..2 {
        let ph = ribbon as f64 * 1.9;
        for e in (0..5).rev() {
            let te = c.t - e as f64 * 0.075;
            let light = 0.74 - e as f64 * 0.095;
            let mut prev: Option<i32> = None;
            for dx in 0..dw {
                let x = dx as f64 / dw as f64;
                let bell = (PI * x).sin();
                let v = (x * k1 * 2.0 * PI + te * s1 + p1 + ph).sin() * 0.5
                    + (x * k2 * 2.0 * PI - te * s2 + p2).sin() * 0.3
                    + (x * k3 * 2.0 * PI + te * s3 + p3 - ph).sin() * 0.2;
                let amp = c.energy * (0.2 + 0.8 * bell) * 0.46 + 0.02;
                let y = ((dh as f64 / 2.0 + v * amp * dh as f64).round() as i32).clamp(0, dh as i32 - 1);
                let (y0, y1) = prev.map_or((y, y), |p| (p.min(y), p.max(y)));
                // gradient between the two palette hues; second ribbon drifts a little off so they separate
                let hue = c.hue_a + (c.hue_b - c.hue_a) * x + ribbon as f64 * 25.0 + e as f64 * 4.0;
                let color = hsl(hue, 0.72, light);
                for yy in y0..=y1 {
                    let (cx, cy) = (dx / 2, yy as usize / 4);
                    let i = cy * w + cx;
                    mask[i] |= 1 << BIT[dx % 2][yy as usize % 4];
                    col[i] = Some(color);
                }
                prev = Some(y);
            }
        }
    }
    (0..h)
        .map(|r| {
            Line::from(
                (0..w)
                    .map(|x| {
                        let i = r * w + x;
                        match col[i] {
                            Some(cl) => Span::styled(char::from_u32(0x2800 + mask[i] as u32).unwrap().to_string(), Style::new().fg(cl)),
                            None => Span::raw(" "),
                        }
                    })
                    .collect::<Vec<_>>(),
            )
        })
        .collect()
}

/// Classic sine-sum plasma on half-block pixels (each cell = 2 vertical pixels via ▀ fg/bg).
fn plasma(c: &Ctx) -> Vec<Line<'static>> {
    let hue = base_hue(c.seed);
    let (a, b, d) = (0.10 + c.f(0) * 0.14, 0.18 + c.f(1) * 0.2, 0.05 + c.f(2) * 0.08);
    let (s1, s2, s3, s4) = (0.8 + c.f(3), 1.1 + c.f(4), 0.6 + c.f(5), 0.9 + c.f(6));
    let (cx, cy) = (c.w as f64 / 2.0, c.h as f64);
    let px = |x: usize, y: usize| -> Color {
        let (xf, yf) = (x as f64, y as f64);
        let ox = cx + (c.t * 0.35).sin() * c.w as f64 * 0.3;
        let v = (xf * a + c.t * s1).sin()
            + (yf * b * 2.0 - c.t * s2).sin()
            + ((xf + yf) * (a + b) * 0.5 + c.t * s3).sin()
            + (((xf - ox).powi(2) + ((yf - cy) * 2.0).powi(2)).sqrt() * d * 2.0 - c.t * s4).sin();
        let u = 0.5 + 0.125 * v * 2.0; // 0..1
        hsl(hue + u * 100.0, 0.75, (0.08 + 0.46 * u.clamp(0.0, 1.0)) * (0.3 + 0.7 * c.energy))
    };
    (0..c.h)
        .map(|r| Line::from((0..c.w).map(|x| Span::styled("▀", Style::new().fg(px(x, r * 2)).bg(px(x, r * 2 + 1)))).collect::<Vec<_>>()))
        .collect()
}

/// The plain bar spectrum.
fn spectrum(c: &Ctx) -> Vec<Line<'static>> {
    let (a, b, cc) = (0.10 + c.f(0) * 0.25, 0.30 + c.f(1) * 0.35, 0.7 + c.f(2) * 0.6);
    let (p1, p2, p3, p4) = (c.f(3) * 6.28, c.f(4) * 6.28, c.f(5) * 6.28, c.f(6) * 6.28);
    let speed = 0.6 + c.f(7) * 0.9;
    let hue = base_hue(c.seed);
    let levels: Vec<f64> = (0..c.w)
        .map(|x| {
            let xf = x as f64;
            let v = (xf * a + c.t * 1.3 * speed + p1).sin() * 0.5 + (xf * b - c.t * 0.9 * speed + p2).sin() * 0.3 + (xf * cc + c.t * 2.1 * speed + p3).sin() * 0.2;
            let env = 0.5 + 0.5 * (xf * 0.045 + c.progress * 14.0 + p4).sin();
            ((v.abs() * 0.5 + 0.15 * env) * c.energy + 0.03).clamp(0.0, 1.0)
        })
        .collect();
    const BARS: [char; 9] = [' ', '▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    (0..c.h)
        .map(|r| {
            Line::from(
                (0..c.w)
                    .map(|x| {
                        let total = levels[x] * c.h as f64 * 8.0;
                        let fill = (total - (c.h - 1 - r) as f64 * 8.0).clamp(0.0, 8.0) as usize;
                        Span::styled(BARS[fill].to_string(), Style::new().fg(hsl(hue + x as f64 / c.w.max(1) as f64 * 50.0, 0.6, 0.55)))
                    })
                    .collect::<Vec<_>>(),
            )
        })
        .collect()
}

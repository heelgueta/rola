//! Album-cover palette. Fetches the (tiny, 64px) cover Spotify serves from its public CDN and
//! extracts the two most prominent *vivid* hues. The UI uses them for the accent color and
//! the ribbons.
//!
//! Readability is guaranteed elsewhere, not here: we only keep the *hue* (and a hint of
//! saturation) and the UI pins lightness to a fixed readable band, so a dark or pale cover can
//! never make text unreadable. Mostly grey covers return None and fall back to the per-track hue.
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    thread,
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    pub h1: f64, // dominant hue, degrees
    pub h2: f64, // second hue (>= 50 degrees away from h1 when the cover has one)
    pub sat: f64, // average saturation of the dominant hue, 0..1
}

#[derive(Clone, Default)]
pub struct Palettes {
    cache: Arc<Mutex<HashMap<String, Option<Palette>>>>,
}

impl Palettes {
    /// Cached palette for a cover URL. First call starts a background fetch and returns None.
    pub fn get(&self, url: &str) -> Option<Palette> {
        if url.is_empty() {
            return None;
        }
        {
            let mut c = self.cache.lock().unwrap();
            if let Some(p) = c.get(url) {
                return *p;
            }
            c.insert(url.to_string(), None); // placeholder so we only fetch once
        }
        let (cache, url) = (self.cache.clone(), url.to_string());
        thread::spawn(move || {
            let p = fetch(&url);
            cache.lock().unwrap().insert(url, p);
        });
        None
    }
}

fn fetch(url: &str) -> Option<Palette> {
    let client = reqwest::blocking::Client::builder().timeout(std::time::Duration::from_secs(10)).user_agent("rola").build().ok()?;
    let bytes = client.get(url).send().ok()?.bytes().ok()?;
    let img = image::load_from_memory(&bytes).ok()?.to_rgb8();
    analyze(img.pixels().map(|p| (p[0], p[1], p[2])))
}

fn rgb_to_hsv(r: u8, g: u8, b: u8) -> (f64, f64, f64) {
    let (r, g, b) = (r as f64 / 255.0, g as f64 / 255.0, b as f64 / 255.0);
    let (mx, mn) = (r.max(g).max(b), r.min(g).min(b));
    let d = mx - mn;
    let h = if d == 0.0 {
        0.0
    } else if mx == r {
        60.0 * (((g - b) / d) % 6.0)
    } else if mx == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    (h.rem_euclid(360.0), if mx == 0.0 { 0.0 } else { d / mx }, mx)
}

/// Histogram of hues weighted by "vividness" (saturation x brightness; greys and near-black
/// pixels count for nothing).
pub fn analyze(pixels: impl Iterator<Item = (u8, u8, u8)>) -> Option<Palette> {
    const BINS: usize = 36;
    let mut weight = [0.0f64; BINS];
    let mut sat_sum = [0.0f64; BINS];
    let mut total = 0.0;
    let mut n = 0.0;
    for (r, g, b) in pixels {
        let (h, s, v) = rgb_to_hsv(r, g, b);
        n += 1.0;
        if s < 0.25 || v < 0.2 {
            continue;
        }
        let w = s * v;
        let bin = ((h / 360.0 * BINS as f64) as usize).min(BINS - 1);
        weight[bin] += w;
        sat_sum[bin] += s * w;
        total += w;
    }
    if n == 0.0 || total / n < 0.04 {
        return None; // essentially grey / black & white cover
    }
    let best = (0..BINS).max_by(|&a, &b| weight[a].total_cmp(&weight[b]))?;
    let center = |b: usize| (b as f64 + 0.5) * 360.0 / BINS as f64;
    let dist = |a: f64, b: f64| {
        let d = (a - b).abs() % 360.0;
        d.min(360.0 - d)
    };
    let h1 = center(best);
    let second = (0..BINS).filter(|&b| dist(center(b), h1) >= 50.0).max_by(|&a, &b| weight[a].total_cmp(&weight[b]));
    let h2 = match second {
        Some(b) if weight[b] >= 0.35 * weight[best] => center(b),
        _ => h1 + 40.0,
    };
    Some(Palette { h1, h2, sat: sat_sum[best] / weight[best].max(1e-9) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_dominant_vivid_hue_and_ignores_grey() {
        // mostly red with some blue and lots of grey/black
        let mut px = vec![(220u8, 30u8, 30u8); 400];
        px.extend(vec![(30, 40, 220); 150]);
        px.extend(vec![(128, 128, 128); 2000]);
        px.extend(vec![(5, 5, 5); 1000]);
        let p = analyze(px.into_iter()).expect("palette");
        assert!(p.h1 < 15.0 || p.h1 > 345.0, "dominant hue should be red, got {}", p.h1);
        assert!((p.h2 - 240.0).abs() < 15.0, "second hue should be blue, got {}", p.h2);
        assert!(analyze(vec![(128u8, 128u8, 128u8); 500].into_iter()).is_none(), "grey cover has no palette");
    }
}

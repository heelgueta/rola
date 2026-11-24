//! Local listening log: Spotify only exposes your last 50 plays, so rola keeps its own.
use crate::model::*;
use serde_json::{json, Value};
use std::{collections::HashMap, fs, io::Write, path::PathBuf, time::{SystemTime, UNIX_EPOCH}};

fn path() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".local/share/rola/history.jsonl")
}

pub fn append(it: &Item) {
    let p = path();
    if let Some(d) = p.parent() {
        let _ = fs::create_dir_all(d);
    }
    let t = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let line = json!({"t": t, "id": it.id, "name": it.name, "sub": it.sub, "dur": it.dur_ms, "date": it.date}).to_string();
    if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(p) {
        let _ = writeln!(f, "{line}");
    }
}

/// Newest first.
pub fn load() -> Vec<(u64, Item)> {
    let text = fs::read_to_string(path()).unwrap_or_default();
    let mut v: Vec<(u64, Item)> = text
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter_map(|j| {
            let mut it = mk(j["id"].as_str()?, Kind::Track, j["name"].as_str()?, j["sub"].as_str().unwrap_or(""));
            it.dur_ms = j["dur"].as_u64().unwrap_or(0) as u32;
            it.date = j["date"].as_str().unwrap_or("").into();
            Some((j["t"].as_u64().unwrap_or(0), it))
        })
        .collect();
    v.reverse();
    v
}

pub fn log_items() -> Vec<Item> {
    load().into_iter().take(1000).map(|(t, mut it)| {
        it.added = fmt_ts(t);
        it
    }).collect()
}

pub fn most_played() -> Vec<Item> {
    let mut counts: HashMap<String, (u32, Item)> = HashMap::new();
    for (_, it) in load() {
        counts.entry(it.id.clone()).or_insert((0, it)).0 += 1;
    }
    let mut v: Vec<(u32, Item)> = counts.into_values().collect();
    v.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.name.cmp(&b.1.name)));
    v.into_iter().take(200).map(|(n, mut it)| {
        it.name = format!("{} ×{n}", it.name);
        it
    }).collect()
}

/// Unix seconds -> "YYYY-MM-DD" (UTC), enough to sort and display.
fn fmt_ts(t: u64) -> String {
    let days = (t / 86400) as i64;
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    format!("{:04}-{:02}-{:02}", if m <= 2 { y + 1 } else { y }, m, d)
}

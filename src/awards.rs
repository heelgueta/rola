//! Bundled "meta-playlists": award winners by year (Grammy album/record/song of the year,
//! Mercury Prize). The data lives in data/awards.tsv, generated from Wikipedia by
//! tools/gen_awards.py and embedded at compile time, so nothing is scraped at runtime.
//!
//! Winners are *names*, not Spotify ids. Albums become `xl:` items and songs `xt:` items;
//! they are resolved to Spotify by search only when you open or play them (see spotify.rs).
use crate::model::*;

const DATA: &str = include_str!("../data/awards.tsv");

/// (award key, menu label, is it an album award?)
const AWARDS: [(&str, &str, bool); 4] = [
    ("grammy-album", "Grammy · Album of the Year", true),
    ("grammy-record", "Grammy · Record of the Year", false),
    ("grammy-song", "Grammy · Song of the Year", false),
    ("mercury-prize", "Mercury Prize", true),
];

pub fn folder_items() -> Vec<Item> {
    AWARDS.iter().map(|(k, label, _)| mk(format!("award:{k}"), Kind::Section, label, "")).collect()
}

/// Winners for an `award:<key>` id, newest first.
pub fn items(id: &str) -> Vec<Item> {
    let key = id.strip_prefix("award:").unwrap_or(id);
    let Some((_, _, album)) = AWARDS.iter().find(|(k, _, _)| *k == key) else { return vec![] };
    let mut out: Vec<Item> = DATA
        .lines()
        .filter_map(|l| {
            let mut f = l.split('\t');
            let (award, year, title, artist) = (f.next()?, f.next()?, f.next()?, f.next()?);
            // data/awards.tsv names the Mercury Prize "Mercury Prize"; menu keys are slugs
            if award.to_lowercase().replace(' ', "-") != key {
                return None;
            }
            let (prefix, kind) = if *album { ("xl", Kind::Album) } else { ("xt", Kind::Track) };
            let mut it = mk(format!("{prefix}:{artist}|{title}"), kind, title, artist);
            it.date = year.to_string();
            Some(it)
        })
        .collect();
    out.sort_by(|a, b| b.date.cmp(&a.date)); // stable: keeps table order within a year
    out
}

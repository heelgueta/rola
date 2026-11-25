//! Acclaim: what the critics, as a panel, think of an album, from the deadwax research project
//! (a one-factor latent model over ~96 critics: Wikipedia ratings boxes, Pitchfork, MusicBrainz
//! users). The bundle in `data/acclaim.tsv` (regenerate with `tools/export_acclaim.py`) holds
//! ~19.5k albums that at least 3 critics rated, with:
//!
//! * `z`    standard deviations above the average album (over all 64k scored albums)
//! * `lb`   z minus one posterior sd: a conservative score, so thinly reviewed albums can't
//!          top a list on luck. Every ranking here sorts by `lb`, not `z`.
//! * `pct`  percentile overall, `gpct` within the genre, `n` critics, `split` how much they disagree
//! * `lists` appearances on "greatest albums" lists, `prize` W won / N nominated / A other award
//!
//! Two jobs: (1) put a score on any album row rola can match by artist + title (`of`), and
//! (2) build the Acclaim lenses (`lens`), several of them personal: they look at what you already
//! own to find acclaimed records you haven't heard, in the genres you actually listen to.
//!
//! When a deadwax server is reachable (`deadwax.rs`), `of` also finds the other ~45k scored albums
//! (including thin, 1–2 critic records, shown with a `~`) once the background lookup answers.
//! Without one, everything here works from the bundle alone.
use crate::model::*;
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    sync::{
        atomic::{AtomicU32, Ordering},
        OnceLock,
    },
};

const DATA: &str = include_str!("../data/acclaim.tsv");

#[derive(Debug, PartialEq)]
pub struct Rec {
    pub artist: String,
    pub title: String,
    pub year: u16,
    pub genre: String,
    pub z: f32,
    pub lb: f32,
    pub pct: f32,
    pub gpct: f32,
    pub n: u8,
    pub split: f32,
    pub lists: u8,
    pub prize: char,
    /// MusicBrainz release-group id: set on live records only (the bundle doesn't carry it).
    pub rgid: String,
}

struct Db {
    recs: Vec<Rec>,
    by_key: HashMap<String, usize>,
    by_artist: HashMap<String, Vec<usize>>,
    hist: Vec<u32>,
    genres: Vec<String>, // by popularity
}

fn db() -> &'static Db {
    static DB: OnceLock<Db> = OnceLock::new();
    DB.get_or_init(|| {
        let mut recs = vec![];
        let mut hist = vec![];
        for l in DATA.lines() {
            if let Some(h) = l.strip_prefix("#hist\t") {
                hist = h.split(',').filter_map(|x| x.parse().ok()).collect();
                continue;
            }
            if l.starts_with('#') {
                continue;
            }
            let f: Vec<&str> = l.split('\t').collect();
            if f.len() < 12 {
                continue;
            }
            let num = |i: usize| f[i].parse::<f32>().unwrap_or(0.0);
            recs.push(Rec {
                artist: f[0].into(),
                title: f[1].into(),
                year: f[2].parse().unwrap_or(0),
                genre: f[3].into(),
                z: num(4),
                lb: num(5),
                pct: num(6),
                gpct: num(7),
                n: f[8].parse().unwrap_or(0),
                split: num(9),
                lists: f[10].parse().unwrap_or(0),
                prize: f[11].chars().next().unwrap_or('-'),
                rgid: String::new(),
            });
        }
        let mut by_key: HashMap<String, usize> = HashMap::new();
        let mut by_artist: HashMap<String, Vec<usize>> = HashMap::new();
        for (i, r) in recs.iter().enumerate() {
            let key = format!("{}|{}", norm_artist(&r.artist), norm_title(&r.title));
            let keep = by_key.get(&key).map_or(true, |&j| recs[j].n < r.n);
            if keep {
                by_key.insert(key, i);
            }
            by_artist.entry(norm_artist(&r.artist)).or_default().push(i);
        }
        let mut gc: HashMap<&str, usize> = HashMap::new();
        for r in &recs {
            *gc.entry(r.genre.as_str()).or_default() += 1;
        }
        let mut genres: Vec<(&str, usize)> = gc.into_iter().collect();
        genres.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
        let genres = genres.into_iter().map(|(g, _)| g.to_string()).collect();
        Db { recs, by_key, by_artist, hist, genres }
    })
}

// ---------- matching ----------

fn alnum(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).collect()
}

/// Drop (...) and [...] groups.
fn strip_brackets(s: &str) -> String {
    let (mut out, mut depth) = (String::new(), 0i32);
    for c in s.chars() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => depth = (depth - 1).max(0),
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out
}

/// "Remain in Light (Deluxe)" / "Remain in Light - Remastered 2008" -> "remaininlight".
pub fn norm_title(s: &str) -> String {
    let mut t = strip_brackets(&s.to_lowercase().replace('&', "and"));
    if let Some((head, tail)) = t.split_once(" - ") {
        const NOISE: [&str; 9] = ["remaster", "deluxe", "edition", "version", "anniversary", "expanded", "mono", "stereo", "bonus"];
        if NOISE.iter().any(|n| tail.contains(n)) {
            t = head.to_string();
        }
    }
    alnum(&t)
}

pub fn norm_artist(s: &str) -> String {
    let t = s.to_lowercase().replace('&', "and");
    let t = t.strip_prefix("the ").unwrap_or(&t);
    alnum(t)
}

/// The bundled record for an album credited to `artist` (Spotify-style: "A, B" or "A · …").
pub fn find(artist: &str, title: &str) -> Option<&'static Rec> {
    let d = db();
    let t = norm_title(title);
    let first = artist.split(" · ").next().unwrap_or(artist);
    let mut tries = vec![norm_artist(first)];
    if let Some((a, _)) = first.split_once(", ") {
        tries.push(norm_artist(a)); // Spotify joins credits with ", "
    }
    tries.iter().find_map(|a| d.by_key.get(&format!("{a}|{t}"))).map(|&i| &d.recs[i])
}

thread_local! {
    /// item id -> (bundled record, live cache key). Live answers are looked up each time, since
    /// they arrive later and follow the live settings.
    static MEMO: RefCell<HashMap<String, (Option<&'static Rec>, String)>> = RefCell::new(HashMap::new());
}

fn memo<R>(it: &Item, f: impl FnOnce(Option<&'static Rec>, &str) -> R) -> R {
    MEMO.with(|m| {
        let mut m = m.borrow_mut();
        let (b, k) = m.entry(it.id.clone()).or_insert_with(|| (find(&it.sub, &it.name), crate::deadwax::album_key(&it.sub, &it.name).0));
        f(*b, k)
    })
}

/// The bundled record for an album row only (no live lookup).
pub fn bundled(it: &Item) -> Option<&'static Rec> {
    if it.kind != Kind::Album {
        return None;
    }
    memo(it, |b, _| b)
}

/// A live record for this normalized key, when live matching is on and it has answered.
pub fn live(key: &str) -> Option<&'static Rec> {
    crate::deadwax::with(|c| match c.resolve_on().then(|| c.look(key)) {
        Some(crate::deadwax::Look::Hit(r)) => Some(r),
        _ => None,
    })
    .flatten()
}

/// The acclaim record for an album row (any source): the bundle first, then live deadwax.
pub fn of(it: &Item) -> Option<&'static Rec> {
    if it.kind != Kind::Album {
        return None;
    }
    memo(it, |b, k| b.or_else(|| live(k)))
}

/// Ask live deadwax (if on) about the album rows of a list that the bundle doesn't know.
/// One deduplicated background batch; rows pick the answers up on a later frame.
pub fn want_live(items: &[Item]) {
    crate::deadwax::with(|c| {
        if !c.resolve_on() {
            return;
        }
        let miss: Vec<(String, String)> = items.iter().filter(|i| i.kind == Kind::Album && bundled(i).is_none()).map(|i| (i.sub.clone(), i.name.clone())).collect();
        if !miss.is_empty() {
            c.want(&miss);
        }
    });
}

// ---------- display ----------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Show {
    Off,
    Z,
    Pct,
    Both,
}

pub fn pct_str(p: f32) -> String {
    let p = p.min(99.99);
    if p >= 99.9 { format!("{p:.2}") } else if p >= 99.0 { format!("{p:.1}") } else { format!("{p:.0}") }
}

/// Right-aligned cell for a row: flags + score. ★ prize, ☰ on greatest-albums lists, ≠ critics split,
/// `~` before the score when only one or two critics rated it (live records; the bundle needs 3).
pub fn cell(r: &Rec, show: Show) -> String {
    let mut flags = String::new();
    if r.prize == 'W' || r.prize == 'N' { flags.push('★') }
    if r.lists > 0 { flags.push('☰') }
    if r.split >= 0.9 { flags.push('≠') }
    let thin = if r.n < 3 { "~" } else { "" };
    let score = match show {
        Show::Off => return String::new(),
        Show::Z => format!("{thin}{:+.1}σ", r.z),
        Show::Pct => format!("{thin}p{}", pct_str(r.pct)),
        Show::Both => format!("{thin}{:+.1}σ p{}", r.z, pct_str(r.pct)),
    };
    if flags.is_empty() { score } else { format!("{flags} {score}") }
}

/// Sort key for "sort by acclaim": the conservative score; unknown albums sink.
pub fn sort_key(it: &Item) -> i32 {
    of(it).map_or(i32::MIN, |r| (r.lb * 100.0) as i32)
}

/// One album row built from the bundle. Resolved to Spotify by search when opened or played.
pub fn item(r: &Rec) -> Item {
    let mut it = mk(format!("xl:{}|{}", r.artist, r.title), Kind::Album, &r.title, &r.artist);
    it.date = r.year.to_string();
    it.genres = r.genre.clone();
    it
}

/// The "card" (`i`): everything we know about one album, with where it sits in the distribution.
pub fn card(r: &Rec) -> Vec<String> {
    let d = db();
    let mut v = vec![
        format!("{}", r.title),
        format!("{} · {} · {}", r.artist, r.year, r.genre),
        String::new(),
        format!("z-score      {:+.2}σ   p{} overall · p{} in its genre", r.z, pct_str(r.pct), pct_str(r.gpct)),
        format!("sure score   {:+.2}σ   (≈84% confident it is at least this good)", r.lb),
        format!(
            "critics      {}   {}",
            r.n,
            if r.n < 3 { "a thin record: treat the score as a hint" } else if r.split >= 0.9 { "they disagree sharply: a split decision" } else if r.split >= 0.6 { "some disagreement" } else if r.n >= 4 { "a consensus" } else { "few critics so far" }
        ),
    ];
    let mut honors = vec![];
    match r.prize {
        'W' => honors.push("won a major prize (Grammy/Mercury/Brit/...)".to_string()),
        'N' => honors.push("nominated for a major prize".to_string()),
        'A' => honors.push("won a smaller award".to_string()),
        _ => {}
    }
    if r.lists > 0 {
        honors.push(format!("on {} \"greatest albums\" list{}", r.lists, if r.lists > 1 { "s" } else { "" }));
    }
    v.push(format!("honors       {}", if honors.is_empty() { "none on record (under the radar?)".to_string() } else { honors.join(" · ") }));
    v.push(String::new());
    // histogram of every scored album, with a marker where this one sits
    if !d.hist.is_empty() {
        const BARS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
        let max = *d.hist.iter().max().unwrap_or(&1) as f32;
        let line: String = d.hist.iter().map(|&c| BARS[(((c as f32 / max).sqrt() * 7.0).round() as usize).min(7)]).collect();
        let pos = (((r.z + 4.0) / 0.2) as usize).min(d.hist.len() - 1);
        v.push(line);
        v.push(format!("{}▲ you are here", " ".repeat(pos)));
        v.push("-4σ                    0                    +4σ".to_string());
    }
    v
}

// ---------- the listener ----------

/// What you already own and play, as the lenses see it.
#[derive(Default)]
pub struct Profile {
    owned: HashSet<String>,   // album keys (artist|title, normalized)
    artists: HashSet<String>, // normalized artist names you own, follow or play a lot
    genre: HashMap<String, f32>, // genre -> weight of your listening
    pub matched: usize,
}

impl Profile {
    /// `albums`: your saved albums; `artists`: artist names you follow / top / play.
    pub fn build(albums: &[Item], artists: &[String]) -> Profile {
        let d = db();
        let mut p = Profile::default();
        for a in albums {
            let first = a.sub.split(" · ").next().unwrap_or("");
            p.owned.insert(format!("{}|{}", norm_artist(first), norm_title(&a.name)));
            p.artists.insert(norm_artist(first));
            if let Some((x, _)) = first.split_once(", ") {
                p.artists.insert(norm_artist(x));
            }
            if let Some(r) = of(a) {
                p.matched += 1;
                *p.genre.entry(r.genre.clone()).or_default() += 1.0;
            }
        }
        for name in artists {
            let k = norm_artist(name);
            if k.is_empty() || !p.artists.insert(k.clone()) {
                continue;
            }
            // an artist you only follow votes for the genre of their best-known record
            if let Some(best) = d.by_artist.get(&k).and_then(|v| v.iter().max_by(|&&a, &&b| d.recs[a].lb.total_cmp(&d.recs[b].lb))) {
                *p.genre.entry(d.recs[*best].genre.clone()).or_default() += 0.5;
            }
        }
        p
    }

    fn knows_album(&self, r: &Rec) -> bool {
        self.owned.contains(&format!("{}|{}", norm_artist(&r.artist), norm_title(&r.title)))
    }
    fn knows_artist(&self, r: &Rec) -> bool {
        self.artists.contains(&norm_artist(&r.artist))
    }
    /// ln(share of your listening in this genre / share if you listened evenly): 0 = neutral.
    fn taste(&self, genre: &str) -> f32 {
        let n = db().genres.len().max(1) as f32;
        let total: f32 = self.genre.values().sum();
        let share = (self.genre.get(genre).copied().unwrap_or(0.0) + 1.0) / (total + n);
        (share * n).ln()
    }

    /// "142 of 310 saved albums are in the panel · mean z +0.9 (p81)", for the sort toast.
    pub fn summary(albums: &[Item]) -> String {
        let hits: Vec<&Rec> = albums.iter().filter_map(of).collect();
        if hits.is_empty() {
            return String::new();
        }
        let mean = hits.iter().map(|r| r.z).sum::<f32>() / hits.len() as f32;
        format!(" · {} of {} known to the panel, mean {:+.1}σ", hits.len(), albums.len(), mean)
    }
}

// ---------- lenses ----------

static ROLL: AtomicU32 = AtomicU32::new(0);

/// `R` re-deals the crate dig.
pub fn reroll() {
    ROLL.fetch_add(1, Ordering::Relaxed);
}

pub fn folder_items() -> Vec<Item> {
    [
        ("next", "Next up for you"),
        ("shelf", "Missing from your shelf"),
        ("dig", "Crate dig · tonight"),
        ("top", "Sure things · all time"),
        ("radar", "Under the radar"),
        ("split", "Split decisions"),
        ("genres", "By genre"),
        ("decades", "By decade"),
    ]
    .iter()
    .map(|(k, n)| mk(format!("acc:{k}"), Kind::Section, n, ""))
    .collect()
}

fn decade_name(d: u16) -> String {
    format!("{d}s")
}

fn decades(g: &str) -> Vec<Item> {
    let d = db();
    let mut ds: Vec<u16> = d.recs.iter().filter(|r| (g == "all" || r.genre == g) && r.year >= 1950).map(|r| r.year / 10 * 10).collect();
    ds.sort_unstable();
    ds.dedup();
    ds.reverse();
    let mut v = vec![mk(format!("acc:gd:{g}:all"), Kind::Section, "All years", "")];
    v.extend(ds.into_iter().map(|x| mk(format!("acc:gd:{g}:{x}"), Kind::Section, &decade_name(x), "")));
    v
}

fn top_by<'a>(it: impl Iterator<Item = &'a Rec>, cap: usize, per_artist: usize) -> Vec<Item> {
    let mut v: Vec<&Rec> = it.collect();
    v.sort_by(|a, b| b.lb.total_cmp(&a.lb).then(b.n.cmp(&a.n)));
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut out = vec![];
    for r in v {
        let c = seen.entry(norm_artist(&r.artist)).or_default();
        if *c >= per_artist {
            continue;
        }
        *c += 1;
        out.push(item(r));
        if out.len() >= cap {
            break;
        }
    }
    out
}

fn splitmix(s: &mut u64) -> u64 {
    *s = s.wrapping_add(0x9E3779B97F4A7C15);
    let mut z = *s;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
    z ^ (z >> 31)
}

fn today() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() / 86400).unwrap_or(0)
}

/// The items of an `acc:*` id. `p` is only used by the personal lenses.
pub fn lens(id: &str, p: &Profile) -> Vec<Item> {
    let d = db();
    let key = id.strip_prefix("acc:").unwrap_or(id);
    match key {
        "top" => top_by(d.recs.iter().filter(|r| r.n >= 4), 150, 2),
        // acclaimed, yet no prize and no "greatest albums" list on record: the canon missed them
        // (list coverage is partial, so this is "under the radar", not "ignored")
        "radar" => top_by(d.recs.iter().filter(|r| r.n >= 4 && r.lists == 0 && r.prize == '-' && r.lb >= 1.2), 150, 1),
        // love-it-or-hate-it: high average, but the critics are far apart
        "split" => {
            let mut v: Vec<&Rec> = d.recs.iter().filter(|r| r.n >= 5 && r.z >= 1.0 && r.split >= 0.9).collect();
            v.sort_by(|a, b| b.split.total_cmp(&a.split));
            v.into_iter().take(100).map(item).collect()
        }
        "genres" => d.genres.iter().map(|g| mk(format!("acc:g:{g}"), Kind::Section, g, "")).collect(),
        "decades" => decades("all"),
        "next" => {
            // acclaimed records by artists you don't know yet, tilted toward the genres you play and
            // away from the ones every canon list and prize already shoves at you
            // (and, mildly, from the most-reviewed: more critics usually means more famous)
            let mut v: Vec<(f32, &Rec)> = d.recs.iter().filter(|r| r.n >= 4 && !p.knows_artist(r) && r.lb >= 1.3).map(|r| (r.lb + 1.0 * p.taste(&r.genre) - if r.lists > 0 { 0.6 } else { 0.0 } - if matches!(r.prize, 'W' | 'N') { 0.5 } else { 0.0 } - 0.15 * (r.n as f32 - 4.0).max(0.0), r)).collect();
            v.sort_by(|a, b| b.0.total_cmp(&a.0));
            let mut seen = HashSet::new();
            v.into_iter().filter(|(_, r)| seen.insert(norm_artist(&r.artist))).take(60).map(|(_, r)| item(r)).collect()
        }
        "shelf" => {
            // acclaimed records by artists you already have, that you don't
            let mut v: Vec<&Rec> = d.recs.iter().filter(|r| r.n >= 3 && p.knows_artist(r) && !p.knows_album(r) && r.lb >= 0.8).collect();
            v.sort_by(|a, b| b.lb.total_cmp(&a.lb));
            v.into_iter().take(100).map(item).collect()
        }
        "dig" => {
            // five records from five different decades, drawn at random (weighted by acclaim and
            // your taste) from what you haven't heard. Same crate all day; `R` deals a new one.
            let mut seed = today().wrapping_mul(0x2545F4914F6CDD1D) ^ ((ROLL.load(Ordering::Relaxed) as u64) << 32);
            let pool: Vec<&Rec> = d.recs.iter().filter(|r| r.n >= 4 && r.lb >= 1.5 && !p.knows_album(r) && r.year >= 1950).collect();
            let mut by_dec: HashMap<u16, Vec<&Rec>> = HashMap::new();
            for r in pool {
                by_dec.entry(r.year / 10 * 10).or_default().push(r);
            }
            let mut decs: Vec<u16> = by_dec.keys().copied().filter(|k| by_dec[k].len() >= 5).collect();
            decs.sort_unstable();
            let mut chosen = vec![];
            while chosen.len() < 5 && !decs.is_empty() {
                chosen.push(decs.remove(splitmix(&mut seed) as usize % decs.len()));
            }
            chosen.sort_unstable();
            chosen
                .into_iter()
                .filter_map(|dec| {
                    let c = &by_dec[&dec];
                    let w: Vec<f32> = c.iter().map(|r| (1.5 * r.lb + 0.5 * p.taste(&r.genre)).exp()).collect();
                    let mut x = (splitmix(&mut seed) >> 11) as f32 / (1u64 << 53) as f32 * w.iter().sum::<f32>();
                    c.iter().zip(&w).find(|(_, &wi)| { x -= wi; x <= 0.0 }).map(|(r, _)| item(r)).or_else(|| c.first().map(|r| item(r)))
                })
                .collect()
        }
        _ => {
            if let Some(g) = key.strip_prefix("g:") {
                return decades(g);
            }
            if let Some(rest) = key.strip_prefix("gd:") {
                let (g, dec) = rest.rsplit_once(':').unwrap_or((rest, "all"));
                let dec: Option<u16> = dec.parse().ok();
                return top_by(d.recs.iter().filter(|r| (g == "all" || r.genre == g) && dec.map_or(true, |x| r.year / 10 * 10 == x) && r.n >= 3), 100, 2);
            }
            vec![]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_spotify_variants() {
        assert_eq!(norm_title("Remain in Light (Deluxe Version)"), norm_title("Remain in Light"));
        assert_eq!(norm_title("Daydream Nation - Remastered 2007"), norm_title("Daydream Nation"));
        assert_eq!(norm_artist("The Clash"), norm_artist("Clash"));
        assert_eq!(norm_artist("Simon & Garfunkel"), norm_artist("Simon and Garfunkel"));
    }

    #[test]
    fn matches_known_albums() {
        let r = find("Talking Heads", "Remain in Light (Deluxe Version)").expect("matched");
        assert!(r.z > 2.0, "z = {}", r.z);
        assert!(find("Nobody At All", "Not An Album").is_none());
    }

    #[test]
    fn lenses_have_content_and_personal_ones_react() {
        let empty = Profile::default();
        for k in ["top", "radar", "split", "genres", "decades", "next", "dig"] {
            assert!(!lens(&format!("acc:{k}"), &empty).is_empty(), "{k}");
        }
        assert!(lens("acc:shelf", &empty).is_empty());
        let mine = vec![{ let mut a = mk("al:1", Kind::Album, "Fear of Music", "Talking Heads"); a.date = "1979".into(); a }];
        let p = Profile::build(&mine, &[]);
        let shelf = lens("acc:shelf", &p);
        assert!(shelf.iter().any(|i| i.name == "Remain in Light"), "{:?}", shelf.iter().map(|i| &i.name).collect::<Vec<_>>());
        assert!(!shelf.iter().any(|i| i.name == "Fear of Music"));
        assert!(!lens("acc:next", &p).iter().any(|i| i.sub == "Talking Heads"));
        assert_eq!(lens("acc:dig", &empty).len(), 5);
    }

    #[test]
    fn card_has_histogram() {
        let r = find("Talking Heads", "Remain in Light").unwrap();
        assert!(card(r).iter().any(|l| l.contains("you are here")));
    }

    fn rec(n: u8) -> Rec {
        Rec { artist: "A".into(), title: "T".into(), year: 2001, genre: "Rock".into(), z: 1.234, lb: 0.5, pct: 88.0, gpct: 80.0, n, split: 0.1, lists: 0, prize: '-', rgid: "r".into() }
    }

    #[test]
    fn thin_records_get_a_tilde() {
        assert_eq!(cell(&rec(2), Show::Z), "~+1.2σ");
        assert_eq!(cell(&rec(1), Show::Both), "~+1.2σ p88");
        assert_eq!(cell(&rec(3), Show::Z), "+1.2σ");
    }

    #[test]
    fn live_records_fill_in_bundle_misses_only_when_on() {
        use crate::deadwax::{fake::Fake, Client, Config, DEFAULT_URL};
        let thin = mk("al:thin", Kind::Album, "Deep Cuts", "Thin Band");
        assert!(of(&thin).is_none(), "no client installed: bundle only");
        let cfg = Config { url: DEFAULT_URL.into(), live: true, cards: true, resolve: true, lenses: true };
        let c = Client::with_transport(cfg.clone(), Fake::new(&["lookup", "album_detail"]), true);
        crate::deadwax::install(&c);
        assert!(of(&thin).is_none(), "not connected yet");
        c.start();
        want_live(&[thin.clone(), mk("al:rl", Kind::Album, "Remain in Light", "Talking Heads")]);
        let r = of(&thin).expect("live record");
        assert_eq!((r.n, r.rgid.as_str()), (2, "rg-deepcuts"));
        assert!(cell(r, Show::Z).starts_with('~'));
        assert_eq!(sort_key(&thin), (r.lb * 100.0) as i32, "sorting uses the conservative lb");
        c.set_config(Config { resolve: false, ..cfg });
        assert!(of(&thin).is_none(), "turning live matching off hides live records again");
        crate::deadwax::install(&Client::disabled());
    }
}

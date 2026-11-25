//! Live deadwax: an optional deadwax server (local by default, `http://127.0.0.1:8820`) that
//! deepens what the bundled `data/acclaim.tsv` can say. See `docs/ARCHITECTURE.md` › Live deadwax.
//!
//! Rules this module keeps:
//!  * **Silent fallback.** Nothing here ever toasts. If the startup probe of `/api/v1/meta` fails,
//!    the client is `Down` for the session and every gate (`on`, `cards_on`, …) answers false, so
//!    rola behaves exactly as with the bundle alone. `R` re-probes only for a URL the user set.
//!  * **Never on the UI thread.** Every request runs on a background thread with a short timeout;
//!    the UI only reads the in-memory caches (`Shared`), which jobs fill in.
//!  * **Circuit breaker.** After `TRIP` consecutive transport failures while connected, the client
//!    goes `Down` and retries a probe after a doubling backoff (30 s … 5 min).
//!  * **Privacy.** Only normalized (artist, title) pairs of albums on screen, or the one album or
//!    artist being inspected, are ever sent. Library, history and the personal lenses stay local.
//!
//! The client is owned by `App` and also installed in a thread-local (`install`/`with`) so that
//! `acclaim::of`, which is called from sorting and drawing without an `App` at hand, can see live
//! records. Each test thread installs its own (or none), so tests stay isolated and offline.
use crate::{
    acclaim::{norm_artist, norm_title, Rec},
    model::{mk, Item, Kind, Load},
};
use serde_json::Value;
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

pub const DEFAULT_URL: &str = "http://127.0.0.1:8820";
const BATCH: usize = 200;
const TRIP: u32 = 3;

#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub url: String,
    pub live: bool,
    pub cards: bool,
    pub resolve: bool,
    pub lenses: bool,
}

impl Config {
    /// "off" or "" disables deadwax entirely (no probe, no requests).
    pub fn disabled(&self) -> bool {
        let u = self.url.trim();
        u.is_empty() || u.eq_ignore_ascii_case("off")
    }
    pub fn base(&self) -> String {
        self.url.trim().trim_end_matches('/').to_string()
    }
    pub fn is_loopback(&self) -> bool {
        let rest = self.base();
        let rest = rest.split_once("://").map(|x| x.1).unwrap_or(&rest);
        let host = rest.split(['/', ':']).next().unwrap_or("");
        matches!(host, "127.0.0.1" | "localhost" | "[" | "::1") || rest.starts_with("[::1]")
    }
    /// Probe timeout: a local server answers in a few ms, so give up fast; remote gets longer.
    pub fn probe_timeout(&self) -> Duration {
        Duration::from_millis(if self.is_loopback() { 150 } else { 1500 })
    }
    pub fn data_timeout(&self) -> Duration {
        Duration::from_millis(if self.is_loopback() { 2000 } else { 5000 })
    }
}

#[derive(Debug, PartialEq)]
pub enum Fail {
    /// Could not talk to the server at all (refused, timeout, DNS): counts toward the breaker.
    Transport(String),
    /// The server answered with an error status (404 for an unknown album, 429, …).
    Status(u16),
}

/// How requests leave the process. `Http` in the app; tests plug in a fake.
pub trait Transport: Send + Sync {
    fn get(&self, url: &str, timeout: Duration) -> Result<String, Fail>;
    fn post(&self, url: &str, body: &str, timeout: Duration) -> Result<String, Fail>;
}

pub struct Http {
    client: reqwest::blocking::Client,
}

impl Http {
    pub fn new(loopback: bool) -> Option<Http> {
        let mut b = reqwest::blocking::Client::builder().user_agent("rola").connect_timeout(Duration::from_millis(if loopback { 150 } else { 1500 }));
        if loopback {
            b = b.no_proxy(); // a local server must not be routed through $HTTP_PROXY
        }
        b.build().ok().map(|client| Http { client })
    }
    fn finish(r: reqwest::Result<reqwest::blocking::Response>) -> Result<String, Fail> {
        let r = r.map_err(|e| Fail::Transport(e.to_string()))?;
        let st = r.status().as_u16();
        if !(200..300).contains(&st) {
            return Err(Fail::Status(st));
        }
        r.text().map_err(|e| Fail::Transport(e.to_string()))
    }
}

impl Transport for Http {
    fn get(&self, url: &str, timeout: Duration) -> Result<String, Fail> {
        Self::finish(self.client.get(url).timeout(timeout).send())
    }
    fn post(&self, url: &str, body: &str, timeout: Duration) -> Result<String, Fail> {
        Self::finish(self.client.post(url).timeout(timeout).header("Content-Type", "application/json").body(body.to_string()).send())
    }
}

// ---------- payloads ----------

#[derive(Clone, Debug, Default)]
pub struct Meta {
    pub mode: String,
    pub caps: Vec<String>,
    pub n_albums: u32,
    // the scale live z / lb are computed on; identical to the bundle's (tests check it)
    #[cfg_attr(not(test), allow(dead_code))]
    pub mu: f32,
    #[cfg_attr(not(test), allow(dead_code))]
    pub sd: f32,
    #[cfg_attr(not(test), allow(dead_code))]
    pub hist: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Critic {
    pub critic: String,
    pub raw: Option<f32>,
    pub z: f32,
    pub retro: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Crowd {
    pub fans: Option<u64>,
    pub gap: Option<f32>,
    pub mb_rating: Option<f32>,
    pub mb_votes: Option<u32>,
    pub sitelinks: Option<u32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Track {
    pub pos: u32,
    pub title: String,
    pub rank: Option<u64>,
    pub secs: Option<u32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Peer {
    pub artist: String,
    pub title: String,
    pub year: u16,
    pub z: f32,
    pub pct: f32,
}

#[derive(Debug, PartialEq)]
pub struct Album {
    pub rec: &'static Rec,
    pub kind: String,
    pub panel: Vec<Critic>,
    pub acc: Vec<(String, Option<u32>)>,
    pub awards: Vec<(String, Option<u16>)>,
    pub crowd: Option<Crowd>,
    pub pitchfork: Option<(f32, String)>,
    pub standout: Vec<Track>,
    pub tracklist: Vec<Track>,
    pub neighbours: Vec<Peer>,
    pub web_path: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ArtistAlbum {
    pub title: String,
    pub year: u16,
    pub kind: String,
    pub z: f32,
    pub lb: f32,
    pub pct: f32,
    pub n: u32,
}

#[derive(Debug, PartialEq)]
pub struct Artist {
    pub name: String,
    pub country: String,
    pub kind: String,
    pub begin: String,
    pub end: String,
    pub n_studio: u32,
    pub mean_z: Option<f32>,
    pub best_z: Option<f32>,
    pub peak: Option<(String, u16)>,
    pub debut_z: Option<f32>,
    pub latest_z: Option<f32>,
    pub albums: Vec<ArtistAlbum>,
}

fn f32_of(v: &Value) -> Option<f32> {
    v.as_f64().map(|x| x as f32)
}
fn str_of(v: &Value) -> String {
    v.as_str().unwrap_or("").to_string()
}

pub fn parse_meta(text: &str) -> Option<Meta> {
    let v: Value = serde_json::from_str(text).ok()?;
    if v["schema_version"].as_u64()? != 1 {
        return None; // a future, incompatible server: behave as if there were none
    }
    Some(Meta {
        mode: str_of(&v["mode"]),
        caps: v["capabilities"].as_array()?.iter().filter_map(|c| c.as_str().map(String::from)).collect(),
        n_albums: v["n_albums"].as_u64().unwrap_or(0) as u32,
        mu: f32_of(&v["mu"]).unwrap_or(0.0),
        sd: f32_of(&v["sd"]).unwrap_or(1.0),
        hist: v["hist"].as_array().map(|h| h.iter().filter_map(|x| x.as_u64().map(|n| n as u32)).collect()).unwrap_or_default(),
    })
}

/// A compact album record (`/api/v1/resolve`, presets, album detail) as a `Rec`.
pub fn parse_rec(v: &Value) -> Option<Rec> {
    Some(Rec {
        artist: v["artist"].as_str()?.to_string(),
        title: v["title"].as_str()?.to_string(),
        year: v["year"].as_u64().unwrap_or(0) as u16,
        genre: str_of(&v["genre"]),
        z: f32_of(&v["z"])?,
        lb: f32_of(&v["lb"])?,
        pct: f32_of(&v["pct"]).unwrap_or(0.0),
        gpct: f32_of(&v["gpct"]).unwrap_or(0.0),
        n: v["n"].as_u64().unwrap_or(0).min(255) as u8,
        split: f32_of(&v["split"]).unwrap_or(0.0),
        lists: v["lists"].as_u64().unwrap_or(0).min(255) as u8,
        prize: v["prize"].as_str().and_then(|s| s.chars().next()).unwrap_or('-'),
        rgid: str_of(&v["rgid"]),
    })
}

/// Live records live for the session and are handed out as `&'static` like the bundled ones.
/// Each distinct album is leaked once (cached by key), so this is bounded by what you browse.
fn leak(r: Rec) -> &'static Rec {
    Box::leak(Box::new(r))
}

fn tracks_of(v: &Value, pos_key: &str) -> Vec<Track> {
    v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|t| {
                    Some(Track {
                        pos: t[pos_key].as_u64().unwrap_or(0) as u32,
                        title: t["title"].as_str()?.to_string(),
                        rank: t["rank"].as_u64(),
                        secs: t["dur"].as_u64().or(t["sec"].as_u64()).map(|s| s as u32),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

pub fn parse_album(text: &str) -> Option<Album> {
    let v: Value = serde_json::from_str(text).ok()?;
    let rec = parse_rec(&v)?;
    let panel = v["panel"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|c| Some(Critic { critic: c["critic"].as_str()?.to_string(), raw: f32_of(&c["raw"]), z: f32_of(&c["z"])?, retro: c["retro"].as_bool().unwrap_or(false) }))
                .collect()
        })
        .unwrap_or_default(); // public servers send {n, split} instead of a list: nothing to show
    let crowd = v["crowd"].is_object().then(|| Crowd {
        fans: v["crowd"]["fans"].as_u64(),
        gap: f32_of(&v["crowd"]["gap"]),
        mb_rating: f32_of(&v["crowd"]["mb_rating"]),
        mb_votes: v["crowd"]["mb_votes"].as_u64().map(|x| x as u32),
        sitelinks: v["crowd"]["wp_sitelinks"].as_u64().map(|x| x as u32),
    });
    Some(Album {
        rec: leak(rec),
        kind: str_of(&v["kind"]),
        panel,
        acc: v["acc_list"].as_array().map(|a| a.iter().filter_map(|x| Some((x["list"].as_str()?.to_string(), x["rank"].as_u64().map(|r| r as u32)))).collect()).unwrap_or_default(),
        awards: v["awards"].as_array().map(|a| a.iter().filter_map(|x| Some((x["award"].as_str()?.to_string(), x["year"].as_u64().map(|r| r as u16)))).collect()).unwrap_or_default(),
        crowd,
        pitchfork: f32_of(&v["pitchfork"]["rating"]).map(|r| (r, str_of(&v["pitchfork"]["reviewer"]))),
        standout: tracks_of(&v["standout_tracks"], "pos"),
        tracklist: tracks_of(&v["tracklist"], "n"),
        neighbours: v["neighbours"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|x| {
                        Some(Peer { artist: x["artist"].as_str()?.to_string(), title: x["title"].as_str()?.to_string(), year: x["year"].as_u64().unwrap_or(0) as u16, z: f32_of(&x["z"])?, pct: f32_of(&x["pct"]).unwrap_or(0.0) })
                    })
                    .collect()
            })
            .unwrap_or_default(),
        web_path: v["web_path"].as_str().map(String::from),
    })
}

pub fn parse_artist(text: &str) -> Option<Artist> {
    let v: Value = serde_json::from_str(text).ok()?;
    let opt = |k: &str| f32_of(&v[k]);
    Some(Artist {
        name: v["name"].as_str()?.to_string(),
        country: str_of(&v["country"]),
        kind: str_of(&v["type"]),
        begin: str_of(&v["begin"]),
        end: str_of(&v["end"]),
        n_studio: v["n_studio"].as_u64().unwrap_or(0) as u32,
        mean_z: opt("mean_z"),
        best_z: opt("best_z"),
        peak: v["peak_title"].as_str().map(|t| (t.to_string(), v["peak_year"].as_u64().unwrap_or(0) as u16)),
        debut_z: opt("debut_z"),
        latest_z: opt("latest_z"),
        albums: v["albums"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|x| {
                        Some(ArtistAlbum {
                            title: x["title"].as_str()?.to_string(),
                            year: x["year"].as_u64().unwrap_or(0) as u16,
                            kind: str_of(&x["kind"]),
                            z: f32_of(&x["z"])?,
                            lb: f32_of(&x["lb"]).unwrap_or(0.0),
                            pct: f32_of(&x["pct"]).unwrap_or(0.0),
                            n: x["n"].as_u64().unwrap_or(0) as u32,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default(),
    })
}

/// The album row a preset row becomes: an `xl:` item, resolved on Spotify by search when opened.
pub fn rec_item(r: &Rec) -> Item {
    let mut it = mk(format!("xl:{}|{}", r.artist, r.title), Kind::Album, &r.title, &r.artist);
    it.date = if r.year > 0 { r.year.to_string() } else { String::new() };
    it.genres = r.genre.clone();
    it
}

// ---------- keys ----------

/// The cache key of an album (normalized "artist|title") and the normalized pairs to ask the
/// server for: the first credit, and for Spotify's "A, B" credits also "A" alone.
pub fn album_key(artist: &str, title: &str) -> (String, Vec<(String, String)>) {
    let first = artist.split(" · ").next().unwrap_or(artist);
    let t = norm_title(title);
    let mut tries = vec![(norm_artist(first), t.clone())];
    if let Some((a, _)) = first.split_once(", ") {
        let a = norm_artist(a);
        if a != tries[0].0 {
            tries.push((a, t.clone()));
        }
    }
    (format!("{}|{t}", tries[0].0), tries)
}

// ---------- the client ----------

#[derive(Clone, Debug, PartialEq)]
pub enum Fetch<T> {
    Pending,
    Ready(T),
    /// The server answered: it has no such album / artist / preset.
    Missing,
    /// It did not answer (transport error, breaker open, 5xx).
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Look {
    /// Not asked yet (or live resolve is off).
    Unknown,
    Pending,
    Hit(&'static Rec),
    Miss,
}

#[derive(Debug, PartialEq, Clone, Copy)]
enum Status {
    Off,
    Unknown,
    Probing,
    Up,
    /// Not reachable. `None`: stay down for the session (failed startup probe).
    Down(Option<Instant>),
}

struct Want {
    key: String,
    tries: Vec<(String, String)>,
}

struct Shared {
    status: Status,
    meta: Option<Meta>,
    ever_up: bool,
    fails: u32,
    backoff: Duration,
    recs: HashMap<String, Option<&'static Rec>>,
    pending: HashSet<String>,
    queue: Vec<Want>,
    resolving: bool,
    albums: HashMap<String, Fetch<Arc<Album>>>,
    artists: HashMap<String, Fetch<Arc<Artist>>>,
    presets: HashMap<String, Fetch<Vec<Item>>>,
    requests: u32, // how many requests went out (tests assert on this)
}

struct Inner {
    cfg: Mutex<Config>,
    shared: Mutex<Shared>,
    transport: Option<Arc<dyn Transport>>,
    /// Tests: run jobs inline instead of on a thread, so results are there when the call returns.
    sync: bool,
}

#[derive(Clone)]
pub struct Client {
    inner: Arc<Inner>,
}

/// The live lenses offered under *Acclaim · deadwax*: (preset key, label, capability needed).
pub const LENSES: [(&str, &str, &str); 4] = [
    ("hidden", "Hidden records · live", "crowd"),
    ("crowd", "Crowd over critics · live", "crowd"),
    ("pitch", "Pitchfork vs the field · live", "pitchfork"),
    ("debut", "Debut masterpieces · live", "presets"),
];

impl Client {
    fn build(cfg: Config, transport: Option<Arc<dyn Transport>>, sync: bool) -> Client {
        let status = if cfg.disabled() || transport.is_none() { Status::Off } else { Status::Unknown };
        let shared = Shared {
            status,
            meta: None,
            ever_up: false,
            fails: 0,
            backoff: Duration::from_secs(30),
            recs: HashMap::new(),
            pending: HashSet::new(),
            queue: vec![],
            resolving: false,
            albums: HashMap::new(),
            artists: HashMap::new(),
            presets: HashMap::new(),
            requests: 0,
        };
        Client { inner: Arc::new(Inner { cfg: Mutex::new(cfg), shared: Mutex::new(shared), transport, sync }) }
    }

    /// The real client. Does nothing until `start()`.
    pub fn new(cfg: Config) -> Client {
        let t: Option<Arc<dyn Transport>> = if cfg.disabled() { None } else { Http::new(cfg.is_loopback()).map(|h| Arc::new(h) as Arc<dyn Transport>) };
        Client::build(cfg, t, false)
    }

    /// No server, never any request (demo mode, tests, `deadwax_url = "off"`).
    pub fn disabled() -> Client {
        Client::build(Config { url: "off".into(), live: false, cards: false, resolve: false, lenses: false }, None, false)
    }

    #[cfg(test)]
    pub fn with_transport(cfg: Config, t: Arc<dyn Transport>, sync: bool) -> Client {
        Client::build(cfg, Some(t), sync)
    }

    fn sh(&self) -> std::sync::MutexGuard<'_, Shared> {
        self.inner.shared.lock().unwrap()
    }
    pub fn config(&self) -> Config {
        self.inner.cfg.lock().unwrap().clone()
    }
    pub fn set_config(&self, c: Config) {
        let mut cur = self.inner.cfg.lock().unwrap();
        // the URL (and so the transport) is fixed for the session; only the toggles change
        let url = cur.url.clone();
        *cur = Config { url, ..c };
    }

    fn spawn(&self, f: impl FnOnce(Client) + Send + 'static) {
        let me = self.clone();
        if self.inner.sync {
            f(me)
        } else {
            std::thread::spawn(move || f(me));
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.config().base())
    }

    fn request(&self, post: Option<&str>, path: &str, timeout: Duration) -> Result<String, Fail> {
        let Some(t) = self.inner.transport.clone() else { return Err(Fail::Transport("off".into())) };
        self.sh().requests += 1;
        let url = self.url(path);
        let r = match post {
            Some(b) => t.post(&url, b, timeout),
            None => t.get(&url, timeout),
        };
        self.note(&r);
        r
    }

    /// The breaker: transport failures (and 5xx / 429) count; anything else resets.
    fn note(&self, r: &Result<String, Fail>) {
        let mut s = self.sh();
        let bad = matches!(r, Err(Fail::Transport(_))) || matches!(r, Err(Fail::Status(c)) if *c >= 500 || *c == 429);
        if !bad {
            s.fails = 0;
            return;
        }
        s.fails += 1;
        if s.status == Status::Up && s.fails >= TRIP {
            s.status = Status::Down(Some(Instant::now() + s.backoff));
            s.backoff = (s.backoff * 2).min(Duration::from_secs(300));
        }
    }

    /// Ask the server for its meta once, in the background. Silent either way.
    pub fn start(&self) {
        {
            let mut s = self.sh();
            if !matches!(s.status, Status::Unknown | Status::Down(_)) {
                return;
            }
            s.status = Status::Probing;
        }
        let first = !self.sh().ever_up;
        self.spawn(move |c| {
            let to = c.config().probe_timeout();
            let meta = c.request(None, "/api/v1/meta", to).ok().and_then(|t| parse_meta(&t));
            let mut s = c.sh();
            match meta {
                Some(m) => {
                    s.meta = Some(m);
                    s.status = Status::Up;
                    s.ever_up = true;
                    s.fails = 0;
                    s.backoff = Duration::from_secs(30);
                }
                // never reached: quiet for the session. Lost after working: retry later.
                None if first => s.status = Status::Down(None),
                None => {
                    let b = s.backoff;
                    s.status = Status::Down(Some(Instant::now() + b));
                    s.backoff = (b * 2).min(Duration::from_secs(300));
                }
            }
        });
    }

    /// `R`: forget failed fetches; re-probe a server that is down if the user configured it
    /// explicitly (the default URL that wasn't there at startup stays quiet).
    pub fn refresh(&self) {
        let explicit = self.config().base() != DEFAULT_URL;
        {
            let mut s = self.sh();
            s.albums.retain(|_, f| !matches!(f, Fetch::Failed));
            s.artists.retain(|_, f| !matches!(f, Fetch::Failed));
            s.presets.clear();
            if matches!(s.status, Status::Down(_)) && (explicit || s.ever_up) {
                s.status = Status::Down(Some(Instant::now()));
            }
        }
        self.tick();
    }

    /// Half-open the breaker when its backoff has passed. Called from `App::tick`.
    pub fn tick(&self) {
        let due = matches!(self.sh().status, Status::Down(Some(t)) if Instant::now() >= t);
        if due {
            self.start();
        }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn connected(&self) -> bool {
        self.sh().status == Status::Up
    }
    /// A server answered at some point this session (the settings menu shows its rows then).
    pub fn ever_up(&self) -> bool {
        self.sh().ever_up
    }
    pub fn has(&self, cap: &str) -> bool {
        self.sh().meta.as_ref().is_some_and(|m| m.caps.iter().any(|c| c == cap))
    }
    pub fn meta(&self) -> Option<Meta> {
        self.sh().meta.clone()
    }
    #[cfg(test)]
    pub fn requests(&self) -> u32 {
        self.sh().requests
    }

    /// The gates below run for every unmatched row on every frame: no allocation, two short locks.
    fn gate(&self, pick: impl Fn(&Config) -> bool, cap: &str) -> bool {
        let ok = {
            let c = self.inner.cfg.lock().unwrap();
            c.live && pick(&c)
        };
        ok && {
            let s = self.sh();
            s.status == Status::Up && (cap.is_empty() || s.meta.as_ref().is_some_and(|m| m.caps.iter().any(|c| c == cap)))
        }
    }
    pub fn on(&self) -> bool {
        self.gate(|_| true, "")
    }
    pub fn cards_on(&self) -> bool {
        self.gate(|c| c.cards, "album_detail")
    }
    pub fn resolve_on(&self) -> bool {
        self.gate(|c| c.resolve, "lookup")
    }
    pub fn lenses_on(&self) -> bool {
        self.gate(|c| c.lenses, "presets")
    }

    // ---------- 64k row matching ----------

    pub fn look(&self, key: &str) -> Look {
        let s = self.sh();
        match s.recs.get(key) {
            Some(Some(r)) => Look::Hit(r),
            Some(None) => Look::Miss,
            None if s.pending.contains(key) => Look::Pending,
            None => Look::Unknown,
        }
    }

    /// Queue albums (artist credit, title) for one batched background lookup. Already known or
    /// queued keys are skipped, so this is cheap to call on every list refresh.
    pub fn want(&self, albums: &[(String, String)]) {
        if !self.resolve_on() {
            return;
        }
        let kick = {
            let mut s = self.sh();
            for (a, t) in albums {
                let (key, tries) = album_key(a, t);
                if key.ends_with('|') || s.recs.contains_key(&key) || s.pending.contains(&key) {
                    continue;
                }
                s.pending.insert(key.clone());
                s.queue.push(Want { key, tries });
            }
            let kick = !s.queue.is_empty() && !s.resolving;
            if kick {
                s.resolving = true;
            }
            kick
        };
        if kick {
            self.spawn(|c| c.drain());
        }
    }

    /// The resolver: batches of at most `BATCH` pairs until the queue is empty.
    fn drain(&self) {
        loop {
            let batch: Vec<Want> = {
                let mut s = self.sh();
                if s.queue.is_empty() || s.status != Status::Up {
                    for w in std::mem::take(&mut s.queue) {
                        s.pending.remove(&w.key); // ask again once the server is back
                    }
                    s.resolving = false;
                    return;
                }
                let mut n = 0;
                let mut take = 0;
                for w in &s.queue {
                    if n + w.tries.len() > BATCH {
                        break;
                    }
                    n += w.tries.len();
                    take += 1;
                }
                s.queue.drain(..take.max(1)).collect()
            };
            let res = self.resolve(&batch);
            let mut s = self.sh();
            for (w, r) in batch.into_iter().zip(res) {
                s.pending.remove(&w.key);
                if let Some(r) = r {
                    s.recs.insert(w.key, r); // (None: no answer, so it stays unknown and is asked again)
                }
            }
        }
    }

    /// One `POST /api/v1/resolve`. Per want: Some(Some(rec)) hit, Some(None) miss, None no answer.
    fn resolve(&self, wants: &[Want]) -> Vec<Option<Option<&'static Rec>>> {
        let pairs: Vec<Value> = wants.iter().flat_map(|w| w.tries.iter()).map(|(a, t)| serde_json::json!({ "artist": a, "title": t })).collect();
        let body = serde_json::json!({ "albums": pairs }).to_string();
        let r = self.request(Some(&body), "/api/v1/resolve", self.config().data_timeout());
        let results: Option<Vec<Value>> = match &r {
            Ok(t) => serde_json::from_str::<Value>(t).ok().and_then(|v| v["results"].as_array().cloned()),
            Err(Fail::Status(c)) if *c < 500 && *c != 429 => Some(vec![]), // the server refused the batch: treat as misses
            Err(_) => None,
        };
        let Some(results) = results else { return wants.iter().map(|_| None).collect() };
        let mut i = 0;
        wants
            .iter()
            .map(|w| {
                let hit = w.tries.iter().enumerate().find_map(|(j, _)| results.get(i + j).and_then(parse_rec));
                i += w.tries.len();
                Some(hit.map(leak))
            })
            .collect()
    }

    /// Records that arrived another way (preset rows, album detail) also serve row scores.
    fn remember(&self, key: String, r: &'static Rec) {
        self.sh().recs.entry(key).or_insert(Some(r));
    }

    // ---------- cards ----------

    /// Album detail for the card. Resolves the album first when its rgid isn't known yet.
    pub fn album(&self, artist: &str, title: &str) -> Fetch<Arc<Album>> {
        let (key, tries) = album_key(artist, title);
        {
            let mut s = self.sh();
            if let Some(f) = s.albums.get(&key) {
                return f.clone();
            }
            if !self.cards_on_locked(&s) {
                return Fetch::Failed;
            }
            s.albums.insert(key.clone(), Fetch::Pending);
        }
        self.spawn(move |c| {
            let known = c.sh().recs.get(&key).copied();
            let rec = match known {
                Some(r) => Some(r),
                None => c.resolve(&[Want { key: key.clone(), tries }]).pop().flatten(),
            };
            let out = match rec {
                None => Fetch::Failed,
                Some(None) => {
                    c.sh().recs.insert(key.clone(), None);
                    Fetch::Missing
                }
                Some(Some(r)) => {
                    c.remember(key.clone(), r);
                    match c.request(None, &format!("/api/v1/album/{}", r.rgid), c.config().data_timeout()) {
                        Ok(t) => parse_album(&t).map(|a| Fetch::Ready(Arc::new(a))).unwrap_or(Fetch::Failed),
                        Err(Fail::Status(404)) => Fetch::Missing,
                        Err(_) => Fetch::Failed,
                    }
                }
            };
            c.sh().albums.insert(key, out);
        });
        self.sh().albums.get(&album_key(artist, title).0).cloned().unwrap_or(Fetch::Pending)
    }

    fn cards_on_locked(&self, s: &Shared) -> bool {
        let c = self.inner.cfg.lock().unwrap();
        c.live && c.cards && s.status == Status::Up && s.meta.as_ref().is_some_and(|m| m.caps.iter().any(|x| x == "album_detail"))
    }

    pub fn artist(&self, name: &str) -> Fetch<Arc<Artist>> {
        let key = norm_artist(name);
        {
            let mut s = self.sh();
            if let Some(f) = s.artists.get(&key) {
                return f.clone();
            }
            let ok = self.cards_on_locked(&s) && s.meta.as_ref().is_some_and(|m| m.caps.iter().any(|x| x == "artists"));
            if !ok || key.is_empty() {
                return Fetch::Failed;
            }
            s.artists.insert(key.clone(), Fetch::Pending);
        }
        let name = name.to_string();
        let k2 = key.clone();
        self.spawn(move |c| {
            let path = format!("/api/v1/artist?name={}", enc(&name));
            let out = match c.request(None, &path, c.config().data_timeout()) {
                Ok(t) => parse_artist(&t).map(|a| Fetch::Ready(Arc::new(a))).unwrap_or(Fetch::Failed),
                Err(Fail::Status(404)) => Fetch::Missing,
                Err(_) => Fetch::Failed,
            };
            c.sh().artists.insert(k2, out);
        });
        self.sh().artists.get(&key).cloned().unwrap_or(Fetch::Pending)
    }

    /// `{url}/#crate?open=<rgid>` when the server offers its web UI.
    pub fn web_url(&self, a: &Album) -> Option<String> {
        if !self.on() || !self.has("web_deeplink") {
            return None;
        }
        let path = a.web_path.clone().or_else(|| (!a.rec.rgid.is_empty()).then(|| format!("/#crate?open={}", a.rec.rgid)))?;
        Some(format!("{}{path}", self.config().base()))
    }

    // ---------- lenses ----------

    /// Section items for the live lenses this server can serve.
    pub fn lens_items(&self) -> Vec<Item> {
        if !self.lenses_on() {
            return vec![];
        }
        LENSES.iter().filter(|(_, _, cap)| self.has(cap)).map(|(k, n, _)| mk(format!("acc:live:{k}"), Kind::Section, n, "")).collect()
    }

    /// Children of `acc:live:<preset>`: fetched once per session through the normal job path.
    pub fn preset(&self, key: &str) -> Load {
        let k = key.strip_prefix("acc:live:").unwrap_or(key).to_string();
        if !self.lenses_on() || !LENSES.iter().any(|(x, _, cap)| *x == k && self.has(cap)) {
            return Load::Failed("this lens needs a live deadwax server".into());
        }
        {
            let mut s = self.sh();
            match s.presets.get(&k) {
                Some(Fetch::Ready(v)) => return Load::Ready(v.clone()),
                Some(Fetch::Pending) => return Load::Loading(vec![]),
                Some(Fetch::Missing) => return Load::Ready(vec![]),
                Some(Fetch::Failed) => return Load::Failed("deadwax did not answer (R retries)".into()),
                None => {
                    s.presets.insert(k.clone(), Fetch::Pending);
                }
            }
        }
        let k2 = k.clone();
        self.spawn(move |c| {
            let out = match c.request(None, &format!("/api/v1/preset/{k2}?limit=80"), c.config().data_timeout()) {
                Ok(t) => {
                    let rows: Vec<&'static Rec> = serde_json::from_str::<Value>(&t)
                        .ok()
                        .and_then(|v| v["rows"].as_array().map(|a| a.iter().filter_map(parse_rec).map(leak).collect()))
                        .unwrap_or_default();
                    for r in &rows {
                        c.remember(album_key(&r.artist, &r.title).0, r);
                    }
                    Fetch::Ready(rows.iter().map(|r| rec_item(r)).collect())
                }
                Err(Fail::Status(404)) => Fetch::Missing,
                Err(_) => Fetch::Failed,
            };
            c.sh().presets.insert(k2, out);
        });
        self.preset(key)
    }
}

fn enc(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

// ---------- the thread-local handle ----------

thread_local! {
    static CUR: RefCell<Option<Client>> = const { RefCell::new(None) };
}

/// Make `c` the client that `acclaim::of` consults on this thread.
pub fn install(c: &Client) {
    CUR.with(|x| *x.borrow_mut() = Some(c.clone()));
}

/// Run `f` with this thread's client, if any.
pub fn with<R>(f: impl FnOnce(&Client) -> R) -> Option<R> {
    CUR.with(|x| x.borrow().as_ref().map(f))
}

#[cfg(test)]
pub mod fake {
    //! A scripted server for tests: answers from a table, counts calls, can be "down".
    use super::*;

    #[derive(Default)]
    pub struct Fake {
        pub down: Mutex<bool>,
        pub calls: Mutex<Vec<String>>,
        pub caps: Mutex<Vec<&'static str>>,
    }

    pub fn rec_json(artist: &str, title: &str, n: u32, z: f32) -> Value {
        serde_json::json!({"rgid": format!("rg-{}", norm_title(title)), "artist": artist, "title": title, "year": 1999, "genre": "Rock", "z": z, "lb": z - 0.5, "pct": 90.0, "gpct": 88.0, "n": n, "split": 0.2, "lists": 0, "prize": "-", "grade": "B"})
    }

    /// Albums this fake server knows, by normalized title.
    pub fn known(title: &str) -> Option<Value> {
        match title {
            "deepcuts" => Some(rec_json("Thin Band", "Deep Cuts", 2, 1.2)),
            "gaia" => Some(rec_json("Mägo de Oz", "Gaia", 4, 0.8)),
            "remaininlight" => Some(rec_json("Talking Heads", "Remain in Light", 9, 3.1)),
            _ => None,
        }
    }

    impl Fake {
        pub fn new(caps: &[&'static str]) -> Arc<Fake> {
            let f = Fake::default();
            *f.caps.lock().unwrap() = caps.to_vec();
            Arc::new(f)
        }
        fn hit(&self, url: &str) -> Result<(), Fail> {
            self.calls.lock().unwrap().push(url.to_string());
            if *self.down.lock().unwrap() {
                return Err(Fail::Transport("connection refused".into()));
            }
            Ok(())
        }
    }

    impl Transport for Fake {
        fn get(&self, url: &str, _t: Duration) -> Result<String, Fail> {
            self.hit(url)?;
            let path = url.split_once("://").map(|x| x.1).unwrap_or(url);
            let path = &path[path.find('/').unwrap_or(0)..];
            if path == "/api/v1/meta" {
                let caps = self.caps.lock().unwrap().clone();
                return Ok(serde_json::json!({"schema_version": 1, "mode": "local", "capabilities": caps, "n_albums": 64205, "mu": 0.014, "sd": 0.457, "hist": vec![1; 40], "presets": []}).to_string());
            }
            if let Some(rg) = path.strip_prefix("/api/v1/album/") {
                let Some(mut v) = known(rg.trim_start_matches("rg-")) else { return Err(Fail::Status(404)) };
                v["kind"] = "studio".into();
                v["panel"] = serde_json::json!([{"critic": "Rolling Stone", "raw": 100, "z": 1.9, "retro": false}, {"critic": "Christgau", "raw": 90, "z": 1.1, "retro": true}]);
                v["acc_list"] = serde_json::json!([{"list": "Rolling Stone 500", "rank": 39}]);
                v["awards"] = serde_json::json!([{"award": "Grammy Hall of Fame", "year": 2018}]);
                v["crowd"] = serde_json::json!({"fans": 123456, "gap": -0.4, "mb_rating": 4.3, "mb_votes": 120, "wp_sitelinks": 31});
                v["standout_tracks"] = serde_json::json!([{"pos": 4, "title": "Once in a Lifetime", "rank": 900000, "dur": 259}, {"pos": 1, "title": "Born Under Punches", "rank": 500000, "dur": 346}]);
                v["tracklist"] = serde_json::json!([{"disc": 1, "n": 1, "title": "Born Under Punches (The Heat Goes On)", "sec": 346}, {"disc": 1, "n": 4, "title": "Once in a Lifetime", "sec": 259}]);
                v["neighbours"] = serde_json::json!([{"rgid": "x", "artist": "Wire", "title": "154", "year": 1979, "z": 2.4, "pct": 99.1}]);
                v["web_path"] = format!("/#crate?open={rg}").into();
                return Ok(v.to_string());
            }
            if path.starts_with("/api/v1/artist?name=") {
                return Ok(serde_json::json!({"aid": "a1", "name": "Talking Heads", "country": "US", "type": "Group", "begin": "1975", "end": "1991", "n_studio": 3, "mean_z": 2.0, "best_z": 3.1, "peak_title": "Remain in Light", "peak_year": 1980, "debut_z": 1.9, "latest_z": 0.4,
                    "albums": [{"rgid": "r1", "title": "Talking Heads: 77", "year": 1977, "kind": "studio", "z": 1.9, "lb": 1.5, "pct": 97.0, "n": 7, "grade": "A"}, {"rgid": "r2", "title": "Remain in Light", "year": 1980, "kind": "studio", "z": 3.1, "lb": 2.6, "pct": 99.9, "n": 9, "grade": "A"}, {"rgid": "r3", "title": "Naked", "year": 1988, "kind": "studio", "z": 0.4, "lb": 0.1, "pct": 70.0, "n": 6, "grade": "C"}]}).to_string());
            }
            if let Some(rest) = path.strip_prefix("/api/v1/preset/") {
                let key = rest.split('?').next().unwrap_or("");
                return Ok(serde_json::json!({"key": key, "label": key, "desc": "", "rows": [rec_json("Thin Band", "Deep Cuts", 2, 1.2), rec_json("Talking Heads", "Remain in Light", 9, 3.1)]}).to_string());
            }
            Err(Fail::Status(404))
        }
        fn post(&self, url: &str, body: &str, _t: Duration) -> Result<String, Fail> {
            self.hit(url)?;
            let v: Value = serde_json::from_str(body).unwrap();
            let albums = v["albums"].as_array().unwrap();
            assert!(albums.len() <= BATCH, "batches are capped");
            let res: Vec<Value> = albums.iter().map(|a| known(a["title"].as_str().unwrap()).unwrap_or(Value::Null)).collect();
            Ok(serde_json::json!({ "results": res }).to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{fake::Fake, *};

    pub fn cfg() -> Config {
        Config { url: DEFAULT_URL.into(), live: true, cards: true, resolve: true, lenses: true }
    }
    const ALL: [&str; 8] = ["lookup", "album_detail", "panel_raw", "crowd", "pitchfork", "artists", "presets", "web_deeplink"];

    fn up(caps: &[&'static str]) -> (Client, Arc<Fake>) {
        let f = Fake::new(caps);
        let c = Client::with_transport(cfg(), f.clone(), true);
        c.start();
        (c, f)
    }

    #[test]
    fn parses_meta_and_rejects_other_schemas() {
        let m = parse_meta(r#"{"schema_version":1,"mode":"public","capabilities":["lookup"],"n_albums":5,"mu":0.01,"sd":0.4,"hist":[1,2]}"#).unwrap();
        assert_eq!((m.mode.as_str(), m.caps.len(), m.hist.len()), ("public", 1, 2));
        assert!((m.mu - 0.01).abs() < 1e-6 && (m.sd - 0.4).abs() < 1e-6);
        assert!(parse_meta(r#"{"schema_version":2,"capabilities":[]}"#).is_none());
        assert!(parse_meta("<html>").is_none());
    }

    #[test]
    fn parses_album_detail_and_public_stripped_variant() {
        let f = Fake::new(&ALL);
        let a = parse_album(&f.get("http://x/api/v1/album/rg-remaininlight", Duration::ZERO).unwrap()).unwrap();
        assert_eq!(a.rec.title, "Remain in Light");
        assert_eq!(a.panel.len(), 2);
        assert!(a.panel[1].retro);
        assert_eq!(a.crowd.as_ref().unwrap().fans, Some(123456));
        assert_eq!(a.standout[0].pos, 4);
        assert_eq!(a.acc[0], ("Rolling Stone 500".into(), Some(39)));
        // what a DEADWAX_PUBLIC server sends: aggregate panel, no crowd / tracks / web path
        let public = r#"{"rgid":"r","artist":"A","title":"T","year":1990,"genre":"Rock","z":1.0,"lb":0.5,"pct":80,"gpct":70,"n":4,"split":0.3,"lists":1,"prize":"-","grade":"B","panel":{"n":4,"split":0.3},"acc_list":[],"awards":[],"neighbours":[],"tracklist":[]}"#;
        let p = parse_album(public).unwrap();
        assert!(p.panel.is_empty() && p.crowd.is_none() && p.standout.is_empty() && p.web_path.is_none());
    }

    #[test]
    fn album_key_matches_bundle_normalization() {
        let (k, tries) = album_key("Talking Heads · Remain in Light", "Remain in Light (Deluxe Version)");
        assert_eq!(k, "talkingheads|remaininlight");
        assert_eq!(tries.len(), 1);
        let (_, tries) = album_key("Kendrick Lamar, SZA", "All The Stars");
        assert_eq!(tries.iter().map(|t| t.0.as_str()).collect::<Vec<_>>(), ["kendricklamarsza", "kendricklamar"]);
    }

    #[test]
    fn offline_is_silent_and_inert() {
        let f = Fake::new(&ALL);
        *f.down.lock().unwrap() = true;
        let c = Client::with_transport(cfg(), f.clone(), true);
        c.start();
        assert!(!c.connected() && !c.ever_up() && !c.on() && !c.cards_on() && !c.lenses_on());
        c.want(&[("Thin Band".into(), "Deep Cuts".into())]);
        assert_eq!(c.album("Thin Band", "Deep Cuts"), Fetch::Failed);
        assert!(c.lens_items().is_empty());
        c.refresh(); // default URL that never answered: R stays quiet
        c.tick();
        assert_eq!(f.calls.lock().unwrap().len(), 1, "only the one startup probe");
    }

    #[test]
    fn explicit_url_reprobes_on_refresh() {
        let f = Fake::new(&ALL);
        *f.down.lock().unwrap() = true;
        let c = Client::with_transport(Config { url: "http://music.example:8820".into(), ..cfg() }, f.clone(), true);
        c.start();
        assert!(!c.connected());
        *f.down.lock().unwrap() = false;
        c.refresh();
        assert!(c.connected(), "R re-probes a server the user configured");
    }

    #[test]
    fn disabled_never_requests() {
        let c = Client::disabled();
        c.start();
        c.want(&[("a".into(), "b".into())]);
        assert!(!c.on());
        assert_eq!(c.requests(), 0);
        let off = Client::new(Config { url: "off".into(), ..cfg() });
        off.start();
        assert!(!off.connected() && off.requests() == 0);
    }

    #[test]
    fn resolves_in_capped_batches_and_caches_misses() {
        let (c, f) = up(&ALL);
        assert!(c.resolve_on());
        let mut many: Vec<(String, String)> = (0..450).map(|i| (format!("Artist {i}"), format!("Nothing {i}"))).collect();
        many.push(("Thin Band".into(), "Deep Cuts".into()));
        c.want(&many);
        let posts = f.calls.lock().unwrap().iter().filter(|u| u.ends_with("/resolve")).count();
        assert_eq!(posts, 3, "451 albums -> 3 batches of <= 200");
        assert!(matches!(c.look("thinband|deepcuts"), Look::Hit(r) if r.n == 2 && r.rgid == "rg-deepcuts"));
        assert_eq!(c.look("artist0|nothing0"), Look::Miss);
        c.want(&many);
        let again = f.calls.lock().unwrap().iter().filter(|u| u.ends_with("/resolve")).count();
        assert_eq!(again, 3, "hits and misses are memoized");
    }

    #[test]
    fn capability_gating() {
        let (c, _) = up(&["lookup", "album_detail", "artists", "presets"]); // a public server
        let lenses: Vec<String> = c.lens_items().into_iter().map(|i| i.id).collect();
        assert_eq!(lenses, ["acc:live:debut"], "crowd and pitchfork lenses need their capabilities");
        assert!(matches!(c.preset("acc:live:hidden"), Load::Failed(_)));
        let Fetch::Ready(a) = c.album("Talking Heads", "Remain in Light") else { panic!() };
        assert_eq!(c.web_url(&a), None, "no web_deeplink capability");
        let (full, _) = up(&ALL);
        assert_eq!(full.lens_items().len(), 4);
        let Fetch::Ready(a) = full.album("Talking Heads", "Remain in Light") else { panic!() };
        assert_eq!(full.web_url(&a).unwrap(), "http://127.0.0.1:8820/#crate?open=rg-remaininlight");
    }

    #[test]
    fn toggles_gate_features() {
        let (c, f) = up(&ALL);
        c.set_config(Config { resolve: false, ..cfg() });
        c.want(&[("Thin Band".into(), "Deep Cuts".into())]);
        assert!(!f.calls.lock().unwrap().iter().any(|u| u.ends_with("/resolve")));
        c.set_config(Config { live: false, ..cfg() });
        assert!(!c.on() && !c.cards_on() && c.lens_items().is_empty());
        c.set_config(Config { cards: false, ..cfg() });
        assert_eq!(c.album("Talking Heads", "Remain in Light"), Fetch::Failed);
        assert_eq!(c.artist("Talking Heads"), Fetch::Failed);
    }

    #[test]
    fn album_artist_and_preset_fetches() {
        let (c, _) = up(&ALL);
        let Fetch::Ready(a) = c.album("Talking Heads, Brian Eno", "Remain in Light - Remastered") else { panic!("album") };
        assert_eq!(a.neighbours[0].artist, "Wire");
        assert!(matches!(c.album("Nobody", "Not An Album"), Fetch::Missing));
        let Fetch::Ready(r) = c.artist("Talking Heads") else { panic!("artist") };
        assert_eq!((r.n_studio, r.albums.len()), (3, 3));
        let Load::Ready(rows) = c.preset("acc:live:hidden") else { panic!("preset") };
        assert_eq!(rows[0].id, "xl:Thin Band|Deep Cuts");
        assert!(matches!(c.look("thinband|deepcuts"), Look::Hit(_)), "preset rows also score rows");
    }

    #[test]
    fn breaker_trips_after_consecutive_failures() {
        let (c, f) = up(&ALL);
        *f.down.lock().unwrap() = true;
        for i in 0..TRIP {
            c.want(&[(format!("A{i}"), format!("T{i}"))]);
        }
        assert!(!c.connected(), "breaker open");
        assert!(!c.on());
        let n = f.calls.lock().unwrap().len();
        c.want(&[("X".into(), "Y".into())]);
        assert_eq!(f.calls.lock().unwrap().len(), n, "no requests while open");
        assert_eq!(c.look("a0|t0"), Look::Unknown, "failed lookups are retried later, not cached as misses");
    }

    #[test]
    fn loopback_detection_and_timeouts() {
        assert!(cfg().is_loopback());
        assert_eq!(cfg().probe_timeout(), Duration::from_millis(150));
        let remote = Config { url: "https://deadwax.example.org/".into(), ..cfg() };
        assert!(!remote.is_loopback());
        assert_eq!(remote.probe_timeout(), Duration::from_millis(1500));
        assert_eq!(remote.base(), "https://deadwax.example.org");
        assert!(Config { url: "http://localhost:9000".into(), ..cfg() }.is_loopback());
        assert!(Config { url: " OFF ".into(), ..cfg() }.disabled());
    }

    /// End to end against a running deadwax (`cargo test -- --ignored live_deadwax`).
    /// Uses DEADWAX_URL if set, else the default local server.
    #[test]
    #[ignore]
    fn live_deadwax() {
        let url = std::env::var("DEADWAX_URL").unwrap_or_else(|_| DEFAULT_URL.into());
        let c = Client::new(Config { url, ..cfg() });
        c.start();
        for _ in 0..40 {
            if c.connected() {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(c.connected(), "no deadwax server answered");
        let m = c.meta().unwrap();
        println!("deadwax: mode {} · {} albums · caps {:?}", m.mode, m.n_albums, m.caps);
        assert_eq!(m.hist.len(), 40);
        // the bundle header says: z over 64205 albums, mu=0.014 sd=0.457 — the live scale must match
        assert!((m.mu - 0.014).abs() < 0.001 && (m.sd - 0.457).abs() < 0.001, "mu {} sd {}", m.mu, m.sd);
        c.want(&[("Talking Heads".into(), "Remain in Light (Deluxe Version)".into()), ("Sonic Youth".into(), "Daydream Nation - Remastered 2007".into())]);
        let mut hit = Look::Unknown;
        for _ in 0..60 {
            hit = c.look("talkingheads|remaininlight");
            if matches!(hit, Look::Hit(_)) {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let Look::Hit(r) = hit else { panic!("not resolved: {hit:?}") };
        println!("resolved: {} — {} {:+.2}σ ({} critics) rgid {}", r.artist, r.title, r.z, r.n, r.rgid);
        let bundled = crate::acclaim::of(&mk("al:x", Kind::Album, "Remain in Light", "Talking Heads")).unwrap();
        assert!((bundled.z - r.z).abs() < 0.011 && (bundled.lb - r.lb).abs() < 0.011, "live and bundled agree");
        let mut album = Fetch::Pending;
        for _ in 0..60 {
            album = c.album("Talking Heads", "Remain in Light");
            if !matches!(album, Fetch::Pending) {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let Fetch::Ready(a) = album else { panic!("album detail: {album:?}") };
        println!("card: {} critics in panel, {} lists, {} awards, {} neighbours, {} standout tracks, web {:?}", a.panel.len(), a.acc.len(), a.awards.len(), a.neighbours.len(), a.standout.len(), c.web_url(&a));
        let mut art = Fetch::Pending;
        for _ in 0..60 {
            art = c.artist("Talking Heads");
            if !matches!(art, Fetch::Pending) {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let Fetch::Ready(ar) = art else { panic!("artist: {art:?}") };
        println!("artist: {} · {} studio albums · peak {:?}", ar.name, ar.n_studio, ar.peak);
        assert!(ar.n_studio >= 6);
    }
}

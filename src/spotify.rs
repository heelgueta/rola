//! Live Spotify source (Web API, cached, fetched on background threads) and the playback
//! controller that drives whichever Spotify Connect device is active (e.g. Spotify.app).
use crate::{auth::Auth, model::*};
use reqwest::Method;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::{mpsc, Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

const LIKED_CAP: usize = 500;
const LIST_CAP: usize = 300;

#[derive(Clone)]
pub struct Api {
    auth: Arc<Auth>,
    http: reqwest::blocking::Client,
}

impl Api {
    pub fn new(auth: Arc<Auth>) -> Api {
        let http = reqwest::blocking::Client::builder().timeout(Duration::from_secs(15)).user_agent("rola").build().expect("http client");
        Api { auth, http }
    }

    fn request(&self, method: Method, path: &str, query: &[(&str, String)], body: Option<Value>) -> Result<Value, String> {
        let url = format!("https://api.spotify.com/v1{path}");
        for attempt in 0..2 {
            let mut rb = self.http.request(method.clone(), &url).bearer_auth(self.auth.token()?).query(query);
            if let Some(b) = &body {
                rb = rb.json(b);
            }
            let resp = rb.send().map_err(|e| e.to_string())?;
            let status = resp.status();
            if status.as_u16() == 401 && attempt == 0 {
                self.auth.invalidate();
                continue;
            }
            if status.as_u16() == 429 && attempt == 0 {
                let wait = resp.headers().get("retry-after").and_then(|h| h.to_str().ok()).and_then(|s| s.parse::<u64>().ok()).unwrap_or(2);
                thread::sleep(Duration::from_secs(wait.min(5)));
                continue;
            }
            let text = resp.text().unwrap_or_default();
            if status.is_success() {
                return Ok(if text.is_empty() { Value::Null } else { serde_json::from_str(&text).unwrap_or(Value::Null) });
            }
            let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
            let msg = v["error"]["message"].as_str().unwrap_or(text.as_str());
            let reason = v["error"]["reason"].as_str().map(|r| format!(" [{r}]")).unwrap_or_default();
            return Err(format!("{}{reason} {msg}", status.as_u16()));
        }
        Err("request failed after retry".into())
    }

    pub fn get(&self, path: &str, query: &[(&str, String)]) -> Result<Value, String> {
        self.request(Method::GET, path, query, None)
    }
}

// ---------- parsing ----------

fn artists_of(v: &Value) -> String {
    v["artists"].as_array().map(|a| a.iter().filter_map(|x| x["name"].as_str()).collect::<Vec<_>>().join(", ")).unwrap_or_default()
}

fn track_item(t: &Value) -> Option<Item> {
    if t["type"].as_str() != Some("track") {
        return None; // episodes, local files
    }
    let id = t["id"].as_str()?;
    let album = t["album"]["name"].as_str().unwrap_or("");
    let artists = artists_of(t);
    let sub = if album.is_empty() { artists } else { format!("{artists} · {album}") };
    Some(Item { id: format!("tr:{id}"), kind: Kind::Track, name: t["name"].as_str()?.into(), sub, dur_ms: t["duration_ms"].as_u64().unwrap_or(0) as u32 })
}
fn album_item(a: &Value) -> Option<Item> {
    Some(mk(format!("al:{}", a["id"].as_str()?), Kind::Album, a["name"].as_str()?, &artists_of(a)))
}
fn artist_item(a: &Value) -> Option<Item> {
    Some(mk(format!("ar:{}", a["id"].as_str()?), Kind::Artist, a["name"].as_str()?, ""))
}
fn playlist_item(p: &Value) -> Option<Item> {
    let owner = p["owner"]["display_name"].as_str().or(p["owner"]["id"].as_str()).unwrap_or("");
    Some(mk(format!("pl:{}", p["id"].as_str()?), Kind::Playlist, p["name"].as_str()?, owner))
}

// ---------- cache ----------

struct Entry {
    load: Load,
    at: Instant,
    inflight: bool,
}
type Cache = Arc<Mutex<HashMap<String, Entry>>>;

fn clone_load(l: &Load) -> Load {
    match l {
        Load::Ready(v) => Load::Ready(v.clone()),
        Load::Loading(v) => Load::Loading(v.clone()),
        Load::Failed(e) => Load::Failed(e.clone()),
    }
}

fn set(cache: &Cache, key: &str, load: Load, inflight: bool) {
    cache.lock().unwrap().insert(key.to_string(), Entry { load, at: Instant::now(), inflight });
}

pub struct Live {
    api: Api,
    cache: Cache,
}

impl Live {
    pub fn new(api: Api) -> Live {
        Live { api, cache: Arc::new(Mutex::new(HashMap::new())) }
    }

    fn ttl(key: &str) -> Duration {
        match key {
            "sec:queue" => Duration::from_secs(3),
            "sec:recent" => Duration::from_secs(20),
            _ => Duration::from_secs(300),
        }
    }
    fn group(key: &str) -> Vec<String> {
        match key {
            k => vec![k.into()],
        }
    }
}

impl Source for Live {
    fn root(&self) -> Vec<Item> {
        root_items()
    }

    fn invalidate(&self) {
        self.cache.lock().unwrap().clear();
    }

    fn children(&self, it: &Item) -> Load {
        let key = it.id.as_str();
        if key == "sec:toptracks" {
            return Load::Ready(top_ranges("tracks"));
        }
        if key == "sec:topartists" {
            return Load::Ready(top_ranges("artists"));
        }
        if key.starts_with("sec:search:") || key.starts_with("hdr:") || !(key.contains(':')) {
            return Load::Ready(vec![]);
        }
        let prev;
        {
            let mut c = self.cache.lock().unwrap();
            match c.get(key) {
                Some(e) if e.inflight => return clone_load(&e.load),
                Some(e) if e.at.elapsed() < if matches!(e.load, Load::Failed(_)) { Duration::from_secs(10) } else { Self::ttl(key) } => return clone_load(&e.load),
                Some(e) => {
                    prev = match &e.load {
                        Load::Ready(v) | Load::Loading(v) => v.clone(),
                        _ => vec![],
                    }
                }
                None => prev = vec![],
            }
            for k in Self::group(key) {
                c.insert(k, Entry { load: Load::Loading(prev.clone()), at: Instant::now(), inflight: true });
            }
        }
        let (api, cache, key) = (self.api.clone(), self.cache.clone(), key.to_string());
        thread::spawn(move || {
            if let Err(e) = fetch(&api, &cache, &key) {
                for k in Live::group(&key) {
                    set(&cache, &k, Load::Failed(e.clone()), false);
                }
            }
        });
        Load::Loading(prev)
    }
}

/// Walk a paged endpoint, publishing partial results after every page.
fn paged(
    api: &Api,
    cache: &Cache,
    key: &str,
    path: &str,
    extra: &[(&str, String)],
    at: &[&str],
    cap: usize,
    page_size: usize,
    parse: &dyn Fn(&Value) -> Option<Item>,
) -> Result<(), String> {
    let mut all: Vec<Item> = vec![];
    let mut offset = 0;
    loop {
        let mut q: Vec<(&str, String)> = vec![("limit", page_size.to_string()), ("offset", offset.to_string())];
        q.extend(extra.iter().map(|(k, v)| (*k, v.clone())));
        let v = api.get(path, &q)?;
        let mut node = &v;
        for k in at {
            node = &node[*k];
        }
        let items = node["items"].as_array().cloned().unwrap_or_default();
        all.extend(items.iter().filter_map(parse));
        let more = !node["next"].is_null() && all.len() < cap && !items.is_empty();
        set(cache, key, if more { Load::Loading(all.clone()) } else { Load::Ready(all.clone()) }, more);
        if !more {
            return Ok(());
        }
        offset += page_size;
    }
}

fn fetch(api: &Api, cache: &Cache, key: &str) -> Result<(), String> {
    let (pre, rest) = key.split_once(':').unwrap_or((key, ""));
    match (pre, rest) {
        ("sec", "playlists") => paged(api, cache, key, "/me/playlists", &[], &[], LIST_CAP, 50, &playlist_item),
        ("sec", "fresh") => {
            let top = api.get("/me/top/artists", &[("limit", "15".into()), ("time_range", "medium_term".into())])?;
            let artists = top["items"].as_array().cloned().unwrap_or_default();
            let mut found: Vec<(String, Item)> = vec![];
            for (i, a) in artists.iter().enumerate() {
                let Some(id) = a["id"].as_str() else { continue };
                let v = api.get(&format!("/artists/{id}/albums"), &[("include_groups", "album,single".into()), ("limit", "5".into())])?;
                for al in v["items"].as_array().cloned().unwrap_or_default() {
                    let date = al["release_date"].as_str().unwrap_or("").to_string();
                    if let Some(mut it) = album_item(&al) {
                        if found.iter().any(|(_, f)| f.id == it.id) {
                            continue;
                        }
                        it.sub = format!("{} · {date}", it.sub);
                        found.push((date, it));
                    }
                }
                found.sort_by(|x, y| y.0.cmp(&x.0));
                let items: Vec<Item> = found.iter().take(40).map(|(_, it)| it.clone()).collect();
                let more = i + 1 < artists.len();
                set(cache, key, if more { Load::Loading(items) } else { Load::Ready(items) }, more);
            }
            if artists.is_empty() {
                set(cache, key, Load::Ready(vec![]), false);
            }
            Ok(())
        }
        ("sec", "albums") => paged(api, cache, key, "/me/albums", &[], &[], LIST_CAP, 50, &|x| album_item(&x["album"])),
        ("sec", "liked") => paged(api, cache, key, "/me/tracks", &[], &[], LIKED_CAP, 50, &|x| track_item(&x["track"])),
        ("sec", "artists") => {
            let mut all = vec![];
            let mut after: Option<String> = None;
            loop {
                let mut q = vec![("type", "artist".to_string()), ("limit", "50".to_string())];
                if let Some(a) = &after {
                    q.push(("after", a.clone()));
                }
                let v = api.get("/me/following", &q)?;
                let items = v["artists"]["items"].as_array().cloned().unwrap_or_default();
                all.extend(items.iter().filter_map(artist_item));
                after = v["artists"]["cursors"]["after"].as_str().map(String::from);
                let more = after.is_some() && all.len() < LIST_CAP && !items.is_empty();
                set(cache, key, if more { Load::Loading(all.clone()) } else { Load::Ready(all.clone()) }, more);
                if !more {
                    return Ok(());
                }
            }
        }
        ("sec", "recent") => {
            let v = api.get("/me/player/recently-played", &[("limit", "50".into())])?; // newest first
            let items: Vec<Item> = v["items"].as_array().map(|a| a.iter().filter_map(|x| track_item(&x["track"])).collect()).unwrap_or_default();
            set(cache, key, Load::Ready(items), false);
            Ok(())
        }
        ("sec", "queue") => {
            let v = api.get("/me/player/queue", &[])?;
            let mut items: Vec<Item> = track_item(&v["currently_playing"]).into_iter().collect();
            items.extend(v["queue"].as_array().map(|a| a.iter().filter_map(track_item).collect::<Vec<_>>()).unwrap_or_default());
            set(cache, key, Load::Ready(items), false);
            Ok(())
        }
        ("top", r) => {
            let (kind, range) = r.split_once(':').unwrap_or(("tracks", "medium_term"));
            let parse: &dyn Fn(&Value) -> Option<Item> = if kind == "tracks" { &track_item } else { &artist_item };
            paged(api, cache, key, &format!("/me/top/{kind}"), &[("time_range", range.to_string())], &[], 50, 50, parse)
        }
        ("pl", id) => {
            let r = paged(api, cache, key, &format!("/playlists/{id}/items"), &[], &[], 500, 50, &|x| track_item(if x["item"].is_object() { &x["item"] } else { &x["track"] }));
            r.map_err(|e| if e.starts_with("403") || e.starts_with("404") { format!("Spotify only exposes tracks of playlists you own ({e})") } else { e })
        }
        ("al", id) => paged(api, cache, key, &format!("/albums/{id}/tracks"), &[], &[], 200, 50, &track_item),
        ("ar", id) => paged(api, cache, key, &format!("/artists/{id}/albums"), &[("include_groups", "album,single".into())], &[], 100, 50, &album_item),
        ("q", r) => {
            let mut p = r.splitn(2, ':');
            let (scope, query) = (p.next().unwrap_or("tracks"), p.next().unwrap_or(""));
            let ty = scope.trim_end_matches('s');
            let parse: &dyn Fn(&Value) -> Option<Item> = match scope {
                "tracks" => &track_item,
                "albums" => &album_item,
                "artists" => &artist_item,
                _ => &playlist_item,
            };
            // dev-mode search is capped at 10 per request: take three pages
            let mut all: Vec<Item> = vec![];
            for page in 0..3 {
                let v = api.get("/search", &[("q", query.to_string()), ("type", ty.to_string()), ("limit", "10".into()), ("offset", (page * 10).to_string())])?;
                let items = v[scope]["items"].as_array().cloned().unwrap_or_default();
                all.extend(items.iter().filter(|x| !x.is_null()).filter_map(parse));
                let more = page < 2 && !v[scope]["next"].is_null();
                set(cache, key, if more { Load::Loading(all.clone()) } else { Load::Ready(all.clone()) }, more);
                if !more {
                    break;
                }
            }
            Ok(())
        }
        _ => {
            set(cache, key, Load::Ready(vec![]), false);
            Ok(())
        }
    }
}

// ---------- playback ----------

pub enum PlayReq {
    Context { uri: String, offset: Option<String> },
    Uris(Vec<String>),
}
pub enum Cmd {
    Play(PlayReq),
    Pause,
    Resume,
    Next,
    Prev,
    Seek(u64),
    Volume(u8),
}
pub struct PlayState {
    pub item: Item,
    pub progress_ms: u64,
    pub playing: bool,
    pub volume: Option<u8>,
    pub device: String,
}
pub enum Event {
    State(Option<PlayState>),
    Error(String),
}

pub struct Player {
    tx: mpsc::Sender<Cmd>,
    rx: mpsc::Receiver<Event>,
}

impl Player {
    pub fn spawn(api: Api) -> Player {
        let (tx, crx) = mpsc::channel();
        let (etx, rx) = mpsc::channel();
        thread::spawn(move || worker(api, crx, etx));
        Player { tx, rx }
    }
    pub fn send(&self, c: Cmd) {
        let _ = self.tx.send(c);
    }
    pub fn drain(&self) -> Vec<Event> {
        self.rx.try_iter().collect()
    }
}

fn pick_device(api: &Api) -> Result<String, String> {
    let v = api.get("/me/player/devices", &[])?;
    let devs = v["devices"].as_array().cloned().unwrap_or_default();
    let name = |d: &Value| d["name"].as_str().unwrap_or("").to_lowercase();
    let usable = |d: &&Value| !d["is_restricted"].as_bool().unwrap_or(false);
    devs.iter()
        .filter(usable)
        .find(|d| d["type"] == "Computer" && !name(d).contains("spotatui"))
        .or_else(|| devs.iter().filter(usable).find(|d| !name(d).contains("spotatui")))
        .or_else(|| devs.iter().find(usable))
        .and_then(|d| d["id"].as_str().map(String::from))
        .ok_or_else(|| "no Spotify device found — open Spotify.app first".to_string())
}

fn run(api: &Api, cmd: &Cmd, device: &mut Option<String>) -> Result<(), String> {
    let (method, path, mut query, body): (Method, &str, Vec<(&str, String)>, Option<Value>) = match cmd {
        Cmd::Play(PlayReq::Context { uri, offset }) => {
            let mut b = json!({ "context_uri": uri });
            if let Some(o) = offset {
                b["offset"] = json!({ "uri": o });
            }
            (Method::PUT, "/me/player/play", vec![], Some(b))
        }
        Cmd::Play(PlayReq::Uris(u)) => (Method::PUT, "/me/player/play", vec![], Some(json!({ "uris": u }))),
        Cmd::Pause => (Method::PUT, "/me/player/pause", vec![], None),
        Cmd::Resume => (Method::PUT, "/me/player/play", vec![], None),
        Cmd::Next => (Method::POST, "/me/player/next", vec![], None),
        Cmd::Prev => (Method::POST, "/me/player/previous", vec![], None),
        Cmd::Seek(ms) => (Method::PUT, "/me/player/seek", vec![("position_ms", ms.to_string())], None),
        Cmd::Volume(p) => (Method::PUT, "/me/player/volume", vec![("volume_percent", p.to_string())], None),
    };
    let first = api.request(method.clone(), path, &query, body.clone());
    match first {
        Err(e) if e.contains("NO_ACTIVE_DEVICE") || e.starts_with("404") => {
            let id = match device.clone() {
                Some(d) => d,
                None => pick_device(api)?,
            };
            query.push(("device_id", id.clone()));
            *device = Some(id);
            api.request(method, path, &query, body).map(|_| ())
        }
        other => other.map(|_| ()),
    }
}

fn poll(api: &Api) -> Result<Option<PlayState>, String> {
    let v = api.get("/me/player", &[])?;
    if v.is_null() {
        return Ok(None);
    }
    let it = &v["item"];
    let item = track_item(it).unwrap_or_else(|| Item {
        id: format!("tr:{}", it["id"].as_str().unwrap_or("")),
        kind: Kind::Track,
        name: it["name"].as_str().unwrap_or("(unknown)").into(),
        sub: it["show"]["name"].as_str().unwrap_or("").into(),
        dur_ms: it["duration_ms"].as_u64().unwrap_or(0) as u32,
    });
    Ok(Some(PlayState {
        item,
        progress_ms: v["progress_ms"].as_u64().unwrap_or(0),
        playing: v["is_playing"].as_bool().unwrap_or(false),
        volume: v["device"]["volume_percent"].as_u64().map(|x| x as u8),
        device: v["device"]["name"].as_str().unwrap_or("").into(),
    }))
}

fn worker(api: Api, rx: mpsc::Receiver<Cmd>, tx: mpsc::Sender<Event>) {
    let mut device: Option<String> = None;
    let mut last_poll = Instant::now() - Duration::from_secs(60);
    let mut poll_at: Option<Instant> = Some(Instant::now());
    let mut last_err = String::new();
    loop {
        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(cmd) => {
                if let Err(e) = run(&api, &cmd, &mut device) {
                    let _ = tx.send(Event::Error(e));
                }
                poll_at = Some(Instant::now() + Duration::from_millis(700));
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        }
        if poll_at.map_or(false, |t| Instant::now() >= t) || last_poll.elapsed() > Duration::from_secs(3) {
            poll_at = None;
            last_poll = Instant::now();
            match poll(&api) {
                Ok(s) => {
                    last_err.clear();
                    let _ = tx.send(Event::State(s));
                }
                Err(e) => {
                    if e != last_err {
                        let _ = tx.send(Event::Error(e.clone()));
                        last_err = e;
                    }
                }
            }
        }
    }
}

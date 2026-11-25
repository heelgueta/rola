//! Application state and input handling (no drawing: see ui.rs).
//!
//! The model is a stack of [`Column`]s (yazi/Miller style). `cols[0]` is the main menu; each
//! `l`/Enter pushes the selected container's children; `h` pops. Data comes from a
//! [`Source`] (live Spotify, or the demo mock) that may answer "still loading" with partial
//! results, so columns re-pull every ~150 ms until complete.
//!
//! Consistency rules (the reason `prefs`/`cursor` exist):
//!  * sort mode is remembered **per container id** (`prefs`, saved to disk) and applied
//!    everywhere that container is listed: as a column *and* as the preview of its parent;
//!  * the cursor is remembered per container as the selected **item id** (not an index), so it
//!    survives re-sorting and progressive loading.
use crate::{
    art::{Palette, Palettes},
    model::{mk, Item, Kind, Load, Source},
    settings::{self, Row, Settings},
    spotify::{Cmd, Event, PlayReq, Player},
};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::{cmp::Ordering, collections::HashMap, fs, path::PathBuf};

pub const SORTS: [&str; 7] = ["default", "name", "artist", "length", "released", "added", "acclaim"];

fn newest_first(a: &str, b: &str) -> Ordering {
    match (a.is_empty(), b.is_empty()) {
        (true, true) => Ordering::Equal,
        (true, false) => Ordering::Greater, // unknown dates sink to the bottom
        (false, true) => Ordering::Less,
        _ => b.cmp(a),
    }
}

/// `idx` (indices into `items`) put in display order for a sort mode. Stable, so "default"
/// is the order Spotify gave us.
pub fn order(items: &[Item], mut idx: Vec<usize>, sort: usize, rev: bool) -> Vec<usize> {
    let it = |i: usize| &items[i];
    match sort {
        1 => idx.sort_by_key(|&i| it(i).name.to_lowercase()),
        2 => idx.sort_by_key(|&i| (it(i).sub.split(" · ").next().unwrap_or("").to_lowercase(), it(i).name.to_lowercase())),
        3 => idx.sort_by_key(|&i| it(i).dur_ms),
        4 => idx.sort_by(|&a, &b| newest_first(&it(a).date, &it(b).date)),
        5 => idx.sort_by(|&a, &b| newest_first(&it(a).added, &it(b).added)),
        6 => idx.sort_by_key(|&i| std::cmp::Reverse(crate::acclaim::sort_key(it(i)))), // best first; unknown albums last
        _ => {}
    }
    if rev {
        idx.reverse();
    }
    idx
}

pub struct Column {
    pub item: Item, // the container this column lists
    pub items: Vec<Item>,
    pub sel: usize, // index into view()
    pub filter: String,
    pub loading: bool,
    pub error: Option<String>,
    pub sort: usize,
    pub rev: bool,
    /// Item id to put the cursor on once it appears (restores the remembered cursor even while
    /// the list is still loading).
    pub want: Option<String>,
}

fn fuzzy(hay: &str, needle: &str) -> bool {
    let mut it = hay.chars().flat_map(|c| c.to_lowercase());
    needle.chars().flat_map(|c| c.to_lowercase()).all(|n| it.any(|c| c == n))
}

impl Column {
    pub fn is_menu(&self) -> bool {
        self.items.iter().any(|i| i.kind == Kind::Header)
    }

    /// Always shown in the header so the active order is never a mystery.
    pub fn sort_label(&self) -> String {
        format!("sort: {} {}", SORTS[self.sort], if self.rev { "↑" } else { "↓" })
    }

    /// Visible rows: filter, then sort. The main menu (has headers) never sorts or filters headers in.
    pub fn view(&self) -> Vec<usize> {
        let v: Vec<usize> = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, i)| {
                self.filter.is_empty() || (i.kind != Kind::Header && (fuzzy(&i.name, &self.filter) || fuzzy(&i.sub, &self.filter)))
            })
            .map(|(n, _)| n)
            .collect();
        if self.is_menu() {
            v
        } else {
            order(&self.items, v, self.sort, self.rev)
        }
    }

    pub fn selected(&self) -> Option<&Item> {
        self.view().get(self.sel).map(|&i| &self.items[i])
    }

    fn resolve_want(&mut self) {
        if let Some(id) = self.want.clone() {
            let v = self.view();
            if let Some(p) = v.iter().position(|&i| self.items[i].id == id) {
                self.sel = p;
                self.want = None;
            } else if !self.loading {
                self.want = None; // finished loading and it isn't there
            }
        }
    }
}

fn split(l: Load) -> (Vec<Item>, bool, Option<String>) {
    let (mut items, loading, err) = match l {
        Load::Ready(v) => (v, false, None),
        Load::Loading(v) => (v, true, None),
        Load::Failed(e) => (vec![], false, Some(e)),
    };
    for (i, it) in items.iter_mut().enumerate() {
        it.pos = i as u32 + 1; // original position, kept through sorting
    }
    (items, loading, err)
}

/// Apply the user's hidden-entries/external settings to the main menu; drop headers that end up empty.
pub fn visible_root(all: &[Item], s: &Settings) -> Vec<Item> {
    let shown = |i: &Item| i.kind == Kind::Header || (!s.hidden.contains(&i.id) && (s.external || i.id != "sec:charts"));
    let kept: Vec<&Item> = all.iter().filter(|i| shown(i)).collect();
    let mut out: Vec<Item> = vec![];
    for (n, it) in kept.iter().enumerate() {
        let has_children = it.kind != Kind::Header || kept[n + 1..].iter().take_while(|x| x.kind != Kind::Header).next().is_some();
        if has_children {
            out.push((*it).clone());
        }
    }
    out
}

// ---- persisted per-container sort preferences ----

fn state_path() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".local/share/rola/state.json")
}
fn load_prefs() -> HashMap<String, (usize, bool)> {
    let v: serde_json::Value = fs::read_to_string(state_path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    v["sorts"]
        .as_object()
        .map(|m| m.iter().filter_map(|(k, x)| Some((k.clone(), (x[0].as_u64()? as usize % SORTS.len(), x[1].as_bool()?)))).collect())
        .unwrap_or_default()
}
fn save_prefs(p: &HashMap<String, (usize, bool)>) {
    if cfg!(test) {
        return; // tests must never touch the user's real files
    }
    let sorts: serde_json::Map<String, serde_json::Value> =
        p.iter().filter(|(_, (s, r))| *s != 0 || *r).map(|(k, (s, r))| (k.clone(), serde_json::json!([s, r]))).collect();
    let path = state_path();
    if let Some(d) = path.parent() {
        let _ = fs::create_dir_all(d);
    }
    let _ = fs::write(path, serde_json::json!({ "sorts": sorts }).to_string());
}

pub struct App {
    pub cols: Vec<Column>,
    pub preview: Vec<Item>, // children of the selected item, already in that container's sort order
    pub preview_loading: bool,
    pub preview_error: Option<String>,
    pub filtering: bool,
    pub search: Option<String>, // scope being typed
    pub query: String,
    pub settings: Settings,
    pub menu: Option<usize>, // settings menu cursor (index into settings::rows())
    pub now: Option<Item>,
    pub playing: bool,
    pub progress_ms: f64,
    pub volume: u8,
    pub device: String,
    pub art_url: String,
    pub t: f64,
    pub energy: f64,
    pub quit: bool,
    pub help: bool,
    /// The acclaim card (`i`) for this album, when open.
    pub card: Option<Item>,
    pub toast: Option<(String, f64)>,
    pub live: bool,
    palettes: Palettes,
    prefs: HashMap<String, (usize, bool)>,   // container id -> (sort, reversed)
    cursor: HashMap<String, String>,         // container id -> selected item id
    hold_until: f64, // ignore polled play/pause state until then (our own optimistic flip wins)
    logged_id: Option<String>,
    history: Vec<Item>, // demo mode only
    queue: Vec<Item>,   // demo mode only
    qi: usize,
    pending_g: bool,
    reload_acc: f64,
    src: Box<dyn Source>,
    player: Option<Player>,
}

impl App {
    pub fn new(src: Box<dyn Source>, player: Option<Player>) -> Self {
        let settings = Settings::load();
        let root = Column {
            item: mk("root", Kind::Section, "rola", ""),
            items: visible_root(&src.root(), &settings),
            sel: 0,
            filter: String::new(),
            loading: false,
            error: None,
            sort: 0,
            rev: false,
            want: None,
        };
        let live = player.is_some();
        let mut a = App {
            cols: vec![root], preview: vec![], preview_loading: false, preview_error: None, filtering: false, search: None,
            query: String::new(), settings, menu: None, now: None, playing: false, progress_ms: 0.0, volume: 70,
            device: String::new(), art_url: String::new(), t: 0.0, energy: 0.15, quit: false, help: false, card: None, toast: None, live,
            palettes: Palettes::default(), prefs: load_prefs(), cursor: HashMap::new(), hold_until: 0.0, logged_id: None,
            history: vec![], queue: vec![], qi: 0, pending_g: false, reload_acc: 0.0, src, player,
        };
        a.settle(1);
        a.refresh_preview();
        a
    }

    pub fn active(&self) -> &Column {
        self.cols.last().unwrap()
    }
    fn active_mut(&mut self) -> &mut Column {
        self.cols.last_mut().unwrap()
    }
    fn toast(&mut self, msg: impl Into<String>) {
        self.toast = Some((msg.into(), self.t + 5.0));
    }
    fn send(&self, c: Cmd) {
        if let Some(p) = &self.player {
            p.send(c);
        }
    }

    /// Accent hues (a, b) for the UI and ribbons, per the `accent` setting. Always safe to call:
    /// falls back to a per-track hue until (or unless) the cover palette is available.
    pub fn hues(&self, seed: u64) -> (f64, f64) {
        use settings::Accent::*;
        let fallback = (crate::viz::base_hue(seed), crate::viz::base_hue(seed) + 70.0);
        match self.settings.accent {
            Fixed => (210.0, 280.0),
            Track => fallback,
            Art => self.palettes.get(&self.art_url).map(|p: Palette| (p.h1, p.h2)).unwrap_or(fallback),
        }
    }

    /// Children of an item; in demo mode the queue/recent folders come from local state.
    pub fn children_of(&self, it: &Item) -> Load {
        if !self.settings.external && (it.id.starts_with("chart:") || it.id == "sec:charts") {
            return Load::Failed("external sources are turned off (press , to change)".into());
        }
        if !self.live {
            match it.id.as_str() {
                "sec:queue" => return Load::Ready(self.queue.get(self.qi..).map(|q| q.to_vec()).unwrap_or_default()),
                "sec:recent" => {
                    if let Load::Ready(rest) = self.src.children(it) {
                        return Load::Ready(self.history.iter().cloned().chain(rest).collect());
                    }
                }
                _ => {}
            }
        }
        self.src.children(it)
    }

    fn build_column(&self, item: Item) -> Column {
        let (items, loading, error) = split(self.children_of(&item));
        let (sort, rev) = self.prefs.get(&item.id).copied().unwrap_or((0, false));
        let want = self.cursor.get(&item.id).cloned();
        let mut c = Column { item, items, sel: 0, filter: String::new(), loading, error, sort, rev, want };
        c.resolve_want();
        c
    }

    fn refresh_preview(&mut self) {
        let sel = self.active().selected().cloned();
        let (mut p, l, e) = match &sel {
            Some(it) if it.is_container() => split(self.children_of(it)),
            _ => (vec![], false, None),
        };
        if let Some(it) = &sel {
            let (sort, rev) = self.prefs.get(&it.id).copied().unwrap_or((0, false));
            let idx = order(&p, (0..p.len()).collect(), sort, rev);
            p = idx.into_iter().map(|i| p[i].clone()).collect();
        }
        self.preview = p;
        self.preview_loading = l;
        self.preview_error = e;
    }

    /// Pull fresh data into any column that is still loading (progressive pages, background refresh).
    fn reload(&mut self) {
        for i in 0..self.cols.len() {
            if self.cols[i].loading || self.cols[i].want.is_some() {
                let (items, loading, error) = split(self.children_of(&self.cols[i].item.clone()));
                let c = &mut self.cols[i];
                let keep = c.selected().map(|s| s.id.clone());
                c.items = items;
                c.loading = loading;
                c.error = error;
                if c.want.is_some() {
                    c.resolve_want();
                } else if let Some(id) = keep {
                    let v = c.view();
                    if let Some(p) = v.iter().position(|&x| c.items[x].id == id) {
                        c.sel = p;
                    }
                }
                let len = c.view().len();
                c.sel = c.sel.min(len.saturating_sub(1));
            }
        }
        if self.preview_loading {
            self.refresh_preview();
        }
    }

    /// Move the cursor off any non-selectable header row, preferring direction `dir`.
    fn settle(&mut self, mut dir: isize) {
        let col = self.active();
        let v = col.view();
        let len = v.len() as isize;
        let mut s = (col.sel as isize).clamp(0, (len - 1).max(0));
        for _ in 0..len * 2 {
            if len == 0 || col.items[v[s as usize]].kind != Kind::Header {
                break;
            }
            if s + dir < 0 || s + dir >= len {
                dir = -dir;
            }
            s += dir;
        }
        self.active_mut().sel = s as usize;
    }

    fn set_sel(&mut self, n: usize, dir: isize) {
        let len = self.active().view().len();
        self.active_mut().sel = n.min(len.saturating_sub(1));
        self.settle(dir);
        self.refresh_preview();
    }
    fn move_sel(&mut self, d: isize) {
        let cur = self.active().sel as isize;
        self.set_sel((cur + d).max(0) as usize, d.signum());
    }

    fn open(&mut self, item: Item) {
        let col = self.build_column(item);
        self.cols.push(col);
        self.refresh_preview();
    }

    fn enter(&mut self) {
        let Some(it) = self.active().selected().cloned() else { return };
        if !it.is_container() {
            return self.play_here();
        }
        if let Some(scope) = it.id.strip_prefix("sec:search:") {
            self.search = Some(scope.to_string());
            self.query.clear();
            return;
        }
        // Playlists whose tracks Spotify won't list can still be played as a whole.
        if self.preview_error.is_some() && it.uri().is_some() {
            return self.play_container(&it);
        }
        self.open(it);
    }

    fn back(&mut self) {
        if self.cols.len() > 1 {
            let c = self.cols.pop().unwrap();
            if let Some(sel) = c.selected().map(|s| s.id.clone()) {
                self.cursor.insert(c.item.id.clone(), sel);
            }
            self.refresh_preview();
        }
    }

    fn cycle_sort(&mut self, reverse: bool) {
        if self.active().is_menu() {
            return self.toast("the main menu has a fixed order");
        }
        let id = self.active().selected().map(|i| i.id.clone());
        let c = self.active_mut();
        if reverse {
            c.rev = !c.rev;
        } else {
            c.sort = (c.sort + 1) % SORTS.len();
            // "acclaim" only makes sense where some album has a score
            if SORTS[c.sort] == "acclaim" && !c.items.iter().any(|i| crate::acclaim::of(i).is_some()) {
                c.sort = (c.sort + 1) % SORTS.len();
            }
            c.rev = false;
        }
        if let Some(id) = id {
            let v = c.view();
            if let Some(p) = v.iter().position(|&i| c.items[i].id == id) {
                c.sel = p; // cursor stays on the same item
            }
        }
        let mut label = c.sort_label();
        if SORTS[c.sort] == "acclaim" {
            label.push_str(&crate::acclaim::Profile::summary(&c.items));
        }
        let (cid, sort, rev) = (c.item.id.clone(), c.sort, c.rev);
        self.prefs.insert(cid, (sort, rev));
        save_prefs(&self.prefs);
        self.toast(label);
        self.refresh_preview();
    }

    fn apply_settings(&mut self) {
        let keep = self.cols[0].selected().map(|i| i.id.clone());
        self.cols[0].items = visible_root(&self.src.root(), &self.settings);
        if let Some(id) = keep {
            let v = self.cols[0].view();
            if let Some(p) = v.iter().position(|&i| self.cols[0].items[i].id == id) {
                self.cols[0].sel = p;
            }
        }
        let len = self.cols[0].view().len();
        self.cols[0].sel = self.cols[0].sel.min(len.saturating_sub(1));
        if self.cols.len() == 1 {
            self.settle(1);
        }
        self.settings.save();
        self.refresh_preview();
    }

    // ---------- playback ----------

    /// What to send to play a list of tracks. Outside-source tracks (`xt:`) have no Spotify id
    /// yet, so they go as (artist, title) pairs to be matched by search.
    fn queue_req(tracks: &[Item]) -> Option<PlayReq> {
        let ext: Vec<(String, String)> = tracks.iter().filter(|t| t.id.starts_with("xt:")).take(30).map(|t| (t.sub.clone(), t.name.clone())).collect();
        if !ext.is_empty() {
            return Some(PlayReq::Search(ext));
        }
        let uris: Vec<String> = tracks.iter().filter_map(|t| t.uri()).take(100).collect();
        (!uris.is_empty()).then_some(PlayReq::Uris(uris))
    }

    fn note_now(&mut self, it: &Item) {
        self.now = Some(it.clone());
        self.progress_ms = 0.0;
        self.playing = true;
        self.hold_until = self.t + 1.5;
        if !self.live && self.history.first().map(|h| h.id != it.id).unwrap_or(true) {
            self.history.insert(0, it.clone());
            self.history.truncate(50);
        }
    }

    fn play_here(&mut self) {
        let col = self.active();
        let view = col.view();
        let Some(pos) = view.iter().position(|&i| Some(&col.items[i].id) == col.selected().map(|s| &s.id)) else { return };
        let tracks: Vec<Item> = view[pos..].iter().map(|&i| col.items[i].clone()).filter(|i| i.kind == Kind::Track).collect();
        let Some(first) = tracks.first().cloned() else { return };
        if self.live {
            // Inside a playlist/album in its natural order, play it as a context so Spotify
            // keeps the whole thing queued; otherwise (sorted/filtered/other lists) send the URIs.
            let ctx = col.item.id.starts_with("pl:") || col.item.id.starts_with("al:");
            let req = match (ctx, col.item.uri(), col.filter.is_empty()) {
                (true, Some(uri), true) if col.sort == 0 && !col.rev => Some(PlayReq::Context { uri, offset: first.uri() }),
                _ => Self::queue_req(&tracks),
            };
            match req {
                Some(r) => self.send(Cmd::Play(r)),
                None => return self.toast("nothing playable here"),
            }
        } else {
            let q: Vec<Item> = col.view().iter().map(|&i| col.items[i].clone()).filter(|i| i.kind == Kind::Track).collect();
            self.qi = q.iter().position(|i| i.id == first.id).unwrap_or(0);
            self.queue = q;
        }
        self.note_now(&first);
    }

    fn play_container(&mut self, it: &Item) {
        if self.live {
            let req = match (it.kind, it.uri()) {
                (Kind::Playlist | Kind::Album | Kind::Artist, Some(uri)) => Some(PlayReq::Context { uri, offset: None }),
                _ => Self::queue_req(&self.preview.iter().filter(|t| t.kind == Kind::Track).cloned().collect::<Vec<_>>()),
            };
            match req {
                Some(r) => {
                    self.send(Cmd::Play(r));
                    self.playing = true;
                    self.hold_until = self.t + 1.5;
                    self.toast(format!("playing {}", it.name));
                }
                None => self.toast("open it first (l), then press P"),
            }
        } else if let Load::Ready(v) = self.children_of(it) {
            self.queue = v.into_iter().filter(|i| i.kind == Kind::Track).collect();
            self.qi = 0;
            if let Some(f) = self.queue.first().cloned() {
                self.note_now(&f);
            }
        }
    }

    fn toggle(&mut self) {
        if self.live {
            self.send(Cmd::Toggle);
            self.playing = !self.playing;
            self.hold_until = self.t + 1.5; // don't let a stale poll flip it straight back
        } else if self.now.is_some() {
            self.playing = !self.playing;
        }
    }
    fn next(&mut self) {
        if self.live {
            return self.send(Cmd::Next);
        }
        if self.qi + 1 < self.queue.len() {
            self.qi += 1;
            let it = self.queue[self.qi].clone();
            self.note_now(&it);
        } else {
            self.playing = false;
            self.progress_ms = 0.0;
        }
    }
    fn prev(&mut self) {
        if self.live {
            return self.send(Cmd::Prev);
        }
        if self.progress_ms > 3000.0 || self.qi == 0 {
            self.progress_ms = 0.0;
        } else {
            self.qi -= 1;
            let it = self.queue[self.qi].clone();
            self.note_now(&it);
        }
    }
    fn seek(&mut self, ms: f64) {
        if let Some(n) = &self.now {
            self.progress_ms = (self.progress_ms + ms).clamp(0.0, n.dur_ms as f64 - 1.0);
            self.send(Cmd::Seek(self.progress_ms as u64));
        }
    }
    fn set_volume(&mut self, v: i32) {
        self.volume = v.clamp(0, 100) as u8;
        self.send(Cmd::Volume(self.volume));
    }

    pub fn tick(&mut self, dt: f64) {
        self.t += dt;
        if let Some((_, until)) = &self.toast {
            if self.t > *until {
                self.toast = None;
            }
        }
        let events = self.player.as_ref().map(|p| p.drain()).unwrap_or_default();
        for e in events {
            match e {
                Event::Error(m) => self.toast(m),
                Event::Info(m) => self.toast(m),
                Event::State(Some(s)) => {
                    if s.playing && s.progress_ms >= 20_000 && self.logged_id.as_deref() != Some(s.item.id.as_str()) {
                        crate::history::append(&s.item); // local listening log (see history.rs)
                        self.logged_id = Some(s.item.id.clone());
                    }
                    self.now = Some(s.item);
                    self.art_url = s.art_url;
                    self.progress_ms = s.progress_ms as f64;
                    if self.t >= self.hold_until {
                        self.playing = s.playing;
                    }
                    self.device = s.device;
                    if let Some(v) = s.volume {
                        self.volume = v;
                    }
                }
                Event::State(None) => {
                    if self.t >= self.hold_until {
                        self.playing = false;
                    }
                }
            }
        }
        self.reload_acc += dt;
        if self.reload_acc > 0.15 {
            self.reload_acc = 0.0;
            self.reload();
        }
        if self.playing {
            self.progress_ms += dt * 1000.0;
            if let Some(n) = &self.now {
                if n.dur_ms > 0 && self.progress_ms >= n.dur_ms as f64 {
                    if self.live {
                        self.progress_ms = n.dur_ms as f64; // wait for the next poll to move on
                    } else {
                        self.next();
                    }
                }
            }
        }
        let target = if self.playing { 1.0 } else { 0.15 };
        self.energy += (target - self.energy) * (dt * 3.0).min(1.0);
    }

    fn menu_key(&mut self, k: KeyEvent, cur: usize) {
        let rows = settings::rows();
        let step = |from: usize, d: isize| -> usize {
            let mut i = from as isize;
            loop {
                i += d;
                if i < 0 || i >= rows.len() as isize {
                    return from;
                }
                if !matches!(rows[i as usize], Row::Title(_)) {
                    return i as usize;
                }
            }
        };
        match k.code {
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char(',') => self.menu = None,
            KeyCode::Char('j') | KeyCode::Down => self.menu = Some(step(cur, 1)),
            KeyCode::Char('k') | KeyCode::Up => self.menu = Some(step(cur, -1)),
            KeyCode::Char('h') | KeyCode::Left => {
                self.settings.change(rows[cur], -1);
                self.apply_settings();
            }
            KeyCode::Char('l') | KeyCode::Right | KeyCode::Enter | KeyCode::Char(' ') => {
                self.settings.change(rows[cur], 1);
                self.apply_settings();
            }
            _ => {}
        }
    }

    pub fn on_key(&mut self, k: KeyEvent) {
        if let Some(cur) = self.menu {
            return self.menu_key(k, cur);
        }
        if self.help || self.card.is_some() {
            self.help = false;
            self.card = None;
            return;
        }
        if let Some(scope) = self.search.clone() {
            match k.code {
                KeyCode::Esc => self.search = None,
                KeyCode::Backspace => {
                    self.query.pop();
                }
                KeyCode::Char(c) => self.query.push(c),
                KeyCode::Enter => {
                    let q = std::mem::take(&mut self.query);
                    self.search = None;
                    if !q.trim().is_empty() {
                        let item = mk(format!("q:{scope}:{}", q.trim()), Kind::Section, &format!("“{}” in {scope}", q.trim()), "");
                        self.open(item);
                        self.set_sel(0, 1);
                    }
                }
                _ => {}
            }
            return;
        }
        if self.filtering {
            match k.code {
                KeyCode::Esc => {
                    self.active_mut().filter.clear();
                    self.filtering = false;
                }
                KeyCode::Enter => self.filtering = false,
                KeyCode::Backspace => {
                    self.active_mut().filter.pop();
                }
                KeyCode::Char(c) => self.active_mut().filter.push(c),
                _ => {}
            }
            self.set_sel(0, 1);
            return;
        }
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let g = std::mem::take(&mut self.pending_g);
        match k.code {
            KeyCode::Char('d') if ctrl => self.move_sel(10),
            KeyCode::Char('u') if ctrl => self.move_sel(-10),
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Esc => {
                if !self.active().filter.is_empty() {
                    self.active_mut().filter.clear();
                    self.set_sel(0, 1);
                }
            }
            KeyCode::Char('j') | KeyCode::Down => self.move_sel(1),
            KeyCode::Char('k') | KeyCode::Up => self.move_sel(-1),
            KeyCode::Char('h') | KeyCode::Left | KeyCode::Backspace => self.back(),
            KeyCode::Char('l') | KeyCode::Right | KeyCode::Enter => self.enter(),
            KeyCode::Char('g') => {
                if g {
                    self.set_sel(0, 1)
                } else {
                    self.pending_g = true
                }
            }
            KeyCode::Char('G') => self.set_sel(usize::MAX, -1),
            KeyCode::Char('P') => {
                if let Some(it) = self.active().selected().cloned() {
                    if it.is_container() {
                        self.play_container(&it);
                    } else {
                        self.play_here();
                    }
                }
            }
            KeyCode::Char('s') => self.cycle_sort(false),
            KeyCode::Char('S') => self.cycle_sort(true),
            KeyCode::Char('R') => {
                self.src.invalidate();
                crate::acclaim::reroll();
                for c in &mut self.cols {
                    c.loading = true;
                }
                self.preview_loading = true;
                self.toast("refreshing…");
            }
            KeyCode::Char(' ') => self.toggle(),
            KeyCode::Char('n') => self.next(),
            KeyCode::Char('p') => self.prev(),
            KeyCode::Char('>') => self.seek(10_000.0),
            KeyCode::Char('<') => self.seek(-10_000.0),
            KeyCode::Char('+') | KeyCode::Char('=') => self.set_volume(self.volume as i32 + 5),
            KeyCode::Char('-') => self.set_volume(self.volume as i32 - 5),
            KeyCode::Char(',') => self.menu = Some(1),
            KeyCode::Char('/') => self.filtering = true,
            KeyCode::Char('?') => self.help = true,
            KeyCode::Char('i') => match self.active().selected().cloned() {
                Some(it) if crate::acclaim::of(&it).is_some() => self.card = Some(it),
                Some(it) if it.kind == Kind::Album => self.toast("the critics' panel has no score for this album"),
                _ => self.toast("i shows the critics' card for an album"),
            },
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::Mock;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn app() -> App {
        let mut a = App::new(Box::new(Mock), None);
        a.prefs.clear(); // don't read the developer's real saved prefs
        a.settings = Settings::default();
        a
    }
    fn k(a: &mut App, c: char) {
        a.on_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
    }
    fn names(items: &[Item]) -> Vec<String> {
        items.iter().map(|i| i.name.to_lowercase()).collect()
    }

    #[test]
    fn sort_cycles_and_keeps_selection() {
        let mut a = app();
        for c in ['j', 'l', 'j', 'j', 'l'] {
            k(&mut a, c); // Playlists > Late night
        }
        let sel = a.active().selected().unwrap().id.clone();
        k(&mut a, 's'); // name
        let v: Vec<String> = a.active().view().iter().map(|&i| a.active().items[i].name.to_lowercase()).collect();
        let mut sorted = v.clone();
        sorted.sort();
        assert_eq!(v, sorted, "sorted by name");
        assert_eq!(a.active().selected().unwrap().id, sel, "selection follows the item");
        k(&mut a, 'S');
        assert_eq!(a.active().sort_label(), "sort: name ↑");
    }

    /// The reported bug: sort a list, go back, and the sort must still apply to the parent's
    /// preview of it, and again when re-entering. Same for the cursor.
    #[test]
    fn sort_and_cursor_are_remembered_per_container() {
        let mut a = app();
        k(&mut a, 'j'); // Playlists (root cursor skips the header)
        k(&mut a, 'l'); // open Playlists
        k(&mut a, 's'); // sort playlists by name
        let pl_sorted = names(&a.active().view().iter().map(|&i| a.active().items[i].clone()).collect::<Vec<_>>());
        k(&mut a, 'j'); // move the cursor
        let sel = a.active().selected().unwrap().id.clone();
        k(&mut a, 'h'); // back to the main menu
        // the main menu is showing a preview of Playlists: it must be in the sorted order too
        assert_eq!(names(&a.preview), pl_sorted, "preview uses the remembered sort");
        k(&mut a, 'l'); // re-enter
        assert_eq!(a.active().sort_label(), "sort: name ↓", "sort restored");
        assert_eq!(a.active().selected().unwrap().id, sel, "cursor restored by item id");
    }

    #[test]
    fn hidden_menu_entries_and_empty_headers_disappear() {
        let mut s = Settings::default();
        for id in ["sec:fresh", "sec:awards", "sec:acclaim", "sec:charts", "sec:toptracks", "sec:topartists"] {
            s.hidden.insert(id.to_string());
        }
        let root = visible_root(&Mock.root(), &s);
        assert!(!root.iter().any(|i| i.id == "hdr:Discover"), "empty Discover header is dropped");
        assert!(root.iter().any(|i| i.id == "sec:playlists"));
        s.external = false;
        s.hidden.clear();
        assert!(!visible_root(&Mock.root(), &s).iter().any(|i| i.id == "sec:charts"), "external off hides charts");
    }
}

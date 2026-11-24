use crate::{
    model::{Item, Kind, Load, Source},
    spotify::{Cmd, Event, PlayReq, Player},
};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::collections::HashMap;

pub const SORTS: [&str; 6] = ["default", "name", "artist", "length", "released", "added"];

fn newest_first(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering::*;
    match (a.is_empty(), b.is_empty()) {
        (true, true) => Equal,
        (true, false) => Greater,
        (false, true) => Less,
        _ => b.cmp(a),
    }
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
}

fn fuzzy(hay: &str, needle: &str) -> bool {
    let mut it = hay.chars().flat_map(|c| c.to_lowercase());
    needle.chars().flat_map(|c| c.to_lowercase()).all(|n| it.any(|c| c == n))
}

impl Column {
    pub fn sort_label(&self) -> String {
        if self.sort == 0 && !self.rev {
            return String::new();
        }
        format!("sort: {} {}", SORTS[self.sort], if self.rev { "↑" } else { "↓" })
    }

    pub fn view(&self) -> Vec<usize> {
        let mut v: Vec<usize> = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, i)| {
                if self.filter.is_empty() {
                    true
                } else {
                    i.kind != Kind::Header && (fuzzy(&i.name, &self.filter) || fuzzy(&i.sub, &self.filter))
                }
            })
            .map(|(n, _)| n)
            .collect();
        if self.items.iter().any(|i| i.kind == Kind::Header) {
            return v; // the root menu keeps its fixed layout
        }
        let it = |i: usize| &self.items[i];
        match self.sort {
            1 => v.sort_by_key(|&i| it(i).name.to_lowercase()),
            2 => v.sort_by_key(|&i| (it(i).sub.split(" · ").next().unwrap_or("").to_lowercase(), it(i).name.to_lowercase())),
            3 => v.sort_by_key(|&i| it(i).dur_ms),
            4 => v.sort_by(|&a, &b| newest_first(&it(a).date, &it(b).date)),
            5 => v.sort_by(|&a, &b| newest_first(&it(a).added, &it(b).added)),
            _ => {}
        }
        if self.rev {
            v.reverse();
        }
        v
    }
    pub fn selected(&self) -> Option<&Item> {
        self.view().get(self.sel).map(|&i| &self.items[i])
    }
}

fn split(l: Load) -> (Vec<Item>, bool, Option<String>) {
    match l {
        Load::Ready(v) => (v, false, None),
        Load::Loading(v) => (v, true, None),
        Load::Failed(e) => (vec![], false, Some(e)),
    }
}

pub struct App {
    pub cols: Vec<Column>,
    pub preview: Vec<Item>,
    pub preview_loading: bool,
    pub preview_error: Option<String>,
    pub filtering: bool,
    pub search: Option<String>, // scope being typed
    pub query: String,
    pub viz_mode: usize,
    pub now: Option<Item>,
    pub playing: bool,
    pub progress_ms: f64,
    pub volume: u8,
    pub device: String,
    pub t: f64,
    pub energy: f64,
    pub quit: bool,
    pub help: bool,
    pub toast: Option<(String, f64)>,
    pub live: bool,
    logged_id: Option<String>,
    history: Vec<Item>, // demo mode only
    queue: Vec<Item>,   // demo mode only
    qi: usize,
    pending_g: bool,
    reload_acc: f64,
    src: Box<dyn Source>,
    player: Option<Player>,
    memory: HashMap<String, usize>,
}

impl App {
    pub fn new(src: Box<dyn Source>, player: Option<Player>) -> Self {
        let root = Column {
            item: crate::model::mk("root", Kind::Section, "rola", ""),
            items: src.root(),
            sel: 0,
            filter: String::new(),
            loading: false,
            error: None,
            sort: 0,
            rev: false,
        };
        let live = player.is_some();
        let mut a = App {
            cols: vec![root], preview: vec![], preview_loading: false, preview_error: None, filtering: false, search: None,
            query: String::new(), viz_mode: 0, now: None, playing: false, progress_ms: 0.0, volume: 70,
            device: String::new(), t: 0.0, energy: 0.15, quit: false, help: false, toast: None, live, logged_id: None, history: vec![],
            queue: vec![], qi: 0, pending_g: false, reload_acc: 0.0, src, player, memory: HashMap::new(),
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

    /// Children of an item; in demo mode the queue/recent folders come from local state.
    pub fn children_of(&self, it: &Item) -> Load {
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

    fn refresh_preview(&mut self) {
        let sel = self.active().selected().cloned();
        let (p, l, e) = match sel {
            Some(it) if it.is_container() => split(self.children_of(&it)),
            _ => (vec![], false, None),
        };
        self.preview = p;
        self.preview_loading = l;
        self.preview_error = e;
    }

    /// Pull fresh data into any column that is still loading.
    fn reload(&mut self) {
        for i in 0..self.cols.len() {
            if self.cols[i].loading || self.cols[i].error.is_some() && i + 1 == self.cols.len() && false {
                let (items, loading, error) = split(self.children_of(&self.cols[i].item.clone()));
                let c = &mut self.cols[i];
                c.items = items;
                c.loading = loading;
                c.error = error;
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
        let sel = self.memory.get(&item.id).copied().unwrap_or(0);
        let (items, loading, error) = split(self.children_of(&item));
        let sel = sel.min(items.len().saturating_sub(1));
        self.cols.push(Column { item, items, sel, filter: String::new(), loading, error, sort: 0, rev: false });
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
            self.memory.insert(c.item.id, c.sel);
            self.refresh_preview();
        }
    }

    fn cycle_sort(&mut self, reverse: bool) {
        let id = self.active().selected().map(|i| i.id.clone());
        let c = self.active_mut();
        if c.items.iter().any(|i| i.kind == Kind::Header) {
            return;
        }
        if reverse {
            c.rev = !c.rev;
        } else {
            c.sort = (c.sort + 1) % SORTS.len();
            c.rev = false;
        }
        if let Some(id) = id {
            let v = c.view();
            if let Some(p) = v.iter().position(|&i| c.items[i].id == id) {
                c.sel = p;
            }
        }
        self.refresh_preview();
    }

    // ---------- playback ----------

    fn note_now(&mut self, it: &Item) {
        self.now = Some(it.clone());
        self.progress_ms = 0.0;
        self.playing = true;
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
            let ctx = col.item.id.starts_with("pl:") || col.item.id.starts_with("al:");
            let req = match (ctx, col.item.uri(), col.filter.is_empty()) {
                (true, Some(uri), true) if col.sort == 0 && !col.rev => PlayReq::Context { uri, offset: first.uri() },
                _ => PlayReq::Uris(tracks.iter().take(100).filter_map(|t| t.uri()).collect()),
            };
            self.send(Cmd::Play(req));
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
                (Kind::Playlist | Kind::Album | Kind::Artist, Some(uri)) => PlayReq::Context { uri, offset: None },
                _ => {
                    let uris: Vec<String> = self.preview.iter().filter(|t| t.kind == Kind::Track).take(100).filter_map(|t| t.uri()).collect();
                    if uris.is_empty() {
                        return self.toast("nothing playable here");
                    }
                    PlayReq::Uris(uris)
                }
            };
            self.send(Cmd::Play(req));
            self.playing = true;
            self.toast(format!("playing {}", it.name));
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
            self.send(if self.playing { Cmd::Pause } else { Cmd::Resume });
            self.playing = !self.playing;
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
                Event::State(Some(s)) => {
                    if s.playing && s.progress_ms >= 20_000 && self.logged_id.as_deref() != Some(s.item.id.as_str()) {
                        crate::history::append(&s.item);
                        self.logged_id = Some(s.item.id.clone());
                    }
                    self.now = Some(s.item);
                    self.progress_ms = s.progress_ms as f64;
                    self.playing = s.playing;
                    self.device = s.device;
                    if let Some(v) = s.volume {
                        self.volume = v;
                    }
                }
                Event::State(None) => self.playing = false,
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

    pub fn on_key(&mut self, k: KeyEvent) {
        if self.help {
            self.help = false;
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
                        let mut item = crate::model::mk(format!("q:{scope}:{}", q.trim()), Kind::Section, &format!("“{}” in {scope}", q.trim()), "");
                        item.sub = scope;
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
            KeyCode::Char('v') => self.viz_mode = (self.viz_mode + 1) % crate::viz::MODES.len(),
            KeyCode::Char('/') => self.filtering = true,
            KeyCode::Char('?') => self.help = true,
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::Mock;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    #[test]
    fn sort_cycles_and_keeps_selection() {
        let mut a = App::new(Box::new(Mock), None);
        let k = |c| KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE);
        for c in ['j', 'l', 'j', 'j', 'l'] {
            a.on_key(k(c)); // Playlists > Late night
        }
        let sel = a.active().selected().unwrap().id.clone();
        a.on_key(k('s')); // name
        let names: Vec<String> = a.active().view().iter().map(|&i| a.active().items[i].name.to_lowercase()).collect();
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(names, sorted, "sorted by name");
        assert_eq!(a.active().selected().unwrap().id, sel, "selection follows the item");
        a.on_key(k('S')); // reverse
        let rev: Vec<String> = a.active().view().iter().map(|&i| a.active().items[i].name.to_lowercase()).collect();
        sorted.reverse();
        assert_eq!(rev, sorted, "reversed");
        assert_eq!(a.active().sort_label(), "sort: name ↑");
    }
}

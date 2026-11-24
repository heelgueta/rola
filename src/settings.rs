//! User settings: which columns each kind of list shows, numbering, accent colors, which parts
//! of the main menu are visible. Stored in ~/.config/rola/settings.conf as simple `key = value`
//! lines (TOML-compatible, hand-editable). Everything here is also reachable from the in-app
//! settings menu (`,`), which is built from `rows()`.
use std::{collections::HashSet, fs, path::PathBuf};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Numbering {
    /// Album -> track number, playlist -> position in the playlist, others none.
    Auto,
    /// Position in the list (as loaded, unaffected by sorting) everywhere there are tracks.
    Position,
    Off,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Accent {
    /// Dominant color of the album cover (clamped so text stays readable).
    Art,
    /// A stable pseudo-random hue per track.
    Track,
    Fixed,
}

#[derive(Clone, Debug)]
pub struct Settings {
    pub viz: bool,
    pub accent: Accent,
    pub numbering: Numbering,
    // track rows
    pub t_artist: bool,
    pub t_album: bool,
    pub t_year: bool,
    pub t_added: bool,
    pub t_length: bool,
    // album rows
    pub a_artist: bool,
    pub a_year: bool,
    pub a_tracks: bool,
    pub a_added: bool,
    // artist rows
    pub r_genres: bool,
    // playlist rows
    pub p_owner: bool,
    pub p_count: bool,
    /// Allow reaching non-Spotify sources (ListenBrainz charts).
    pub external: bool,
    /// Main-menu entries (item ids like "sec:charts") the user has hidden.
    pub hidden: HashSet<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            viz: true,
            accent: Accent::Art,
            numbering: Numbering::Auto,
            t_artist: true,
            t_album: false,
            t_year: false,
            t_added: false,
            t_length: true,
            a_artist: true,
            a_year: true,
            a_tracks: false,
            a_added: false,
            r_genres: false,
            p_owner: false,
            p_count: true,
            external: true,
            hidden: HashSet::new(),
        }
    }
}

fn path() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".config/rola/settings.conf")
}

/// Main-menu entries that can be hidden, with their labels. (Headers disappear on their own
/// when everything under them is hidden.)
pub const ROOT_TOGGLES: [(&str, &str); 16] = [
    ("sec:queue", "Now playing · Queue"),
    ("sec:playlists", "Playlists"),
    ("sec:albums", "Albums"),
    ("sec:artists", "Artists"),
    ("sec:liked", "Liked songs"),
    ("sec:recent", "Recently played"),
    ("sec:stats", "History (local)"),
    ("sec:fresh", "New from your artists"),
    ("sec:awards", "Awards & lists"),
    ("sec:charts", "Charts · ListenBrainz"),
    ("sec:toptracks", "Your top tracks"),
    ("sec:topartists", "Your top artists"),
    ("sec:search:tracks", "Search · Tracks"),
    ("sec:search:albums", "Search · Albums"),
    ("sec:search:artists", "Search · Artists"),
    ("sec:search:playlists", "Search · Playlists"),
];

/// One line in the settings menu.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Row {
    Title(&'static str),
    Viz,
    Accent,
    Numbering,
    TArtist,
    TAlbum,
    TYear,
    TAdded,
    TLength,
    AArtist,
    AYear,
    ATracks,
    AAdded,
    RGenres,
    POwner,
    PCount,
    External,
    Root(usize),
}

pub fn rows() -> Vec<Row> {
    use Row::*;
    let mut v = vec![
        Title("Look"),
        Viz,
        Accent,
        Title("Track rows"),
        Numbering,
        TArtist,
        TAlbum,
        TYear,
        TAdded,
        TLength,
        Title("Album rows"),
        AArtist,
        AYear,
        ATracks,
        AAdded,
        Title("Artist rows"),
        RGenres,
        Title("Playlist rows"),
        POwner,
        PCount,
        Title("Sources"),
        External,
        Title("Main menu entries"),
    ];
    v.extend((0..ROOT_TOGGLES.len()).map(Root));
    v
}

impl Settings {
    pub fn label(&self, r: Row) -> (String, String) {
        let b = |x: bool| (if x { "on" } else { "off" }).to_string();
        match r {
            Row::Title(t) => (t.to_string(), String::new()),
            Row::Viz => ("Visualizer (ribbons)".into(), b(self.viz)),
            Row::Accent => (
                "Accent color from".into(),
                match self.accent {
                    Accent::Art => "album cover",
                    Accent::Track => "track (random)",
                    Accent::Fixed => "fixed blue",
                }
                .into(),
            ),
            Row::Numbering => (
                "Numbers".into(),
                match self.numbering {
                    Numbering::Auto => "auto (album: track no. · playlist: position)",
                    Numbering::Position => "position in list",
                    Numbering::Off => "off",
                }
                .into(),
            ),
            Row::TArtist => ("Artist".into(), b(self.t_artist)),
            Row::TAlbum => ("Album".into(), b(self.t_album)),
            Row::TYear => ("Year".into(), b(self.t_year)),
            Row::TAdded => ("Date added".into(), b(self.t_added)),
            Row::TLength => ("Length".into(), b(self.t_length)),
            Row::AArtist => ("Artist".into(), b(self.a_artist)),
            Row::AYear => ("Year".into(), b(self.a_year)),
            Row::ATracks => ("Track count".into(), b(self.a_tracks)),
            Row::AAdded => ("Date added".into(), b(self.a_added)),
            Row::RGenres => ("Genres".into(), b(self.r_genres)),
            Row::POwner => ("Owner".into(), b(self.p_owner)),
            Row::PCount => ("Track count".into(), b(self.p_count)),
            Row::External => ("External sources (ListenBrainz)".into(), b(self.external)),
            Row::Root(i) => (ROOT_TOGGLES[i].1.to_string(), b(!self.hidden.contains(ROOT_TOGGLES[i].0))),
        }
    }

    /// Change a setting: toggles booleans, cycles enums by `dir` (+1/-1).
    pub fn change(&mut self, r: Row, dir: i32) {
        let cyc = |n: usize, cur: usize| ((cur as i32 + dir).rem_euclid(n as i32)) as usize;
        match r {
            Row::Title(_) => {}
            Row::Viz => self.viz = !self.viz,
            Row::Accent => {
                let all = [Accent::Art, Accent::Track, Accent::Fixed];
                self.accent = all[cyc(3, all.iter().position(|a| *a == self.accent).unwrap_or(0))];
            }
            Row::Numbering => {
                let all = [Numbering::Auto, Numbering::Position, Numbering::Off];
                self.numbering = all[cyc(3, all.iter().position(|a| *a == self.numbering).unwrap_or(0))];
            }
            Row::TArtist => self.t_artist = !self.t_artist,
            Row::TAlbum => self.t_album = !self.t_album,
            Row::TYear => self.t_year = !self.t_year,
            Row::TAdded => self.t_added = !self.t_added,
            Row::TLength => self.t_length = !self.t_length,
            Row::AArtist => self.a_artist = !self.a_artist,
            Row::AYear => self.a_year = !self.a_year,
            Row::ATracks => self.a_tracks = !self.a_tracks,
            Row::AAdded => self.a_added = !self.a_added,
            Row::RGenres => self.r_genres = !self.r_genres,
            Row::POwner => self.p_owner = !self.p_owner,
            Row::PCount => self.p_count = !self.p_count,
            Row::External => self.external = !self.external,
            Row::Root(i) => {
                let id = ROOT_TOGGLES[i].0.to_string();
                if !self.hidden.remove(&id) {
                    self.hidden.insert(id);
                }
            }
        }
    }

    pub fn load() -> Settings {
        let mut s = Settings::default();
        let Ok(text) = fs::read_to_string(path()) else { return s };
        for line in text.lines() {
            let line = line.trim();
            if line.starts_with('#') {
                continue;
            }
            let Some((k, v)) = line.split_once('=') else { continue };
            let (k, v) = (k.trim(), v.trim().trim_matches('"'));
            let b = v == "true";
            match k {
                "viz" => s.viz = b,
                "accent" => s.accent = match v { "track" => Accent::Track, "fixed" => Accent::Fixed, _ => Accent::Art },
                "numbering" => s.numbering = match v { "position" => Numbering::Position, "off" => Numbering::Off, _ => Numbering::Auto },
                "track_artist" => s.t_artist = b,
                "track_album" => s.t_album = b,
                "track_year" => s.t_year = b,
                "track_added" => s.t_added = b,
                "track_length" => s.t_length = b,
                "album_artist" => s.a_artist = b,
                "album_year" => s.a_year = b,
                "album_tracks" => s.a_tracks = b,
                "album_added" => s.a_added = b,
                "artist_genres" => s.r_genres = b,
                "playlist_owner" => s.p_owner = b,
                "playlist_count" => s.p_count = b,
                "external" => s.external = b,
                "hidden" => s.hidden = v.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect(),
                _ => {}
            }
        }
        s
    }

    pub fn save(&self) {
        if cfg!(test) {
            return; // tests must never touch the user's real files
        }
        let mut hidden: Vec<&String> = self.hidden.iter().collect();
        hidden.sort();
        let hidden = hidden.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(",");
        let acc = match self.accent { Accent::Art => "art", Accent::Track => "track", Accent::Fixed => "fixed" };
        let num = match self.numbering { Numbering::Auto => "auto", Numbering::Position => "position", Numbering::Off => "off" };
        let text = format!(
            "# rola settings. Edit here or press `,` inside rola.\n\
             viz = {}\naccent = \"{acc}\"          # art | track | fixed\nnumbering = \"{num}\"      # auto | position | off\n\n\
             track_artist = {}\ntrack_album = {}\ntrack_year = {}\ntrack_added = {}\ntrack_length = {}\n\n\
             album_artist = {}\nalbum_year = {}\nalbum_tracks = {}\nalbum_added = {}\n\n\
             artist_genres = {}\n\nplaylist_owner = {}\nplaylist_count = {}\n\n\
             external = {}\nhidden = \"{hidden}\"   # comma-separated main-menu ids\n",
            self.viz, self.t_artist, self.t_album, self.t_year, self.t_added, self.t_length, self.a_artist, self.a_year,
            self.a_tracks, self.a_added, self.r_genres, self.p_owner, self.p_count, self.external
        );
        let p = path();
        if let Some(d) = p.parent() {
            let _ = fs::create_dir_all(d);
        }
        let _ = fs::write(p, text);
    }
}

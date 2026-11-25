#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Header,
    Section,
    Playlist,
    Album,
    Artist,
    Track,
}

/// One row in any column: a track, album, artist, playlist or menu entry.
///
/// `id` is a typed key that doubles as the cache key and (for Spotify things) the URI source:
///   `tr:<id>` track   `al:<id>` album   `ar:<id>` artist   `pl:<id>` playlist
///   `sec:<name>` menu folder   `hdr:<name>` non-selectable header   `q:<scope>:<query>` search
///   `top:<tracks|artists>:<range>`   `chart:<kind>:<arg>` (ListenBrainz)   `hist:<log|top>` (local)
///   `award:<name>` (bundled data)   `xa:<artist>` / `xl:<artist>|<album>` / `xt:<artist>|<title>`
///   = "external": named things from outside sources, resolved to Spotify by search when opened.
#[derive(Clone, Debug)]
pub struct Item {
    pub id: String,
    pub kind: Kind,
    pub name: String,
    /// Tracks: "Artist · Album". Albums: artist. Playlists: owner. Used for display, filter, sort.
    pub sub: String,
    pub dur_ms: u32,
    pub date: String,   // release date (YYYY[-MM-DD]) when known
    pub added: String,  // when added to your library/playlist (ISO date), when known
    pub num: u32,       // track number within its album (0 = unknown)
    pub total: u32,     // number of tracks in an album/playlist (0 = unknown)
    pub genres: String, // artists: first genres, comma separated
    pub pos: u32,       // 1-based position in the list as loaded (before any sorting)
}

impl Item {
    pub fn is_container(&self) -> bool {
        self.kind != Kind::Track
    }
    /// spotify:<type>:<id> for ids shaped like "tr:<id>", "pl:<id>", "al:<id>", "ar:<id>".
    pub fn uri(&self) -> Option<String> {
        let (p, id) = self.id.split_once(':')?;
        let ty = match p {
            "tr" => "track",
            "pl" => "playlist",
            "al" => "album",
            "ar" => "artist",
            _ => return None,
        };
        Some(format!("spotify:{ty}:{id}"))
    }
}

pub enum Load {
    Ready(Vec<Item>),
    Loading(Vec<Item>), // partial results so far
    Failed(String),
}

pub trait Source {
    fn root(&self) -> Vec<Item>;
    fn children(&self, it: &Item) -> Load;
    /// Drop cached data so the next read refetches.
    fn invalidate(&self) {}
}

pub fn mk(id: impl Into<String>, kind: Kind, name: &str, sub: &str) -> Item {
    Item {
        id: id.into(),
        kind,
        name: name.into(),
        sub: sub.into(),
        dur_ms: 0,
        date: String::new(),
        added: String::new(),
        num: 0,
        total: 0,
        genres: String::new(),
        pos: 0,
    }
}
pub fn hdr(n: &str) -> Item {
    mk(format!("hdr:{n}"), Kind::Header, n, "")
}
pub fn sec(id: &str, n: &str) -> Item {
    mk(format!("sec:{id}"), Kind::Section, n, "")
}

/// The first column, shared by every source.
pub fn root_items() -> Vec<Item> {
    vec![
        hdr("Now"),
        sec("queue", "Now playing · Queue"),
        hdr("Your library"),
        sec("playlists", "Playlists"),
        sec("albums", "Albums"),
        sec("artists", "Artists"),
        sec("liked", "Liked songs"),
        sec("recent", "Recently played"),
        sec("stats", "History (local)"),
        hdr("Discover"),
        sec("fresh", "New from your artists"),
        sec("awards", "Awards & lists"),
        sec("acclaim", "Acclaim · deadwax"),
        sec("charts", "Charts · ListenBrainz"),
        sec("toptracks", "Your top tracks"),
        sec("topartists", "Your top artists"),
        hdr("Search"),
        sec("search:tracks", "Tracks"),
        sec("search:albums", "Albums"),
        sec("search:artists", "Artists"),
        sec("search:playlists", "Playlists"),
    ]
}

pub fn charts_items() -> Vec<Item> {
    [("artists:week", "Trending artists · week"), ("artists:month", "Trending artists · month"), ("new:14", "Hot new releases · 2 weeks")]
        .iter()
        .map(|(k, n)| mk(format!("chart:{k}"), Kind::Section, n, ""))
        .collect()
}

pub fn stats_items() -> Vec<Item> {
    [("log", "Full history"), ("top", "Most played")].iter().map(|(k, n)| mk(format!("hist:{k}"), Kind::Section, n, "")).collect()
}

pub const RANGES: [(&str, &str); 3] = [("short_term", "Last 4 weeks"), ("medium_term", "Last 6 months"), ("long_term", "All time")];

pub fn top_ranges(kind: &str) -> Vec<Item> {
    RANGES.iter().map(|(k, n)| mk(format!("top:{kind}:{k}"), Kind::Section, n, "")).collect()
}

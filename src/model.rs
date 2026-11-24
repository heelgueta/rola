#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Header,
    Section,
    Playlist,
    Album,
    Artist,
    Track,
}

#[derive(Clone, Debug)]
pub struct Item {
    pub id: String,
    pub kind: Kind,
    pub name: String,
    pub sub: String,
    pub dur_ms: u32,
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
    Item { id: id.into(), kind, name: name.into(), sub: sub.into(), dur_ms: 0 }
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
        hdr("Discover"),
        sec("fresh", "New from your artists"),
        sec("toptracks", "Your top tracks"),
        sec("topartists", "Your top artists"),
        hdr("Search"),
        sec("search:tracks", "Tracks"),
        sec("search:albums", "Albums"),
        sec("search:artists", "Artists"),
        sec("search:playlists", "Playlists"),
    ]
}

pub const RANGES: [(&str, &str); 3] = [("short_term", "Last 4 weeks"), ("medium_term", "Last 6 months"), ("long_term", "All time")];

pub fn top_ranges(kind: &str) -> Vec<Item> {
    RANGES.iter().map(|(k, n)| mk(format!("top:{kind}:{k}"), Kind::Section, n, "")).collect()
}

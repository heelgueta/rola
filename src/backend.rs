//! Mock library so the UI can be built and felt before the Spotify API is wired in.
use crate::model::*;

const ARTISTS: &[(&str, &[&str])] = &[
    ("Korn", &["Untouchables", "Issues", "Follow the Leader"]),
    ("Faith No More", &["Angel Dust", "King for a Day"]),
    ("Muse", &["Absolution", "Origin of Symmetry", "Drones"]),
    ("In Flames", &["Reroute to Remain", "Clayman"]),
    ("Christopher Larkin", &["Silksong OST", "Hollow Knight OST"]),
    ("Mägo de Oz", &["Gaia", "Finisterra"]),
];
const PLAYLISTS: &[(&str, usize)] = &[("Liked Songs", 1), ("Heavy rotation", 3), ("Late night", 5), ("Discover Weekly", 7)];
const FEATURED: &[(&str, usize)] = &[("Today's Top Hits", 2), ("Rock Classics", 4), ("Metal Essentials", 6), ("Deep Focus", 9)];
const W1: &[&str] = &["Dead", "Blind", "Coming", "Freak", "Hollow", "Lost", "Silent", "Burning", "Angel", "Falling", "Sweet", "Broken"];
const W2: &[&str] = &["Verdania", "Leash", "Undone", "Hysteria", "Symmetry", "Machine", "Dust", "Heaven", "Skin", "Radio", "Orbit", "Light"];

fn h(s: &str) -> u64 {
    let mut x: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        x ^= b as u64;
        x = x.wrapping_mul(0x100000001b3);
    }
    x
}

pub struct Mock;

impl Mock {
    fn tracks_of(&self, ar: usize, al: usize) -> Vec<Item> {
        let (artist, albums) = ARTISTS[ar];
        let album = albums[al];
        (0..8)
            .map(|t| {
                let id = format!("tr:{ar}:{al}:{t}");
                let x = h(&id);
                Item {
                    name: format!("{} {}", W1[(x % 12) as usize], W2[((x >> 8) % 12) as usize]),
                    sub: format!("{artist} · {album}"),
                    dur_ms: 150_000 + (x >> 16) as u32 % 160_000,
                    kind: Kind::Track,
                    date: format!("{}", 1994 + (x >> 24) % 30),
                    added: String::new(),
                    num: t as u32 + 1,
                    total: 0,
                    genres: String::new(),
                    pos: 0,
                    id,
                }
            })
            .collect()
    }

    fn all_tracks(&self) -> Vec<Item> {
        let mut v = vec![];
        for (ar, (_, albums)) in ARTISTS.iter().enumerate() {
            for al in 0..albums.len() {
                v.extend(self.tracks_of(ar, al));
            }
        }
        v
    }
    fn album_item(&self, ar: usize, al: usize) -> Item {
        let (artist, albums) = ARTISTS[ar];
        mk(format!("al:{ar}:{al}"), Kind::Album, albums[al], artist)
    }
    fn all_albums(&self) -> Vec<Item> {
        (0..ARTISTS.len()).flat_map(|a| (0..ARTISTS[a].1.len()).map(move |l| (a, l))).map(|(a, l)| self.album_item(a, l)).collect()
    }
    fn artist_items(&self) -> Vec<Item> {
        ARTISTS.iter().enumerate().map(|(i, (n, _))| mk(format!("ar:{i}"), Kind::Artist, n, "")).collect()
    }
    fn playlist_items(list: &[(&str, usize)]) -> Vec<Item> {
        list.iter().map(|(n, k)| mk(format!("pl:{k}"), Kind::Playlist, n, "")).collect()
    }

    fn children_vec(&self, it: &Item) -> Vec<Item> {
        let p: Vec<&str> = it.id.splitn(3, ':').collect();
        match p[0] {
            "award" => crate::awards::items(&it.id),
            "q" => self.search(p[1], p.get(2).copied().unwrap_or("")),
            "top" => match p[1] {
                "tracks" => self.all_tracks().into_iter().step_by(4).take(25).collect(),
                _ => self.artist_items(),
            },
            "sec" => match p[1] {
                "playlists" => Self::playlist_items(PLAYLISTS),
                "stats" => stats_items(),
                "awards" => crate::awards::folder_items(),
                "charts" => charts_items(),
                "fresh" => self.all_albums().into_iter().rev().take(8).collect(),
                "toptracks" => top_ranges("tracks"),
                "topartists" => top_ranges("artists"),
                "albums" => self.all_albums(),
                "artists" => self.artist_items(),
                "liked" => self.all_tracks(),
                "recent" => self.all_tracks().into_iter().rev().step_by(3).take(30).collect(), // newest first
                _ => vec![],
            },
            "pl" => {
                let stride: usize = p[1].parse().unwrap_or(1);
                self.all_tracks().into_iter().step_by(stride).collect()
            }
            "ar" => {
                let a: usize = p[1].parse().unwrap();
                (0..ARTISTS[a].1.len()).map(|l| self.album_item(a, l)).collect()
            }
            "al" => self.tracks_of(p[1].parse().unwrap(), p[2].parse().unwrap()),
            _ => vec![],
        }
    }

    pub fn search(&self, scope: &str, q: &str) -> Vec<Item> {
        let q = q.to_lowercase();
        let pool = match scope {
            "tracks" => self.all_tracks(),
            "albums" => self.all_albums(),
            "artists" => self.artist_items(),
            _ => Self::playlist_items(PLAYLISTS).into_iter().chain(Self::playlist_items(FEATURED)).collect(),
        };
        pool.into_iter().filter(|i| i.name.to_lowercase().contains(&q) || i.sub.to_lowercase().contains(&q)).take(50).collect()
    }
}

impl Source for Mock {
    fn root(&self) -> Vec<Item> {
        root_items()
    }
    fn children(&self, it: &Item) -> Load {
        Load::Ready(self.children_vec(it))
    }
}

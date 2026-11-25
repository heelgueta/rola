# 🎵 rola

A keyboard-first Spotify TUI with Miller columns, inspired by [`spotify-tui`](https://github.com/Rigellute/spotify-tui) but adapted to personal taste, and built around the same yazi-style navigation as [`pato`](https://github.com/heelgueta/pato).

Browse your playlists, albums, artists, liked songs and recent plays in `h` `j` `k` `l` panes, search, and control playback, with a generative, track-seeded visualizer in the now-playing bar.

> **rola** is slang for "a song" in Mexico and Argentina.

---

## How it works

`rola` is a **remote control**. It browses your library through the Spotify Web API and tells an already-running Spotify Connect device (e.g. the Spotify desktop app) what to play. It does not stream audio itself: Spotify refuses third-party streaming engines (librespot) on some accounts, so this is the dependable route. **Spotify Premium is required** for playback control.

## Installation

```bash
git clone https://github.com/heelgueta/rola.git
cd rola
cargo build --release
install -m 755 target/release/rola ~/.local/bin/rola
```

### Authentication

1. Create an app at <https://developer.spotify.com/dashboard>
   - Redirect URI: `http://127.0.0.1:8888/callback` (exactly)
   - APIs used: **Web API**
2. Save the app's **Client ID** (no secret needed, it uses PKCE):
   ```bash
   mkdir -p ~/.config/rola && echo "YOUR_CLIENT_ID" > ~/.config/rola/client_id
   ```
3. Run `rola login`, approve in the browser, then just run `rola`.

Keep the Spotify app open (or any Connect device) so `rola` has somewhere to play.

`rola --demo` runs against built-in mock data, no account needed.

## Keybindings

| Key | Action |
| --- | --- |
| `h` `j` `k` `l` / Arrows | Move; step out (`h`), in (`l` / `Enter`) |
| `gg` / `G` | Top / bottom |
| `Ctrl-d` / `Ctrl-u` | Half page down / up |
| `Enter` | Open container, play track from here, or start a search |
| `P` | Play the whole selected playlist / album / list |
| `/` | Fuzzy filter the current column (`Esc` clears) |
| `s` / `S` | Cycle sort (default, name, artist, length, released, added) / reverse. Remembered per list |
| `,` | Settings menu |
| `space` | Play / pause |
| `n` / `p` | Next / previous |
| `<` / `>` | Seek -10s / +10s |
| `+` / `-` | Volume |
| `R` | Refresh from Spotify |
| `?` | Help |
| `q` | Quit |

The first column holds everything: now playing and queue, your library (playlists, albums, artists, liked songs, recently played), discovery (new from your artists, your top tracks and artists), and search (tracks, albums, artists, playlists).

## Settings

Press `,` for the settings menu (also editable in `~/.config/rola/settings.conf`):

- **Rows:** which fields tracks, albums, artists and playlists show (artist, album, year, date added, length, track count, genres, owner), and **numbers** (album track number, playlist position, or off).
- **Look:** visualizer on/off; accent color from the album cover, a per-track hue, or a fixed blue.
- **Main menu:** hide any entry (Discover, Charts, Search, ...) or turn off external sources entirely.

## Awards & lists

Under Discover: Grammy Album / Record / Song of the Year and the Mercury Prize, by year, newest first. Open an album winner to browse it on Spotify; press `Enter` or `P` on a song list to play the winners (each matched on Spotify by search). The data is bundled (`data/awards.tsv`) and regenerated with `python3 tools/gen_awards.py` (source: Wikipedia, CC BY-SA). Years are ceremony years.

## Acclaim: what the critics think

`rola` carries a bundled slice of deadwax, a research project that scores albums by fitting a latent model to ~96 critics (Wikipedia ratings boxes, Pitchfork, MusicBrainz users). Any album row it recognizes shows a **z-score** (`+2.6σ` = 2.6 standard deviations above the average album; percentile optional in settings). Press `i` for the card (z, percentile, honors, where it sits among 64k albums); press `s` to sort a list by acclaim.

Under Discover › *Acclaim · deadwax*: *Next up for you*, *Missing from your shelf*, *Crate dig · tonight* (five records from five decades, `R` re-deals), *Sure things*, *Under the radar*, *Split decisions*, and genre and decade browsers. Lists rank by a conservative lower bound (z minus one standard deviation of uncertainty), so they are short and sciencey rather than endless. Coverage is the critics' sample: ~19.5k albums, thin on mainstream pop and non-English music. Data: ratings published on Wikipedia (CC BY-SA), MusicBrainz (CC0) and other critic sources, reduced to one derived score per album; the bundle holds no individual critic's ratings.

## Beyond the API

Spotify's API is stingy, so `rola` fills the gaps itself:

- **Disk cache** (`~/.cache/rola/`): everything you browse is saved, so lists open instantly next time and refresh in the background.
- **Local history** (`~/.local/share/rola/history.jsonl`): Spotify only exposes your last 50 plays; `rola` logs every track you play through it (after 20s), giving a full history and a **Most played** ranking.
- **Charts** from [ListenBrainz](https://listenbrainz.org) (public API, no account, nothing about you is sent): trending artists and hot new releases, resolved to Spotify when you open them. These reflect ListenBrainz's own community, so expect them to skew niche.

## Files rola uses

| Path | What |
| --- | --- |
| `~/.config/rola/client_id`, `auth.json` | your Spotify Client ID and login token (`auth.json` is owner-only) |
| `~/.config/rola/settings.conf` | settings |
| `~/.cache/rola/cache.json` | disk cache |
| `~/.local/share/rola/history.jsonl` | local listening log |
| `~/.local/share/rola/state.json` | remembered sort per list |

See [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) for how it all fits together.

## Visuals and terminals

The ribbons are drawn with braille characters and truecolor, colored from the album cover. Use a truecolor terminal (iTerm2, Ghostty, kitty, WezTerm). Apple's Terminal.app falls back to 256 colors and shows `256c` in the corner.

## Spotify API limits

Spotify's development-mode API is restrictive (see their [February 2026 changes](https://developer.spotify.com/documentation/web-api/tutorials/february-2026-migration-guide)). `rola` works within them:

- Tracks of playlists you don't own can't be listed; `P` still plays them.
- Spotify's own algorithmic playlists (Discover Weekly, Daily Mix, ...) are not exposed to the API.
- No new-releases, categories or artist top-tracks endpoints; there is no audio analysis, so the visualizer is generative rather than beat-synced.
- Search and artist-album endpoints return at most 10 results per request, so `rola` pages through them (search shows about 30).

## Status

Early: `v0.0.1` is released; `main` has fixes for pause/next/previous (`411`), opening artists (`400`), plus sorting, the cache, local history and charts.

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
| `space` | Play / pause |
| `n` / `p` | Next / previous |
| `<` / `>` | Seek -10s / +10s |
| `+` / `-` | Volume |
| `v` | Cycle visualizer: ribbons, plasma, spectrum |
| `R` | Refresh from Spotify |
| `?` | Help |
| `q` | Quit |

The first column holds everything: now playing and queue, your library (playlists, albums, artists, liked songs, recently played), discovery (new from your artists, your top tracks and artists), and search (tracks, albums, artists, playlists).

## Visuals and terminals

The ribbons are drawn with braille characters and truecolor. Use a truecolor terminal (iTerm2, Ghostty, kitty, WezTerm). Apple's Terminal.app falls back to 256 colors and shows `256c` in the corner.

## Spotify API limits

Spotify's development-mode API is restrictive (see their [February 2026 changes](https://developer.spotify.com/documentation/web-api/tutorials/february-2026-migration-guide)). `rola` works within them:

- Tracks of playlists you don't own can't be listed; `P` still plays them.
- Spotify's own algorithmic playlists (Discover Weekly, Daily Mix, ...) are not exposed to the API.
- No new-releases, categories or artist top-tracks endpoints; there is no audio analysis, so the visualizer is generative rather than beat-synced.
- Search returns at most 10 results per request.

## Status

Early: `v0.0.1`. It works, but expect rough edges.

# Changelog

All notable changes to rola. Newest first.

## 0.1.0

### Added
- **Acclaim** (Discover › *Acclaim · deadwax*): what a panel of ~96 critics thinks of an album, from the deadwax research project (a one-factor latent model over Wikipedia ratings boxes, Pitchfork and MusicBrainz users, validated at r = .89 against Metacritic). 19.5k albums rated by 3+ critics are bundled in `data/acclaim.tsv` (`tools/export_acclaim.py` regenerates it).
  - **Scores on every album row**, wherever it comes from (library, search, artist pages, awards): z-score (`+2.6σ`), percentile, or both (setting *Critics' score*). Flags: `★` major prize, `☰` on "greatest albums" lists, `≠` critics split.
  - **Sort by acclaim** (`s`): your saved albums, best first, by the conservative score; the notice tells you how many the panel knows and your library's mean σ.
  - **The card** (`i`): z, percentile overall and in genre, the conservative "sure score", how much the critics agree, honors, and a histogram of all 64k albums marking where this one sits.
  - **Lenses**, ranked by a conservative *lower bound* (z minus one posterior sd, so a record with three lucky reviews can't beat one with ten solid ones) and never more than 150 long:
    - *Next up for you*: acclaimed records by artists you don't know, tilted toward the genres you actually play, away from canon-list and prize favourites and the most-reviewed.
    - *Missing from your shelf*: acclaimed records by artists you already own, that you don't.
    - *Crate dig · tonight*: five records from five different decades, drawn at random (weighted by acclaim and your taste) from what you haven't heard. Same crate all day; `R` deals a new one.
    - *Sure things*, *Under the radar* (acclaimed, with no prize or list on record), *Split decisions* (high average, critics far apart), *By genre*, *By decade*.
  - Personal lenses read your saved albums, top artists and local history; nothing leaves your machine.

### Changed
- `R` also re-deals the crate dig. The first column gained *Acclaim · deadwax* (hide it in settings if unwanted).

### Earlier, unreleased until now

### Fixed
- Pause was hit-and-miss and sometimes failed with `403 Restriction violated`. Space now asks Spotify what it is doing before choosing pause/resume, treats "already in that state" as success, and the UI keeps its own state for 1.5 s so a stale poll can't flip it back.
- Sorting was inconsistent: a list's sort was lost on leaving it, and the parent's preview showed a different order than the list itself. Sort is now remembered **per list** (and saved between sessions) and applied everywhere that list appears. The cursor is remembered per list by item, so it survives re-sorting.

### Added
- **Settings menu** (`,`): choose which columns rows show for tracks (artist, album, year, date added, length), albums (artist, year, track count, date added), artists (genres) and playlists (owner, track count); numbering; accent color source; and which main-menu entries and sources are visible. Saved to `~/.config/rola/settings.conf`.
- **Numbers**: album track numbers, playlist positions (stable when sorted), or off.
- **Awards & lists**: Grammy Album / Record / Song of the Year and the Mercury Prize, by year, built from Wikipedia (`tools/gen_awards.py`) and bundled. Albums open resolved on Spotify; song lists play by matching each track on Spotify.
- **Colors from the album cover**: accent and ribbon hues come from the cover art (lightness is pinned so text always stays readable).
- The header always shows the current sort (`sort: name ↓`); changing it also shows a notice.
- `docs/ARCHITECTURE.md`, and a pass of code comments.

### Changed
- Only the ribbon visualizer is shown (the `v` key is gone); plasma and spectrum are parked in the code. The visualizer can be turned off in settings.
- Albums now show track count, year and genres where enabled; tracks without a length (award lists) no longer show `0:00`.

## 0.0.2

### Fixed
- Pause / next / previous failed with `411 Length Required`: body-less `PUT`/`POST` requests now send an explicit `Content-Length: 0`.
- Opening an artist failed with `400 Invalid limit`: Spotify now caps the artist-albums endpoint at 10 per page, so rola pages by 10.

### Added
- Sorting: `s` cycles (default, name, artist, length, released, added), `S` reverses. The cursor stays on the same item.
- Disk cache (`~/.cache/rola/cache.json`): lists open instantly and refresh in the background (stale-while-revalidate).
- Local listening log (`~/.local/share/rola/history.jsonl`): full history and a "Most played" ranking, since Spotify only exposes the last 50 plays.
- Charts from ListenBrainz (trending artists, hot new releases), resolved to Spotify when opened.
- Albums and tracks show their artist (and album year) next to the name.

## 0.0.1

First working release: Miller-column browsing of playlists, albums, artists, liked songs, recent plays, top items and search; PKCE login; playback control of the active Spotify Connect device; generative visualizer (ribbons, plasma, spectrum).

# Changelog

All notable changes to rola. Newest first.

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

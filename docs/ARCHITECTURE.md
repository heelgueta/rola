# rola architecture

A map for future-us. Read this first, then the module docs at the top of each file.

## The big picture

```
 keys ──▶ app.rs (state + input) ──▶ ui.rs (draw, pure function of &App)
              │  ▲
              │  └── Event::State/Error/Info (mpsc) ◀── spotify.rs  Player worker thread
              │                                           ├─ polls GET /me/player every ~3 s
              ▼                                           └─ runs playback commands (Cmd)
        Source trait (model.rs)
         ├─ spotify.rs  Live   : cached, background fetch threads, ListenBrainz, award/external resolvers
         └─ backend.rs  Mock   : demo data for `rola --demo` and tests
```

rola is a **remote control**: it browses via the Spotify Web API and tells whichever Spotify
Connect device is active (e.g. Spotify.app) what to play. It never streams audio.

## Modules

| File | Role |
| --- | --- |
| `main.rs` | startup (login if needed), terminal setup, the 30 fps loop; headless render tests |
| `app.rs` | `App` state, key handling, sort/cursor memory, playback intents. No drawing. |
| `ui.rs` | all drawing: columns, rows (configurable), now-playing bar, overlays |
| `model.rs` | `Item`, `Kind`, the `Source` trait, `Load`, the main-menu definition |
| `spotify.rs` | `Api` (HTTP + token refresh + retry), `Live` source + cache, `Player` worker |
| `auth.rs` | OAuth Authorization-Code + PKCE (browser login, local callback on :8888, token refresh) |
| `settings.rs` | settings struct, file load/save, the settings-menu rows |
| `art.rs` | album-cover palette (dominant vivid hues) from Spotify's CDN |
| `viz.rs` | the ribbon visualizer (+ parked plasma/spectrum), truecolor/256-color handling |
| `history.rs` | local listening log (full history, most played) |
| `awards.rs` | bundled award winners (`data/awards.tsv`), turned into `Item`s |
| `acclaim.rs` | bundled critic scores (`data/acclaim.tsv`): matching albums by normalized artist+title, row cells, the card, the Acclaim lenses and the listener `Profile` |
| `backend.rs` | the mock source |
| `tools/gen_awards.py` | regenerates `data/awards.tsv` from Wikipedia |
| `tools/export_acclaim.py` | exports `data/acclaim.tsv` from a deadwax database |

## Data flow and caching

`Source::children(&Item) -> Load` where `Load` is `Ready | Loading(partial) | Failed`. A live
fetch runs on a background thread and publishes partial pages into a shared cache, so the UI
shows rows immediately and fills in. `App::reload()` re-pulls every ~150 ms for any column
still loading.

The cache (`Live.cache`) is also written to `~/.cache/rola/cache.json` after each finished list.
On the next launch those entries load as `stale`: shown instantly, refetched in the background
(stale-while-revalidate). TTLs: queue 3 s, recent 20 s, everything else 5 min.

## Ids

`Item.id` is a typed key (see the doc comment on `Item`). It is the cache key, the source of the
Spotify URI (`Item::uri`), and the key for sort/cursor memory. "External" ids (`xa:`, `xl:`,
`xt:`) name things from non-Spotify sources (ListenBrainz, award lists) by artist/title; they are
resolved to Spotify by search only when opened (`fetch` in spotify.rs) or played (`exec` in the
`Player` worker, with a small delay between searches to respect dev-mode rate limits).

## Acclaim

`acclaim::of(&Item)` scores any album row by matching normalized artist + title (brackets and
"- Remastered" tails stripped), memoized per item id. It is used by `ui::right_text`, the
"acclaim" sort (`app::order`) and the card (`i`). Lens ids are `acc:*`; `Live::acclaim` builds the
personal ones (`next`, `shelf`, `dig`) from `sec:albums`, top artists and local history through the
normal cache, so they stay `Loading` until those are complete. Ranking is by `lb` (z minus one
posterior sd). `dig` is seeded by the day plus a counter that `R` bumps (`acclaim::reroll`).

## Consistency rules (sorting and cursor)

* Sort mode is stored **per container id** (`App.prefs`, persisted in `~/.local/share/rola/state.json`)
  and applied wherever that container is listed: as a column *and* as its parent's preview.
* The cursor is stored per container as the selected **item id** (`App.cursor`, session only),
  not an index, so it survives re-sorting and progressive loading (`Column.want`).
* `Item.pos` is the 1-based position as loaded, assigned before sorting, so playlist numbering
  stays meaningful when the view is sorted.
* Playing from a playlist/album uses Spotify's *context* only when the view is in natural order
  and unfiltered; otherwise it sends explicit URIs so what you hear matches what you see.

## Playback robustness

* `Cmd::Toggle` asks Spotify what it is doing *now* before choosing pause/resume. The UI also
  holds its optimistic state for 1.5 s (`App.hold_until`) so a stale poll can't flip it back.
* Pause/resume "403 Restriction violated" means the goal state already holds: treated as success.
* Body-less `PUT`/`POST` send `Content-Length: 0` (Spotify answers 411 otherwise).
* No active device (404) triggers a device pick: prefers a Computer that isn't `spotatui`.

## Spotify API limits that shape the design (dev-mode apps, 2026)

* Playlist contents only for playlists you own/collaborate on; Spotify-made playlists are not returned.
* No new-releases, categories, artist top-tracks, audio analysis/features, recommendations.
* Search and artist-albums endpoints cap at 10 per request: we page.
* Quotas are per developer account: cache aggressively, load lazily.

## Colors

Text uses three fixed greys (`bright`, `soft`, `faint`) plus one accent. The accent *hue* comes
from the album cover (`art.rs`), but saturation and lightness are pinned in `ui.rs`, so any
cover is readable. Setting `accent` can switch to a per-track hue or a fixed blue. Without
truecolor (Apple Terminal) every color is mapped to the nearest xterm-256 entry (`viz::rgb`).

## Testing

* `cargo test` runs headless: render tests via ratatui's `TestBackend`, sort/cursor/settings
  logic, and the palette extractor. No network.
* `cargo test -- --ignored --nocapture` runs the live tests against your real account (read-only
  except `live_bodyless_put`, which sets volume to its current value). They need a saved login.

## Recipes

* **New main-menu entry:** add it to `model::root_items`, handle its id in `Live::children`
  (and `Mock::children_vec`), add it to `settings::ROOT_TOGGLES` so it can be hidden.
* **New row field:** add a field to `Item`, fill it in the parsers (`spotify.rs`), persist it in
  the cache arrays (`persist`/`load_disk`), show it in `ui::secondary`/`right_text`, add a
  setting in `settings.rs` (struct, `rows`, `label`, `change`, `load`, `save`).
* **New outside source:** fetch with `Api::get_public`, emit `x*:` items, add a resolver arm in
  `fetch`. Keep it behind the `external` setting.

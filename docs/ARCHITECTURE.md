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

        acclaim.rs (bundled data/acclaim.tsv) ──▶ deadwax.rs Client (optional live server, /api/v1/)
                                                    background jobs · caches · circuit breaker
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
| `acclaim.rs` | bundled critic scores (`data/acclaim.tsv`): matching albums by normalized artist+title, row cells, the card's core block, the Acclaim lenses and the listener `Profile` |
| `deadwax.rs` | optional live deadwax client: probe, batch resolve, album/artist detail, live lenses, settings gates, the thread-local handle, a scripted fake for tests |
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

## Live deadwax

deadwax (the research project behind the bundle) can run as a server with a versioned API
(`/api/v1/`, contract in deadwax's `docs/API.md`). `deadwax.rs` makes it an optional layer on top of
the bundle. The bundle stays the source of truth offline and is never replaced, only extended.

**Lifecycle.** `App::new` builds a `deadwax::Client` from the settings. In the real app (not
`--demo`, not `cargo test`) it calls `start()`: one `GET {deadwax_url}/api/v1/meta` on a background
thread, timeout 150 ms for loopback, 1.5 s for remote URLs. `schema_version` must be 1. On failure the
client is `Down` for the session and every gate answers false: no requests, no toasts, no live
lenses, no settings rows. `R` re-probes only when the user set a non-default URL (or the server had
answered before). `deadwax_url = "off"` (or `""`) builds a client with no transport at all.

**Gates.** `on()` = `deadwax_live` and connected; `resolve_on` / `cards_on` / `lenses_on` add their
toggle and a server capability (`lookup`, `album_detail`, `presets`). Individual features also check
capabilities: artist cards need `artists`, the live lenses `crowd` / `pitchfork` / `presets`, the
`o` deep link `web_deeplink`. A public deadwax (`DEADWAX_PUBLIC=1`) lacks the restricted ones, so
those features quietly disappear.

**Never on the UI thread.** The UI only reads `Shared` (behind a mutex). Every request is a job on
its own thread (`spawn`; tests run jobs inline with `sync`). Album detail, artist detail and presets
are `Fetch::{Pending, Ready, Missing, Failed}` entries filled by those jobs; the card redraws each
frame, so it hydrates by itself.

**64k rows.** `acclaim::of(item)` checks the bundle (memoized per item id) and then, if
`resolve_on`, the live record cache by normalized `artist|title` key. Live records are leaked to
`&'static Rec` once per album, so they plug into everything that takes bundle records (rows, sort,
card core, `Profile`). `App` calls `acclaim::want_live(items)` whenever a column or preview gets
items: bundle misses are queued (deduplicated), and one resolver drains the queue in POSTs of ≤ 200
normalized pairs (Spotify's `A, B` credits send `A, B` and `A`). Hits and misses are cached;
transport failures leave keys unknown so they are retried after recovery.

**Circuit breaker.** Three consecutive transport failures (or 5xx/429) while connected set `Down`
with a retry time; `App::tick` calls `Client::tick`, which half-opens with one probe after the
backoff (30 s, doubling to 5 min).

**Thread-local handle.** `acclaim::of` runs inside sorting (`Column::view`) and drawing, with no
`App` at hand, so the client is also installed in a thread-local (`deadwax::install` / `with`).
Production has one UI thread; each test thread installs its own client (or none), which keeps
tests isolated and offline.

**Cards.** `App.card: Option<Card>` is an album (`artist`, `title`, optional `track`) or an artist.
`i` on an album/track opens at once if the bundle knows the album, or if live cards are on (then
with a placeholder until the server answers); `I` does the same for `App.now`. `ui::card` builds the
lines each frame, writes the maximum useful scroll into `Card.max` (a `Cell`, the one place the UI
writes state) and the key handler clamps to it. `o` opens `{deadwax_url}/#crate?open=<rgid>`.

**Privacy.** Only normalized artist/title pairs of albums on screen, or the album/artist being
inspected, are sent. The personal lenses (*Next up*, *Missing from your shelf*, *Crate dig*) and
everything about the library and history stay local.

**Settings.** `deadwax_url`, `deadwax_live`, `deadwax_cards`, `deadwax_resolve`,
`deadwax_lenses`. `settings::rows(show)` includes the *Deadwax live* section only when
`Settings::show_deadwax(client.ever_up())` holds (a server answered, or the URL isn't the default).
Changing a toggle calls `Client::set_config`; the URL is fixed for the session.

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
  logic, the palette extractor, and the live-deadwax features against `deadwax::fake::Fake` (a
  scripted server; offline fallback, batching, breaker, gating, cards, lenses). No network, and no
  reads or writes of the user's settings, prefs or cache.
* `cargo test -- --ignored --nocapture` runs the live tests against your real account (read-only
  except `live_bodyless_put`, which sets volume to its current value). They need a saved login.
  `live_deadwax` and `live_cards_render` need a deadwax server instead (`DEADWAX_URL`, default
  `http://127.0.0.1:8820`): end-to-end lookup, bundle/live agreement, card hydration and rendering.

## Recipes

* **New main-menu entry:** add it to `model::root_items`, handle its id in `Live::children`
  (and `Mock::children_vec`), add it to `settings::ROOT_TOGGLES` so it can be hidden.
* **New row field:** add a field to `Item`, fill it in the parsers (`spotify.rs`), persist it in
  the cache arrays (`persist`/`load_disk`), show it in `ui::secondary`/`right_text`, add a
  setting in `settings.rs` (struct, `rows`, `label`, `change`, `load`, `save`).
* **New outside source:** fetch with `Api::get_public`, emit `x*:` items, add a resolver arm in
  `fetch`. Keep it behind the `external` setting.

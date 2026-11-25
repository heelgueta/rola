//! rola: a keyboard-first Spotify TUI (Miller columns, yazi/pato style). See docs/ARCHITECTURE.md.
//!
//! Startup: build the app (`--demo` = mock data; otherwise load the Client ID, log in via the
//! browser if there is no saved token, spawn the playback worker), then loop at ~30 fps:
//! tick state -> draw -> handle one key event.
mod acclaim;
mod app;
mod art;
mod auth;
mod awards;
mod backend;
mod deadwax;
mod history;
mod model;
mod settings;
mod spotify;
mod ui;
mod viz;

use ratatui::crossterm::event::{self, Event, KeyEventKind};
use std::{io, sync::Arc, time::{Duration, Instant}};

fn build_app() -> Result<app::App, String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--demo") {
        return Ok(app::App::new(Box::new(backend::Mock), None));
    }
    let auth = Arc::new(auth::Auth::new()?);
    if args.first().map(|a| a == "login").unwrap_or(false) || !auth.logged_in() {
        auth.login()?;
        println!("Logged in.");
        if args.first().map(|a| a == "login").unwrap_or(false) {
            std::process::exit(0);
        }
    }
    let api = spotify::Api::new(auth);
    let player = spotify::Player::spawn(api.clone());
    Ok(app::App::new(Box::new(spotify::Live::new(api)), Some(player)))
}

fn main() -> io::Result<()> {
    let mut app = match build_app() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("rola: {e}");
            std::process::exit(1);
        }
    };
    let mut term = ratatui::init();
    let mut last = Instant::now();
    let res = (|| -> io::Result<()> {
        while !app.quit {
            let now = Instant::now();
            app.tick(now.duration_since(last).as_secs_f64());
            last = now;
            term.draw(|f| ui::draw(f, &app))?;
            if event::poll(Duration::from_millis(33))? {
                if let Event::Key(k) = event::read()? {
                    if k.kind == KeyEventKind::Press {
                        app.on_key(k);
                    }
                }
            }
        }
        Ok(())
    })();
    ratatui::restore();
    res
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{backend::TestBackend, crossterm::event::{KeyCode, KeyEvent, KeyModifiers}, Terminal};

    fn key(c: KeyCode) -> KeyEvent {
        KeyEvent::new(c, KeyModifiers::NONE)
    }
    fn dump(t: &Terminal<TestBackend>) -> String {
        let b = t.backend().buffer();
        (0..b.area.height)
            .map(|y| (0..b.area.width).map(|x| b[(x, y)].symbol().to_string()).collect::<String>().trim_end().to_string())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Overlays and the now-playing bar must survive absurdly small terminals.
    #[test]
    fn tiny_terminals_do_not_panic() {
        for (w, h) in [(20u16, 6u16), (40, 12), (80, 24), (200, 60)] {
            let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
            let mut app = app::App::new(Box::new(backend::Mock), None);
            for c in [KeyCode::Char('j'), KeyCode::Char('l'), KeyCode::Char('j'), KeyCode::Enter, KeyCode::Char(','), KeyCode::Char('j')] {
                app.on_key(key(c));
                term.draw(|f| ui::draw(f, &app)).unwrap();
            }
            app.on_key(key(KeyCode::Esc));
            app.on_key(key(KeyCode::Char('?')));
            term.draw(|f| ui::draw(f, &app)).unwrap();
        }
    }

    /// Acclaim: lenses render with scores, the card opens, and sorting by acclaim works.
    #[test]
    fn acclaim_walkthrough() {
        let mut term = Terminal::new(TestBackend::new(120, 30)).unwrap();
        let mut app = app::App::new(Box::new(backend::Mock), None);
        for _ in 0..40 {
            if app.active().selected().map(|i| i.id.as_str()) == Some("sec:acclaim") {
                break;
            }
            app.on_key(key(KeyCode::Char('j')));
        }
        assert_eq!(app.active().selected().unwrap().id, "sec:acclaim");
        app.on_key(key(KeyCode::Char('l')));
        for _ in 0..3 {
            app.on_key(key(KeyCode::Char('j')));
        }
        app.on_key(key(KeyCode::Char('l'))); // Sure things
        app.tick(1.0);
        term.draw(|f| ui::draw(f, &app)).unwrap();
        let first = dump(&term);
        println!("{first}\n");
        assert!(first.contains("σ"), "scores are shown");
        app.on_key(key(KeyCode::Char('i')));
        term.draw(|f| ui::draw(f, &app)).unwrap();
        let card = dump(&term);
        println!("{card}\n");
        assert!(card.contains("you are here"));
        app.on_key(key(KeyCode::Esc));
        app.on_key(key(KeyCode::Char('s')));
        term.draw(|f| ui::draw(f, &app)).unwrap();
    }

    #[test]
    fn demo_walkthrough() {
        let mut term = Terminal::new(TestBackend::new(110, 26)).unwrap();
        let mut app = app::App::new(Box::new(backend::Mock), None);
        for c in [KeyCode::Char('j'), KeyCode::Char('l'), KeyCode::Char('j'), KeyCode::Char('j'), KeyCode::Char('l'), KeyCode::Char('j'), KeyCode::Enter] {
            app.on_key(key(c));
        }
        app.tick(3.0);
        term.draw(|f| ui::draw(f, &app)).unwrap();
        println!("{}\n", dump(&term));
        for c in [KeyCode::Char('h'), KeyCode::Char('h'), KeyCode::Char('G'), KeyCode::Char('k'), KeyCode::Char('k'), KeyCode::Char('k'), KeyCode::Enter, KeyCode::Char('d'), KeyCode::Char('u'), KeyCode::Char('s'), KeyCode::Char('t'), KeyCode::Enter] {
            app.on_key(key(c));
        }
        term.draw(|f| ui::draw(f, &app)).unwrap();
        println!("{}", dump(&term));
    }
}

#[cfg(test)]
mod live_tests {
    use super::*;
    use ratatui::{backend::TestBackend, crossterm::event::{KeyCode, KeyEvent, KeyModifiers}, Terminal};

    fn key(c: KeyCode) -> KeyEvent {
        KeyEvent::new(c, KeyModifiers::NONE)
    }
    fn dump(t: &Terminal<TestBackend>) -> String {
        let b = t.backend().buffer();
        (0..b.area.height)
            .map(|y| (0..b.area.width).map(|x| b[(x, y)].symbol().to_string()).collect::<String>().trim_end().to_string())
            .collect::<Vec<_>>()
            .join("\n")
    }
    fn settle(app: &mut app::App, term: &mut Terminal<TestBackend>, secs: f64) {
        for _ in 0..(secs / 0.15) as usize {
            std::thread::sleep(Duration::from_millis(150));
            app.tick(0.16);
        }
        term.draw(|f| ui::draw(f, app)).unwrap();
    }
    /// Move the cursor in the active column to the entry with this name.
    fn goto(app: &mut app::App, name: &str) {
        for _ in 0..60 {
            if app.active().selected().map(|i| i.name == name).unwrap_or(false) {
                return;
            }
            app.on_key(key(KeyCode::Char('j')));
        }
        panic!("could not find {name}");
    }

    /// Read-only: browses real data, sends no playback commands, changes no settings/prefs.
    #[test]
    #[ignore]
    fn live_browse() {
        let auth = Arc::new(auth::Auth::new().unwrap());
        let api = spotify::Api::new(auth);
        let mut app = app::App::new(Box::new(spotify::Live::new(api.clone())), Some(spotify::Player::spawn(api)));
        let mut term = Terminal::new(TestBackend::new(130, 38)).unwrap();
        settle(&mut app, &mut term, 3.0);

        // album art palette of whatever is playing
        let seed = app.now.as_ref().map(|n| viz::seed_of(&n.id)).unwrap_or(1);
        println!("now playing: {:?}", app.now.as_ref().map(|n| (&n.name, &n.sub)));
        println!("cover palette hues (a, b): {:?}  vs per-track fallback {:?}", app.hues(seed), (viz::base_hue(seed), viz::base_hue(seed) + 70.0));

        // Awards & lists > Grammy · Album of the Year
        goto(&mut app, "Awards & lists");
        app.on_key(key(KeyCode::Char('l')));
        goto(&mut app, "Grammy · Album of the Year");
        settle(&mut app, &mut term, 0.5);
        println!("\n=== Awards folder (preview of Grammy album list)\n{}", dump(&term).lines().take(16).collect::<Vec<_>>().join("\n"));
        app.on_key(key(KeyCode::Char('l')));
        settle(&mut app, &mut term, 0.5);
        goto(&mut app, "Folklore");
        app.on_key(key(KeyCode::Char('l'))); // resolve + open the album
        settle(&mut app, &mut term, 4.0);
        println!("\n=== Folklore opened (resolved on Spotify; track numbers)\n{}", dump(&term).lines().take(20).collect::<Vec<_>>().join("\n"));

        // settings overlay
        app.on_key(key(KeyCode::Char(',')));
        settle(&mut app, &mut term, 0.3);
        println!("\n=== settings overlay\n{}", dump(&term));
    }
}

#[cfg(test)]
mod palette_live {
    use super::*;

    /// Uses whatever is playing right now: fetches its cover and prints the extracted hues.
    #[test]
    #[ignore]
    fn live_palette() {
        let auth = Arc::new(auth::Auth::new().unwrap());
        let api = spotify::Api::new(auth);
        let mut app = app::App::new(Box::new(spotify::Live::new(api.clone())), Some(spotify::Player::spawn(api)));
        for _ in 0..20 {
            std::thread::sleep(Duration::from_millis(150));
            app.tick(0.15);
        }
        let seed = app.now.as_ref().map(|n| viz::seed_of(&n.id)).unwrap_or(1);
        let _ = app.hues(seed); // first call starts the background fetch
        std::thread::sleep(Duration::from_secs(3));
        let h = app.hues(seed);
        println!("now: {:?}\nart_url set: {}\nhues from cover: {:?}\nper-track fallback: {:?}", app.now.as_ref().map(|n| (&n.name, &n.sub)), !app.art_url.is_empty(), h, (viz::base_hue(seed), viz::base_hue(seed) + 70.0));
    }
}

/// Live deadwax features against a scripted server (no network): cards, 64k rows, lenses, the
/// settings rule, and the silent offline fallback.
#[cfg(test)]
mod deadwax_ui {
    use super::*;
    use crate::deadwax::{fake::Fake, Client, Config, DEFAULT_URL};
    use ratatui::{backend::TestBackend, crossterm::event::{KeyCode, KeyEvent, KeyModifiers}, Terminal};

    const ALL: [&str; 8] = ["lookup", "album_detail", "panel_raw", "crowd", "pitchfork", "artists", "presets", "web_deeplink"];

    fn key(c: KeyCode) -> KeyEvent {
        KeyEvent::new(c, KeyModifiers::NONE)
    }
    fn ch(a: &mut app::App, c: char) {
        a.on_key(key(KeyCode::Char(c)));
    }
    fn dump(t: &Terminal<TestBackend>) -> String {
        let b = t.backend().buffer();
        (0..b.area.height).map(|y| (0..b.area.width).map(|x| b[(x, y)].symbol().to_string()).collect::<String>().trim_end().to_string()).collect::<Vec<_>>().join("\n")
    }
    fn draw(a: &app::App, t: &mut Terminal<TestBackend>) -> String {
        t.draw(|f| ui::draw(f, a)).unwrap();
        dump(t)
    }
    fn goto(a: &mut app::App, name: &str) {
        ch(a, 'g');
        ch(a, 'g');
        for _ in 0..80 {
            if a.active().selected().is_some_and(|i| i.name == name) {
                return;
            }
            ch(a, 'j');
        }
        panic!("no {name} in {:?}", a.active().items.iter().map(|i| &i.name).collect::<Vec<_>>());
    }
    fn client(caps: &[&'static str], down: bool) -> (Client, std::sync::Arc<Fake>) {
        let f = Fake::new(caps);
        *f.down.lock().unwrap() = down;
        let c = Client::with_transport(Config { url: DEFAULT_URL.into(), live: true, cards: true, resolve: true, lenses: true }, f.clone(), true);
        c.start();
        (c, f)
    }
    fn app_with(c: Client) -> app::App {
        let mut a = app::App::new(Box::new(backend::Mock), None);
        a.settings = settings::Settings::default();
        a.set_deadwax(c);
        a
    }

    #[test]
    fn offline_server_changes_nothing() {
        let (c, f) = client(&ALL, true);
        let mut a = app_with(c);
        let mut t = Terminal::new(TestBackend::new(120, 40)).unwrap();
        goto(&mut a, "Acclaim · deadwax");
        ch(&mut a, 'l');
        assert!(!a.active().items.iter().any(|i| i.id.starts_with("acc:live:")), "no live lenses");
        ch(&mut a, 'h');
        goto(&mut a, "Albums");
        ch(&mut a, 'l');
        goto(&mut a, "Gaia");
        ch(&mut a, 'i');
        assert!(a.card.is_none(), "no live card for an album the bundle lacks");
        a.toast = None;
        for _ in 0..20 {
            a.tick(0.2);
        }
        assert!(a.toast.is_none(), "no deadwax toasts, ever");
        ch(&mut a, ',');
        let screen = draw(&a, &mut t);
        assert!(!screen.contains("DEADWAX LIVE") && !screen.contains("Live deadwax"), "{screen}");
        assert_eq!(a.menu_rows(), settings::rows(false));
        assert_eq!(f.calls.lock().unwrap().len(), 1, "just the startup probe");
    }

    #[test]
    fn rows_resolve_live_and_card_hydrates_scrolls_and_deep_links() {
        let (c, _) = client(&ALL, false);
        let mut a = app_with(c);
        let mut t = Terminal::new(TestBackend::new(110, 30)).unwrap();
        goto(&mut a, "Albums");
        ch(&mut a, 'l');
        a.tick(0.2);
        let screen = draw(&a, &mut t);
        let gaia = screen.lines().find(|l| l.contains("Gaia")).unwrap();
        assert!(gaia.contains("+0.8σ"), "the 64k lookup scores a row the bundle lacks: {gaia}");
        goto(&mut a, "Gaia");
        ch(&mut a, 'i');
        assert!(a.card.is_some());
        let card = draw(&a, &mut t);
        println!("{card}");
        assert!(card.contains("CRITICS' PANEL") && card.contains("Christgau (retro)"), "{card}");
        assert!(card.contains("o open in deadwax"));
        let max = a.card.as_ref().unwrap().max.get();
        assert!(max > 0 && max < usize::MAX, "the card is taller than the screen");
        ch(&mut a, 'j');
        ch(&mut a, 'j');
        assert_eq!(a.card.as_ref().unwrap().scroll, 2);
        ch(&mut a, 'k');
        assert_eq!(a.card.as_ref().unwrap().scroll, 1);
        a.on_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL));
        assert_eq!(a.card.as_ref().unwrap().scroll, max.min(11));
        ch(&mut a, 'G');
        assert_eq!(a.card.as_ref().unwrap().scroll, max);
        let bottom = draw(&a, &mut t);
        for s in ["SIMILAR ERA", "Wire — 154", "STANDOUT TRACKS", "CROWD VS CRITICS", "123,456"] {
            assert!(bottom.contains(s) || card.contains(s), "{s} missing:\n{bottom}");
        }
        ch(&mut a, 'g');
        assert_eq!(a.card.as_ref().unwrap().scroll, 0);
        ch(&mut a, 'x'); // other keys don't close it
        assert!(a.card.is_some());
        ch(&mut a, 'o');
        assert_eq!(a.opened.as_deref(), Some("http://127.0.0.1:8820/#crate?open=rg-gaia"));
        ch(&mut a, 'i');
        assert!(a.card.is_none(), "i closes");
        ch(&mut a, 'i');
        a.on_key(key(KeyCode::Esc));
        assert!(a.card.is_none(), "esc closes");
        // acclaim sorting picks the live record up
        for _ in 0..7 {
            if a.active().sort_label() == "sort: acclaim ↓" {
                break;
            }
            ch(&mut a, 's');
        }
        assert_eq!(a.active().sort_label(), "sort: acclaim ↓", "acclaim sort is offered: a live record exists");
        let first = a.active().selected().map(|_| a.active().items[a.active().view()[0]].name.clone()).unwrap();
        assert_eq!(first, "Origin of Symmetry", "best by lb first (bundled 0.99 beats live Gaia 0.3)");
    }

    #[test]
    fn track_and_now_playing_open_the_album_card() {
        // bundle only: `i` on a track of a bundled album works with no server at all
        let mut a = app_with(Client::disabled());
        let mut t = Terminal::new(TestBackend::new(110, 40)).unwrap();
        goto(&mut a, "Albums");
        ch(&mut a, 'l');
        goto(&mut a, "Angel Dust");
        ch(&mut a, 'l');
        let track = a.active().selected().unwrap().name.clone();
        ch(&mut a, 'i');
        let card = draw(&a, &mut t);
        assert!(card.contains("Angel Dust") && card.contains(&track), "{card}");
        assert!(!card.contains("o open in deadwax") && !card.contains("live"), "offline card is the bundle card");
        a.on_key(key(KeyCode::Esc));
        // with live: a track of an album only deadwax knows, highlighted, and `I` on what is playing
        let (c, _) = client(&ALL, false);
        a.set_deadwax(c);
        ch(&mut a, 'h');
        goto(&mut a, "Gaia");
        ch(&mut a, 'l');
        a.on_key(key(KeyCode::Enter)); // play it (demo playback)
        assert!(a.now.is_some());
        ch(&mut a, 'h');
        ch(&mut a, 'h'); // main menu: nothing selected that could be inspected
        ch(&mut a, 'I');
        let Some(app::Card { of: app::CardOf::Album { title, track, .. }, .. }) = &a.card else { panic!("no card for now playing") };
        assert_eq!(title, "Gaia");
        assert_eq!(track.as_deref(), Some(a.now.as_ref().unwrap().name.as_str()));
        let card = draw(&a, &mut t);
        assert!(card.contains("TRACKLIST") && card.contains("track"), "{card}");
    }

    #[test]
    fn artist_card_and_clean_toast_without_live() {
        let mut a = app_with(Client::disabled());
        goto(&mut a, "Artists");
        ch(&mut a, 'l');
        ch(&mut a, 'i');
        assert!(a.card.is_none());
        assert!(a.toast.as_ref().unwrap().0.starts_with("i shows the critics' card"));
        let (c, _) = client(&ALL, false);
        a.set_deadwax(c);
        ch(&mut a, 'i');
        let mut t = Terminal::new(TestBackend::new(110, 40)).unwrap();
        let card = draw(&a, &mut t);
        println!("{card}");
        for s in ["Talking Heads", "studio albums", "CAREER CURVE", "DISCOGRAPHY", "Remain in Light (1980)", "1977"] {
            assert!(card.contains(s), "{s} missing:\n{card}");
        }
        assert!(!card.contains("o open in deadwax"), "artist cards have no deep link");
        // a public server without the artists capability: back to the toast
        let (c, _) = client(&["lookup", "album_detail", "presets"], false);
        a.set_deadwax(c);
        a.card = None;
        ch(&mut a, 'i');
        assert!(a.card.is_none());
    }

    #[test]
    fn live_lenses_and_settings_rows() {
        let (c, _) = client(&ALL, false);
        let mut a = app_with(c);
        let mut t = Terminal::new(TestBackend::new(120, 40)).unwrap();
        goto(&mut a, "Acclaim · deadwax");
        ch(&mut a, 'l');
        let ids: Vec<String> = a.active().items.iter().filter(|i| i.id.starts_with("acc:live:")).map(|i| i.id.clone()).collect();
        assert_eq!(ids, ["acc:live:hidden", "acc:live:crowd", "acc:live:pitch", "acc:live:debut"]);
        goto(&mut a, "Hidden records · live");
        ch(&mut a, 'l');
        let screen = draw(&a, &mut t);
        let row = screen.lines().find(|l| l.contains("Deep Cuts")).unwrap_or_else(|| panic!("{screen}"));
        assert!(row.contains("~+1.2σ"), "thin record marked: {row}");
        assert!(a.active().items[0].id.starts_with("xl:"), "opens like the bundled lenses (Spotify search)");
        // the settings menu now has the deadwax section; turning live off removes the lenses
        ch(&mut a, ',');
        let menu = draw(&a, &mut t);
        assert!(menu.contains("DEADWAX LIVE") && menu.contains("Live deadwax (127.0.0.1:8820)"), "{menu}");
        let rows = a.menu_rows();
        let at = rows.iter().position(|r| *r == settings::Row::DwLive).unwrap();
        a.menu = Some(at);
        ch(&mut a, ' ');
        assert!(!a.settings.deadwax_live && !a.dw.on());
        a.on_key(key(KeyCode::Esc));
        assert!(a.menu_rows().contains(&settings::Row::DwLive), "rows stay once a server was seen");
        ch(&mut a, 'h');
        ch(&mut a, 'h');
        assert!(!a.children_of(&model::sec("acclaim", "")).items_any(|i| i.id.starts_with("acc:live:")));
    }

    /// Real cards from a running deadwax (`DEADWAX_URL`, default local), drawn headless.
    #[test]
    #[ignore]
    fn live_cards_render() {
        let url = std::env::var("DEADWAX_URL").unwrap_or_else(|_| DEFAULT_URL.into());
        let c = Client::new(Config { url, live: true, cards: true, resolve: true, lenses: true });
        c.start();
        let wait = |a: &mut app::App| {
            for _ in 0..30 {
                std::thread::sleep(std::time::Duration::from_millis(60));
                a.tick(0.06);
            }
        };
        let mut a = app_with(c);
        wait(&mut a);
        assert!(a.dw.on(), "no deadwax at that URL");
        let mut t = Terminal::new(TestBackend::new(120, 46)).unwrap();
        goto(&mut a, "Albums");
        ch(&mut a, 'l');
        wait(&mut a);
        println!("=== album rows (64k lookup)\n{}", draw(&a, &mut t));
        goto(&mut a, "Angel Dust");
        ch(&mut a, 'i');
        wait(&mut a);
        println!("=== album card\n{}", draw(&a, &mut t));
        ch(&mut a, 'G');
        println!("=== album card, bottom\n{}", draw(&a, &mut t));
        a.on_key(key(KeyCode::Esc));
        ch(&mut a, 'h');
        goto(&mut a, "Artists");
        ch(&mut a, 'l');
        goto(&mut a, "Muse");
        ch(&mut a, 'i');
        wait(&mut a);
        println!("=== artist card\n{}", draw(&a, &mut t));
        a.on_key(key(KeyCode::Esc));
        ch(&mut a, 'h');
        goto(&mut a, "Acclaim · deadwax");
        ch(&mut a, 'l');
        goto(&mut a, "Hidden records · live");
        ch(&mut a, 'l');
        wait(&mut a);
        println!("=== live lens\n{}", draw(&a, &mut t));
    }

    trait AnyItem {
        fn items_any(&self, f: impl Fn(&model::Item) -> bool) -> bool;
    }
    impl AnyItem for model::Load {
        fn items_any(&self, f: impl Fn(&model::Item) -> bool) -> bool {
            match self {
                model::Load::Ready(v) | model::Load::Loading(v) => v.iter().any(f),
                model::Load::Failed(_) => false,
            }
        }
    }
}

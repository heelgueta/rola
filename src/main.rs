//! rola: a keyboard-first Spotify TUI (Miller columns, yazi/pato style). See docs/ARCHITECTURE.md.
//!
//! Startup: build the app (`--demo` = mock data; otherwise load the Client ID, log in via the
//! browser if there is no saved token, spawn the playback worker), then loop at ~30 fps:
//! tick state -> draw -> handle one key event.
mod app;
mod art;
mod auth;
mod awards;
mod backend;
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

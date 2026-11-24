mod app;
mod auth;
mod backend;
mod model;
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
        (0..b.area.height - 9)
            .map(|y| (0..b.area.width).map(|x| b[(x, y)].symbol().to_string()).collect::<String>().trim_end().to_string())
            .collect::<Vec<_>>()
            .join("\n")
    }
    fn settle(app: &mut app::App, term: &mut Terminal<TestBackend>) {
        for _ in 0..40 {
            std::thread::sleep(Duration::from_millis(150));
            app.tick(0.16);
        }
        term.draw(|f| ui::draw(f, app)).unwrap();
    }

    /// Needs a saved login. Read-only: browses real data, sends no playback commands.
    #[test]
    #[ignore]
    fn live_browse() {
        let auth = Arc::new(auth::Auth::new().unwrap());
        assert!(auth.logged_in());
        let api = spotify::Api::new(auth);
        let mut app = app::App::new(Box::new(spotify::Live::new(api.clone())), Some(spotify::Player::spawn(api)));
        let mut term = Terminal::new(TestBackend::new(120, 36)).unwrap();
        let steps: &[(&str, &[KeyCode])] = &[
            ("Playlists", &[KeyCode::Char('j'), KeyCode::Char('j'), KeyCode::Char('l')]),
            ("Liked songs", &[KeyCode::Char('h'), KeyCode::Char('j'), KeyCode::Char('j'), KeyCode::Char('j'), KeyCode::Char('l')]),
            ("Recently played", &[KeyCode::Char('h'), KeyCode::Char('j'), KeyCode::Char('l')]),
            ("Made for you", &[KeyCode::Char('h'), KeyCode::Char('j'), KeyCode::Char('j'), KeyCode::Char('l')]),
            ("Albums", &[KeyCode::Char('h'), KeyCode::Char('G'), KeyCode::Char('g'), KeyCode::Char('g'), KeyCode::Char('j'), KeyCode::Char('j'), KeyCode::Char('j'), KeyCode::Char('j'), KeyCode::Char('l')]),
        ];
        settle(&mut app, &mut term);
        println!("=== root\n{}", dump(&term));
        for (name, keys) in steps {
            for k in *keys {
                app.on_key(key(*k));
            }
            settle(&mut app, &mut term);
            println!("\n=== {name}\n{}", dump(&term));
        }
    }
}

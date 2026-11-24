use crate::{app::App, model::{Item, Kind}, viz};
use ratatui::{prelude::*, widgets::{Clear, Paragraph}};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

// Palette: primary text is near-white; "soft" is still clearly readable; "faint" is for chrome only.
fn bright() -> Color { viz::rgb(248, 248, 252) }
fn soft() -> Color { viz::rgb(196, 196, 208) }
fn faint() -> Color { viz::rgb(128, 128, 142) }
fn rail() -> Color { viz::rgb(64, 64, 74) }
fn sel_dim() -> Color { viz::rgb(88, 88, 104) }

fn fmt_ms(ms: f64) -> String {
    let s = (ms / 1000.0) as u32;
    format!("{}:{:02}", s / 60, s % 60)
}

fn fit(s: &str, w: usize) -> String {
    if s.width() <= w {
        return s.to_string();
    }
    let mut out = String::new();
    let mut cur = 0;
    for c in s.chars() {
        let cw = c.width().unwrap_or(0);
        if cur + cw + 1 > w {
            break;
        }
        out.push(c);
        cur += cw;
    }
    out.push('…');
    out
}

fn inset(r: Rect) -> Rect {
    Rect { x: r.x + 1, y: r.y, width: r.width.saturating_sub(2), height: r.height }
}

pub fn draw(f: &mut Frame, app: &App) {
    let seed = app.now.as_ref().map(|n| viz::seed_of(&n.id)).unwrap_or(0x5eed);
    let accent = viz::hsl(viz::base_hue(seed), 0.65, 0.66);
    let [header, main, np, status] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(3), Constraint::Length(8), Constraint::Length(1)]).areas(f.area());

    // breadcrumb
    let mut crumbs: Vec<Span> = vec![Span::styled(" ♪ ", Style::new().fg(accent).bold())];
    for (i, c) in app.cols.iter().enumerate() {
        if i > 0 {
            crumbs.push(Span::styled(" › ", Style::new().fg(faint())));
        }
        let st = if i + 1 == app.cols.len() { Style::new().fg(bright()).bold() } else { Style::new().fg(soft()) };
        crumbs.push(Span::styled(c.item.name.clone(), st));
    }
    f.render_widget(Paragraph::new(Line::from(crumbs)), header);

    // miller columns: [parent | current | preview]; at the root, [root | preview]
    let n = app.cols.len();
    let cur = app.active();
    let v = cur.view();
    let items: Vec<&Item> = v.iter().map(|&i| &cur.items[i]).collect();
    if n == 1 {
        let [a, b, _] = Layout::horizontal([Constraint::Length(26), Constraint::Percentage(40), Constraint::Min(0)]).areas(main);
        list(f, inset(a), &items, Some(cur.sel), accent, false, app);
        preview(f, inset(b), app, accent);
    } else {
        let [a, b, c] = Layout::horizontal([Constraint::Length(26), Constraint::Percentage(36), Constraint::Min(0)]).areas(main);
        let p = &app.cols[n - 2];
        let pv = p.view();
        let pitems: Vec<&Item> = pv.iter().map(|&i| &p.items[i]).collect();
        list(f, inset(a), &pitems, Some(p.sel), accent, true, app);
        if items.is_empty() {
            let msg = col_msg(cur.loading, cur.error.as_deref(), !cur.filter.is_empty());
            f.render_widget(Paragraph::new(msg).style(Style::new().fg(soft())).wrap(ratatui::widgets::Wrap { trim: true }), inset(b));
        } else {
            list(f, inset(b), &items, Some(cur.sel), accent, false, app);
        }
        preview(f, inset(c), app, accent);
    }

    draw_now(f, np, app, seed, accent);

    // status line
    let line = if let Some(scope) = &app.search {
        Line::from(vec![
            Span::styled(format!(" search {scope}: "), Style::new().fg(accent).bold()),
            Span::styled(app.query.clone(), Style::new().fg(bright())),
            Span::styled("▏", Style::new().fg(faint())),
            Span::styled("   enter run · esc cancel", Style::new().fg(faint())),
        ])
    } else if app.filtering || !cur.filter.is_empty() {
        Line::from(vec![
            Span::styled(" / ", Style::new().fg(accent).bold()),
            Span::styled(cur.filter.clone(), Style::new().fg(bright())),
            Span::styled(if app.filtering { "▏" } else { "   esc clears" }, Style::new().fg(faint())),
        ])
    } else if let Some((msg, _)) = &app.toast {
        Line::styled(format!(" {msg}"), Style::new().fg(accent))
    } else {
        Line::styled(" hjkl move · enter play/open · space pause · n/p track · / filter · v visuals · ? help · q quit", Style::new().fg(soft()))
    };
    f.render_widget(Paragraph::new(line), status);

    if app.help {
        help(f, f.area(), accent);
    }
}

fn col_msg(loading: bool, error: Option<&str>, filtered: bool) -> String {
    if let Some(e) = error {
        format!("{e}\n\nenter or P plays it as a whole")
    } else if loading {
        "loading…".into()
    } else if filtered {
        "no matches".into()
    } else {
        "empty".into()
    }
}

fn preview(f: &mut Frame, area: Rect, app: &App, accent: Color) {
    let Some(it) = app.active().selected() else { return };
    if it.id.starts_with("sec:search:") {
        let scope = it.id.rsplit(':').next().unwrap_or("");
        let lines = vec![
            Line::styled(format!("search {scope}"), Style::new().fg(accent).bold()),
            Line::raw(""),
            Line::styled("enter ▸ type a query", Style::new().fg(soft())),
        ];
        return f.render_widget(Paragraph::new(lines), area);
    }
    if it.is_container() {
        if app.preview.is_empty() {
            let msg = if it.id == "sec:queue" && !app.preview_loading && app.preview_error.is_none() {
                "nothing queued — play a track".to_string()
            } else {
                col_msg(app.preview_loading, app.preview_error.as_deref(), false)
            };
            return f.render_widget(Paragraph::new(msg).style(Style::new().fg(soft())).wrap(ratatui::widgets::Wrap { trim: true }), area);
        }
        let pv: Vec<&Item> = app.preview.iter().collect();
        list(f, area, &pv, None, accent, true, app);
    } else {
        track_info(f, area, it, accent);
    }
}

fn list(f: &mut Frame, area: Rect, items: &[&Item], sel: Option<usize>, accent: Color, dim: bool, app: &App) {
    let h = area.height as usize;
    let w = area.width as usize;
    if h == 0 || w < 6 {
        return;
    }
    let s = sel.unwrap_or(0);
    let off = s.saturating_sub(h / 2).min(items.len().saturating_sub(h));
    let playing_id = app.now.as_ref().map(|n| n.id.as_str());
    let lines: Vec<Line> = items
        .iter()
        .enumerate()
        .skip(off)
        .take(h)
        .map(|(i, it)| {
            if it.kind == Kind::Header {
                let pad = w.saturating_sub(it.name.width() + 3);
                return Line::from(vec![
                    Span::styled(format!(" {} ", it.name.to_uppercase()), Style::new().fg(faint()).bold()),
                    Span::styled("─".repeat(pad), Style::new().fg(rail())),
                ]);
            }
            let right = match it.kind {
                Kind::Track => fmt_ms(it.dur_ms as f64),
                _ => "›".into(),
            };
            let mark = if Some(it.id.as_str()) == playing_id { "♪ " } else { "  " };
            let avail = w.saturating_sub(right.width() + mark.width() + 1);
            // secondary text: artist for albums/tracks, shown only if there is room
            let sec_full = match it.kind {
                Kind::Album => it.sub.clone(),
                Kind::Track => it.sub.split(" · ").next().unwrap_or("").to_string(),
                _ => String::new(),
            };
            let name = fit(&it.name, avail);
            let room = avail.saturating_sub(name.width() + 2);
            let secondary = if room >= 6 && !sec_full.is_empty() { format!("  {}", fit(&sec_full, room)) } else { String::new() };
            let pad = w.saturating_sub(mark.width() + name.width() + secondary.width() + right.width());
            let is_sel = sel == Some(i);
            let style = if is_sel {
                if dim { Style::new().bg(sel_dim()).fg(bright()) } else { Style::new().bg(accent).fg(Color::Black).bold() }
            } else if Some(it.id.as_str()) == playing_id {
                Style::new().fg(accent).bold()
            } else if dim {
                Style::new().fg(soft())
            } else {
                Style::new().fg(bright())
            };
            if is_sel {
                let text = format!("{mark}{name}{secondary}{}{right}", " ".repeat(pad));
                return Line::styled(text, style);
            }
            Line::from(vec![
                Span::styled(format!("{mark}{name}"), style),
                Span::styled(secondary, Style::new().fg(faint())),
                Span::styled(format!("{}{right}", " ".repeat(pad)), Style::new().fg(soft())),
            ])
        })
        .collect();
    f.render_widget(Paragraph::new(lines), area);
}

fn track_info(f: &mut Frame, area: Rect, it: &Item, accent: Color) {
    let (artist, album) = it.sub.split_once(" · ").unwrap_or((&it.sub, ""));
    let row = |k: &str, v: String| Line::from(vec![Span::styled(format!("{k:<8}"), Style::new().fg(soft())), Span::styled(v, Style::new().fg(bright()))]);
    let lines = vec![
        Line::styled(it.name.clone(), Style::new().fg(accent).bold()),
        Line::raw(""),
        row("artist", artist.to_string()),
        row("album", album.to_string()),
        row("length", fmt_ms(it.dur_ms as f64)),
        Line::raw(""),
        Line::styled("enter ▸ play from here", Style::new().fg(soft())),
    ];
    f.render_widget(Paragraph::new(lines), area);
}

fn draw_now(f: &mut Frame, area: Rect, app: &App, seed: u64, accent: Color) {
    let area = inset(area);
    let [title, vz, prog] = Layout::vertical([Constraint::Length(1), Constraint::Min(1), Constraint::Length(1)]).areas(area);

    let (left, dur) = match &app.now {
        Some(n) => (
            Line::from(vec![
                Span::styled(if app.playing { "▶ " } else { "⏸ " }, Style::new().fg(accent)),
                Span::styled(n.name.clone(), Style::new().fg(bright()).bold()),
                Span::styled(format!("  {}", n.sub), Style::new().fg(soft())),
            ]),
            n.dur_ms as f64,
        ),
        None => (Line::styled("nothing playing — enter on a track", Style::new().fg(soft())), 1.0),
    };
    f.render_widget(Paragraph::new(left), title);
    let color_note = if viz::is_truecolor() { "" } else { " · 256c" };
    f.render_widget(
        Paragraph::new(Line::styled(
            format!(
                "{}{}{} · vol {}%",
                viz::MODES[app.viz_mode % viz::MODES.len()],
                color_note,
                if app.device.is_empty() { String::new() } else { format!(" · {}", app.device) },
                app.volume
            ),
            Style::new().fg(soft()),
        ))
        .right_aligned(),
        title,
    );

    let ratio = (app.progress_ms / dur).clamp(0.0, 1.0);
    let ctx = viz::Ctx { w: vz.width as usize, h: vz.height as usize, seed, t: app.t, progress: ratio, energy: app.energy };
    f.render_widget(Paragraph::new(viz::render(app.viz_mode, &ctx)), vz);

    let (cur, tot) = (fmt_ms(app.progress_ms), fmt_ms(if app.now.is_some() { dur } else { 0.0 }));
    let bar_w = (prog.width as usize).saturating_sub(cur.len() + tot.len() + 2);
    let filled = (bar_w as f64 * ratio) as usize;
    let line = Line::from(vec![
        Span::styled(format!("{cur} "), Style::new().fg(soft())),
        Span::styled("━".repeat(filled), Style::new().fg(accent)),
        Span::styled("─".repeat(bar_w - filled), Style::new().fg(rail())),
        Span::styled(format!(" {tot}"), Style::new().fg(soft())),
    ]);
    f.render_widget(Paragraph::new(line), prog);
}

fn help(f: &mut Frame, area: Rect, accent: Color) {
    let rows = [
        ("h j k l / arrows", "move · out / down / up / in"),
        ("enter", "open · play track · start search"),
        ("P", "play whole playlist/album/list"),
        ("R", "refresh from Spotify"),
        ("gg  G", "top · bottom"),
        ("ctrl-d  ctrl-u", "half page"),
        ("/", "fuzzy filter  (esc clears)"),
        ("space", "play / pause"),
        ("n  p", "next · previous / restart"),
        ("<  >", "seek ∓10s"),
        ("+  -", "volume"),
        ("v", "cycle visuals"),
        ("q", "quit"),
    ];
    let w = 56.min(area.width);
    let h = (rows.len() as u16 + 4).min(area.height);
    let r = Rect { x: area.x + (area.width - w) / 2, y: area.y + (area.height - h) / 2, width: w, height: h };
    f.render_widget(Clear, r);
    let mut lines = vec![Line::styled(" keys", Style::new().fg(accent).bold()), Line::raw("")];
    lines.extend(rows.iter().map(|(k, d)| Line::from(vec![Span::styled(format!(" {k:<18}"), Style::new().fg(accent)), Span::styled(*d, Style::new().fg(bright()))])));
    f.render_widget(Paragraph::new(lines).block(ratatui::widgets::Block::bordered().border_style(Style::new().fg(faint()))), r);
}

//! All drawing. Pure function of `&App`: nothing here mutates state.
//!
//! Layout: header (breadcrumb + count/sort) / Miller columns / now-playing bar / status line.
//! Columns are [parent | current | preview]; at the main menu it is [menu | preview].
//!
//! Colors: three greys (`bright` > `soft` > `faint`) for text hierarchy plus one *accent*.
//! The accent's hue comes from the album cover (see art.rs) but its saturation/lightness are
//! pinned here, so any cover yields readable text/selection (black on accent, accent on dark).
use crate::{
    app::{App, Card, CardOf},
    deadwax::Fetch,
    model::{mk, Item, Kind},
    settings::{Numbering, Row, Settings},
    viz,
};
use ratatui::{
    prelude::*,
    widgets::{Block, Clear, Paragraph, Wrap},
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

fn bright() -> Color { viz::rgb(248, 248, 252) }
fn soft() -> Color { viz::rgb(196, 196, 208) }
fn faint() -> Color { viz::rgb(128, 128, 142) }
fn rail() -> Color { viz::rgb(64, 64, 74) }
fn sel_dim() -> Color { viz::rgb(88, 88, 104) }

fn fmt_ms(ms: f64) -> String {
    let s = (ms / 1000.0) as u32;
    format!("{}:{:02}", s / 60, s % 60)
}

/// Truncate to `w` display cells with an ellipsis.
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

#[derive(Clone, Copy, PartialEq)]
enum RowNum {
    None,
    Position, // place in the list as loaded (stable under sorting)
    TrackNo,  // track number on the album
}

/// Which numbers to show for the tracks of `container`, per the Numbers setting.
fn row_num(s: &Settings, container: &Item) -> RowNum {
    match s.numbering {
        Numbering::Off => RowNum::None,
        Numbering::Position => RowNum::Position,
        Numbering::Auto => {
            if container.id.starts_with("al:") || container.id.starts_with("xl:") {
                RowNum::TrackNo
            } else if container.id.starts_with("pl:") {
                RowNum::Position
            } else {
                RowNum::None
            }
        }
    }
}

pub fn draw(f: &mut Frame, app: &App) {
    let seed = app.now.as_ref().map(|n| viz::seed_of(&n.id)).unwrap_or(0x5eed);
    let (hue_a, hue_b) = app.hues(seed);
    let accent = viz::hsl(hue_a, 0.65, 0.66);
    let np_h = if app.settings.viz { 8 } else { 2 };
    let [header, main, np, status] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(3), Constraint::Length(np_h), Constraint::Length(1)]).areas(f.area());

    // breadcrumb (left) and item count + sort (right)
    let mut crumbs: Vec<Span> = vec![Span::styled(" ♪ ", Style::new().fg(accent).bold())];
    for (i, c) in app.cols.iter().enumerate() {
        if i > 0 {
            crumbs.push(Span::styled(" › ", Style::new().fg(faint())));
        }
        let st = if i + 1 == app.cols.len() { Style::new().fg(bright()).bold() } else { Style::new().fg(soft()) };
        crumbs.push(Span::styled(c.item.name.clone(), st));
    }
    f.render_widget(Paragraph::new(Line::from(crumbs)), header);
    if app.cols.len() > 1 {
        let c = app.active();
        let (total, shown) = (c.items.len(), c.view().len());
        let count = if shown == total { format!("{total} items") } else { format!("{shown}/{total}") };
        let loading = if c.loading { " · loading…" } else { "" };
        let text = format!("{count}{loading} · {} ", c.sort_label());
        f.render_widget(Paragraph::new(Line::styled(text, Style::new().fg(soft()))).right_aligned(), header);
    }

    // miller columns: [parent | current | preview]; at the main menu, [menu | preview]
    let n = app.cols.len();
    let cur = app.active();
    let v = cur.view();
    let items: Vec<&Item> = v.iter().map(|&i| &cur.items[i]).collect();
    if n == 1 {
        let [a, b, _] = Layout::horizontal([Constraint::Length(26), Constraint::Percentage(40), Constraint::Min(0)]).areas(main);
        list(f, inset(a), &items, Some(cur.sel), accent, false, app, RowNum::None);
        preview(f, inset(b), app, accent);
    } else {
        let [a, b, c] = Layout::horizontal([Constraint::Length(26), Constraint::Percentage(36), Constraint::Min(0)]).areas(main);
        let p = &app.cols[n - 2];
        let pv = p.view();
        let pitems: Vec<&Item> = pv.iter().map(|&i| &p.items[i]).collect();
        list(f, inset(a), &pitems, Some(p.sel), accent, true, app, row_num(&app.settings, &p.item));
        if items.is_empty() {
            let msg = col_msg(cur.loading, cur.error.as_deref(), !cur.filter.is_empty());
            f.render_widget(Paragraph::new(msg).style(Style::new().fg(soft())).wrap(Wrap { trim: true }), inset(b));
        } else {
            list(f, inset(b), &items, Some(cur.sel), accent, false, app, row_num(&app.settings, &cur.item));
        }
        preview(f, inset(c), app, accent);
    }

    draw_now(f, np, app, seed, accent, (hue_a, hue_b));

    // status line: search prompt > filter > toast > key hints
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
        Line::styled(" hjkl move · enter play/open · space pause · s sort · / filter · , settings · ? help · q quit", Style::new().fg(soft()))
    };
    f.render_widget(Paragraph::new(line), status);

    if app.help {
        help(f, f.area(), accent);
    }
    if let Some(c) = &app.card {
        card(f, f.area(), app, c, accent);
    }
    if let Some(cur) = app.menu {
        settings_menu(f, f.area(), &app.settings, &app.menu_rows(), cur, accent);
    }
}

fn col_msg(loading: bool, error: Option<&str>, filtered: bool) -> String {
    if let Some(e) = error {
        if e.contains("you own") { format!("{e}\n\nenter or P plays it as a whole") } else { e.to_string() }
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
            return f.render_widget(Paragraph::new(msg).style(Style::new().fg(soft())).wrap(Wrap { trim: true }), area);
        }
        let pv: Vec<&Item> = app.preview.iter().collect();
        list(f, area, &pv, None, accent, true, app, row_num(&app.settings, it));
    } else {
        track_info(f, area, it, accent);
    }
}

/// The secondary text of a row: the enabled fields for this kind of item, joined with " · ".
fn secondary(it: &Item, s: &Settings) -> String {
    let year = |d: &str| if d.len() >= 4 { d[..4].to_string() } else { String::new() };
    let mut parts: Vec<String> = vec![];
    match it.kind {
        Kind::Track => {
            let (artist, album) = it.sub.split_once(" · ").unwrap_or((&it.sub, ""));
            if s.t_artist { parts.push(artist.to_string()) }
            if s.t_album { parts.push(album.to_string()) }
            if s.t_year { parts.push(year(&it.date)) }
            if s.t_added && it.added.len() >= 10 { parts.push(it.added[..10].to_string()) }
        }
        Kind::Album => {
            if s.a_artist { parts.push(it.sub.clone()) }
            if s.a_year { parts.push(year(&it.date)) }
            if s.a_added && it.added.len() >= 10 { parts.push(it.added[..10].to_string()) }
        }
        Kind::Artist => {
            if s.r_genres { parts.push(it.genres.clone()) }
        }
        Kind::Playlist => {
            if s.p_owner { parts.push(it.sub.clone()) }
        }
        _ => {}
    }
    parts.retain(|p| !p.is_empty());
    parts.join(" · ")
}

/// The right-aligned text of a row.
fn right_text(it: &Item, s: &Settings) -> String {
    match it.kind {
        Kind::Track if s.t_length && it.dur_ms > 0 => fmt_ms(it.dur_ms as f64),
        Kind::Track => String::new(),
        Kind::Album => {
            let mut parts: Vec<String> = vec![];
            if s.acclaim != crate::acclaim::Show::Off {
                if let Some(r) = crate::acclaim::of(it) {
                    parts.push(crate::acclaim::cell(r, s.acclaim));
                }
            }
            if s.a_tracks && it.total > 0 {
                parts.push(format!("{} tr", it.total));
            }
            parts.push("›".into());
            parts.join("  ")
        }
        Kind::Playlist if s.p_count && it.total > 0 => format!("{} ›", it.total),
        _ => "›".into(),
    }
}

#[allow(clippy::too_many_arguments)]
fn list(f: &mut Frame, area: Rect, items: &[&Item], sel: Option<usize>, accent: Color, dim: bool, app: &App, num: RowNum) {
    let h = area.height as usize;
    let w = area.width as usize;
    if h == 0 || w < 6 {
        return;
    }
    let s = &app.settings;
    let cursor = sel.unwrap_or(0);
    let off = cursor.saturating_sub(h / 2).min(items.len().saturating_sub(h));
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
            let right = right_text(it, s);
            let is_playing = Some(it.id.as_str()) == playing_id;
            let mark = if is_playing { "♪ " } else { "  " };
            // optional leading number (tracks only)
            let number = match (num, it.kind) {
                (RowNum::Position, Kind::Track) => format!("{:>3} ", it.pos),
                (RowNum::TrackNo, Kind::Track) => format!("{:>3} ", if it.num > 0 { it.num } else { it.pos }),
                _ => String::new(),
            };
            let avail = w.saturating_sub(right.width() + mark.width() + number.width() + 1);
            let name = fit(&it.name, avail);
            // the name always wins; secondary fields only get what is left
            let room = avail.saturating_sub(name.width() + 2);
            let sec_full = secondary(it, s);
            let sec = if room >= 6 && !sec_full.is_empty() { format!("  {}", fit(&sec_full, room)) } else { String::new() };
            let pad = w.saturating_sub(mark.width() + number.width() + name.width() + sec.width() + right.width());
            let is_sel = sel == Some(i);
            let style = if is_sel {
                if dim { Style::new().bg(sel_dim()).fg(bright()) } else { Style::new().bg(accent).fg(Color::Black).bold() }
            } else if is_playing {
                Style::new().fg(accent).bold()
            } else if dim {
                Style::new().fg(soft())
            } else {
                Style::new().fg(bright())
            };
            if is_sel {
                // one flat span so the highlight bar is continuous
                return Line::styled(format!("{mark}{number}{name}{sec}{}{right}", " ".repeat(pad)), style);
            }
            Line::from(vec![
                Span::styled(mark.to_string(), style),
                Span::styled(number, Style::new().fg(faint())),
                Span::styled(name, style),
                Span::styled(sec, Style::new().fg(faint())),
                Span::styled(format!("{}{right}", " ".repeat(pad)), Style::new().fg(soft())),
            ])
        })
        .collect();
    f.render_widget(Paragraph::new(lines), area);
}

fn track_info(f: &mut Frame, area: Rect, it: &Item, accent: Color) {
    let (artist, album) = it.sub.split_once(" · ").unwrap_or((&it.sub, ""));
    let row = |k: &str, v: String| Line::from(vec![Span::styled(format!("{k:<9}"), Style::new().fg(soft())), Span::styled(v, Style::new().fg(bright()))]);
    let mut lines = vec![Line::styled(it.name.clone(), Style::new().fg(accent).bold()), Line::raw(""), row("artist", artist.to_string())];
    if !album.is_empty() {
        lines.push(row("album", album.to_string()));
    }
    if it.num > 0 {
        lines.push(row("track", format!("{}", it.num)));
    }
    if it.date.len() >= 4 {
        lines.push(row("released", it.date.clone()));
    }
    if it.added.len() >= 10 {
        lines.push(row("added", it.added[..10].to_string()));
    }
    if it.dur_ms > 0 {
        lines.push(row("length", fmt_ms(it.dur_ms as f64)));
    }
    lines.push(Line::raw(""));
    lines.push(Line::styled("enter ▸ play from here", Style::new().fg(soft())));
    f.render_widget(Paragraph::new(lines), area);
}

fn draw_now(f: &mut Frame, area: Rect, app: &App, seed: u64, accent: Color, hues: (f64, f64)) {
    let area = inset(area);
    let viz_on = app.settings.viz;
    let [title, vz, prog] = Layout::vertical([Constraint::Length(1), Constraint::Min(0), Constraint::Length(1)]).areas(area);

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
    let color_note = if viz::is_truecolor() { "" } else { "256c · " };
    let device = if app.device.is_empty() { String::new() } else { format!("{} · ", app.device) };
    f.render_widget(
        Paragraph::new(Line::styled(format!("{color_note}{device}vol {}%", app.volume), Style::new().fg(soft()))).right_aligned(),
        title,
    );

    let ratio = (app.progress_ms / dur).clamp(0.0, 1.0);
    if viz_on {
        let ctx = viz::Ctx { w: vz.width as usize, h: vz.height as usize, seed, t: app.t, progress: ratio, energy: app.energy, hue_a: hues.0, hue_b: hues.1 };
        f.render_widget(Paragraph::new(viz::render(0, &ctx)), vz);
    }

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

fn popup(area: Rect, w: u16, h: u16) -> Rect {
    let (w, h) = (w.min(area.width), h.min(area.height));
    Rect { x: area.x + (area.width - w) / 2, y: area.y + (area.height - h) / 2, width: w, height: h }
}

fn help(f: &mut Frame, area: Rect, accent: Color) {
    let rows = [
        ("h j k l / arrows", "move · out / down / up / in"),
        ("enter", "open · play track · start search"),
        ("P", "play whole playlist/album/list"),
        ("gg  G", "top · bottom"),
        ("ctrl-d  ctrl-u", "half page"),
        ("/", "fuzzy filter  (esc clears)"),
        ("s  S", "cycle sort (incl. acclaim) · reverse, remembered per list"),
        ("i", "card: album/track (critics, honors) · artist career*"),
        ("I", "card for what is playing now"),
        ("space", "play / pause"),
        ("n  p", "next · previous / restart"),
        ("<  >", "seek ∓10s"),
        ("+  -", "volume"),
        (",", "settings: columns, numbers, menu, colors"),
        ("R", "refresh from Spotify (and deadwax)"),
        ("q", "quit"),
    ];
    let r = popup(area, 64, rows.len() as u16 + 4);
    f.render_widget(Clear, r);
    let mut lines = vec![Line::styled(" keys", Style::new().fg(accent).bold()), Line::raw("")];
    lines.extend(rows.iter().map(|(k, d)| Line::from(vec![Span::styled(format!(" {k:<18}"), Style::new().fg(accent)), Span::styled(*d, Style::new().fg(bright()))])));
    f.render_widget(Paragraph::new(lines).block(Block::bordered().border_style(Style::new().fg(faint()))), r);
}

// ---------- the card (`i` / `I`) ----------

fn heading(t: &str) -> Line<'static> {
    Line::styled(format!(" {}", t.to_uppercase()), Style::new().fg(faint()).bold())
}
fn kv(k: &str, v: String) -> Line<'static> {
    Line::from(vec![Span::styled(format!("   {k:<15}"), Style::new().fg(soft())), Span::styled(v, Style::new().fg(bright()))])
}
fn note(t: &str) -> Line<'static> {
    Line::styled(format!(" {t}"), Style::new().fg(faint()))
}
fn secs(s: Option<u32>) -> String {
    s.map(|s| format!("{}:{:02}", s / 60, s % 60)).unwrap_or_default()
}
fn thousands(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}
fn same_track(a: &str, b: &str) -> bool {
    let (a, b) = (crate::acclaim::norm_title(a), crate::acclaim::norm_title(b));
    !a.is_empty() && a == b
}

/// The core block from the bundle-shaped record: score lines and the histogram.
fn core_lines(r: &crate::acclaim::Rec, accent: Color) -> Vec<Line<'static>> {
    let text = crate::acclaim::card(r);
    let mut out = vec![Line::styled(format!(" {}", text[0]), Style::new().fg(accent).bold())];
    for (i, l) in text.iter().enumerate().skip(1) {
        let style = match i {
            1 => Style::new().fg(soft()),
            _ if l.starts_with(['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█']) => Style::new().fg(accent),
            _ if l.trim_start().starts_with('▲') => Style::new().fg(accent).bold(),
            _ if l.starts_with("-4σ") => Style::new().fg(faint()),
            _ => Style::new().fg(bright()),
        };
        out.push(Line::styled(format!(" {l}"), style));
    }
    out
}

fn album_lines(app: &App, artist: &str, title: &str, track: Option<&str>, accent: Color, w: usize) -> Vec<Line<'static>> {
    let probe = mk(format!("xl:{artist}|{title}"), Kind::Album, title, artist);
    let live = app.dw.album(artist, title); // a cache read; `Failed` without side effects when off
    let detail = match &live {
        Fetch::Ready(a) => Some(a.clone()),
        _ => None,
    };
    let Some(r) = crate::acclaim::of(&probe).or(detail.as_ref().map(|a| a.rec)) else {
        let msg = match live {
            Fetch::Pending => "looking it up in deadwax…",
            Fetch::Missing => "deadwax has no score for this album",
            _ => "the critics' panel has no score for this album",
        };
        return vec![Line::styled(format!(" {title}"), Style::new().fg(accent).bold()), Line::styled(format!(" {artist}"), Style::new().fg(soft())), Line::raw(""), note(msg)];
    };
    let mut v = core_lines(r, accent);
    let standout_rank = |t: &str| detail.as_ref().and_then(|a| a.standout.iter().position(|s| same_track(&s.title, t)));
    if let Some(t) = track {
        let extra = standout_rank(t).map(|i| format!("   · #{} standout on Deezer", i + 1)).unwrap_or_default();
        v.insert(3, Line::from(vec![Span::styled(format!(" {:<13}", "track"), Style::new().fg(bright())), Span::styled(format!("{t}{extra}"), Style::new().fg(accent))]));
    }
    if matches!(live, Fetch::Pending) {
        v.push(Line::raw(""));
        v.push(note("fetching critics, honors and crowd from deadwax…"));
    }
    let Some(a) = detail else { return v };
    if !a.panel.is_empty() {
        v.push(Line::raw(""));
        v.push(heading(&format!("critics' panel · {}", a.panel.len())));
        for c in a.panel.iter().take(14) {
            let name = if c.retro { format!("{} (retro)", c.critic) } else { c.critic.clone() };
            let raw = c.raw.map(|x| format!("{x:.0}")).unwrap_or_else(|| "–".into());
            let line = format!("   {:<34} {raw:>4}/100   {:+.2}σ", fit(&name, 34), c.z);
            v.push(Line::styled(line, Style::new().fg(if c.z >= 0.0 { bright() } else { soft() })));
        }
        if a.panel.len() > 14 {
            v.push(note(&format!("  … and {} more", a.panel.len() - 14)));
        }
        if let Some((_, who)) = a.pitchfork.as_ref().filter(|p| !p.1.is_empty()) {
            v.push(note(&format!("  Pitchfork review by {who}")));
        }
    }
    if !a.acc.is_empty() || !a.awards.is_empty() {
        v.push(Line::raw(""));
        v.push(heading("honors"));
        for (l, rank) in &a.acc {
            v.push(Line::styled(fit(&format!("   ☰ {l}{}", rank.map(|r| format!(" · #{r}")).unwrap_or_default()), w), Style::new().fg(bright())));
        }
        for (aw, y) in &a.awards {
            v.push(Line::styled(fit(&format!("   ★ {aw}{}", y.map(|y| format!(" · {y}")).unwrap_or_default()), w), Style::new().fg(bright())));
        }
    }
    if let Some(c) = &a.crowd {
        let mut rows = vec![];
        if let Some(f) = c.fans {
            let gap = c.gap.map(|g| {
                let x = 10f32.powf(g.abs());
                if g.abs() < 0.05 { "about what its critics' rank predicts".to_string() } else { format!("{x:.1}× {} than its critics' rank predicts", if g > 0.0 { "more" } else { "fewer" }) }
            });
            rows.push(kv("Deezer fans", format!("{}{}", thousands(f), gap.map(|g| format!("   {g}")).unwrap_or_default())));
        }
        if let Some(m) = c.mb_rating {
            rows.push(kv("MusicBrainz", format!("{m:.1}/5{}", c.mb_votes.map(|n| format!(" from {n} votes")).unwrap_or_default())));
        }
        if let Some(sl) = c.sitelinks {
            rows.push(kv("Wikipedia", format!("{sl} language editions")));
        }
        if !rows.is_empty() {
            v.push(Line::raw(""));
            v.push(heading("crowd vs critics"));
            v.extend(rows);
        }
    }
    if !a.standout.is_empty() {
        v.push(Line::raw(""));
        v.push(heading("standout tracks · by Deezer rank"));
        for t in &a.standout {
            let hot = track.is_some_and(|x| same_track(x, &t.title));
            let line = format!("   {}{:>2}. {}   {}", if hot { "▶ " } else { "  " }, t.pos, fit(&t.title, w.saturating_sub(18)), secs(t.secs));
            v.push(Line::styled(line, if hot { Style::new().fg(accent).bold() } else { Style::new().fg(bright()) }));
        }
    }
    if let Some(t) = track.filter(|_| !a.tracklist.is_empty()) {
        v.push(Line::raw(""));
        v.push(heading("tracklist"));
        for x in &a.tracklist {
            let hot = same_track(t, &x.title);
            let line = format!("   {}{:>2}. {}   {}", if hot { "▶ " } else { "  " }, x.pos, fit(&x.title, w.saturating_sub(18)), secs(x.secs));
            v.push(Line::styled(line, if hot { Style::new().fg(accent).bold() } else { Style::new().fg(soft()) }));
        }
    }
    if !a.neighbours.is_empty() {
        v.push(Line::raw(""));
        let era = if r.year > 0 { format!(" · {}, {}–{}", r.genre, r.year.saturating_sub(3), r.year + 3) } else { String::new() };
        v.push(heading(&format!("similar era{era}")));
        for p in &a.neighbours {
            let right = format!("{:+.1}σ p{}", p.z, crate::acclaim::pct_str(p.pct));
            let left = fit(&format!("   {}  {} — {}", p.year, p.artist, p.title), w.saturating_sub(right.width() + 2));
            let pad = w.saturating_sub(left.width() + right.width());
            v.push(Line::from(vec![Span::styled(left, Style::new().fg(bright())), Span::styled(format!("{}{right}", " ".repeat(pad)), Style::new().fg(soft()))]));
        }
    }
    v
}

/// ▁..█ for a z-score between -1.5σ and +3.5σ.
fn spark(z: f32) -> char {
    const B: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    B[(((z + 1.5) / 5.0 * 7.0).round().clamp(0.0, 7.0)) as usize]
}

fn artist_lines(app: &App, name: &str, accent: Color, w: usize) -> Vec<Line<'static>> {
    let a = match app.dw.artist(name) {
        Fetch::Ready(a) => a,
        other => {
            let msg = match other {
                Fetch::Pending => "looking up the career in deadwax…",
                Fetch::Missing => "deadwax has no scored albums for this artist",
                _ => "deadwax did not answer",
            };
            return vec![Line::styled(format!(" {name}"), Style::new().fg(accent).bold()), Line::raw(""), note(msg)];
        }
    };
    let years = match (a.begin.get(..4), a.end.get(..4)) {
        (Some(b), Some(e)) => format!("{b}–{e}"),
        (Some(b), None) => format!("{b}–"),
        _ => String::new(),
    };
    let meta: Vec<String> = [a.country.clone(), a.kind.to_lowercase(), years].into_iter().filter(|x| !x.is_empty()).collect();
    let z = |x: Option<f32>| x.map(|x| format!("{x:+.2}σ")).unwrap_or_else(|| "–".into());
    let mut v = vec![Line::styled(format!(" {}", a.name), Style::new().fg(accent).bold()), Line::styled(format!(" {}", meta.join(" · ")), Style::new().fg(soft())), Line::raw("")];
    v.push(kv("studio albums", format!("{}   mean {} · best {}", a.n_studio, z(a.mean_z), z(a.best_z))));
    if let Some((t, y)) = &a.peak {
        v.push(kv("peak", format!("{t} ({y})")));
    }
    v.push(kv("debut → latest", format!("{} → {}", z(a.debut_z), z(a.latest_z))));
    let studio: Vec<&crate::deadwax::ArtistAlbum> = a.albums.iter().filter(|x| x.kind == "studio").collect();
    if studio.len() >= 2 {
        v.push(Line::raw(""));
        v.push(heading("career curve · studio albums, oldest → newest"));
        let curve: String = studio.iter().map(|x| spark(x.z)).collect();
        let (y0, y1) = (studio[0].year, studio[studio.len() - 1].year);
        v.push(Line::from(vec![Span::styled(format!("   {y0} "), Style::new().fg(faint())), Span::styled(curve, Style::new().fg(accent)), Span::styled(format!(" {y1}"), Style::new().fg(faint()))]));
    }
    v.push(Line::raw(""));
    v.push(heading(&format!("discography · {} scored", a.albums.len())));
    const BAR: usize = 12;
    for x in &a.albums {
        let cells = (((x.z + 2.0) / 6.0 * BAR as f32).round().clamp(0.0, BAR as f32)) as usize;
        let bar = format!("{}{}", "█".repeat(cells), "·".repeat(BAR - cells));
        let tag = if x.kind != "studio" && !x.kind.is_empty() { format!(" ({})", x.kind) } else { String::new() };
        let right = format!("{:+.1}σ p{:<5} {:>2} cr", x.z, crate::acclaim::pct_str(x.pct), x.n);
        let title_w = w.saturating_sub(7 + BAR + 2 + right.width() + 2);
        let left = format!("   {}  {}", if x.year > 0 { x.year.to_string() } else { "    ".into() }, fit(&format!("{}{tag}", x.title), title_w));
        let pad = w.saturating_sub(left.width() + BAR + 2 + right.width());
        v.push(Line::from(vec![
            Span::styled(left, Style::new().fg(bright())),
            Span::raw(" ".repeat(pad)),
            Span::styled(bar, Style::new().fg(if x.lb >= 1.0 { accent } else { soft() })),
            Span::styled(format!("  {right}"), Style::new().fg(soft())),
        ]));
    }
    v
}

/// The card overlay: scrollable, fills in as live data arrives.
fn card(f: &mut Frame, area: Rect, app: &App, c: &Card, accent: Color) {
    let w = 84u16.min(area.width);
    let inner_w = w.saturating_sub(3) as usize;
    let mut lines = match &c.of {
        CardOf::Album { artist, title, track } => album_lines(app, artist, title, track.as_deref(), accent, inner_w),
        CardOf::Artist { name } => artist_lines(app, name, accent, inner_w),
    };
    lines.push(Line::raw(""));
    let src = match app.dw.meta().filter(|_| app.dw.on()) {
        Some(m) => format!("deadwax: latent model over ~96 critics · live, {} scored albums ({})", m.n_albums, m.mode),
        None => "deadwax: latent model over ~96 critics".to_string(),
    };
    lines.push(note(&src));
    let r = popup(area, w, lines.len() as u16 + 3);
    f.render_widget(Clear, r);
    let inner_h = r.height.saturating_sub(3) as usize; // two borders + the footer
    let max = lines.len().saturating_sub(inner_h);
    c.max.set(max);
    let off = c.scroll.min(max);
    let mut shown: Vec<Line> = lines.into_iter().skip(off).take(inner_h).collect();
    let more = match (off > 0, off < max) {
        (true, true) => "  ↑↓",
        (false, true) => "  ↓ more",
        (true, false) => "  ↑",
        _ => "",
    };
    let link = if app.card_url().is_some() { "o open in deadwax · " } else { "" };
    shown.push(Line::styled(format!(" j/k scroll · {link}esc/i close{more}"), Style::new().fg(faint())));
    f.render_widget(Paragraph::new(shown).block(Block::bordered().border_style(Style::new().fg(faint()))), r);
}

/// The settings overlay (`,`): one scrolling list built from `settings::rows()`.
fn settings_menu(f: &mut Frame, area: Rect, s: &Settings, rows: &[Row], cursor: usize, accent: Color) {
    let r = popup(area, 72, rows.len() as u16 + 4);
    f.render_widget(Clear, r);
    let inner_h = r.height.saturating_sub(4) as usize; // title + blank + footer + border
    let off = cursor.saturating_sub(inner_h / 2).min(rows.len().saturating_sub(inner_h));
    let w = r.width.saturating_sub(4) as usize;
    let mut lines = vec![Line::styled(" settings", Style::new().fg(accent).bold())];
    for (i, row) in rows.iter().enumerate().skip(off).take(inner_h) {
        let (label, value) = s.label(*row);
        if let Row::Title(_) = row {
            lines.push(Line::styled(format!(" {}", label.to_uppercase()), Style::new().fg(faint()).bold()));
            continue;
        }
        let shown = match value.as_str() {
            "on" => "● on".to_string(),
            "off" => "○ off".to_string(),
            v => v.to_string(),
        };
        let pad = w.saturating_sub(label.width() + shown.width() + 3);
        let text = format!("  {label}{}{shown} ", " ".repeat(pad));
        lines.push(if i == cursor {
            Line::styled(text, Style::new().bg(accent).fg(Color::Black).bold())
        } else {
            Line::from(vec![Span::styled(format!("  {label}"), Style::new().fg(bright())), Span::styled(format!("{}{shown} ", " ".repeat(pad)), Style::new().fg(if value == "off" { faint() } else { accent }))])
        });
    }
    lines.push(Line::styled(" j/k move · space/enter/l change · h change back · esc close", Style::new().fg(faint())));
    f.render_widget(Paragraph::new(lines).block(Block::bordered().border_style(Style::new().fg(faint()))), r);
}

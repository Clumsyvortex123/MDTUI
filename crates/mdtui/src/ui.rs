//! Drawing: title bar, outline, editor | preview split, statusline, command line and overlays.

use mdtui_editor::Mode;
use mdtui_plugin::Level;
use mdtui_render::Viewport;
use ratatui::layout::{Constraint, Layout as RLayout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, Borders, Clear, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Widget, Wrap,
};
use ratatui::Frame;
use ratatui_image::Image;
use unicode_width::UnicodeWidthStr;

use crate::app::{App, FsButton, Input, Pane};

pub fn draw(f: &mut Frame, app: &mut App) {
    let area = f.area();
    let theme = app.theme.clone();
    if app.fullscreen.is_some() {
        draw_fullscreen(f, app, area);
        return;
    }
    let [title, main, status, cmd] = RLayout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(area);

    // ---- panes
    let mut rest = main;
    if app.show_toc {
        let w = app.cfg.toc_width.min(main.width / 3).max(12);
        let [toc, r] = RLayout::horizontal([Constraint::Length(w), Constraint::Min(1)]).areas(main);
        rest = r;
        draw_toc(f, app, toc);
    } else {
        app.toc_area = Rect::default();
    }
    let preview_area;
    let focused = app.pane == Pane::Editor && app.input == Input::None;
    if let Some(ed) = &mut app.editor {
        let ew = ((rest.width as f32 * app.split) as u16).clamp(10, rest.width.saturating_sub(10));
        let ed_area = Rect { width: ew, ..rest };
        let div = Rect { x: rest.x + ew, width: 1, ..rest };
        preview_area = Rect {
            x: rest.x + ew + 1,
            width: rest.width.saturating_sub(ew + 1),
            ..rest
        };
        ed.render(ed_area, f.buffer_mut(), &theme, focused);
        app.editor_area = ed_area;
        app.divider_x = Some(div.x);
        let div_style = Style::default().fg(if app.dragging_divider { theme.accent } else { theme.border });
        for y in div.y..div.bottom() {
            f.buffer_mut().set_string(div.x, y, "┃", div_style);
        }
    } else {
        app.editor_area = Rect::default();
        app.divider_x = None;
        preview_area = rest;
    }

    // preview
    if app.view.width != preview_area.width || app.view.height != preview_area.height {
        app.view = preview_area;
        app.invalidate_layout();
    }
    app.view = preview_area;
    app.relayout_if_needed();
    app.prepare_media();
    let focus = app.focus_idx.and_then(|i| app.layout.hits.focusables().get(i).map(|h| (*h).clone()));
    let (matches, current) = match &app.search {
        Some(s) => (s.matches.as_slice(), s.current),
        None => (&[][..], None),
    };
    let vp = Viewport {
        layout: &app.layout,
        scroll: app.scroll,
        media: &app.media,
        theme: &theme,
        matches,
        current_match: current,
        focus: focus.as_ref(),
        selection: app.selection,
        x_offset: app.x_offset,
    };
    f.render_widget(vp, preview_area);
    if app.layout.height() > preview_area.height as usize {
        let mut sb = ScrollbarState::new(app.layout.height().saturating_sub(preview_area.height as usize))
            .position(app.scroll);
        f.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(None)
                .end_symbol(None)
                .track_symbol(Some(" "))
                .thumb_style(Style::default().fg(theme.border)),
            preview_area,
            &mut sb,
        );
    }

    // completion popup
    if let Some(ed) = &mut app.editor {
        let pos = ed.cursor_screen();
        if let (Some(p), Some(pos)) = (&mut ed.popup, pos) {
            p.render(pos, main, &theme, f.buffer_mut());
        }
    }

    draw_title(f, app, title);
    draw_status(f, app, status);
    draw_cmdline(f, app, cmd, main);
    draw_notifications(f, app, main);
    if app.help {
        draw_help(f, app, main);
    }
}

fn draw_title(f: &mut Frame, app: &App, area: Rect) {
    let t = &app.theme;
    let base = Style::default().bg(t.sidebar_bg).fg(t.fg);
    let dirty = app.editor.as_ref().is_some_and(|e| e.is_dirty());
    let mut left = vec![
        Span::styled(" mdtui ", base.fg(t.accent).add_modifier(Modifier::BOLD)),
        Span::styled("─ ", base.fg(t.muted)),
        Span::styled(app.file_name(), base.add_modifier(Modifier::BOLD)),
    ];
    if dirty {
        left.push(Span::styled(" [+]", base.fg(t.warn)));
    }
    let mut right = Vec::new();
    if let Some(p) = &app.plugins {
        if !p.loaded().is_empty() {
            right.push(Span::styled(format!("⚙ {} ", p.loaded().len()), base.fg(t.muted)));
        }
    }
    right.push(Span::styled(format!("▣ {} ", app.media.protocol_name()), base.fg(t.muted)));
    if app.is_live() {
        right.push(Span::styled("● live ", base.fg(t.ok)));
    } else if app.stdin_mode {
        right.push(Span::styled("○ stdin ", base.fg(t.muted)));
    }
    fill_line(f, area, left, right, base);
}

fn fill_line(f: &mut Frame, area: Rect, left: Vec<Span<'static>>, right: Vec<Span<'static>>, base: Style) {
    let lw: usize = left.iter().map(|s| s.content.width()).sum();
    let rw: usize = right.iter().map(|s| s.content.width()).sum();
    let pad = (area.width as usize).saturating_sub(lw + rw);
    let mut spans = left;
    spans.push(Span::styled(" ".repeat(pad), base));
    spans.extend(right);
    f.render_widget(Paragraph::new(Line::from(spans)).style(base), area);
}

fn draw_toc(f: &mut Frame, app: &mut App, area: Rect) {
    let t = app.theme.clone();
    let focused = app.pane == Pane::Toc;
    let block = Block::default()
        .borders(Borders::RIGHT)
        .border_style(Style::default().fg(if focused { t.accent } else { t.border }))
        .title(Span::styled(" OUTLINE ", Style::default().fg(t.muted).add_modifier(Modifier::BOLD)))
        .style(Style::default().bg(t.sidebar_bg));
    let inner = block.inner(area);
    let inner = Rect { y: inner.y + 1, height: inner.height.saturating_sub(1), ..inner };
    f.render_widget(block, area);
    app.toc_area = inner;

    let current = app.layout.headings.iter().rposition(|(_, r)| *r <= app.scroll + 1);
    let h = inner.height as usize;
    if focused {
        if app.toc_selected < app.toc_scroll {
            app.toc_scroll = app.toc_selected;
        } else if app.toc_selected >= app.toc_scroll + h {
            app.toc_scroll = app.toc_selected + 1 - h;
        }
    } else if let Some(c) = current {
        // keep the current section visible
        if c < app.toc_scroll || c >= app.toc_scroll + h {
            app.toc_scroll = c.saturating_sub(h / 2);
        }
    }
    let min_level = app.layout.headings.iter().map(|(e, _)| e.level).min().unwrap_or(1);
    for (i, (e, _)) in app.layout.headings.iter().enumerate().skip(app.toc_scroll).take(h) {
        let y = inner.y + (i - app.toc_scroll) as u16;
        let indent = "  ".repeat((e.level - min_level) as usize);
        let glyph = if app.is_folded(&e.anchor) { "▸ " } else if e.level <= min_level + 1 { "▾ " } else { "• " };
        let mut style = Style::default().fg(if e.level == min_level { t.fg } else { t.muted }).bg(t.sidebar_bg);
        if Some(i) == current {
            style = style.fg(t.accent).add_modifier(Modifier::BOLD);
        }
        if focused && i == app.toc_selected {
            style = style.bg(t.focus_bg);
        }
        let mut text = format!(" {indent}{glyph}{}", e.text);
        while text.width() > inner.width as usize {
            text.pop();
        }
        let pad = (inner.width as usize).saturating_sub(text.width());
        f.buffer_mut().set_string(inner.x, y, format!("{text}{}", " ".repeat(pad)), style);
    }
}

fn mode_badge(app: &App) -> (&'static str, ratatui::style::Color) {
    let t = &app.theme;
    match &app.input {
        Input::Command { .. } => return ("COMMAND", t.warn),
        Input::Search { .. } => return ("SEARCH", t.warn),
        Input::None => {}
    }
    if app.pane == Pane::Editor {
        if let Some(ed) = &app.editor {
            return match ed.mode() {
                Mode::Normal => ("NORMAL", t.accent),
                Mode::Insert => ("INSERT", t.ok),
                Mode::Visual => ("VISUAL", t.headings[3]),
                Mode::Search => ("SEARCH", t.warn),
            };
        }
    }
    if app.pane == Pane::Toc {
        return ("OUTLINE", t.headings[1]);
    }
    ("VIEW", t.accent)
}

fn draw_status(f: &mut Frame, app: &App, area: Rect) {
    let t = &app.theme;
    let base = Style::default().bg(t.sidebar_bg).fg(t.fg);
    let (mode, color) = mode_badge(app);
    let mut left = vec![
        Span::styled(format!(" {mode} "), Style::default().bg(color).fg(t.statusbar_fg).add_modifier(Modifier::BOLD)),
        Span::styled(format!(" {}", app.file_name()), base),
    ];
    if app.editor.as_ref().is_some_and(|e| e.is_dirty()) {
        left.push(Span::styled(" [+]", base.fg(t.warn)));
    }
    let crumbs: Vec<String> = app
        .layout
        .breadcrumb(app.scroll + 1)
        .iter()
        .map(|h| h.text.clone())
        .collect();
    if !crumbs.is_empty() {
        let mut c = crumbs.join(" › ");
        let max = (area.width as usize / 3).max(10);
        if c.width() > max {
            c = format!("…{}", c.chars().rev().take(max).collect::<String>().chars().rev().collect::<String>());
        }
        left.push(Span::styled(format!("   ¶ {c}"), base.fg(t.muted)));
    }

    let mut right = Vec::new();
    if let Some(ed) = &app.editor {
        if let Some(d) = ed.diagnostic_at_cursor() {
            let mut msg = d.message.clone();
            msg.truncate(50);
            right.push(Span::styled(format!("{msg}  "), base.fg(if d.severity == 1 { t.error } else { t.warn })));
        }
        if let Some(l) = &app.lsp {
            let (sym, c) = if l.is_ready() { ("✓", t.ok) } else { ("…", t.muted) };
            right.push(Span::styled(format!("{} {sym}  ", l.name), base.fg(c)));
        }
    }
    if let Some(s) = &app.search {
        if !s.matches.is_empty() {
            right.push(Span::styled(
                format!("/{} [{}/{}]  ", s.query, s.current.map(|c| c + 1).unwrap_or(0), s.matches.len()),
                base.fg(t.warn),
            ));
        }
    }
    let total = app.layout.height().max(1);
    let pct = if total <= app.view.height as usize {
        "All".to_string()
    } else if app.scroll == 0 {
        "Top".into()
    } else if app.scroll + app.view.height as usize >= total {
        "Bot".into()
    } else {
        format!("{}%", app.scroll * 100 / total.saturating_sub(app.view.height as usize).max(1))
    };
    let pos = match (&app.editor, app.pane) {
        (Some(ed), Pane::Editor) => {
            let (r, c) = ed.cursor();
            format!("ln {}, col {}", r + 1, c + 1)
        }
        _ => format!("ln {}/{}", app.layout.line_for_row(app.scroll), app.source.lines().count()),
    };
    right.push(Span::styled(format!("{pct}  "), base.fg(t.muted)));
    right.push(Span::styled(format!(" {pos} "), Style::default().bg(t.statusbar_bg).fg(t.statusbar_fg)));
    fill_line(f, area, left, right, base);
}

fn draw_cmdline(f: &mut Frame, app: &App, area: Rect, main: Rect) {
    let t = &app.theme;
    let base = Style::default().fg(t.fg);
    match &app.input {
        Input::Command { text, wild } => {
            let line = Line::from(vec![Span::styled(":", base.fg(t.accent)), Span::styled(text.clone(), base)]);
            f.render_widget(Paragraph::new(line), area);
            f.set_cursor_position((area.x + 1 + text.width() as u16, area.y));
            // wildmenu above the statusline
            let cands = app.command_candidates(if wild.is_some() { &app.wild_base } else { text });
            if !cands.is_empty() && (wild.is_some() || !text.is_empty()) {
                let y = main.bottom().saturating_sub(1);
                let wm = Rect { y, height: 1, ..main };
                let mut spans = vec![Span::raw(" ")];
                for (i, c) in cands.iter().enumerate().take(30) {
                    let label = c.rsplit(' ').next().unwrap_or(c).to_string();
                    let st = if Some(i) == *wild {
                        Style::default().bg(t.focus_bg).fg(t.fg).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(t.fg)
                    };
                    spans.push(Span::styled(label, st));
                    spans.push(Span::raw("  "));
                }
                f.render_widget(Clear, wm);
                f.render_widget(Paragraph::new(Line::from(spans)).style(Style::default().bg(t.sidebar_bg)), wm);
            }
        }
        Input::Search { text, .. } => {
            let line = Line::from(vec![Span::styled("/", base.fg(t.accent)), Span::styled(text.clone(), base)]);
            f.render_widget(Paragraph::new(line), area);
            f.set_cursor_position((area.x + 1 + text.width() as u16, area.y));
        }
        Input::None => {
            let mut left = Vec::new();
            if let (Pane::Editor, Some(ed)) = (app.pane, &app.editor) {
                match ed.mode() {
                    Mode::Search => {
                        // edtui's own status line is hidden, so echo the pattern here like vim
                        let pat = ed.state.search_pattern();
                        f.render_widget(
                            Paragraph::new(Line::from(vec![Span::styled("/", base.fg(t.accent)), Span::styled(pat.clone(), base)])),
                            area,
                        );
                        f.set_cursor_position((area.x + 1 + pat.width() as u16, area.y));
                        return;
                    }
                    Mode::Insert => left.push(Span::styled("-- INSERT --", base.add_modifier(Modifier::BOLD))),
                    Mode::Visual => left.push(Span::styled("-- VISUAL --", base.add_modifier(Modifier::BOLD))),
                    _ => {}
                }
            } else if app.focus_idx.is_some() {
                if let Some(a) = app.focused_action() {
                    left.push(Span::styled(describe(&a), Style::default().fg(t.muted)));
                }
            } else {
                left.push(Span::styled("? help  : command  / search  e edit  Tab links  o outline", Style::default().fg(t.muted)));
            }
            let right = vec![Span::styled(app.keymap.pending(), base.fg(t.muted))];
            fill_line(f, area, left, right, Style::default());
        }
    }
}

fn describe(a: &mdtui_core::Action) -> String {
    use mdtui_core::Action::*;
    match a {
        OpenLink(u) => format!("↗ {u}   (Enter open · y copy)"),
        ToggleTask { line, .. } => format!("✓ task on line {line}   (Enter/x toggle)"),
        CopyCode(_) => "⧉ copy code   (Enter)".into(),
        OpenMedia(_) => "⤢ open fullscreen   (Enter)".into(),
        ToggleSource(_) => "toggle diagram source   (Enter)".into(),
        ToggleRenderer(_) => "switch diagram between text and image   (Enter)".into(),
        Footnote(n) => format!("footnote {n}   (Enter jump · H back)"),
        Fold(a) => format!("fold #{a}"),
        FrontMatter => "front matter".into(),
    }
}

fn draw_notifications(f: &mut Frame, app: &App, main: Rect) {
    let t = &app.theme;
    let w = 44.min(main.width.saturating_sub(2));
    let mut y = main.y;
    for n in app.notifications.iter().rev().take(5) {
        let (color, icon) = match n.level {
            Level::Info => (t.info, "ℹ"),
            Level::Warn => (t.warn, "⚠"),
            Level::Error => (t.error, "✖"),
        };
        let inner_w = w.saturating_sub(4) as usize;
        let lines = textwrap_simple(&format!("{icon} {}", n.msg), inner_w.max(1));
        let h = (lines.len() as u16 + 2).min(6);
        if y + h > main.bottom() {
            break;
        }
        let area = Rect::new(main.right().saturating_sub(w + 1), y, w, h);
        f.render_widget(Clear, area);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(color))
            .style(Style::default().bg(t.sidebar_bg).fg(t.fg));
        let p = Paragraph::new(lines.into_iter().map(Line::from).collect::<Vec<_>>()).block(block);
        f.render_widget(p, area);
        y += h;
    }
}

fn textwrap_simple(s: &str, width: usize) -> Vec<String> {
    let mut out = Vec::new();
    for para in s.lines() {
        let mut line = String::new();
        for word in para.split(' ') {
            if !line.is_empty() && line.width() + 1 + word.width() > width {
                out.push(std::mem::take(&mut line));
            }
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(word);
        }
        out.push(line);
    }
    out
}

const HELP: &[(&str, &str)] = &[
    ("j k ↑ ↓ / wheel", "scroll"),
    ("Ctrl-d Ctrl-u  Space", "half page / page"),
    ("gg G", "top / bottom"),
    ("]] [[  } {", "next / previous heading"),
    ("Tab S-Tab", "cycle links, tasks, buttons"),
    ("Enter  click", "follow link / toggle / open image"),
    ("x", "toggle focused (or first visible) task"),
    ("y  drag", "copy link or code / copy selection"),
    ("/ n N", "search"),
    ("o  gO", "outline sidebar"),
    ("za zM zR", "fold section / fold all / unfold"),
    ("H L  Ctrl-o", "back / forward"),
    ("e  :vsplit", "open editor split"),
    ("Ctrl-w h/l/w", "switch pane"),
    ("Ctrl-Space", "completion (editor)"),
    (":w :q :wq :e file", "write / quit / open"),
    (":theme :set :toc", "options"),
    (":plugins :lsp", "status"),
    ("[⤢] / Enter on diagram", "fullscreen: drag to pan, 0 recenter"),
    ("r", "reload (re-render diagrams)"),
    ("q", "quit"),
];

fn draw_help(f: &mut Frame, app: &App, main: Rect) {
    let t = &app.theme;
    let w = 66.min(main.width);
    let h = (HELP.len() as u16 + 4).min(main.height);
    let area = Rect::new(main.x + (main.width - w) / 2, main.y + (main.height - h) / 2, w, h);
    f.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(t.accent))
        .title(" mdtui keys ")
        .title_bottom(Line::from(" any key to close ").right_aligned())
        .style(Style::default().bg(t.sidebar_bg).fg(t.fg));
    let lines: Vec<Line> = HELP
        .iter()
        .map(|(k, d)| {
            Line::from(vec![
                Span::styled(format!(" {k:<22}"), Style::default().fg(t.accent).add_modifier(Modifier::BOLD)),
                Span::raw(*d),
            ])
        })
        .collect();
    f.render_widget(Paragraph::new(lines).block(block).wrap(Wrap { trim: false }), area);
}

fn draw_fullscreen(f: &mut Frame, app: &mut App, area: Rect) {
    let t = app.theme.clone();
    let [title, body, foot] = RLayout::vertical([Constraint::Length(1), Constraint::Min(1), Constraint::Length(1)]).areas(area);
    // the view's limits depend on the area: set it, then position / clamp before drawing
    app.fs_body = body;
    if app.fullscreen.as_ref().is_some_and(|f| f.needs_init) {
        app.fs_init_view();
    }
    app.fs_clamp();
    let Some(fs) = &app.fullscreen else { return };
    let (key, zoom, pan, dragging) = (fs.key.clone(), fs.zoom, fs.pan, fs.drag.is_some());
    let base = Style::default().bg(t.sidebar_bg).fg(t.fg);
    let is_text = app.media.text_lines(&key).is_some();
    let dims = app
        .media
        .image_dims(&key)
        .map(|(w, h)| format!("{w}×{h}"))
        .or_else(|| app.media.text_lines(&key).map(|l| format!("{}×{} cells", mdtui_render::diagram::width(l), l.len())))
        .unwrap_or_default();
    let name = match key.strip_prefix("img:") {
        Some(url) => url.to_string(),
        None => format!("{} diagram", key.split(':').next().unwrap_or("")),
    };

    // title bar with clickable buttons on the right
    let btn = Style::default().bg(t.sidebar_bg).fg(t.link);
    let buttons = [(" [⌖ recenter] ", FsButton::Recenter), (" [✕ close] ", FsButton::Close)];
    let right_w: u16 = buttons.iter().map(|(l, _)| l.width() as u16).sum();
    let mut bx = title.right().saturating_sub(right_w);
    app.fs_buttons.clear();
    let mut right = Vec::new();
    for (label, b) in buttons {
        let w = label.width() as u16;
        app.fs_buttons.push((Rect::new(bx, title.y, w, 1), b));
        right.push(Span::styled(label, btn));
        bx += w;
    }
    fill_line(
        f,
        title,
        vec![
            Span::styled(format!(" {name} "), base.add_modifier(Modifier::BOLD)),
            Span::styled(if is_text { format!("─ {dims}") } else { format!("─ {dims} ─ {zoom}×") }, base.fg(t.muted)),
        ],
        right,
        base,
    );

    // footer: hints and where we are
    let pos = match app.fs_bounds() {
        Some((lo, hi)) if hi != lo => {
            let pct = |v: i32, a: i32, b: i32| if b > a { (v - a) * 100 / (b - a) } else { 0 };
            format!(" ↔ {}%  ↕ {}% ", pct(pan.0, lo.0, hi.0), pct(pan.1, lo.1, hi.1))
        }
        _ => " fits ".into(),
    };
    let hints = if is_text {
        " drag / h j k l / wheel (Shift: sideways) to pan · 0 recenter · Esc close "
    } else {
        " drag / h j k l to pan · wheel or +/- zoom · 0 recenter · Esc close "
    };
    fill_line(
        f,
        foot,
        vec![Span::styled(hints, base.fg(t.muted))],
        vec![Span::styled(pos, base.fg(if dragging { t.accent } else { t.muted }))],
        base,
    );

    f.render_widget(Clear, body);
    if let Some(lines) = app.media.text_lines(&key) {
        let p = Paragraph::new(lines.to_vec()).scroll((pan.1.max(0) as u16, pan.0.max(0) as u16));
        let w = mdtui_render::diagram::width(lines) as u16;
        let h = lines.len() as u16;
        // centre when it fits, pan otherwise
        let x = if w < body.width { body.x + (body.width - w) / 2 } else { body.x };
        let y = if h < body.height { body.y + (body.height - h) / 2 } else { body.y };
        f.render_widget(p, Rect::new(x, y, body.width - (x - body.x), body.height - (y - body.y)));
        return;
    }
    if let Some(proto) = app.media.fullscreen(&key, (body.width, body.height), zoom, pan) {
        let size = proto.size();
        let x = body.x + body.width.saturating_sub(size.width) / 2;
        let y = body.y + body.height.saturating_sub(size.height) / 2;
        Image::new(proto).render(Rect::new(x, y, size.width.min(body.width), size.height.min(body.height)), f.buffer_mut());
    }
}

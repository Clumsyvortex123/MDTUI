//! Keyboard and mouse event routing.

use mdtui_editor::Mode;
use ratatui::crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};

use crate::app::{App, FsButton, Input, Pane};
use crate::keys::{Cmd, Outcome};

pub fn handle(app: &mut App, ev: Event) {
    match ev {
        Event::Key(k) if k.kind != KeyEventKind::Release => key(app, k),
        Event::Mouse(m) => mouse(app, m),
        Event::Paste(text) => paste(app, text),
        Event::Resize(..) => app.invalidate_layout(),
        _ => {}
    }
}

fn paste(app: &mut App, text: String) {
    match &mut app.input {
        Input::Command { text: t, .. } | Input::Search { text: t, .. } => {
            t.push_str(text.lines().next().unwrap_or(""));
            app.dirty = true;
        }
        Input::None => {
            if let (Pane::Editor, Some(ed)) = (app.pane, &mut app.editor) {
                if ed.paste(text) {
                    app.editor_changed();
                }
            }
        }
    }
}

fn key(app: &mut App, k: KeyEvent) {
    app.dirty = true;
    if app.help {
        app.help = false;
        return;
    }
    if app.fullscreen.is_some() {
        fullscreen_key(app, k);
        return;
    }
    match app.input.clone() {
        Input::Command { text, wild } => return command_key(app, k, text, wild),
        Input::Search { text, origin } => return search_key(app, k, text, origin),
        Input::None => {}
    }
    match app.pane {
        Pane::Editor if app.editor.is_some() => editor_key(app, k),
        Pane::Toc => toc_key(app, k),
        _ => viewer_key(app, k),
    }
}

fn viewer_key(app: &mut App, k: KeyEvent) {
    match app.keymap.feed(&k) {
        Outcome::Run(cmd) => app.run(cmd),
        Outcome::Pending | Outcome::None => {}
    }
}

fn toc_key(app: &mut App, k: KeyEvent) {
    let n = app.layout.headings.len();
    match k.code {
        KeyCode::Char('j') | KeyCode::Down if n > 0 => app.toc_selected = (app.toc_selected + 1).min(n - 1),
        KeyCode::Char('k') | KeyCode::Up => app.toc_selected = app.toc_selected.saturating_sub(1),
        KeyCode::Char('g') => app.toc_selected = 0,
        KeyCode::Char('G') => app.toc_selected = n.saturating_sub(1),
        KeyCode::Enter | KeyCode::Char('o') | KeyCode::Char(' ') => app.toc_jump(app.toc_selected),
        KeyCode::Char('l') | KeyCode::Right => {
            app.toc_jump(app.toc_selected);
            app.focus_pane('l');
        }
        KeyCode::Esc | KeyCode::Char('h') => app.pane = Pane::Preview,
        KeyCode::Char('q') => app.run(Cmd::ToggleToc),
        _ => viewer_key(app, k),
    }
}

fn editor_key(app: &mut App, k: KeyEvent) {
    let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
    let ed = app.editor.as_mut().unwrap();
    let mode = ed.mode();

    // completion popup navigation
    if ed.popup.is_some() {
        match (k.code, ctrl) {
            (KeyCode::Down, _) | (KeyCode::Char('n'), true) => {
                ed.popup.as_mut().unwrap().next();
                return;
            }
            (KeyCode::Up, _) | (KeyCode::Char('p'), true) => {
                ed.popup.as_mut().unwrap().prev();
                return;
            }
            (KeyCode::Tab, _) | (KeyCode::Enter, _) => {
                app.accept_completion();
                return;
            }
            (KeyCode::Esc, _) => {
                ed.popup = None;
                // fall through so Esc also leaves insert mode, like vim
            }
            _ => {}
        }
    }

    // window commands and app-level keys
    if app.keymap.pending() == "<C-w>" || (ctrl && k.code == KeyCode::Char('w') && mode == Mode::Normal) {
        viewer_key(app, k);
        return;
    }
    if ctrl && k.code == KeyCode::Char('s') {
        app.save();
        return;
    }
    if ctrl && k.code == KeyCode::Char(' ') {
        app.trigger_completion(true);
        return;
    }
    if mode == Mode::Normal && k.code == KeyCode::Char(':') && !ctrl {
        app.input = Input::Command { text: String::new(), wild: None };
        return;
    }

    let ed = app.editor.as_mut().unwrap();
    let changed = ed.key(k);
    if changed {
        app.editor_changed();
    }
    let ed = app.editor.as_mut().unwrap();
    if ed.mode() != Mode::Insert {
        ed.popup = None;
        return;
    }
    if changed && app.cfg.editor.auto_complete {
        // auto-trigger on typing, keep open while the prefix still matches
        let typed = matches!(k.code, KeyCode::Char(_) | KeyCode::Backspace);
        if typed {
            app.trigger_completion(false);
        }
    }
}

fn command_key(app: &mut App, k: KeyEvent, mut text: String, wild: Option<usize>) {
    match k.code {
        KeyCode::Esc => app.input = Input::None,
        KeyCode::Enter => {
            app.input = Input::None;
            app.ex(&text);
        }
        KeyCode::Tab | KeyCode::BackTab => {
            let base = match wild {
                // cycling: complete from what was typed before the first Tab
                Some(_) => app.wild_base.clone(),
                None => {
                    app.wild_base = text.clone();
                    text.clone()
                }
            };
            let cands = app.command_candidates(&base);
            if cands.is_empty() {
                return;
            }
            let i = match (wild, k.code == KeyCode::BackTab) {
                (None, false) => 0,
                (None, true) => cands.len() - 1,
                (Some(i), false) => (i + 1) % cands.len(),
                (Some(i), true) => (i + cands.len() - 1) % cands.len(),
            };
            app.input = Input::Command { text: cands[i].clone(), wild: Some(i) };
        }
        KeyCode::Up | KeyCode::Down => {
            if let Some(h) = app.history_step(k.code == KeyCode::Up) {
                app.input = Input::Command { text: h, wild: None };
            }
        }
        KeyCode::Backspace => {
            if text.pop().is_none() {
                app.input = Input::None;
            } else {
                app.input = Input::Command { text, wild: None };
            }
        }
        KeyCode::Char('u') if k.modifiers.contains(KeyModifiers::CONTROL) => {
            app.input = Input::Command { text: String::new(), wild: None };
        }
        KeyCode::Char(c) => {
            text.push(c);
            app.input = Input::Command { text, wild: None };
        }
        _ => {}
    }
}

fn search_key(app: &mut App, k: KeyEvent, mut text: String, origin: usize) {
    match k.code {
        KeyCode::Esc => {
            app.input = Input::None;
            app.search = None;
            app.scroll_to(origin);
        }
        KeyCode::Enter => {
            app.input = Input::None;
            app.finish_search(text);
        }
        KeyCode::Backspace => {
            if text.pop().is_none() {
                app.input = Input::None;
                app.search = None;
                return;
            }
            app.update_search(&text, origin);
            app.input = Input::Search { text, origin };
        }
        KeyCode::Char(c) => {
            text.push(c);
            app.update_search(&text, origin);
            app.input = Input::Search { text, origin };
        }
        _ => {}
    }
}

fn fullscreen_key(app: &mut App, k: KeyEvent) {
    let (sx, sy) = (4, 2);
    match k.code {
        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Enter => app.fullscreen = None,
        KeyCode::Char('+') | KeyCode::Char('=') => app.fs_zoom(1),
        KeyCode::Char('-') => app.fs_zoom(-1),
        KeyCode::Char('0') | KeyCode::Char('c') => app.fs_recenter(),
        KeyCode::Char('h') | KeyCode::Left => app.fs_pan_by(-sx, 0),
        KeyCode::Char('l') | KeyCode::Right => app.fs_pan_by(sx, 0),
        KeyCode::Char('k') | KeyCode::Up => app.fs_pan_by(0, -sy),
        KeyCode::Char('j') | KeyCode::Down => app.fs_pan_by(0, sy),
        KeyCode::Char('g') | KeyCode::Home => app.fs_pan_by(-100_000, -100_000),
        KeyCode::Char('G') | KeyCode::End => app.fs_pan_by(100_000, 100_000),
        _ => {}
    }
}

fn fullscreen_mouse(app: &mut App, m: MouseEvent) {
    let (x, y) = (m.column, m.row);
    let is_text = app.fullscreen.as_ref().is_some_and(|f| app.media.text_lines(&f.key).is_some());
    let shift = m.modifiers.contains(KeyModifiers::SHIFT);
    match m.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            let button = app.fs_buttons.iter().find(|(r, _)| inside(*r, x, y)).map(|(_, b)| *b);
            match button {
                Some(FsButton::Recenter) => app.fs_recenter(),
                Some(FsButton::Close) => app.fullscreen = None,
                None => app.fs_drag_start(x, y),
            }
        }
        MouseEventKind::Drag(MouseButton::Left) => app.fs_drag_to(x, y),
        MouseEventKind::Up(MouseButton::Left) => app.fs_drag_end(),
        // text diagrams scroll with the wheel (Shift = sideways); images zoom
        MouseEventKind::ScrollDown if is_text && shift => app.fs_pan_by(4, 0),
        MouseEventKind::ScrollUp if is_text && shift => app.fs_pan_by(-4, 0),
        MouseEventKind::ScrollDown if is_text => app.fs_pan_by(0, 3),
        MouseEventKind::ScrollUp if is_text => app.fs_pan_by(0, -3),
        MouseEventKind::ScrollRight => app.fs_pan_by(4, 0),
        MouseEventKind::ScrollLeft => app.fs_pan_by(-4, 0),
        MouseEventKind::ScrollUp => app.fs_zoom(1),
        MouseEventKind::ScrollDown => app.fs_zoom(-1),
        _ => return,
    }
    app.dirty = true;
}

fn inside(r: ratatui::layout::Rect, x: u16, y: u16) -> bool {
    x >= r.x && x < r.right() && y >= r.y && y < r.bottom()
}

fn mouse(app: &mut App, m: MouseEvent) {
    let (x, y) = (m.column, m.row);
    if app.help {
        if matches!(m.kind, MouseEventKind::Down(_)) {
            app.help = false;
            app.dirty = true;
        }
        return;
    }
    if app.fullscreen.is_some() {
        fullscreen_mouse(app, m);
        return;
    }

    // divider drag
    if let Some(dx) = app.divider_x {
        match m.kind {
            MouseEventKind::Down(MouseButton::Left) if x == dx && inside(app.editor_area.union(app.view), x, y) => {
                app.dragging_divider = true;
                return;
            }
            MouseEventKind::Drag(MouseButton::Left) if app.dragging_divider => {
                let total = app.editor_area.union(app.view);
                if total.width > 0 {
                    app.split = ((x.saturating_sub(total.x)) as f32 / total.width as f32).clamp(0.15, 0.85);
                    app.invalidate_layout();
                }
                return;
            }
            MouseEventKind::Up(MouseButton::Left) if app.dragging_divider => {
                app.dragging_divider = false;
                return;
            }
            _ => {}
        }
    }

    let step = app.cfg.scroll_step;
    if inside(app.view, x, y) {
        match m.kind {
            MouseEventKind::ScrollDown => app.run(Cmd::Down(step)),
            MouseEventKind::ScrollUp => app.run(Cmd::Up(step)),
            MouseEventKind::Down(MouseButton::Left) => app.preview_press(x, y),
            MouseEventKind::Drag(MouseButton::Left) => app.preview_drag(x, y),
            MouseEventKind::Up(MouseButton::Left) => app.preview_release(x, y),
            _ => {}
        }
        return;
    }
    if matches!(m.kind, MouseEventKind::Drag(MouseButton::Left)) {
        // selection drag leaving the preview keeps extending it
        app.preview_drag(x, y);
        return;
    }
    if matches!(m.kind, MouseEventKind::Up(MouseButton::Left)) {
        app.preview_release(x, y);
    }
    if app.editor.is_some() && inside(app.editor_area, x, y) {
        if matches!(m.kind, MouseEventKind::Down(_)) {
            app.pane = Pane::Editor;
            if let Some(ed) = &mut app.editor {
                ed.popup = None;
            }
        }
        if let Some(ed) = &mut app.editor {
            ed.mouse(m);
        }
        app.dirty = true;
        return;
    }
    if app.show_toc && inside(app.toc_area, x, y) {
        match m.kind {
            MouseEventKind::ScrollDown => {
                app.toc_scroll = (app.toc_scroll + step).min(app.layout.headings.len().saturating_sub(1))
            }
            MouseEventKind::ScrollUp => app.toc_scroll = app.toc_scroll.saturating_sub(step),
            MouseEventKind::Down(MouseButton::Left) => {
                let i = app.toc_scroll + (y - app.toc_area.y) as usize;
                if i < app.layout.headings.len() {
                    app.pane = Pane::Toc;
                    app.toc_jump(i);
                }
            }
            _ => {}
        }
        app.dirty = true;
    }
}

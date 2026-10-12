//! The editor pane: an edtui vim-modal editor plus completion popup and LSP diagnostics.

pub mod completion;
pub mod lsp;

use std::path::{Path, PathBuf};

use edtui::actions::{DeleteChar, InsertChar, LineBreak};
use edtui::{
    EditorEventHandler, EditorMode, EditorState, EditorTheme, EditorView, Lines, LineNumbers,
    SyntaxHighlighter,
};
use mdtui_core::Theme;
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{KeyEvent, MouseEvent};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::Widget;

pub use completion::{CompletionSet, Item, Kind, Popup};
pub use edtui::EditorMode as Mode;
pub use lsp::{Diagnostic, Lsp, LspEvent};

pub struct Editor {
    pub state: EditorState,
    handler: EditorEventHandler,
    pub path: Option<PathBuf>,
    saved: String,
    text_cache: String,
    pub popup: Option<Popup>,
    pub diagnostics: Vec<Diagnostic>,
    pub relative_numbers: bool,
    area: Rect,
}

impl Editor {
    pub fn new(text: &str, path: Option<PathBuf>) -> Self {
        Editor {
            state: EditorState::new(Lines::from(text)),
            handler: EditorEventHandler::default(),
            path,
            saved: text.to_string(),
            text_cache: text.to_string(),
            popup: None,
            diagnostics: Vec::new(),
            relative_numbers: false,
            area: Rect::default(),
        }
    }

    pub fn text(&self) -> &str {
        &self.text_cache
    }

    fn refresh(&mut self) -> bool {
        let t = self.state.lines.to_string();
        if t != self.text_cache {
            self.text_cache = t;
            true
        } else {
            false
        }
    }

    pub fn is_dirty(&self) -> bool {
        self.text_cache != self.saved
    }

    pub fn mark_saved(&mut self) {
        self.saved = self.text_cache.clone();
    }

    /// Replace the buffer (e.g. after an external change), keeping the cursor where possible.
    pub fn set_text(&mut self, text: &str) {
        let cursor = self.state.cursor;
        self.state.lines = Lines::from(text);
        let last = self.state.lines.len().saturating_sub(1);
        self.state.cursor.row = cursor.row.min(last);
        self.state.cursor.col = cursor
            .col
            .min(self.state.lines.len_col(self.state.cursor.row).unwrap_or(0));
        self.text_cache = text.to_string();
        self.saved = text.to_string();
    }

    /// Replace the buffer contents as an edit (the buffer becomes dirty; cursor kept).
    pub fn replace_text(&mut self, text: &str) {
        let cursor = self.state.cursor;
        self.state.lines = Lines::from(text);
        let last = self.state.lines.len().saturating_sub(1);
        self.state.cursor.row = cursor.row.min(last);
        self.state.cursor.col = cursor
            .col
            .min(self.state.lines.len_col(self.state.cursor.row).unwrap_or(0));
        self.text_cache = text.to_string();
    }

    pub fn mode(&self) -> EditorMode {
        self.state.mode
    }

    /// (row, col), 0-based, col in chars.
    pub fn cursor(&self) -> (usize, usize) {
        (self.state.cursor.row, self.state.cursor.col)
    }

    pub fn set_cursor_row(&mut self, row: usize) {
        let last = self.state.lines.len().saturating_sub(1);
        self.state.cursor.row = row.min(last);
        self.state.cursor.col = 0;
    }

    pub fn line_count(&self) -> usize {
        self.state.lines.len()
    }

    /// Current line up to the cursor.
    pub fn before_cursor(&self) -> String {
        let (r, c) = self.cursor();
        self.text_cache
            .lines()
            .nth(r)
            .map(|l| l.chars().take(c).collect())
            .unwrap_or_default()
    }

    pub fn current_line(&self) -> String {
        self.text_cache.lines().nth(self.cursor().0).unwrap_or("").to_string()
    }

    pub fn dir(&self) -> PathBuf {
        self.path
            .as_deref()
            .and_then(Path::parent)
            .filter(|p| !p.as_os_str().is_empty())
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."))
    }

    /// Feed a key to edtui. Returns true if the text changed.
    pub fn key(&mut self, key: KeyEvent) -> bool {
        self.handler.on_key_event(key, &mut self.state);
        self.refresh()
    }

    pub fn mouse(&mut self, ev: MouseEvent) {
        self.handler.on_mouse_event(ev, &mut self.state);
    }

    pub fn paste(&mut self, text: String) -> bool {
        self.handler.on_paste_event(text, &mut self.state);
        self.refresh()
    }

    /// Delete `replace_chars` before the cursor and insert `text` (with `$0` cursor marker).
    pub fn insert_completion(&mut self, replace_chars: usize, text: &str) {
        if replace_chars > 0 {
            self.state.execute(DeleteChar(replace_chars));
        }
        // continuation lines of multi-line snippets keep the current indentation
        let indent: String = self.current_line().chars().take_while(|c| c.is_whitespace()).collect();
        let (pre, post) = match text.find("$0") {
            Some(i) => (&text[..i], &text[i + 2..]),
            None => (text, ""),
        };
        let put = |s: &str, st: &mut EditorState| {
            for (i, part) in s.split('\n').enumerate() {
                if i > 0 {
                    st.execute(LineBreak(1));
                    for c in indent.chars() {
                        st.execute(InsertChar(c));
                    }
                }
                for c in part.chars() {
                    st.execute(InsertChar(c));
                }
            }
        };
        put(pre, &mut self.state);
        let mark = self.state.cursor;
        put(post, &mut self.state);
        self.state.cursor = mark;
        self.state.mode = EditorMode::Insert;
        self.refresh();
    }

    pub fn area(&self) -> Rect {
        self.area
    }

    /// Screen position of the cursor (after the last render).
    pub fn cursor_screen(&self) -> Option<(u16, u16)> {
        self.state.cursor_screen_position().map(|p| (p.x, p.y))
    }

    pub fn render(&mut self, area: Rect, buf: &mut Buffer, theme: &Theme, focused: bool) {
        self.area = area;
        // sign column for diagnostics
        let sign_w = 2u16.min(area.width);
        let ed_area = Rect {
            x: area.x + sign_w,
            width: area.width.saturating_sub(sign_w),
            ..area
        };
        let mut et = EditorTheme::default()
            .base(Style::default().fg(theme.fg))
            .selection_style(Style::default().bg(theme.selection_bg))
            .line_numbers_style(Style::default().fg(theme.muted))
            .hide_status_line();
        et = if focused {
            et.cursor_style(Style::default().bg(theme.fg).fg(theme.bg))
        } else {
            et.hide_cursor()
        };
        let hl = SyntaxHighlighter::new("dracula", "md")
            .ok()
            .map(|h| h.custom_theme(mdtui_core::highlight::syntect_theme(theme)));
        EditorView::new(&mut self.state)
            .theme(et)
            .wrap(false)
            .line_numbers(if self.relative_numbers { LineNumbers::Relative } else { LineNumbers::Absolute })
            .syntax_highlighter(hl)
            .render(ed_area, buf);

        let (_, off_y) = self.state.viewport_offset();
        for d in &self.diagnostics {
            if d.line < off_y || d.line >= off_y + area.height as usize {
                continue;
            }
            let y = area.y + (d.line - off_y) as u16;
            let (sym, color) = match d.severity {
                1 => ("E", theme.error),
                2 => ("W", theme.warn),
                _ => ("I", theme.info),
            };
            buf.set_string(area.x, y, sym, Style::default().fg(color).add_modifier(Modifier::BOLD));
        }
    }

    /// Diagnostic on the cursor line, if any.
    pub fn diagnostic_at_cursor(&self) -> Option<&Diagnostic> {
        let r = self.cursor().0;
        self.diagnostics.iter().find(|d| d.line == r)
    }
}

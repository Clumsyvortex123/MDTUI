//! The scrolling document view: draws visible layout lines, overlays search matches, focus and
//! selection, and draws images over the rows reserved for them.

use mdtui_core::hitmap::Hit;
use mdtui_core::layout::Layout;
use mdtui_core::search::Match;
use mdtui_core::Theme;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::Widget;
use ratatui_image::sliced::{SignedPosition, SlicedImage};

use crate::media::{placeholder_style, MediaStore};

/// A text selection in document coordinates: (row, col) start and end, inclusive start,
/// exclusive end, normalised so start <= end.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    pub start: (usize, u16),
    pub end: (usize, u16),
}

impl Selection {
    pub fn normalized(a: (usize, u16), b: (usize, u16)) -> Self {
        if a <= b {
            Selection { start: a, end: b }
        } else {
            Selection { start: b, end: a }
        }
    }

    pub fn contains(&self, row: usize, col: u16) -> bool {
        (row, col) >= self.start && (row, col) < self.end
    }

    /// Extract the selected text from a layout.
    pub fn text(&self, layout: &Layout) -> String {
        let mut out = String::new();
        for row in self.start.0..=self.end.0.min(layout.height().saturating_sub(1)) {
            let line = layout.plain_line(row);
            let mut col = 0u16;
            let mut s = String::new();
            for ch in line.chars() {
                let w = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0) as u16;
                if self.contains(row, col) {
                    s.push(ch);
                }
                col += w.max(1);
            }
            if row > self.start.0 {
                out.push('\n');
            }
            out.push_str(s.trim_end());
        }
        out
    }
}

pub struct Viewport<'a> {
    pub layout: &'a Layout,
    pub scroll: usize,
    pub media: &'a MediaStore,
    pub theme: &'a Theme,
    pub matches: &'a [Match],
    pub current_match: Option<usize>,
    pub focus: Option<&'a Hit>,
    pub selection: Option<Selection>,
    /// Left offset of the document inside the area (centering / padding).
    pub x_offset: u16,
}

impl Widget for Viewport<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let h = area.height as usize;
        let end = (self.scroll + h).min(self.layout.height());
        for (i, row) in (self.scroll..end).enumerate() {
            let y = area.y + i as u16;
            let x = area.x + self.x_offset;
            let w = area.width.saturating_sub(self.x_offset);
            buf.set_line(x, y, &self.layout.lines[row], w);
        }

        let patch = |buf: &mut Buffer, row: usize, c0: u16, c1: u16, style: Style| {
            if row < self.scroll || row >= self.scroll + h {
                return;
            }
            let y = area.y + (row - self.scroll) as u16;
            for c in c0..c1 {
                let x = area.x + self.x_offset + c;
                if x >= area.right() {
                    break;
                }
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.set_style(style);
                }
            }
        };

        for (i, m) in self.matches.iter().enumerate() {
            let bg = if Some(i) == self.current_match {
                self.theme.search_current_bg
            } else {
                self.theme.search_bg
            };
            patch(buf, m.row, m.col_start, m.col_end, Style::default().bg(bg));
        }
        if let Some(f) = self.focus {
            // highlight every row of a multi-line link
            for h in self.layout.hits.all().iter().filter(|h| h.action == f.action) {
                if h.row + 3 >= f.row && h.row <= f.row + 3 {
                    patch(buf, h.row, h.col_start, h.col_end, Style::default().bg(self.theme.focus_bg));
                }
            }
        }
        if let Some(sel) = self.selection {
            for row in sel.start.0..=sel.end.0 {
                let c0 = if row == sel.start.0 { sel.start.1 } else { 0 };
                let c1 = if row == sel.end.0 { sel.end.1 } else { area.width };
                patch(buf, row, c0, c1, Style::default().bg(self.theme.selection_bg));
            }
        }

        // images
        let inner = Rect {
            x: area.x + self.x_offset,
            width: area.width.saturating_sub(self.x_offset),
            ..area
        };
        for p in &self.layout.media {
            let top = p.row as i64 - self.scroll as i64;
            let bottom = top + p.height as i64;
            if bottom <= 0 || top >= h as i64 {
                continue;
            }
            match self.media.protocol(&p.key, p.width, p.height) {
                Some(proto) => {
                    let pos = SignedPosition {
                        x: p.col as i16,
                        y: top as i16,
                    };
                    SlicedImage::new(proto, pos).render(inner, buf);
                }
                None => {
                    // placeholder shading until the encoder thread finishes
                    for r in top.max(0)..bottom.min(h as i64) {
                        let y = inner.y + r as u16;
                        for c in 0..p.width {
                            let x = inner.x + p.col + c;
                            if x >= inner.right() {
                                break;
                            }
                            if let Some(cell) = buf.cell_mut((x, y)) {
                                cell.set_symbol("░").set_style(placeholder_style());
                            }
                        }
                    }
                }
            }
        }
    }
}

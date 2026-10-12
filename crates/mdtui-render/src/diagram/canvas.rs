//! A character grid for drawing diagrams. Line segments are stored as direction masks so that
//! crossings and corners merge into the right box-drawing glyph (`┼`, `┬`, `└`, …).

use ratatui::style::Style;
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthChar;

pub const UP: u8 = 1;
pub const DOWN: u8 = 2;
pub const LEFT: u8 = 4;
pub const RIGHT: u8 = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stroke {
    Light,
    Dashed,
    Heavy,
}

#[derive(Clone)]
struct Cell {
    ch: char,
    style: Style,
    mask: u8,
    stroke: Stroke,
    /// Set by text / boxes; lines never draw over it.
    solid: bool,
    /// Second half of a wide character.
    cont: bool,
}

impl Default for Cell {
    fn default() -> Self {
        Cell {
            ch: ' ',
            style: Style::default(),
            mask: 0,
            stroke: Stroke::Light,
            solid: false,
            cont: false,
        }
    }
}

pub struct Canvas {
    pub width: usize,
    pub height: usize,
    cells: Vec<Cell>,
}

impl Canvas {
    pub fn new(width: usize, height: usize) -> Self {
        Canvas {
            width,
            height,
            cells: vec![Cell::default(); width * height],
        }
    }

    fn cell(&mut self, x: i64, y: i64) -> Option<&mut Cell> {
        if x < 0 || y < 0 || x as usize >= self.width || y as usize >= self.height {
            return None;
        }
        Some(&mut self.cells[y as usize * self.width + x as usize])
    }

    pub fn is_solid(&self, x: i64, y: i64) -> bool {
        if x < 0 || y < 0 || x as usize >= self.width || y as usize >= self.height {
            return true;
        }
        self.cells[y as usize * self.width + x as usize].solid
    }

    /// Write text; it becomes solid (lines don't overwrite it).
    pub fn text(&mut self, x: i64, y: i64, s: &str, style: Style) {
        let mut cx = x;
        for ch in s.chars() {
            let w = ch.width().unwrap_or(0) as i64;
            if w == 0 {
                continue;
            }
            if let Some(c) = self.cell(cx, y) {
                *c = Cell { ch, style, mask: 0, stroke: Stroke::Light, solid: true, cont: false };
            }
            if w == 2 {
                if let Some(c) = self.cell(cx + 1, y) {
                    *c = Cell { ch: ' ', style, mask: 0, stroke: Stroke::Light, solid: true, cont: true };
                }
            }
            cx += w;
        }
    }

    /// A single glyph that is not merged with lines (arrow heads, markers).
    pub fn glyph(&mut self, x: i64, y: i64, ch: char, style: Style) {
        if let Some(c) = self.cell(x, y) {
            *c = Cell { ch, style, mask: 0, stroke: Stroke::Light, solid: true, cont: false };
        }
    }

    fn add_mask(&mut self, x: i64, y: i64, m: u8, stroke: Stroke, style: Style) {
        if let Some(c) = self.cell(x, y) {
            if c.solid {
                return;
            }
            c.mask |= m;
            if c.mask == m || stroke != Stroke::Light {
                c.stroke = stroke;
            }
            c.style = style;
        }
    }

    /// Horizontal line from x0 to x1 (inclusive) on row y.
    pub fn hline(&mut self, x0: i64, x1: i64, y: i64, stroke: Stroke, style: Style) {
        let (a, b) = (x0.min(x1), x0.max(x1));
        for x in a..=b {
            let mut m = 0;
            if x > a {
                m |= LEFT;
            }
            if x < b {
                m |= RIGHT;
            }
            if a == b {
                m = LEFT | RIGHT;
            }
            self.add_mask(x, y, m, stroke, style);
        }
    }

    /// Vertical line from y0 to y1 (inclusive) on column x.
    pub fn vline(&mut self, x: i64, y0: i64, y1: i64, stroke: Stroke, style: Style) {
        let (a, b) = (y0.min(y1), y0.max(y1));
        for y in a..=b {
            let mut m = 0;
            if y > a {
                m |= UP;
            }
            if y < b {
                m |= DOWN;
            }
            if a == b {
                m = UP | DOWN;
            }
            self.add_mask(x, y, m, stroke, style);
        }
    }

    /// Orthogonal polyline through `pts` (consecutive points must share x or y).
    pub fn polyline(&mut self, pts: &[(i64, i64)], stroke: Stroke, style: Style) {
        for w in pts.windows(2) {
            let ((x0, y0), (x1, y1)) = (w[0], w[1]);
            if y0 == y1 {
                self.hline(x0, x1, y0, stroke, style);
            } else {
                self.vline(x0, y0, y1, stroke, style);
            }
        }
    }

    pub fn to_lines(&self) -> Vec<Line<'static>> {
        let mut out = Vec::with_capacity(self.height);
        for y in 0..self.height {
            let row = &self.cells[y * self.width..(y + 1) * self.width];
            let mut spans: Vec<Span<'static>> = Vec::new();
            let mut buf = String::new();
            let mut cur = Style::default();
            for c in row {
                if c.cont {
                    continue;
                }
                let ch = if c.mask != 0 { glyph(c.mask, c.stroke) } else { c.ch };
                let st = if ch == ' ' { Style::default() } else { c.style };
                if st != cur && !buf.is_empty() {
                    spans.push(Span::styled(std::mem::take(&mut buf), cur));
                }
                cur = st;
                buf.push(ch);
            }
            let trimmed = buf.trim_end().to_string();
            if !trimmed.is_empty() {
                spans.push(Span::styled(trimmed, cur));
            }
            // drop trailing whitespace-only spans
            while spans.last().is_some_and(|s| s.content.trim().is_empty()) {
                spans.pop();
            }
            out.push(Line::from(spans));
        }
        // trim empty rows at the bottom
        while out.last().is_some_and(|l| l.spans.is_empty()) {
            out.pop();
        }
        out
    }
}

fn glyph(mask: u8, stroke: Stroke) -> char {
    let straight_h = matches!(mask, LEFT | RIGHT) || mask == LEFT | RIGHT;
    let straight_v = matches!(mask, UP | DOWN) || mask == UP | DOWN;
    match stroke {
        Stroke::Dashed if straight_h => return '┄',
        Stroke::Dashed if straight_v => return '┆',
        Stroke::Heavy if straight_h => return '━',
        Stroke::Heavy if straight_v => return '┃',
        _ => {}
    }
    match mask {
        m if m == LEFT | RIGHT || m == LEFT || m == RIGHT => '─',
        m if m == UP | DOWN || m == UP || m == DOWN => '│',
        m if m == DOWN | RIGHT => '┌',
        m if m == DOWN | LEFT => '┐',
        m if m == UP | RIGHT => '└',
        m if m == UP | LEFT => '┘',
        m if m == UP | DOWN | RIGHT => '├',
        m if m == UP | DOWN | LEFT => '┤',
        m if m == LEFT | RIGHT | DOWN => '┬',
        m if m == LEFT | RIGHT | UP => '┴',
        _ => '┼',
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(c: &Canvas) -> Vec<String> {
        c.to_lines()
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect()
    }

    #[test]
    fn junctions_merge() {
        let mut c = Canvas::new(5, 3);
        c.polyline(&[(2, 0), (2, 1), (0, 1), (0, 2)], Stroke::Light, Style::default());
        c.polyline(&[(2, 0), (2, 1), (4, 1), (4, 2)], Stroke::Light, Style::default());
        assert_eq!(text(&c), vec!["  │", "┌─┴─┐", "│   │"]);
    }

    #[test]
    fn text_is_not_overwritten() {
        let mut c = Canvas::new(5, 1);
        c.text(1, 0, "ab", Style::default());
        c.hline(0, 4, 0, Stroke::Light, Style::default());
        assert_eq!(text(&c), vec!["─ab──"]);
    }
}

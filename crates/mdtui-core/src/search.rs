//! Case-smart search over laid-out lines.

use unicode_width::UnicodeWidthStr;

use crate::layout::Layout;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Match {
    pub row: usize,
    pub col_start: u16,
    pub col_end: u16,
}

/// Find all matches of `query` (smart case: case-insensitive unless it contains uppercase).
pub fn search(layout: &Layout, query: &str) -> Vec<Match> {
    if query.is_empty() {
        return Vec::new();
    }
    let case_sensitive = query.chars().any(|c| c.is_uppercase());
    let q = if case_sensitive {
        query.to_string()
    } else {
        query.to_lowercase()
    };
    let mut out = Vec::new();
    for (row, line) in layout.lines.iter().enumerate() {
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        let hay = if case_sensitive {
            text.clone()
        } else {
            text.to_lowercase()
        };
        // lowercasing can change byte lengths for some scripts; fall back to exact-case search then
        let hay = if hay.len() == text.len() { hay } else { text.clone() };
        let mut from = 0;
        while let Some(i) = hay[from..].find(&q) {
            let s = from + i;
            let e = s + q.len();
            let col_start = text[..s].width() as u16;
            let col_end = col_start + text[s..e].width() as u16;
            out.push(Match {
                row,
                col_start,
                col_end,
            });
            from = e.max(s + 1);
            while from < hay.len() && !hay.is_char_boundary(from) {
                from += 1;
            }
        }
    }
    out
}

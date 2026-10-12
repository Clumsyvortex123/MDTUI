//! Word wrapping of styled, possibly clickable text fragments.

use ratatui::style::Style;
use ratatui::text::Span;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::hitmap::Action;

#[derive(Debug, Clone)]
pub(crate) enum Piece {
    Frag {
        text: String,
        style: Style,
        action: Option<Action>,
    },
    /// Hard line break.
    Break,
}

impl Piece {
    pub fn new(text: impl Into<String>, style: Style) -> Self {
        Piece::Frag {
            text: text.into(),
            style,
            action: None,
        }
    }
}

#[derive(Debug, Default)]
pub(crate) struct Wrapped {
    pub lines: Vec<Vec<Span<'static>>>,
    /// (row, col_start, col_end, action), columns relative to the wrapped text's origin.
    pub hits: Vec<(usize, usize, usize, Action)>,
}

#[derive(Clone)]
struct Atom {
    text: String,
    style: Style,
    action: Option<Action>,
    space: bool,
}

fn atoms(pieces: &[Piece]) -> Vec<Option<Atom>> {
    // None = hard break
    let mut out = Vec::new();
    for p in pieces {
        match p {
            Piece::Break => out.push(None),
            Piece::Frag {
                text,
                style,
                action,
            } => {
                let mut cur = String::new();
                let mut cur_space = None;
                for g in text.graphemes(true) {
                    let is_space = g == " " || g == "\t";
                    let g = if g == "\t" { " " } else { g };
                    if cur_space.is_some_and(|s| s != is_space) {
                        out.push(Some(Atom {
                            text: std::mem::take(&mut cur),
                            style: *style,
                            action: action.clone(),
                            space: cur_space.unwrap(),
                        }));
                    }
                    cur_space = Some(is_space);
                    cur.push_str(g);
                }
                if !cur.is_empty() {
                    out.push(Some(Atom {
                        text: cur,
                        style: *style,
                        action: action.clone(),
                        space: cur_space.unwrap_or(false),
                    }));
                }
            }
        }
    }
    out
}

struct Builder {
    width: usize,
    lines: Vec<Vec<Span<'static>>>,
    cur: Vec<Span<'static>>,
    col: usize,
    hits: Vec<(usize, usize, usize, Action)>,
}

impl Builder {
    fn newline(&mut self) {
        // drop trailing whitespace
        while let Some(last) = self.cur.last_mut() {
            let t = last.content.trim_end().to_string();
            if t.is_empty() {
                self.cur.pop();
            } else {
                last.content = t.into();
                break;
            }
        }
        self.lines.push(std::mem::take(&mut self.cur));
        self.col = 0;
    }

    fn push(&mut self, a: &Atom) {
        let w = a.text.width();
        if let Some(act) = &a.action {
            let row = self.lines.len();
            match self.hits.last_mut() {
                Some((r, _, end, last)) if *r == row && *end == self.col && last == act => {
                    *end += w
                }
                _ => self.hits.push((row, self.col, self.col + w, act.clone())),
            }
        }
        match self.cur.last_mut() {
            Some(last) if last.style == a.style => {
                last.content = format!("{}{}", last.content, a.text).into()
            }
            _ => self.cur.push(Span::styled(a.text.clone(), a.style)),
        }
        self.col += w;
    }

    fn push_word(&mut self, word: &[Atom]) {
        let w: usize = word.iter().map(|a| a.text.width()).sum();
        if self.col > 0 && self.col + w > self.width {
            self.newline();
        }
        if w <= self.width.saturating_sub(self.col) || self.width == 0 {
            for a in word {
                self.push(a);
            }
            return;
        }
        // word longer than the line: break by grapheme
        for a in word {
            for g in a.text.graphemes(true) {
                let gw = g.width();
                if self.col + gw > self.width && self.col > 0 {
                    self.newline();
                }
                self.push(&Atom {
                    text: g.to_string(),
                    ..a.clone()
                });
            }
        }
    }
}

pub(crate) fn wrap(pieces: &[Piece], width: usize) -> Wrapped {
    let width = width.max(1);
    let mut b = Builder {
        width,
        lines: Vec::new(),
        cur: Vec::new(),
        col: 0,
        hits: Vec::new(),
    };
    let mut word: Vec<Atom> = Vec::new();
    for a in atoms(pieces) {
        match a {
            None => {
                b.push_word(&std::mem::take(&mut word));
                b.newline();
            }
            Some(a) if a.space => {
                b.push_word(&std::mem::take(&mut word));
                if b.col > 0 && b.col < width {
                    b.push(&Atom {
                        text: " ".into(),
                        ..a
                    });
                }
            }
            Some(a) => word.push(a),
        }
    }
    b.push_word(&word);
    if !b.cur.is_empty() || b.lines.is_empty() {
        b.newline();
    }
    Wrapped {
        lines: b.lines,
        hits: b.hits,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(w: &Wrapped) -> Vec<String> {
        w.lines
            .iter()
            .map(|l| l.iter().map(|s| s.content.as_ref()).collect())
            .collect()
    }

    #[test]
    fn wraps_words() {
        let w = wrap(&[Piece::new("the quick brown fox jumps", Style::default())], 10);
        assert_eq!(text(&w), vec!["the quick", "brown fox", "jumps"]);
    }

    #[test]
    fn keeps_adjacent_fragments_together() {
        let w = wrap(
            &[
                Piece::new("aaaa ", Style::default()),
                Piece::new("bold", Style::default().bold()),
                Piece::new(", next", Style::default()),
            ],
            9,
        );
        assert_eq!(text(&w), vec!["aaaa", "bold,", "next"]);
    }

    #[test]
    fn records_hits() {
        let w = wrap(
            &[
                Piece::new("see ", Style::default()),
                Piece::Frag {
                    text: "the docs".into(),
                    style: Style::default(),
                    action: Some(Action::OpenLink("x".into())),
                },
            ],
            80,
        );
        assert_eq!(w.hits, vec![(0, 4, 12, Action::OpenLink("x".into()))]);
    }
}

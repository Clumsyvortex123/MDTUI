//! Maps screen-independent document coordinates (layout row, column) to clickable actions.

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Action {
    /// A link target: URL, `#anchor`, or relative path.
    OpenLink(String),
    /// Toggle a task checkbox; `(line, column)` of the `[ ]` symbol in the source (1-based).
    ToggleTask { line: usize, col: usize },
    /// Copy the code to the clipboard.
    CopyCode(String),
    /// Show an image or diagram fullscreen (media key).
    OpenMedia(String),
    /// Toggle between rendered diagram and source for a custom block (media key).
    ToggleSource(String),
    /// Switch a diagram between text and image rendering (media key).
    ToggleRenderer(String),
    /// Jump to footnote definition.
    Footnote(String),
    /// Fold / unfold the section under a heading (anchor).
    Fold(String),
    /// Show / hide front matter.
    FrontMatter,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    pub row: usize,
    pub col_start: u16,
    pub col_end: u16,
    pub action: Action,
}

#[derive(Debug, Clone, Default)]
pub struct HitMap {
    hits: Vec<Hit>,
}

impl HitMap {
    pub fn push(&mut self, hit: Hit) {
        self.hits.push(hit);
    }

    pub fn finish(&mut self) {
        self.hits.sort_by_key(|h| (h.row, h.col_start));
    }

    /// The action under (row, col), if any. Specific targets (links) win over a heading's fold.
    pub fn at(&self, row: usize, col: u16) -> Option<&Hit> {
        let mut found = None;
        for h in self.on_row(row).filter(|h| col >= h.col_start && col < h.col_end) {
            if !matches!(h.action, Action::Fold(_)) {
                return Some(h);
            }
            found = Some(h);
        }
        found
    }

    /// Hits whose row range covers `row` (any column).
    pub fn on_row(&self, row: usize) -> impl Iterator<Item = &Hit> {
        let start = self.hits.partition_point(|h| h.row < row);
        self.hits[start..].iter().take_while(move |h| h.row == row)
    }

    /// Focusable targets (links, tasks, code copy, media) in document order, for `Tab` cycling.
    /// Multi-row links appear once (their first row).
    pub fn focusables(&self) -> Vec<&Hit> {
        let mut out: Vec<&Hit> = Vec::new();
        for h in &self.hits {
            if matches!(h.action, Action::Fold(_) | Action::FrontMatter) {
                continue;
            }
            if let Some(prev) = out.last() {
                if prev.action == h.action && h.row <= prev.row + 1 {
                    continue;
                }
            }
            out.push(h);
        }
        out
    }

    pub fn all(&self) -> &[Hit] {
        &self.hits
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup() {
        let mut m = HitMap::default();
        m.push(Hit { row: 3, col_start: 2, col_end: 5, action: Action::FrontMatter });
        m.push(Hit { row: 1, col_start: 0, col_end: 1, action: Action::OpenLink("a".into()) });
        m.finish();
        assert!(m.at(3, 4).is_some());
        assert!(m.at(3, 5).is_none());
        assert_eq!(m.at(1, 0).unwrap().action, Action::OpenLink("a".into()));
    }
}

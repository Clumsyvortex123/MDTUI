//! Built-in completion sources and the VS Code-style popup.

use std::path::Path;

use mdtui_core::Theme;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Widget};
use unicode_width::UnicodeWidthStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    File,
    Folder,
    Heading,
    Reference,
    Emoji,
    Language,
    Snippet,
    Plugin,
    Lsp,
}

impl Kind {
    fn icon(self) -> &'static str {
        match self {
            Kind::File => "📄",
            Kind::Folder => "📁",
            Kind::Heading => "# ",
            Kind::Reference => "↗ ",
            Kind::Emoji => "☺ ",
            Kind::Language => "λ ",
            Kind::Snippet => "✂ ",
            Kind::Plugin => "⚙ ",
            Kind::Lsp => "◆ ",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub label: String,
    /// Text inserted in place of the replaced prefix. `$0` marks the final cursor position.
    pub insert: String,
    pub detail: String,
    pub kind: Kind,
}

/// A set of items that all replace the same number of characters before the cursor.
#[derive(Debug, Clone)]
pub struct CompletionSet {
    pub replace_chars: usize,
    pub items: Vec<Item>,
}

/// What the completion engine needs to know about the document.
pub struct Ctx<'a> {
    /// Current line up to the cursor.
    pub before: &'a str,
    /// Whole buffer text (for headings / reference definitions).
    pub text: &'a str,
    /// Directory of the file, for path completion.
    pub dir: &'a Path,
    /// Manual trigger (Ctrl-Space): also offer snippets.
    pub manual: bool,
}

const SNIPPETS: &[(&str, &str, &str)] = &[
    ("table", "| Column | Column |\n| ------ | ------ |\n| $0     |        |", "GFM table"),
    ("task", "- [ ] $0", "task list item"),
    ("code", "```$0\n\n```", "fenced code block"),
    ("mermaid", "```mermaid\nflowchart LR\n    A[$0] --> B\n```", "mermaid diagram"),
    ("sequence", "```mermaid\nsequenceDiagram\n    Alice->>Bob: $0\n```", "mermaid sequence diagram"),
    ("note", "> [!NOTE]\n> $0", "alert: note"),
    ("tip", "> [!TIP]\n> $0", "alert: tip"),
    ("warning", "> [!WARNING]\n> $0", "alert: warning"),
    ("important", "> [!IMPORTANT]\n> $0", "alert: important"),
    ("caution", "> [!CAUTION]\n> $0", "alert: caution"),
    ("link", "[$0]()", "inline link"),
    ("image", "![$0]()", "image"),
    ("details", "<details>\n<summary>$0</summary>\n\n</details>", "collapsible section"),
    ("footnote", "[^1]$0\n\n[^1]: ", "footnote"),
    ("math", "```math\n$0\n```", "math block"),
    ("frontmatter", "---\ntitle: $0\n---", "YAML front matter"),
];

fn char_len(s: &str) -> usize {
    s.chars().count()
}

/// Built-in sources: paths & anchors in links, emoji, fence languages, reference labels, snippets.
pub fn builtin(ctx: &Ctx) -> Option<CompletionSet> {
    let before = ctx.before;

    // 1. link / image targets: `](prefix`
    if let Some(i) = before.rfind("](") {
        let target = &before[i + 2..];
        if !target.contains([')', ' ']) {
            return Some(link_target(ctx, target));
        }
    }

    // 2. reference links: `][prefix`
    if let Some(i) = before.rfind("][") {
        let label = &before[i + 2..];
        if !label.contains(']') {
            let items = reference_labels(ctx.text)
                .into_iter()
                .filter(|l| l.to_lowercase().starts_with(&label.to_lowercase()))
                .map(|l| Item {
                    insert: format!("{l}]"),
                    label: l,
                    detail: "reference".into(),
                    kind: Kind::Reference,
                })
                .collect();
            return Some(CompletionSet {
                replace_chars: char_len(label),
                items,
            });
        }
    }

    // 3. code fence language
    let trimmed = before.trim_start();
    if let Some(rest) = trimmed.strip_prefix("```").or_else(|| trimmed.strip_prefix("~~~")) {
        if !rest.contains(' ') && !rest.starts_with('`') {
            let items = mdtui_core::highlight::languages()
                .into_iter()
                .filter(|l| l.starts_with(rest))
                .take(200)
                .map(|l| Item {
                    insert: l.clone(),
                    label: l,
                    detail: "language".into(),
                    kind: Kind::Language,
                })
                .collect();
            return Some(CompletionSet {
                replace_chars: char_len(rest),
                items,
            });
        }
    }

    // 4. emoji `:sm`
    if let Some(i) = before.rfind(':') {
        let word = &before[i + 1..];
        let boundary = i == 0 || before[..i].ends_with([' ', '(', '\t']);
        if boundary
            && word.len() >= 2
            && word.chars().all(|c| c.is_ascii_alphanumeric() || "_+-".contains(c))
        {
            let mut items: Vec<Item> = emojis::iter()
                .flat_map(|e| e.shortcodes().map(move |sc| (e, sc)))
                .filter(|(_, sc)| sc.contains(word))
                .map(|(e, sc)| Item {
                    label: format!("{} :{sc}:", e.as_str()),
                    insert: format!(":{sc}:"),
                    detail: e.name().to_string(),
                    kind: Kind::Emoji,
                })
                .collect();
            // prefix matches first
            items.sort_by_key(|it| (!it.insert[1..].starts_with(word), it.insert.len()));
            items.truncate(100);
            return Some(CompletionSet {
                replace_chars: char_len(word) + 1,
                items,
            });
        }
    }

    // 5. snippets: on manual trigger, or when typing a snippet name at line start
    let word_start = before
        .rfind(|c: char| c.is_whitespace())
        .map(|i| i + 1)
        .unwrap_or(0);
    let word = &before[word_start..];
    let at_line_start = before[..word_start].trim().is_empty();
    if ctx.manual || (at_line_start && word.len() >= 2) {
        let items: Vec<Item> = SNIPPETS
            .iter()
            .filter(|(n, _, _)| n.starts_with(word))
            .map(|(n, body, d)| Item {
                label: n.to_string(),
                insert: body.to_string(),
                detail: d.to_string(),
                kind: Kind::Snippet,
            })
            .collect();
        if !items.is_empty() {
            return Some(CompletionSet {
                replace_chars: char_len(word),
                items,
            });
        }
    }
    None
}

fn link_target(ctx: &Ctx, target: &str) -> CompletionSet {
    if let Some(anchor) = target.strip_prefix('#') {
        let items = headings(ctx.text)
            .into_iter()
            .filter(|(_, slug)| slug.starts_with(anchor))
            .map(|(text, slug)| Item {
                label: format!("#{slug}"),
                insert: format!("#{slug}"),
                detail: text,
                kind: Kind::Heading,
            })
            .collect();
        return CompletionSet {
            replace_chars: char_len(target),
            items,
        };
    }
    let (dir_part, file_prefix) = match target.rfind('/') {
        Some(i) => (&target[..=i], &target[i + 1..]),
        None => ("", target),
    };
    let dir = ctx.dir.join(dir_part);
    let mut items = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') && !file_prefix.starts_with('.') {
                continue;
            }
            if !name.to_lowercase().starts_with(&file_prefix.to_lowercase()) {
                continue;
            }
            let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
            let ext = Path::new(&name)
                .extension()
                .map(|x| x.to_string_lossy().into_owned())
                .unwrap_or_default();
            items.push(Item {
                label: if is_dir { format!("{name}/") } else { name.clone() },
                insert: format!("{dir_part}{name}{}", if is_dir { "/" } else { "" }),
                detail: if is_dir { "dir".into() } else { ext },
                kind: if is_dir { Kind::Folder } else { Kind::File },
            });
        }
    }
    items.sort_by(|a, b| {
        (a.kind != Kind::Folder, a.label.to_lowercase()).cmp(&(b.kind != Kind::Folder, b.label.to_lowercase()))
    });
    // anchors are useful even without `#` typed yet
    if target.is_empty() {
        items.extend(headings(ctx.text).into_iter().map(|(text, slug)| Item {
            label: format!("#{slug}"),
            insert: format!("#{slug}"),
            detail: text,
            kind: Kind::Heading,
        }));
    }
    CompletionSet {
        replace_chars: char_len(target),
        items,
    }
}

/// (text, slug) for each ATX heading in the buffer.
fn headings(text: &str) -> Vec<(String, String)> {
    let mut slugger = mdtui_core::slug::Slugger::default();
    let mut in_fence = false;
    let mut out = Vec::new();
    for l in text.lines() {
        let t = l.trim_start();
        if t.starts_with("```") || t.starts_with("~~~") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        if let Some(h) = t.strip_prefix('#') {
            let h = h.trim_start_matches('#');
            if h.starts_with(' ') {
                let title = h.trim().trim_end_matches('#').trim().to_string();
                let slug = slugger.slug(&title);
                out.push((title, slug));
            }
        }
    }
    out
}

fn reference_labels(text: &str) -> Vec<String> {
    let mut v: Vec<String> = text
        .lines()
        .filter_map(|l| {
            let l = l.trim_start();
            let rest = l.strip_prefix('[')?;
            let end = rest.find("]:")?;
            let label = &rest[..end];
            (!label.starts_with('^')).then(|| label.to_string())
        })
        .collect();
    v.sort();
    v.dedup();
    v
}

/// The popup state.
#[derive(Debug, Clone)]
pub struct Popup {
    pub set: CompletionSet,
    pub selected: usize,
    pub scroll: usize,
}

impl Popup {
    pub fn new(set: CompletionSet) -> Option<Self> {
        (!set.items.is_empty()).then_some(Popup {
            set,
            selected: 0,
            scroll: 0,
        })
    }

    pub fn next(&mut self) {
        self.selected = (self.selected + 1) % self.set.items.len();
    }

    pub fn prev(&mut self) {
        let n = self.set.items.len();
        self.selected = (self.selected + n - 1) % n;
    }

    pub fn current(&self) -> &Item {
        &self.set.items[self.selected]
    }

    /// Render below (or above) the cursor at screen position `at`, clamped into `bounds`.
    pub fn render(&mut self, at: (u16, u16), bounds: Rect, theme: &Theme, buf: &mut Buffer) {
        const MAX_ROWS: usize = 10;
        let n = self.set.items.len();
        let rows = n.min(MAX_ROWS);
        if self.selected < self.scroll {
            self.scroll = self.selected;
        } else if self.selected >= self.scroll + rows {
            self.scroll = self.selected + 1 - rows;
        }
        let label_w = self.set.items.iter().map(|i| i.label.width()).max().unwrap_or(0).min(40);
        let detail_w = self.set.items.iter().map(|i| i.detail.width()).max().unwrap_or(0).min(24);
        let w = (label_w + detail_w + 7).min(bounds.width as usize) as u16;
        let h = rows as u16 + 2;
        let mut x = at.0.saturating_sub(1);
        if x + w > bounds.right() {
            x = bounds.right().saturating_sub(w);
        }
        let y = if at.1 + 1 + h <= bounds.bottom() {
            at.1 + 1
        } else {
            at.1.saturating_sub(h)
        };
        let area = Rect::new(x, y, w, h).intersection(bounds);
        Clear.render(area, buf);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(theme.border))
            .style(Style::default().bg(theme.sidebar_bg).fg(theme.fg));
        let inner = block.inner(area);
        block.render(area, buf);
        for (i, item) in self.set.items.iter().enumerate().skip(self.scroll).take(rows) {
            let y = inner.y + (i - self.scroll) as u16;
            if y >= inner.bottom() {
                break;
            }
            let sel = i == self.selected;
            let base = if sel {
                Style::default().bg(theme.focus_bg).fg(theme.fg)
            } else {
                Style::default().bg(theme.sidebar_bg).fg(theme.fg)
            };
            let mut label = item.label.clone();
            while label.width() > label_w {
                label.pop();
            }
            let pad = (inner.width as usize).saturating_sub(3 + label.width() + item.detail.width().min(detail_w));
            let detail: String = item.detail.chars().take(detail_w).collect();
            let line = Line::from(vec![
                Span::styled(format!("{} ", item.kind.icon()), base.fg(theme.accent)),
                Span::styled(label, if sel { base.add_modifier(Modifier::BOLD) } else { base }),
                Span::styled(" ".repeat(pad), base),
                Span::styled(detail, base.fg(theme.muted)),
            ]);
            buf.set_line(inner.x, y, &line, inner.width);
        }
        // scrollbar hint
        if n > rows {
            let txt = format!(" {}/{} ", self.selected + 1, n);
            let x = area.right().saturating_sub(txt.width() as u16 + 1);
            buf.set_string(x, area.bottom() - 1, txt, Style::default().fg(theme.muted).bg(theme.sidebar_bg));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx<'a>(before: &'a str, text: &'a str) -> Ctx<'a> {
        Ctx { before, text, dir: Path::new("."), manual: false }
    }

    #[test]
    fn anchors() {
        let text = "# Intro\n## Install Guide\n";
        let s = builtin(&ctx("see [x](#ins", text)).unwrap();
        assert_eq!(s.replace_chars, 4);
        assert_eq!(s.items[0].insert, "#install-guide");
    }

    #[test]
    fn emoji_and_lang_and_snippets() {
        let s = builtin(&ctx("hi :rock", "")).unwrap();
        assert!(s.items.iter().any(|i| i.insert == ":rocket:"));
        assert_eq!(s.replace_chars, 5);
        let s = builtin(&ctx("```ru", "")).unwrap();
        assert!(s.items.iter().any(|i| i.insert == "rs" || i.insert == "rust"));
        let s = builtin(&ctx("tab", "")).unwrap();
        assert_eq!(s.items[0].label, "table");
        assert!(builtin(&ctx("just words", "")).is_none());
    }
}

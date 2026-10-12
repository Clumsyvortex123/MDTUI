//! IR → terminal lines at a given width.
//!
//! Produces styled lines, a [`HitMap`] of clickable regions, placements for images/diagrams
//! (drawn later by the viewport), and row ↔ source-line maps for scroll sync.

use std::collections::{HashMap, HashSet};
use std::hash::{DefaultHasher, Hash, Hasher};

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::highlight::highlight;
use crate::hitmap::{Action, Hit, HitMap};
use crate::ir::{plain_text, Align, Block, Document, HeadingEntry, Inline, ListItem};
use crate::math;
use crate::theme::{parse_color, Theme};
use crate::wrap::{wrap, Piece};

#[derive(Debug, Clone, Default)]
pub struct LayoutOptions {
    pub width: u16,
    /// Heading anchors whose sections are folded.
    pub folded: HashSet<String>,
    pub show_front_matter: bool,
    pub code_line_numbers: bool,
    /// Media keys of custom blocks that should show their source instead of the rendering.
    pub show_source: HashSet<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKind {
    Image,
    Diagram,
}

/// What the media provider knows about an image / diagram.
#[derive(Debug, Clone)]
pub enum MediaInfo {
    /// Still loading / rendering.
    Pending,
    /// A raster ready to draw at this size (in cells), fitted to the requested max width.
    Ready { width: u16, height: u16 },
    /// Text rendering (ASCII diagram, plugin output).
    Lines(Vec<Line<'static>>),
    Failed(String),
}

pub trait MediaProvider {
    fn media(&self, key: &str, max_width: u16) -> MediaInfo;
}

/// A provider that knows nothing; images show as alt text.
pub struct NoMedia;
impl MediaProvider for NoMedia {
    fn media(&self, _key: &str, _max_width: u16) -> MediaInfo {
        MediaInfo::Failed("images disabled".into())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaPlacement {
    pub key: String,
    pub kind: MediaKind,
    pub row: usize,
    pub col: u16,
    pub width: u16,
    pub height: u16,
}

#[derive(Debug, Clone, Default)]
pub struct Layout {
    pub width: u16,
    pub lines: Vec<Line<'static>>,
    pub hits: HitMap,
    pub media: Vec<MediaPlacement>,
    /// Outline entries with the row they start at.
    pub headings: Vec<(HeadingEntry, usize)>,
    pub anchors: HashMap<String, usize>,
    pub footnotes: HashMap<String, usize>,
    /// (source line, row) for each block start, in row order.
    pub src_rows: Vec<(usize, usize)>,
}

impl Layout {
    pub fn height(&self) -> usize {
        self.lines.len()
    }

    /// Row of the last block starting at or before `line` (1-based).
    pub fn row_for_line(&self, line: usize) -> usize {
        self.src_rows
            .iter()
            .filter(|(l, _)| *l <= line && *l > 0)
            .max_by_key(|(l, r)| (*l, std::cmp::Reverse(*r)))
            .map(|(_, r)| *r)
            .unwrap_or(0)
    }

    /// Source line of the block shown at `row`.
    pub fn line_for_row(&self, row: usize) -> usize {
        self.src_rows
            .iter()
            .take_while(|(_, r)| *r <= row)
            .filter(|(l, _)| *l > 0)
            .last()
            .map(|(l, _)| *l)
            .unwrap_or(1)
    }

    /// The heading path (breadcrumb) for `row`.
    pub fn breadcrumb(&self, row: usize) -> Vec<&HeadingEntry> {
        let mut stack: Vec<&HeadingEntry> = Vec::new();
        for (h, r) in &self.headings {
            if *r > row {
                break;
            }
            while stack.last().is_some_and(|l| l.level >= h.level) {
                stack.pop();
            }
            stack.push(h);
        }
        stack
    }

    pub fn plain_line(&self, row: usize) -> String {
        self.lines
            .get(row)
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
            .unwrap_or_default()
    }
}

/// Media key of a custom fenced block.
pub fn custom_key(handler: &str, content: &str) -> String {
    let mut h = DefaultHasher::new();
    content.hash(&mut h);
    format!("{handler}:{:016x}", h.finish())
}

/// Media key of an image URL.
pub fn image_key(url: &str) -> String {
    format!("img:{url}")
}

pub fn layout(doc: &Document, opts: &LayoutOptions, theme: &Theme, media: &dyn MediaProvider) -> Layout {
    let mut footnote_ix = HashMap::new();
    collect_footnote_refs(doc, &mut footnote_ix);
    let mut l = L {
        opts,
        theme,
        media,
        out: Layout {
            width: opts.width,
            ..Default::default()
        },
        prefixes: Vec::new(),
        style: vec![theme.base()],
        pending_gap: false,
        list_depth: 0,
        footnote_ix,
        footnotes_started: false,
    };
    if let Some(fm) = &doc.front_matter {
        l.front_matter(fm);
    }
    l.blocks(&doc.blocks, true);
    l.out.hits.finish();
    l.out
}

fn collect_footnote_refs(doc: &Document, map: &mut HashMap<String, u32>) {
    fn inl(i: &[Inline], map: &mut HashMap<String, u32>) {
        for x in i {
            match x {
                Inline::FootnoteRef { name, ix } => {
                    let n = map.len() as u32 + 1;
                    map.entry(name.clone()).or_insert(if *ix > 0 { *ix } else { n });
                }
                other => {
                    if let Some(c) = other.children() {
                        inl(c, map)
                    }
                }
            }
        }
    }
    doc.walk_blocks(&mut |b| match b {
        Block::Paragraph { content, .. } | Block::Heading { content, .. } => inl(content, map),
        Block::Table { header, rows, .. } => {
            header.iter().for_each(|c| inl(c, map));
            rows.iter().flatten().for_each(|c| inl(c, map));
        }
        _ => {}
    });
}

/// One level of line prefix (quote bar, list marker, indentation).
struct Prefix {
    first: Vec<(Span<'static>, Option<Action>)>,
    rest: Vec<Span<'static>>,
    used: bool,
}

struct L<'a> {
    opts: &'a LayoutOptions,
    theme: &'a Theme,
    media: &'a dyn MediaProvider,
    out: Layout,
    prefixes: Vec<Prefix>,
    style: Vec<Style>,
    pending_gap: bool,
    list_depth: usize,
    footnote_ix: HashMap<String, u32>,
    footnotes_started: bool,
}

struct InlineImage {
    url: String,
    alt: String,
    action: Option<Action>,
}

fn width_of(spans: &[Span]) -> usize {
    spans.iter().map(|s| s.content.width()).sum()
}

impl<'a> L<'a> {
    // ---------- low level emission ----------

    fn prefix_width(&self) -> usize {
        self.prefixes
            .iter()
            .map(|p| {
                if p.used {
                    width_of(&p.rest)
                } else {
                    p.first.iter().map(|(s, _)| s.content.width()).sum()
                }
            })
            .sum()
    }

    fn avail(&self) -> usize {
        (self.opts.width as usize)
            .saturating_sub(self.prefix_width())
            .max(8)
    }

    fn base(&self) -> Style {
        *self.style.last().unwrap()
    }

    /// Emit one line of content. `hits` columns are relative to the content start.
    fn emit(&mut self, content: Vec<Span<'static>>, hits: Vec<(usize, usize, Action)>) {
        if self.pending_gap {
            self.pending_gap = false;
            if !self.out.lines.is_empty() {
                self.emit_blank();
            }
        }
        let row = self.out.lines.len();
        let mut spans = Vec::new();
        let mut col = 0usize;
        for p in &mut self.prefixes {
            if p.used {
                for s in &p.rest {
                    col += s.content.width();
                    spans.push(s.clone());
                }
            } else {
                for (s, act) in &p.first {
                    let w = s.content.width();
                    if let Some(a) = act {
                        self.out.hits.push(Hit {
                            row,
                            col_start: col as u16,
                            col_end: (col + w) as u16,
                            action: a.clone(),
                        });
                    }
                    col += w;
                    spans.push(s.clone());
                }
                p.used = true;
            }
        }
        for (s, e, a) in hits {
            self.out.hits.push(Hit {
                row,
                col_start: (col + s) as u16,
                col_end: (col + e) as u16,
                action: a,
            });
        }
        spans.extend(content);
        self.out.lines.push(Line::from(spans));
    }

    fn emit_blank(&mut self) {
        let mut spans: Vec<Span<'static>> = Vec::new();
        for p in &self.prefixes {
            if p.used {
                spans.extend(p.rest.iter().cloned());
            } else {
                // a blank line before a prefix's first line keeps the "rest" decoration
                spans.extend(p.rest.iter().cloned());
            }
        }
        // trim trailing whitespace-only spans
        while spans.last().is_some_and(|s| s.content.trim().is_empty()) {
            spans.pop();
        }
        self.out.lines.push(Line::from(spans));
    }

    fn gap(&mut self) {
        self.pending_gap = true;
    }

    fn mark_src(&mut self, line: usize) {
        if line > 0 {
            let row = self.out.lines.len() + usize::from(self.pending_gap && !self.out.lines.is_empty());
            self.out.src_rows.push((line, row));
        }
    }

    fn push_prefix(&mut self, first: Vec<(Span<'static>, Option<Action>)>, rest: Vec<Span<'static>>) {
        // a pending gap belongs to the outer container, not the new one
        if self.pending_gap {
            self.pending_gap = false;
            if !self.out.lines.is_empty() {
                self.emit_blank();
            }
        }
        self.prefixes.push(Prefix {
            first,
            rest,
            used: false,
        });
    }

    fn pop_prefix(&mut self) {
        self.prefixes.pop();
    }

    fn emit_wrapped(&mut self, pieces: &[Piece], width: usize) {
        let w = wrap(pieces, width);
        let mut hits_by_row: HashMap<usize, Vec<(usize, usize, Action)>> = HashMap::new();
        for (r, s, e, a) in w.hits {
            hits_by_row.entry(r).or_default().push((s, e, a));
        }
        for (i, line) in w.lines.into_iter().enumerate() {
            let hits = hits_by_row.remove(&i).unwrap_or_default();
            self.emit(line, hits);
        }
    }

    // ---------- blocks ----------

    fn blocks(&mut self, blocks: &[Block], gaps: bool) {
        let mut fold_level: Option<u8> = None;
        for (i, b) in blocks.iter().enumerate() {
            if let Some(lvl) = fold_level {
                match b {
                    Block::Heading { level, .. } if *level <= lvl => fold_level = None,
                    _ => continue,
                }
            }
            if i > 0 && gaps {
                self.gap();
            }
            if let Block::Heading { level, anchor, .. } = b {
                if self.opts.folded.contains(anchor) {
                    fold_level = Some(*level);
                }
            }
            self.block(b);
        }
    }

    fn block(&mut self, b: &Block) {
        self.mark_src(b.line());
        match b {
            Block::Heading {
                level,
                content,
                anchor,
                line,
            } => self.heading(*level, content, anchor, *line),
            Block::Paragraph { content, .. } => self.paragraph(content),
            Block::List {
                ordered,
                start,
                tight,
                items,
                ..
            } => self.list(*ordered, *start, *tight, items),
            Block::CodeBlock { lang, code, .. } => self.code_block(lang, code, None),
            Block::Quote { blocks, .. } => {
                let bar = Style::default().fg(self.theme.quote);
                self.push_prefix(
                    vec![(Span::styled("▎ ", bar), None)],
                    vec![Span::styled("▎ ", bar)],
                );
                self.style.push(Style::default().fg(self.theme.quote).add_modifier(Modifier::ITALIC));
                self.blocks(blocks, true);
                self.style.pop();
                self.pop_prefix();
            }
            Block::Alert {
                kind, title, blocks, ..
            } => {
                let (color, icon, default_title) = self.theme.alert(*kind);
                let bar = Style::default().fg(color);
                self.push_prefix(
                    vec![(Span::styled("▎ ", bar), None)],
                    vec![Span::styled("▎ ", bar)],
                );
                let t = title.clone().unwrap_or_else(|| default_title.to_string());
                self.emit(
                    vec![Span::styled(format!("{icon} {t}"), bar.add_modifier(Modifier::BOLD))],
                    vec![],
                );
                self.blocks(blocks, true);
                self.pop_prefix();
            }
            Block::Table {
                aligns,
                header,
                rows,
                ..
            } => self.table(aligns, header, rows),
            Block::Rule { .. } => {
                let w = self.avail();
                self.emit(vec![Span::styled("─".repeat(w), self.theme.muted())], vec![]);
            }
            Block::Html { raw, .. } => self.html_block(raw),
            Block::Math { tex, .. } => self.math_block(tex),
            Block::FootnoteDef { name, blocks, .. } => self.footnote_def(name, blocks),
            Block::DescriptionList { items, .. } => {
                for (i, it) in items.iter().enumerate() {
                    if i > 0 {
                        self.gap();
                    }
                    let mut pieces = Vec::new();
                    let mut imgs = Vec::new();
                    let st = self.base().add_modifier(Modifier::BOLD);
                    self.pieces(&it.term, st, None, &mut pieces, &mut imgs);
                    let w = self.avail();
                    self.emit_wrapped(&pieces, w);
                    self.push_prefix(
                        vec![(Span::styled("  : ", self.theme.muted()), None)],
                        vec![Span::raw("    ")],
                    );
                    self.blocks(&it.details, true);
                    self.pop_prefix();
                }
            }
            Block::Custom {
                handler, content, ..
            } => self.custom(handler, content),
            Block::Lines { lines, .. } => {
                for l in lines {
                    let mut pieces = Vec::new();
                    let mut imgs = Vec::new();
                    self.pieces(l, self.base(), None, &mut pieces, &mut imgs);
                    let w = self.avail();
                    self.emit_wrapped(&pieces, w);
                }
            }
        }
    }

    fn front_matter(&mut self, fm: &str) {
        let n = fm.lines().count();
        let arrow = if self.opts.show_front_matter { "▾" } else { "▸" };
        let text = format!("{arrow} front matter ({n} lines)");
        let w = text.width();
        self.emit(
            vec![Span::styled(text, self.theme.muted())],
            vec![(0, w, Action::FrontMatter)],
        );
        if self.opts.show_front_matter {
            self.code_block("yaml", fm, None);
        }
        self.gap();
    }

    fn heading(&mut self, level: u8, content: &[Inline], anchor: &str, line: usize) {
        let row = self.out.lines.len() + usize::from(self.pending_gap && !self.out.lines.is_empty());
        let style = self.theme.heading(level);
        let mut pieces = Vec::new();
        let mut imgs = Vec::new();
        let marker = match level {
            1 => "█ ",
            2 => "▌ ",
            _ => "",
        };
        if !marker.is_empty() {
            pieces.push(Piece::new(marker, style.remove_modifier(Modifier::UNDERLINED)));
        }
        self.pieces(content, style, None, &mut pieces, &mut imgs);
        if self.opts.folded.contains(anchor) {
            pieces.push(Piece::new(" ⋯", self.theme.muted()));
        }
        let w = self.avail();
        let before = self.out.lines.len();
        self.emit_wrapped(&pieces, w);
        // the whole heading toggles its fold (links inside take precedence, see HitMap::at)
        for r in before..self.out.lines.len() {
            let pw = self.prefix_width();
            let lw = width_of(&self.out.lines[r].spans);
            self.out.hits.push(Hit {
                row: r,
                col_start: pw as u16,
                col_end: lw.max(pw + 1) as u16,
                action: Action::Fold(anchor.to_string()),
            });
        }
        if level <= 2 {
            let ch = if level == 1 { "═" } else { "─" };
            let st = if level == 1 {
                Style::default().fg(self.theme.headings[0])
            } else {
                self.theme.muted()
            };
            self.emit(vec![Span::styled(ch.repeat(w), st)], vec![]);
        }
        self.images(imgs);
        let entry = HeadingEntry {
            level,
            text: plain_text(content),
            anchor: anchor.to_string(),
            line,
        };
        self.out.anchors.insert(anchor.to_string(), row);
        self.out.headings.push((entry, row));
    }

    fn paragraph(&mut self, content: &[Inline]) {
        // a paragraph that is just `$$ … $$` is a display-math block
        let significant: Vec<&Inline> = content
            .iter()
            .filter(|i| !matches!(i, Inline::SoftBreak | Inline::LineBreak))
            .filter(|i| !matches!(i, Inline::Text { text } if text.trim().is_empty()))
            .collect();
        if let [Inline::Math { tex, display: true }] = significant.as_slice() {
            self.math_block(tex);
            return;
        }
        let mut pieces = Vec::new();
        let mut imgs = Vec::new();
        self.pieces(content, self.base(), None, &mut pieces, &mut imgs);
        let has_text = pieces.iter().any(|p| match p {
            Piece::Frag { text, .. } => !text.trim().is_empty(),
            Piece::Break => false,
        });
        if has_text {
            let w = self.avail();
            self.emit_wrapped(&pieces, w);
        }
        self.images(imgs);
    }

    fn list(&mut self, ordered: bool, start: usize, tight: bool, items: &[ListItem]) {
        let num_w = format!("{}", start + items.len().saturating_sub(1)).len();
        let bullets = ["•", "◦", "▪", "▹"];
        let bullet = bullets[self.list_depth % bullets.len()];
        self.list_depth += 1;
        for (i, item) in items.iter().enumerate() {
            if i > 0 && !tight {
                self.gap();
            }
            self.mark_src(item.line);
            let (first, rest_w) = match item.task {
                Some(done) => {
                    let (glyph, color) = if done {
                        ("☑ ", self.theme.task_done)
                    } else {
                        ("☐ ", self.theme.task_todo)
                    };
                    let act = item.task_pos.map(|(line, col)| Action::ToggleTask { line, col });
                    (
                        vec![(Span::styled(glyph, Style::default().fg(color).add_modifier(Modifier::BOLD)), act)],
                        2,
                    )
                }
                None if ordered => {
                    let m = format!("{:>w$}. ", start + i, w = num_w);
                    let w = m.width();
                    (vec![(Span::styled(m, self.theme.muted()), None)], w)
                }
                None => (
                    vec![(Span::styled(format!("{bullet} "), Style::default().fg(self.theme.accent)), None)],
                    2,
                ),
            };
            self.push_prefix(first, vec![Span::raw(" ".repeat(rest_w))]);
            if item.task == Some(true) {
                self.style.push(self.base().fg(self.theme.muted));
            }
            if item.blocks.is_empty() {
                self.emit(vec![], vec![]);
            }
            self.blocks(&item.blocks, !tight);
            if item.task == Some(true) {
                self.style.pop();
            }
            self.pop_prefix();
        }
        self.list_depth -= 1;
    }

    fn code_block(&mut self, lang: &str, code: &str, extra_buttons: Option<Vec<(String, Action)>>) {
        let width = self.avail();
        let inner = width.saturating_sub(4).max(4);
        let border = Style::default().fg(self.theme.border);
        let bg = Style::default().bg(self.theme.code_bg);

        // top border: ┌ lang ───── [copy] ┐
        let mut buttons: Vec<(String, Action)> = extra_buttons.unwrap_or_default();
        buttons.push(("[copy]".into(), Action::CopyCode(code.to_string())));
        let label = if lang.is_empty() { String::new() } else { format!(" {lang} ") };
        let btn_w: usize = buttons.iter().map(|(t, _)| t.width() + 1).sum();
        let fill = width.saturating_sub(2 + label.width() + btn_w + 1);
        let mut top = vec![
            Span::styled("┌", border),
            Span::styled(label.clone(), Style::default().fg(self.theme.accent)),
            Span::styled("─".repeat(fill), border),
        ];
        let mut hits = Vec::new();
        let mut col = 1 + label.width() + fill;
        for (t, a) in buttons {
            top.push(Span::styled(" ", border));
            col += 1;
            let w = t.width();
            hits.push((col, col + w, a));
            top.push(Span::styled(t, Style::default().fg(self.theme.link)));
            col += w;
        }
        top.push(Span::styled(" ┐", border));
        self.emit(top, hits);

        let hl = highlight(code, lang, self.theme);
        let n = hl.len();
        let num_w = if self.opts.code_line_numbers { format!("{n}").len() + 1 } else { 0 };
        let text_w = inner.saturating_sub(num_w).max(1);
        for (i, spans) in hl.into_iter().enumerate() {
            let chunks = split_spans(spans, text_w);
            for (j, chunk) in chunks.into_iter().enumerate() {
                let mut line = vec![Span::styled("│", border), Span::styled(" ", bg)];
                if num_w > 0 {
                    let num = if j == 0 { format!("{:>w$} ", i + 1, w = num_w - 1) } else { " ".repeat(num_w) };
                    line.push(Span::styled(num, self.theme.muted().bg(self.theme.code_bg)));
                }
                let used = width_of(&chunk);
                line.extend(chunk.into_iter().map(|s| {
                    let st = s.style.bg(self.theme.code_bg);
                    Span::styled(s.content, st)
                }));
                line.push(Span::styled(" ".repeat(text_w.saturating_sub(used) + 1), bg));
                line.push(Span::styled("│", border));
                self.emit(line, vec![]);
            }
        }
        self.emit(
            vec![Span::styled(format!("└{}┘", "─".repeat(width.saturating_sub(2))), border)],
            vec![],
        );
    }

    fn custom(&mut self, handler: &str, content: &str) {
        let key = custom_key(handler, content);
        let src_btn = |showing_src: bool| {
            vec![(
                if showing_src { "[render]" } else { "[src]" }.to_string(),
                Action::ToggleSource(key.clone()),
            )]
        };
        if self.opts.show_source.contains(&key) {
            self.code_block(handler, content, Some(src_btn(true)));
            return;
        }
        let width = self.avail();
        let border = Style::default().fg(self.theme.border);
        let is_mermaid = handler == "mermaid";
        match self.media.media(&key, width.saturating_sub(4) as u16) {
            MediaInfo::Ready { width: w, height: h } => {
                let mut buttons = vec![("[src]".to_string(), Action::ToggleSource(key.clone()))];
                if is_mermaid {
                    buttons.push(("[txt]".into(), Action::ToggleRenderer(key.clone())));
                }
                buttons.push(("[⤢]".into(), Action::OpenMedia(key.clone())));
                self.box_top(handler, buttons, width);
                let x = (width.saturating_sub(w as usize)) / 2;
                let row = self.out.lines.len();
                let pw = self.prefix_width();
                for _ in 0..h {
                    self.emit(
                        vec![Span::styled("│", border), Span::raw(" ".repeat(width.saturating_sub(2))), Span::styled("│", border)],
                        vec![(x, x + w as usize, Action::OpenMedia(key.clone()))],
                    );
                }
                self.out.media.push(MediaPlacement {
                    key: key.clone(),
                    kind: MediaKind::Diagram,
                    row,
                    col: (pw + x) as u16,
                    width: w,
                    height: h,
                });
                self.box_bottom(width);
            }
            MediaInfo::Lines(lines) => {
                let mut buttons = vec![("[src]".to_string(), Action::ToggleSource(key.clone()))];
                if is_mermaid {
                    buttons.push(("[img]".into(), Action::ToggleRenderer(key.clone())));
                }
                buttons.push(("[⤢]".into(), Action::OpenMedia(key.clone())));
                self.box_top(handler, buttons, width);
                let inner = width.saturating_sub(4);
                let lw = lines.iter().map(|l| width_of(&l.spans)).max().unwrap_or(0);
                // diagrams are pre-laid-out: centre them, and clip (never wrap) when too wide
                let pad = if is_mermaid && lw < inner { (inner - lw) / 2 } else { 0 };
                let clipped = lw > inner;
                for l in lines {
                    let chunk = split_spans(l.spans, inner).into_iter().next().unwrap_or_default();
                    let used = width_of(&chunk) + pad;
                    let mut v = vec![Span::styled("│ ", border), Span::raw(" ".repeat(pad))];
                    v.extend(chunk);
                    v.push(Span::raw(" ".repeat(inner.saturating_sub(used))));
                    v.push(Span::styled(" │", border));
                    self.emit(v, vec![(2, 2 + inner, Action::OpenMedia(key.clone()))]);
                }
                if clipped {
                    let note = format!("⋯ {} columns wide — [⤢] / Enter to pan", lw);
                    let used = note.width();
                    self.emit(
                        vec![
                            Span::styled("│ ", border),
                            Span::styled(note, self.theme.muted()),
                            Span::raw(" ".repeat(inner.saturating_sub(used))),
                            Span::styled(" │", border),
                        ],
                        vec![(2, 2 + inner, Action::OpenMedia(key.clone()))],
                    );
                }
                self.box_bottom(width);
            }
            MediaInfo::Pending => {
                self.emit(
                    vec![Span::styled(format!("⟳ rendering {handler} diagram…"), self.theme.muted())],
                    vec![],
                );
                self.code_block(handler, content, None);
            }
            MediaInfo::Failed(msg) => {
                self.emit(
                    vec![Span::styled(format!("⚠ {handler}: {msg}"), Style::default().fg(self.theme.warn))],
                    vec![],
                );
                self.code_block(handler, content, None);
            }
        }
    }

    /// `┌ label ─────── [a] [b] ┐` with clickable buttons.
    fn box_top(&mut self, label: &str, buttons: Vec<(String, Action)>, width: usize) {
        let border = Style::default().fg(self.theme.border);
        let label = format!(" {label} ");
        let btn_w: usize = buttons.iter().map(|(t, _)| t.width() + 1).sum();
        let fill = width.saturating_sub(2 + label.width() + btn_w + 1);
        let mut spans = vec![
            Span::styled("┌", border),
            Span::styled(label.clone(), Style::default().fg(self.theme.accent)),
            Span::styled("─".repeat(fill), border),
        ];
        let mut hits = Vec::new();
        let mut col = 1 + label.width() + fill;
        for (t, a) in buttons {
            spans.push(Span::styled(" ", border));
            col += 1;
            let w = t.width();
            hits.push((col, col + w, a));
            spans.push(Span::styled(t, Style::default().fg(self.theme.link)));
            col += w;
        }
        spans.push(Span::styled(" ┐", border));
        self.emit(spans, hits);
    }

    fn box_bottom(&mut self, width: usize) {
        let border = Style::default().fg(self.theme.border);
        self.emit(vec![Span::styled(format!("└{}┘", "─".repeat(width.saturating_sub(2))), border)], vec![]);
    }

    fn table(&mut self, aligns: &[Align], header: &[Vec<Inline>], rows: &[Vec<Vec<Inline>>]) {
        let ncols = aligns.len().max(header.len()).max(rows.iter().map(|r| r.len()).max().unwrap_or(0));
        if ncols == 0 {
            return;
        }
        let border = Style::default().fg(self.theme.border);
        let to_pieces = |l: &mut L, cell: &[Inline], st: Style| {
            let mut p = Vec::new();
            let mut imgs = Vec::new();
            l.pieces(cell, st, None, &mut p, &mut imgs);
            for im in imgs {
                p.push(Piece::new(format!("🖼 {}", im.alt), l.theme.muted()));
            }
            p
        };
        let head_style = self.base().add_modifier(Modifier::BOLD);
        let mut all: Vec<Vec<Vec<Piece>>> = Vec::new();
        all.push((0..ncols).map(|c| header.get(c).map(|x| to_pieces(self, x, head_style)).unwrap_or_default()).collect());
        for r in rows {
            let base = self.base();
            all.push((0..ncols).map(|c| r.get(c).map(|x| to_pieces(self, x, base)).unwrap_or_default()).collect());
        }
        // natural widths
        let mut widths = vec![3usize; ncols];
        for row in &all {
            for (c, p) in row.iter().enumerate() {
                let w = wrap(p, 10_000).lines.iter().map(|l| width_of(l)).max().unwrap_or(0);
                widths[c] = widths[c].max(w);
            }
        }
        let avail = self.avail().saturating_sub(3 * ncols + 1);
        while widths.iter().sum::<usize>() > avail {
            let (i, &m) = widths.iter().enumerate().max_by_key(|(_, w)| **w).unwrap();
            if m <= 6 {
                break;
            }
            widths[i] -= 1;
        }
        let hline = |l: char, m: char, r: char| {
            let mut s = String::new();
            s.push(l);
            for (i, w) in widths.iter().enumerate() {
                s.push_str(&"─".repeat(w + 2));
                s.push(if i + 1 == widths.len() { r } else { m });
            }
            Span::styled(s, border)
        };
        self.emit(vec![hline('┌', '┬', '┐')], vec![]);
        for (ri, row) in all.iter().enumerate() {
            let wrapped: Vec<_> = row.iter().enumerate().map(|(c, p)| wrap(p, widths[c])).collect();
            let h = wrapped.iter().map(|w| w.lines.len()).max().unwrap_or(1);
            for line_i in 0..h {
                let mut spans = vec![Span::styled("│", border)];
                let mut hits = Vec::new();
                let mut col = 1;
                for (c, w) in wrapped.iter().enumerate() {
                    let content = w.lines.get(line_i).cloned().unwrap_or_default();
                    let cw = width_of(&content);
                    let pad = widths[c].saturating_sub(cw);
                    let (lp, rp) = match aligns.get(c).copied().unwrap_or_default() {
                        Align::Right => (pad, 0),
                        Align::Center => (pad / 2, pad - pad / 2),
                        _ => (0, pad),
                    };
                    spans.push(Span::raw(" ".repeat(lp + 1)));
                    for (r, s, e, a) in &w.hits {
                        if *r == line_i {
                            hits.push((col + 1 + lp + s, col + 1 + lp + e, a.clone()));
                        }
                    }
                    spans.extend(content);
                    spans.push(Span::raw(" ".repeat(rp + 1)));
                    spans.push(Span::styled("│", border));
                    col += widths[c] + 3;
                }
                self.emit(spans, hits);
            }
            if ri == 0 {
                self.emit(vec![hline('├', '┼', '┤')], vec![]);
            }
        }
        self.emit(vec![hline('└', '┴', '┘')], vec![]);
    }

    fn html_block(&mut self, raw: &str) {
        let srcs = html_img_srcs(raw);
        let text = strip_tags(raw);
        let text = text.trim();
        if !text.is_empty() {
            let w = self.avail();
            let pieces: Vec<Piece> = text
                .lines()
                .flat_map(|l| [Piece::new(l.trim().to_string(), self.base()), Piece::Break])
                .collect();
            let mut pieces = pieces;
            pieces.pop();
            self.emit_wrapped(&pieces, w);
        }
        let imgs = srcs
            .into_iter()
            .map(|(url, alt)| InlineImage { url, alt, action: None })
            .collect();
        self.images(imgs);
    }

    fn math_block(&mut self, tex: &str) {
        let w = self.avail();
        let st = Style::default().fg(self.theme.math).add_modifier(Modifier::ITALIC);
        for l in math::to_unicode(tex.trim()).lines() {
            let l = l.trim();
            let pad = w.saturating_sub(l.width()) / 2;
            self.emit(vec![Span::raw(" ".repeat(pad)), Span::styled(l.to_string(), st)], vec![]);
        }
    }

    fn footnote_def(&mut self, name: &str, blocks: &[Block]) {
        if !self.footnotes_started {
            self.footnotes_started = true;
            let w = self.avail();
            self.emit(vec![Span::styled("─".repeat(w.min(20)), self.theme.muted())], vec![]);
        }
        let ix = self.footnote_ix.get(name).copied().unwrap_or(0);
        let label = format!("{ix}. ");
        let lw = label.width();
        let row = self.out.lines.len() + usize::from(self.pending_gap);
        self.out.footnotes.insert(name.to_string(), row);
        self.push_prefix(
            vec![(Span::styled(label, Style::default().fg(self.theme.link)), None)],
            vec![Span::raw(" ".repeat(lw))],
        );
        self.style.push(self.base().fg(self.theme.muted));
        self.blocks(blocks, true);
        self.style.pop();
        self.pop_prefix();
    }

    /// Lay out images in a left-to-right flow (so README badge rows stay on one line).
    fn images(&mut self, imgs: Vec<InlineImage>) {
        if imgs.is_empty() {
            return;
        }
        let avail = self.avail();
        let mut row_items: Vec<(InlineImage, String, u16, u16)> = Vec::new();
        let mut x = 0usize;
        let flush = |l: &mut L, items: &mut Vec<(InlineImage, String, u16, u16)>| {
            if items.is_empty() {
                return;
            }
            let h = items.iter().map(|i| i.3).max().unwrap_or(1);
            if l.pending_gap {
                l.pending_gap = false;
                if !l.out.lines.is_empty() {
                    l.emit_blank();
                }
            }
            let row = l.out.lines.len();
            let pw = l.prefix_width();
            let mut cx = 0usize;
            let mut hits = Vec::new();
            for (img, key, w, ih) in items.iter() {
                l.out.media.push(MediaPlacement {
                    key: key.clone(),
                    kind: MediaKind::Image,
                    row,
                    col: (pw + cx) as u16,
                    width: *w,
                    height: *ih,
                });
                let act = img.action.clone().unwrap_or(Action::OpenMedia(key.clone()));
                hits.push((cx, cx + *w as usize, act));
                cx += *w as usize + 1;
            }
            for _ in 0..h {
                l.emit(vec![], hits.clone());
            }
            items.clear();
        };
        for img in imgs {
            let key = image_key(&img.url);
            match self.media.media(&key, avail as u16) {
                MediaInfo::Ready { width, height } => {
                    if x > 0 && x + width as usize > avail {
                        flush(self, &mut row_items);
                        x = 0;
                    }
                    x += width as usize + 1;
                    row_items.push((img, key, width, height));
                }
                other => {
                    flush(self, &mut row_items);
                    x = 0;
                    let alt = if img.alt.is_empty() { img.url.clone() } else { img.alt.clone() };
                    let note = match other {
                        MediaInfo::Pending => " (loading…)".to_string(),
                        MediaInfo::Failed(e) => format!(" ({e})"),
                        _ => String::new(),
                    };
                    let text = format!("🖼 {alt}");
                    let tw = text.width();
                    let act = img.action.clone().unwrap_or(Action::OpenLink(img.url.clone()));
                    self.emit(
                        vec![
                            Span::styled(text, Style::default().fg(self.theme.link)),
                            Span::styled(note, self.theme.muted()),
                        ],
                        vec![(0, tw, act)],
                    );
                }
            }
        }
        flush(self, &mut row_items);
    }

    // ---------- inlines ----------

    fn pieces(
        &self,
        inlines: &[Inline],
        style: Style,
        action: Option<&Action>,
        out: &mut Vec<Piece>,
        imgs: &mut Vec<InlineImage>,
    ) {
        let t = self.theme;
        let frag = |text: String, style: Style, action: Option<&Action>| Piece::Frag {
            text,
            style,
            action: action.cloned(),
        };
        for i in inlines {
            match i {
                Inline::Text { text } => out.push(frag(text.replace('\n', " "), style, action)),
                Inline::Code { text } => out.push(frag(
                    text.clone(),
                    style.fg(t.inline_code_fg).bg(t.inline_code_bg),
                    action,
                )),
                Inline::SoftBreak => out.push(frag(" ".into(), style, action)),
                Inline::LineBreak => out.push(Piece::Break),
                Inline::Emph { content } => {
                    self.pieces(content, style.add_modifier(Modifier::ITALIC), action, out, imgs)
                }
                Inline::Strong { content } => {
                    self.pieces(content, style.add_modifier(Modifier::BOLD), action, out, imgs)
                }
                Inline::Strike { content } => {
                    self.pieces(content, style.add_modifier(Modifier::CROSSED_OUT), action, out, imgs)
                }
                Inline::Underline { content } => {
                    self.pieces(content, style.add_modifier(Modifier::UNDERLINED), action, out, imgs)
                }
                Inline::Highlight { content } => {
                    self.pieces(content, style.bg(t.highlight_bg), action, out, imgs)
                }
                Inline::Superscript { content } => {
                    let s = plain_text(content);
                    out.push(frag(math::to_unicode(&format!("^{{{s}}}")), style, action))
                }
                Inline::Subscript { content } => {
                    let s = plain_text(content);
                    out.push(frag(math::to_unicode(&format!("_{{{s}}}")), style, action))
                }
                Inline::Spoiler { content } => {
                    self.pieces(content, style.fg(t.muted).bg(t.muted), action, out, imgs)
                }
                Inline::Link { url, content, .. } => {
                    let act = Action::OpenLink(url.clone());
                    let st = style.fg(t.link).add_modifier(Modifier::UNDERLINED);
                    // images inside links (badges) become clickable images
                    let mut inner_imgs = Vec::new();
                    self.pieces(content, st, Some(&act), out, &mut inner_imgs);
                    for mut im in inner_imgs {
                        im.action = Some(act.clone());
                        imgs.push(im);
                    }
                }
                Inline::Image { url, alt, .. } => imgs.push(InlineImage {
                    url: url.clone(),
                    alt: alt.clone(),
                    action: action.cloned(),
                }),
                Inline::FootnoteRef { name, .. } => {
                    let ix = self.footnote_ix.get(name).copied().unwrap_or(0);
                    out.push(Piece::Frag {
                        text: format!("[{ix}]"),
                        style: Style::default().fg(t.link),
                        action: Some(Action::Footnote(name.clone())),
                    })
                }
                Inline::Math { tex, display } => {
                    let st = style.fg(t.math).add_modifier(Modifier::ITALIC);
                    if *display {
                        out.push(Piece::Break);
                    }
                    for (i, l) in math::to_unicode(tex.trim()).lines().enumerate() {
                        if i > 0 {
                            out.push(Piece::Break);
                        }
                        out.push(frag(l.trim().to_string(), st, action));
                    }
                    if *display {
                        out.push(Piece::Break);
                    }
                }
                Inline::Html { raw } => {
                    let lower = raw.to_ascii_lowercase();
                    if lower.starts_with("<br") {
                        out.push(Piece::Break);
                    } else if lower.starts_with("<img") {
                        for (url, alt) in html_img_srcs(raw) {
                            imgs.push(InlineImage { url, alt, action: action.cloned() });
                        }
                    }
                    // other inline tags (<kbd>, <sup>, …) are dropped; their text still renders
                }
                Inline::Styled {
                    content,
                    fg,
                    bg,
                    bold,
                    italic,
                    underline,
                    link,
                } => {
                    let mut st = style;
                    if let Some(c) = fg.as_deref().and_then(parse_color) {
                        st = st.fg(c);
                    }
                    if let Some(c) = bg.as_deref().and_then(parse_color) {
                        st = st.bg(c);
                    }
                    if *bold {
                        st = st.add_modifier(Modifier::BOLD);
                    }
                    if *italic {
                        st = st.add_modifier(Modifier::ITALIC);
                    }
                    if *underline {
                        st = st.add_modifier(Modifier::UNDERLINED);
                    }
                    let act = link.as_ref().map(|l| Action::OpenLink(l.clone()));
                    if act.is_some() && fg.is_none() {
                        st = st.fg(t.link);
                    }
                    self.pieces(content, st, act.as_ref().or(action), out, imgs)
                }
            }
        }
    }
}

/// Split styled spans into chunks of at most `width` columns (hard wrap, for code).
fn split_spans(spans: Vec<Span<'static>>, width: usize) -> Vec<Vec<Span<'static>>> {
    let width = width.max(1);
    let mut out = vec![Vec::new()];
    let mut col = 0;
    for s in spans {
        let mut buf = String::new();
        for g in s.content.graphemes(true) {
            let gw = g.width();
            if col + gw > width {
                if !buf.is_empty() {
                    out.last_mut().unwrap().push(Span::styled(std::mem::take(&mut buf), s.style));
                }
                out.push(Vec::new());
                col = 0;
            }
            buf.push_str(g);
            col += gw;
        }
        if !buf.is_empty() {
            out.last_mut().unwrap().push(Span::styled(buf, s.style));
        }
    }
    out
}

/// Remove HTML tags (and comments) leaving text content.
pub fn strip_tags(html: &str) -> String {
    let mut out = String::new();
    let mut rest = html;
    while let Some(i) = rest.find('<') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let end = if rest.starts_with("<!--") {
            rest.find("-->").map(|e| e + 3)
        } else {
            rest.find('>').map(|e| e + 1)
        };
        match end {
            Some(e) => {
                let tag = rest[..e].to_ascii_lowercase();
                if tag.starts_with("<br") || tag.starts_with("</p") || tag.starts_with("</div") || tag.starts_with("</summary") {
                    out.push('\n');
                }
                rest = &rest[e..];
            }
            None => {
                out.push_str(rest);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out.replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
}

fn attr(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let mut from = 0;
    while let Some(i) = lower[from..].find(name) {
        let s = from + i;
        let before_ok = s == 0 || !lower.as_bytes()[s - 1].is_ascii_alphanumeric();
        let after = lower[s + name.len()..].trim_start();
        if before_ok && after.starts_with('=') {
            let v = after[1..].trim_start();
            let off = tag.len() - v.len();
            let v = &tag[off..];
            return Some(match v.chars().next()? {
                q @ ('"' | '\'') => v[1..].split(q).next().unwrap_or("").to_string(),
                _ => v.split(|c: char| c.is_whitespace() || c == '>').next().unwrap_or("").to_string(),
            });
        }
        from = s + name.len();
    }
    None
}

/// `(src, alt)` of every `<img>` tag in an HTML fragment.
pub fn html_img_srcs(html: &str) -> Vec<(String, String)> {
    let lower = html.to_ascii_lowercase();
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(i) = lower[from..].find("<img") {
        let s = from + i;
        let e = lower[s..].find('>').map(|e| s + e + 1).unwrap_or(html.len());
        let tag = &html[s..e];
        if let Some(src) = attr(tag, "src") {
            out.push((src, attr(tag, "alt").unwrap_or_default()));
        }
        from = e;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::{parse, ParseOptions};

    fn render(md: &str, width: u16) -> (Layout, String) {
        let doc = parse(md, &ParseOptions::default());
        let opts = LayoutOptions { width, ..Default::default() };
        let l = layout(&doc, &opts, &Theme::dark(), &NoMedia);
        let text = (0..l.height()).map(|r| l.plain_line(r)).collect::<Vec<_>>().join("\n");
        (l, text)
    }

    #[test]
    fn task_hits_point_at_source() {
        let (l, text) = render("# T\n\n- [ ] one\n- [x] two\n", 40);
        let row = text.lines().position(|x| x.contains("one")).unwrap();
        let hit = l.hits.at(row, 0).unwrap();
        assert_eq!(hit.action, Action::ToggleTask { line: 3, col: 4 });
    }

    #[test]
    fn img_attr_parsing() {
        let v = html_img_srcs(r#"<p align="center"><img src="a.png" alt='logo' width=100></p>"#);
        assert_eq!(v, vec![("a.png".to_string(), "logo".to_string())]);
    }

    #[test]
    fn row_line_mapping() {
        let (l, _) = render("# A\n\npara\n\n## B\n\ntext\n", 40);
        assert_eq!(l.line_for_row(l.row_for_line(5)), 5);
        assert_eq!(l.breadcrumb(l.row_for_line(7)).len(), 2);
    }

    #[test]
    fn snapshot_kitchen() {
        let md = "# Title\n\nSome *em* and **strong** and `code` with a [link](https://x.y).\n\n> [!NOTE]\n> Alerts work.\n\n| a | b |\n|:--|--:|\n| 1 | 22 |\n\n```rust\nfn main() {}\n```\n\n1. one\n2. two\n   - nested\n\n---\n\nFoot[^1].\n\n[^1]: The note.\n";
        let (_, text) = render(md, 40);
        insta::assert_snapshot!(text);
    }
}

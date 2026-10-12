//! Owned intermediate representation of a markdown document.
//!
//! comrak's AST lives in an arena and borrows the source, so we lower it into this owned tree.
//! The IR is serde-serializable so Lua plugins can receive it as plain tables, edit it and hand
//! it back. Every block carries the 1-based source line it starts on (`line`), which is used for
//! editor ↔ preview scroll sync.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Document {
    #[serde(default)]
    pub front_matter: Option<String>,
    #[serde(default)]
    pub blocks: Vec<Block>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Align {
    #[default]
    None,
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AlertKind {
    Note,
    Tip,
    Important,
    Warning,
    Caution,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ListItem {
    /// `Some(checked)` for task-list items.
    #[serde(default)]
    pub task: Option<bool>,
    /// Source position (1-based line, 1-based byte column) of the checkbox symbol, for toggling.
    #[serde(default)]
    pub task_pos: Option<(usize, usize)>,
    #[serde(default)]
    pub blocks: Vec<Block>,
    #[serde(default)]
    pub line: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DescItem {
    pub term: Vec<Inline>,
    pub details: Vec<Block>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Block {
    Heading {
        level: u8,
        content: Vec<Inline>,
        #[serde(default)]
        anchor: String,
        #[serde(default)]
        line: usize,
    },
    Paragraph {
        content: Vec<Inline>,
        #[serde(default)]
        line: usize,
    },
    List {
        #[serde(default)]
        ordered: bool,
        #[serde(default = "one")]
        start: usize,
        #[serde(default)]
        tight: bool,
        items: Vec<ListItem>,
        #[serde(default)]
        line: usize,
    },
    CodeBlock {
        #[serde(default)]
        lang: String,
        code: String,
        #[serde(default)]
        line: usize,
    },
    Quote {
        blocks: Vec<Block>,
        #[serde(default)]
        line: usize,
    },
    Alert {
        kind: AlertKind,
        #[serde(default)]
        title: Option<String>,
        blocks: Vec<Block>,
        #[serde(default)]
        line: usize,
    },
    Table {
        aligns: Vec<Align>,
        header: Vec<Vec<Inline>>,
        rows: Vec<Vec<Vec<Inline>>>,
        #[serde(default)]
        line: usize,
    },
    Rule {
        #[serde(default)]
        line: usize,
    },
    Html {
        raw: String,
        #[serde(default)]
        line: usize,
    },
    Math {
        tex: String,
        #[serde(default)]
        line: usize,
    },
    FootnoteDef {
        name: String,
        #[serde(default)]
        ix: u32,
        blocks: Vec<Block>,
        #[serde(default)]
        line: usize,
    },
    DescriptionList {
        items: Vec<DescItem>,
        #[serde(default)]
        line: usize,
    },
    /// A fenced block claimed by a fence handler (mermaid, plugin languages, …).
    Custom {
        handler: String,
        content: String,
        #[serde(default)]
        line: usize,
    },
    /// Pre-styled lines produced by a plugin.
    Lines {
        lines: Vec<Vec<Inline>>,
        #[serde(default)]
        line: usize,
    },
}

fn one() -> usize {
    1
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Inline {
    Text {
        text: String,
    },
    Code {
        text: String,
    },
    Emph {
        content: Vec<Inline>,
    },
    Strong {
        content: Vec<Inline>,
    },
    Strike {
        content: Vec<Inline>,
    },
    Underline {
        content: Vec<Inline>,
    },
    Highlight {
        content: Vec<Inline>,
    },
    Superscript {
        content: Vec<Inline>,
    },
    Subscript {
        content: Vec<Inline>,
    },
    Spoiler {
        content: Vec<Inline>,
    },
    Link {
        url: String,
        #[serde(default)]
        title: String,
        content: Vec<Inline>,
    },
    Image {
        url: String,
        #[serde(default)]
        alt: String,
        #[serde(default)]
        title: String,
    },
    FootnoteRef {
        name: String,
        #[serde(default)]
        ix: u32,
    },
    Math {
        tex: String,
        #[serde(default)]
        display: bool,
    },
    Html {
        raw: String,
    },
    SoftBreak,
    LineBreak,
    /// Arbitrary styling, produced by plugins (`register_inline`).
    Styled {
        content: Vec<Inline>,
        #[serde(default)]
        fg: Option<String>,
        #[serde(default)]
        bg: Option<String>,
        #[serde(default)]
        bold: bool,
        #[serde(default)]
        italic: bool,
        #[serde(default)]
        underline: bool,
        #[serde(default)]
        link: Option<String>,
    },
}

impl Inline {
    pub fn text(s: impl Into<String>) -> Self {
        Inline::Text { text: s.into() }
    }

    /// Nested inline children, if this inline is a container.
    pub fn children(&self) -> Option<&Vec<Inline>> {
        match self {
            Inline::Emph { content }
            | Inline::Strong { content }
            | Inline::Strike { content }
            | Inline::Underline { content }
            | Inline::Highlight { content }
            | Inline::Superscript { content }
            | Inline::Subscript { content }
            | Inline::Spoiler { content }
            | Inline::Link { content, .. }
            | Inline::Styled { content, .. } => Some(content),
            _ => None,
        }
    }
}

/// Concatenated plain text of inlines (used for headings, anchors, TOC, alt text).
pub fn plain_text(inlines: &[Inline]) -> String {
    let mut out = String::new();
    fn go(i: &[Inline], out: &mut String) {
        for x in i {
            match x {
                Inline::Text { text } | Inline::Code { text } => out.push_str(text),
                Inline::Math { tex, .. } => out.push_str(tex),
                Inline::Image { alt, .. } => out.push_str(alt),
                Inline::SoftBreak | Inline::LineBreak => out.push(' '),
                Inline::FootnoteRef { name, .. } => {
                    out.push_str("[^");
                    out.push_str(name);
                    out.push(']')
                }
                Inline::Html { .. } => {}
                other => {
                    if let Some(c) = other.children() {
                        go(c, out)
                    }
                }
            }
        }
    }
    go(inlines, &mut out);
    out
}

impl Block {
    pub fn line(&self) -> usize {
        match self {
            Block::Heading { line, .. }
            | Block::Paragraph { line, .. }
            | Block::List { line, .. }
            | Block::CodeBlock { line, .. }
            | Block::Quote { line, .. }
            | Block::Alert { line, .. }
            | Block::Table { line, .. }
            | Block::Rule { line }
            | Block::Html { line, .. }
            | Block::Math { line, .. }
            | Block::FootnoteDef { line, .. }
            | Block::DescriptionList { line, .. }
            | Block::Custom { line, .. }
            | Block::Lines { line, .. } => *line,
        }
    }
}

/// Outline entry, used by the TOC sidebar and breadcrumbs.
#[derive(Debug, Clone, PartialEq)]
pub struct HeadingEntry {
    pub level: u8,
    pub text: String,
    pub anchor: String,
    pub line: usize,
}

impl Document {
    pub fn headings(&self) -> Vec<HeadingEntry> {
        let mut v = Vec::new();
        for b in &self.blocks {
            if let Block::Heading {
                level,
                content,
                anchor,
                line,
            } = b
            {
                v.push(HeadingEntry {
                    level: *level,
                    text: plain_text(content),
                    anchor: anchor.clone(),
                    line: *line,
                });
            }
        }
        v
    }

    /// Visit every block (depth first) mutably.
    pub fn walk_blocks_mut(&mut self, f: &mut dyn FnMut(&mut Block)) {
        fn go(blocks: &mut [Block], f: &mut dyn FnMut(&mut Block)) {
            for b in blocks {
                f(b);
                match b {
                    Block::Quote { blocks, .. }
                    | Block::Alert { blocks, .. }
                    | Block::FootnoteDef { blocks, .. } => go(blocks, f),
                    Block::List { items, .. } => {
                        for it in items {
                            go(&mut it.blocks, f)
                        }
                    }
                    Block::DescriptionList { items, .. } => {
                        for it in items {
                            go(&mut it.details, f)
                        }
                    }
                    _ => {}
                }
            }
        }
        go(&mut self.blocks, f)
    }

    /// Visit every block immutably.
    pub fn walk_blocks(&self, f: &mut dyn FnMut(&Block)) {
        fn go(blocks: &[Block], f: &mut dyn FnMut(&Block)) {
            for b in blocks {
                f(b);
                match b {
                    Block::Quote { blocks, .. }
                    | Block::Alert { blocks, .. }
                    | Block::FootnoteDef { blocks, .. } => go(blocks, f),
                    Block::List { items, .. } => items.iter().for_each(|it| go(&it.blocks, f)),
                    Block::DescriptionList { items, .. } => {
                        items.iter().for_each(|it| go(&it.details, f))
                    }
                    _ => {}
                }
            }
        }
        go(&self.blocks, f)
    }

    /// All image URLs referenced by the document.
    pub fn image_urls(&self) -> Vec<String> {
        let mut urls = Vec::new();
        fn inl(i: &[Inline], urls: &mut Vec<String>) {
            for x in i {
                match x {
                    Inline::Image { url, .. } => urls.push(url.clone()),
                    other => {
                        if let Some(c) = other.children() {
                            inl(c, urls)
                        }
                    }
                }
            }
        }
        self.walk_blocks(&mut |b| match b {
            Block::Paragraph { content, .. } | Block::Heading { content, .. } => {
                inl(content, &mut urls)
            }
            Block::Table { header, rows, .. } => {
                header.iter().for_each(|c| inl(c, &mut urls));
                rows.iter().flatten().for_each(|c| inl(c, &mut urls));
            }
            _ => {}
        });
        urls
    }
}

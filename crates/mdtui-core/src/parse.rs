//! comrak AST → owned IR.

use std::collections::HashSet;

use comrak::nodes::{AlertType, AstNode, ListType, NodeValue, TableAlignment};
use comrak::{parse_document, Arena, Options};

use crate::ir::{plain_text, AlertKind, Align, Block, DescItem, Document, Inline, ListItem};
use crate::slug::Slugger;

#[derive(Debug, Clone, Default)]
pub struct ParseOptions {
    /// Fenced code languages that are rendered by a handler (mermaid, plugin fences, …)
    /// instead of as highlighted code. They become [`Block::Custom`].
    pub fence_handlers: HashSet<String>,
}

pub fn comrak_options() -> Options<'static> {
    let mut o = Options::default();
    let e = &mut o.extension;
    e.strikethrough = true;
    e.table = true;
    e.autolink = true;
    e.tasklist = true;
    e.superscript = false; // `^` collides with footnotes/math too often
    e.footnotes = true;
    e.description_lists = true;
    e.front_matter_delimiter = Some("---".into());
    e.multiline_block_quotes = true;
    e.alerts = true;
    e.math_dollars = true;
    e.math_code = true;
    e.shortcodes = true;
    e.wikilinks_title_after_pipe = true;
    e.underline = false;
    e.subscript = false;
    e.spoiler = true;
    e.highlight = true;
    o.parse.relaxed_tasklist_matching = true;
    o.parse.tasklist_in_table = true;
    o.parse.relaxed_autolinks = true;
    o
}

pub fn parse(src: &str, opts: &ParseOptions) -> Document {
    let arena = Arena::new();
    let root = parse_document(&arena, src, &comrak_options());
    let mut cx = Cx {
        opts,
        slugger: Slugger::default(),
        front_matter: None,
    };
    let blocks = cx.blocks(root);
    Document {
        front_matter: cx.front_matter,
        blocks,
    }
}

struct Cx<'o> {
    opts: &'o ParseOptions,
    slugger: Slugger,
    front_matter: Option<String>,
}

impl Cx<'_> {
    fn blocks<'a>(&mut self, parent: &'a AstNode<'a>) -> Vec<Block> {
        let mut out = Vec::new();
        for child in parent.children() {
            if let Some(b) = self.block(child) {
                out.push(b);
            }
        }
        out
    }

    fn block<'a>(&mut self, node: &'a AstNode<'a>) -> Option<Block> {
        let ast = node.data();
        let line = ast.sourcepos.start.line;
        let b = match &ast.value {
            NodeValue::FrontMatter(fm) => {
                let fm = fm.trim().trim_start_matches("---").trim_end_matches("---").trim();
                self.front_matter = Some(fm.to_string());
                return None;
            }
            NodeValue::Paragraph => Block::Paragraph {
                content: inlines(node),
                line,
            },
            NodeValue::Heading(h) => {
                let content = inlines(node);
                let anchor = self.slugger.slug(&plain_text(&content));
                Block::Heading {
                    level: h.level,
                    content,
                    anchor,
                    line,
                }
            }
            NodeValue::ThematicBreak => Block::Rule { line },
            NodeValue::CodeBlock(cb) => {
                let lang = cb.info.split_whitespace().next().unwrap_or("").to_string();
                let code = cb.literal.trim_end_matches('\n').to_string();
                if lang == "math" {
                    Block::Math { tex: code, line }
                } else if self.opts.fence_handlers.contains(&lang) {
                    Block::Custom {
                        handler: lang,
                        content: code,
                        line,
                    }
                } else {
                    Block::CodeBlock { lang, code, line }
                }
            }
            NodeValue::BlockQuote | NodeValue::MultilineBlockQuote(_) => {
                drop(ast);
                return Some(Block::Quote {
                    blocks: self.blocks(node),
                    line,
                });
            }
            NodeValue::Alert(a) => {
                let kind = match a.alert_type {
                    AlertType::Note => AlertKind::Note,
                    AlertType::Tip => AlertKind::Tip,
                    AlertType::Important => AlertKind::Important,
                    AlertType::Warning => AlertKind::Warning,
                    AlertType::Caution => AlertKind::Caution,
                };
                let title = a.title.clone();
                drop(ast);
                return Some(Block::Alert {
                    kind,
                    title,
                    blocks: self.blocks(node),
                    line,
                });
            }
            NodeValue::List(l) => {
                let ordered = l.list_type == ListType::Ordered;
                let start = l.start;
                let tight = l.tight;
                drop(ast);
                let mut items = Vec::new();
                for item in node.children() {
                    let d = item.data();
                    let item_line = d.sourcepos.start.line;
                    let (task, task_pos) = match &d.value {
                        NodeValue::TaskItem(t) => (
                            Some(t.symbol.is_some_and(|c| c != ' ')),
                            Some((
                                t.symbol_sourcepos.start.line,
                                t.symbol_sourcepos.start.column,
                            )),
                        ),
                        _ => (None, None),
                    };
                    drop(d);
                    items.push(ListItem {
                        task,
                        task_pos,
                        blocks: self.blocks(item),
                        line: item_line,
                    });
                }
                return Some(Block::List {
                    ordered,
                    start,
                    tight,
                    items,
                    line,
                });
            }
            NodeValue::Table(t) => {
                let aligns = t
                    .alignments
                    .iter()
                    .map(|a| match a {
                        TableAlignment::None => Align::None,
                        TableAlignment::Left => Align::Left,
                        TableAlignment::Center => Align::Center,
                        TableAlignment::Right => Align::Right,
                    })
                    .collect();
                drop(ast);
                let mut header = Vec::new();
                let mut rows = Vec::new();
                for row in node.children() {
                    let is_header = matches!(row.data().value, NodeValue::TableRow(true));
                    let cells: Vec<Vec<Inline>> = row.children().map(inlines).collect();
                    if is_header {
                        header = cells;
                    } else {
                        rows.push(cells);
                    }
                }
                return Some(Block::Table {
                    aligns,
                    header,
                    rows,
                    line,
                });
            }
            NodeValue::HtmlBlock(h) => {
                let raw = h.literal.trim_end().to_string();
                if raw.starts_with("<!--") && raw.ends_with("-->") {
                    return None;
                }
                Block::Html { raw, line }
            }
            NodeValue::FootnoteDefinition(f) => {
                let name = f.name.clone();
                drop(ast);
                return Some(Block::FootnoteDef {
                    ix: 0,
                    name,
                    blocks: self.blocks(node),
                    line,
                });
            }
            NodeValue::DescriptionList => {
                drop(ast);
                let mut items = Vec::new();
                for item in node.children() {
                    let mut term = Vec::new();
                    let mut details = Vec::new();
                    for part in item.children() {
                        match part.data().value {
                            NodeValue::DescriptionTerm => {
                                // term contains a paragraph
                                for p in part.children() {
                                    term.extend(inlines(p));
                                }
                            }
                            NodeValue::DescriptionDetails => details.extend(self.blocks(part)),
                            _ => {}
                        }
                    }
                    items.push(DescItem { term, details });
                }
                return Some(Block::DescriptionList { items, line });
            }
            NodeValue::Math(m) if m.display_math => Block::Math {
                tex: m.literal.clone(),
                line,
            },
            _ => {
                // Unknown container: flatten its children into a quote-less group.
                drop(ast);
                let inner = self.blocks(node);
                return inner.into_iter().next();
            }
        };
        Some(b)
    }
}

fn inlines<'a>(node: &'a AstNode<'a>) -> Vec<Inline> {
    let mut out = Vec::new();
    for child in node.children() {
        inline(child, &mut out);
    }
    merge_text(out)
}

fn inline<'a>(node: &'a AstNode<'a>, out: &mut Vec<Inline>) {
    let ast = node.data();
    let i = match &ast.value {
        NodeValue::Text(t) => Inline::Text {
            text: t.to_string(),
        },
        NodeValue::Code(c) => Inline::Code {
            text: c.literal.clone(),
        },
        NodeValue::SoftBreak => Inline::SoftBreak,
        NodeValue::LineBreak => Inline::LineBreak,
        NodeValue::Emph => Inline::Emph {
            content: inlines(node),
        },
        NodeValue::Strong => Inline::Strong {
            content: inlines(node),
        },
        NodeValue::Strikethrough => Inline::Strike {
            content: inlines(node),
        },
        NodeValue::Underline => Inline::Underline {
            content: inlines(node),
        },
        NodeValue::Highlight => Inline::Highlight {
            content: inlines(node),
        },
        NodeValue::Superscript => Inline::Superscript {
            content: inlines(node),
        },
        NodeValue::Subscript => Inline::Subscript {
            content: inlines(node),
        },
        NodeValue::SpoileredText => Inline::Spoiler {
            content: inlines(node),
        },
        NodeValue::Link(l) => Inline::Link {
            url: l.url.clone(),
            title: l.title.clone(),
            content: inlines(node),
        },
        NodeValue::WikiLink(w) => Inline::Link {
            url: w.url.clone(),
            title: String::new(),
            content: inlines(node),
        },
        NodeValue::Image(l) => Inline::Image {
            url: l.url.clone(),
            alt: plain_text(&inlines(node)),
            title: l.title.clone(),
        },
        NodeValue::FootnoteReference(f) => Inline::FootnoteRef {
            name: f.name.clone(),
            ix: f.ix,
        },
        NodeValue::ShortCode(s) => Inline::Text {
            text: s.emoji.clone(),
        },
        NodeValue::Math(m) => Inline::Math {
            tex: m.literal.clone(),
            display: m.display_math,
        },
        NodeValue::HtmlInline(h) => Inline::Html { raw: h.clone() },
        NodeValue::Escaped => {
            drop(ast);
            for c in node.children() {
                inline(c, out);
            }
            return;
        }
        NodeValue::Raw(r) => Inline::Text { text: r.clone() },
        _ => {
            drop(ast);
            for c in node.children() {
                inline(c, out);
            }
            return;
        }
    };
    out.push(i);
}

/// comrak splits text at every special character; glue adjacent text nodes back together.
fn merge_text(v: Vec<Inline>) -> Vec<Inline> {
    let mut out: Vec<Inline> = Vec::with_capacity(v.len());
    for i in v {
        if let (Some(Inline::Text { text: prev }), Inline::Text { text }) = (out.last_mut(), &i) {
            prev.push_str(text);
            continue;
        }
        out.push(i);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tasks_with_positions() {
        let doc = parse("- [ ] a\n- [x] b\n", &ParseOptions::default());
        let Block::List { items, .. } = &doc.blocks[0] else {
            panic!()
        };
        assert_eq!(items[0].task, Some(false));
        assert_eq!(items[1].task, Some(true));
        assert_eq!(items[0].task_pos, Some((1, 4)));
        assert_eq!(items[1].task_pos, Some((2, 4)));
    }

    #[test]
    fn custom_fences_and_front_matter() {
        let mut o = ParseOptions::default();
        o.fence_handlers.insert("mermaid".into());
        let doc = parse("---\ntitle: x\n---\n\n```mermaid\ngraph TD\n```\n", &o);
        assert_eq!(doc.front_matter.as_deref(), Some("title: x"));
        assert!(matches!(&doc.blocks[0], Block::Custom { handler, .. } if handler == "mermaid"));
    }

    #[test]
    fn alerts_and_headings() {
        let doc = parse("# Hi there\n\n> [!WARNING]\n> careful\n", &ParseOptions::default());
        assert!(matches!(&doc.blocks[0], Block::Heading { anchor, .. } if anchor == "hi-there"));
        assert!(matches!(&doc.blocks[1], Block::Alert { kind: AlertKind::Warning, .. }));
    }
}

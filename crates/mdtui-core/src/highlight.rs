//! Syntax highlighting for fenced code blocks (syntect + the extra grammars from two-face).

use std::sync::OnceLock;

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;
use syntect::easy::HighlightLines;
use syntect::highlighting::FontStyle;
use syntect::parsing::SyntaxSet;

use crate::theme::Theme;

fn syntaxes() -> &'static SyntaxSet {
    static S: OnceLock<SyntaxSet> = OnceLock::new();
    S.get_or_init(two_face::syntax::extra_newlines)
}

fn themes() -> &'static two_face::theme::EmbeddedLazyThemeSet {
    static T: OnceLock<two_face::theme::EmbeddedLazyThemeSet> = OnceLock::new();
    T.get_or_init(two_face::theme::extra)
}

/// All language tokens we know (used for completion after ```).
pub fn languages() -> Vec<String> {
    let mut v: Vec<String> = syntaxes()
        .syntaxes()
        .iter()
        .flat_map(|s| {
            s.file_extensions
                .iter()
                .cloned()
                .chain(std::iter::once(s.name.to_lowercase().replace(' ', "")))
        })
        .filter(|e| e.chars().all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '#'))
        .collect();
    v.extend(["mermaid", "math", "text"].map(String::from));
    v.sort();
    v.dedup();
    v
}

/// Highlight `code` written in `lang`, returning one `Vec<Span>` per source line.
/// Falls back to plain text for unknown languages.
pub fn highlight(code: &str, lang: &str, theme: &Theme) -> Vec<Vec<Span<'static>>> {
    let ss = syntaxes();
    let syntax = (!lang.is_empty())
        .then(|| {
            ss.find_syntax_by_token(lang)
                .or_else(|| ss.find_syntax_by_extension(lang))
        })
        .flatten();
    let Some(syntax) = syntax else {
        return code
            .lines()
            .map(|l| vec![Span::styled(l.to_string(), Style::default().fg(theme.fg))])
            .collect();
    };
    let st = themes().get(theme.syntax_theme);
    let mut h = HighlightLines::new(syntax, st);
    let mut out = Vec::new();
    for line in syntect::util::LinesWithEndings::from(code) {
        let ranges = match h.highlight_line(line, ss) {
            Ok(r) => r,
            Err(_) => {
                out.push(vec![Span::raw(line.trim_end_matches('\n').to_string())]);
                continue;
            }
        };
        let spans = ranges
            .into_iter()
            .filter_map(|(style, text)| {
                let text = text.trim_end_matches(['\n', '\r']);
                if text.is_empty() {
                    return None;
                }
                let fg = style.foreground;
                let mut s = Style::default().fg(Color::Rgb(fg.r, fg.g, fg.b));
                if style.font_style.contains(FontStyle::BOLD) {
                    s = s.add_modifier(Modifier::BOLD);
                }
                if style.font_style.contains(FontStyle::ITALIC) {
                    s = s.add_modifier(Modifier::ITALIC);
                }
                Some(Span::styled(text.replace('\t', "    "), s))
            })
            .collect();
        out.push(spans);
    }
    if code.ends_with('\n') || out.is_empty() {
        out.push(Vec::new());
    }
    out
}

/// The syntect theme matching an mdtui theme (shared with the editor's highlighter).
pub fn syntect_theme(theme: &Theme) -> syntect::highlighting::Theme {
    themes().get(theme.syntax_theme).clone()
}

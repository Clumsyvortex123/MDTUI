//! Colour themes, modelled on VS Code's Dark+ / Light+ markdown preview.

use ratatui::style::{Color, Modifier, Style};

use crate::ir::AlertKind;

#[derive(Debug, Clone)]
pub struct Theme {
    pub name: &'static str,
    pub dark: bool,
    pub fg: Color,
    pub bg: Color,
    pub muted: Color,
    pub accent: Color,
    pub headings: [Color; 6],
    pub link: Color,
    pub inline_code_fg: Color,
    pub inline_code_bg: Color,
    pub code_bg: Color,
    pub border: Color,
    pub quote: Color,
    pub task_done: Color,
    pub task_todo: Color,
    pub highlight_bg: Color,
    pub search_bg: Color,
    pub search_current_bg: Color,
    pub focus_bg: Color,
    pub selection_bg: Color,
    pub math: Color,
    pub error: Color,
    pub warn: Color,
    pub info: Color,
    pub ok: Color,
    pub statusbar_bg: Color,
    pub statusbar_fg: Color,
    pub sidebar_bg: Color,
    /// syntect theme used for fenced code (from `two_face`'s theme set).
    pub syntax_theme: two_face::theme::EmbeddedThemeName,
}

impl Theme {
    pub fn dark() -> Self {
        Theme {
            name: "dark",
            dark: true,
            fg: Color::Rgb(212, 212, 212),
            bg: Color::Rgb(30, 30, 30),
            muted: Color::Rgb(128, 128, 128),
            accent: Color::Rgb(0, 122, 204),
            headings: [
                Color::Rgb(86, 156, 214),
                Color::Rgb(78, 201, 176),
                Color::Rgb(220, 220, 170),
                Color::Rgb(197, 134, 192),
                Color::Rgb(206, 145, 120),
                Color::Rgb(156, 220, 254),
            ],
            link: Color::Rgb(55, 148, 255),
            inline_code_fg: Color::Rgb(206, 145, 120),
            inline_code_bg: Color::Rgb(45, 45, 45),
            code_bg: Color::Rgb(37, 37, 38),
            border: Color::Rgb(68, 68, 68),
            quote: Color::Rgb(110, 118, 129),
            task_done: Color::Rgb(78, 201, 176),
            task_todo: Color::Rgb(156, 156, 156),
            highlight_bg: Color::Rgb(97, 83, 30),
            search_bg: Color::Rgb(81, 92, 106),
            search_current_bg: Color::Rgb(158, 106, 3),
            focus_bg: Color::Rgb(38, 79, 120),
            selection_bg: Color::Rgb(38, 79, 120),
            math: Color::Rgb(181, 206, 168),
            error: Color::Rgb(244, 71, 71),
            warn: Color::Rgb(204, 167, 0),
            info: Color::Rgb(55, 148, 255),
            ok: Color::Rgb(137, 209, 133),
            statusbar_bg: Color::Rgb(0, 122, 204),
            statusbar_fg: Color::Rgb(255, 255, 255),
            sidebar_bg: Color::Rgb(37, 37, 38),
            syntax_theme: two_face::theme::EmbeddedThemeName::OneHalfDark,
        }
    }

    pub fn light() -> Self {
        Theme {
            name: "light",
            dark: false,
            fg: Color::Rgb(36, 41, 47),
            bg: Color::Rgb(255, 255, 255),
            muted: Color::Rgb(110, 119, 129),
            accent: Color::Rgb(0, 95, 184),
            headings: [
                Color::Rgb(0, 0, 128),
                Color::Rgb(0, 112, 96),
                Color::Rgb(121, 94, 38),
                Color::Rgb(175, 0, 219),
                Color::Rgb(163, 21, 21),
                Color::Rgb(0, 16, 128),
            ],
            link: Color::Rgb(9, 105, 218),
            inline_code_fg: Color::Rgb(163, 21, 21),
            inline_code_bg: Color::Rgb(239, 241, 243),
            code_bg: Color::Rgb(246, 248, 250),
            border: Color::Rgb(208, 215, 222),
            quote: Color::Rgb(101, 109, 118),
            task_done: Color::Rgb(26, 127, 55),
            task_todo: Color::Rgb(101, 109, 118),
            highlight_bg: Color::Rgb(255, 248, 197),
            search_bg: Color::Rgb(255, 236, 153),
            search_current_bg: Color::Rgb(255, 180, 80),
            focus_bg: Color::Rgb(173, 214, 255),
            selection_bg: Color::Rgb(173, 214, 255),
            math: Color::Rgb(9, 134, 88),
            error: Color::Rgb(207, 34, 46),
            warn: Color::Rgb(154, 103, 0),
            info: Color::Rgb(9, 105, 218),
            ok: Color::Rgb(26, 127, 55),
            statusbar_bg: Color::Rgb(0, 122, 204),
            statusbar_fg: Color::Rgb(255, 255, 255),
            sidebar_bg: Color::Rgb(243, 243, 243),
            syntax_theme: two_face::theme::EmbeddedThemeName::InspiredGithub,
        }
    }

    pub fn by_name(name: &str) -> Option<Self> {
        match name {
            "dark" => Some(Self::dark()),
            "light" => Some(Self::light()),
            _ => None,
        }
    }

    pub fn base(&self) -> Style {
        Style::default().fg(self.fg)
    }

    pub fn muted(&self) -> Style {
        Style::default().fg(self.muted)
    }

    pub fn heading(&self, level: u8) -> Style {
        let c = self.headings[(level.clamp(1, 6) - 1) as usize];
        let s = Style::default().fg(c).add_modifier(Modifier::BOLD);
        if level == 1 {
            s.add_modifier(Modifier::UNDERLINED)
        } else {
            s
        }
    }

    pub fn alert(&self, kind: AlertKind) -> (Color, &'static str, &'static str) {
        match kind {
            AlertKind::Note => (self.info, "ℹ", "Note"),
            AlertKind::Tip => (self.ok, "✦", "Tip"),
            AlertKind::Important => (Color::Rgb(163, 113, 247), "‼", "Important"),
            AlertKind::Warning => (self.warn, "▲", "Warning"),
            AlertKind::Caution => (self.error, "✖", "Caution"),
        }
    }
}

/// Parse a colour name from config / Lua (`"red"`, `"#ff8800"`, `"cyan"`).
pub fn parse_color(s: &str) -> Option<Color> {
    let s = s.trim();
    if let Some(hex) = s.strip_prefix('#') {
        if hex.len() == 6 {
            let v = u32::from_str_radix(hex, 16).ok()?;
            return Some(Color::Rgb((v >> 16) as u8, (v >> 8) as u8, v as u8));
        }
        return None;
    }
    s.parse::<Color>().ok()
}

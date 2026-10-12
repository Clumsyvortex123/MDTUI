//! Vim-style key notation (`gg`, `<C-d>`, `<leader>wc`) and multi-key sequence matching.

use std::time::{Duration, Instant};

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Viewer actions bound to keys.
#[derive(Debug, Clone, PartialEq)]
pub enum Cmd {
    Down(usize),
    Up(usize),
    HalfDown,
    HalfUp,
    PageDown,
    PageUp,
    Top,
    Bottom,
    NextHeading,
    PrevHeading,
    FocusNext,
    FocusPrev,
    Activate,
    ToggleTask,
    Yank,
    ToggleToc,
    FoldToggle,
    FoldAll,
    UnfoldAll,
    Search,
    SearchNext,
    SearchPrev,
    CommandLine,
    Edit,
    Reload,
    Back,
    Forward,
    Help,
    Escape,
    Quit,
    FocusPane(char),
    /// Run an ex command (`:toc`, `:WordCount`), from config or plugin keymaps.
    Ex(String),
}

/// Turn a key event into a notation token: `a`, `G`, `<C-d>`, `<CR>`, `<Space>`.
pub fn token(k: &KeyEvent) -> String {
    let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
    let alt = k.modifiers.contains(KeyModifiers::ALT);
    let base = match k.code {
        KeyCode::Char(' ') => "Space".to_string(),
        KeyCode::Char(c) => {
            if !ctrl && !alt {
                return c.to_string();
            }
            c.to_ascii_lowercase().to_string()
        }
        KeyCode::Enter => "CR".into(),
        KeyCode::Esc => "Esc".into(),
        KeyCode::Tab => "Tab".into(),
        KeyCode::BackTab => "S-Tab".into(),
        KeyCode::Backspace => "BS".into(),
        KeyCode::Up => "Up".into(),
        KeyCode::Down => "Down".into(),
        KeyCode::Left => "Left".into(),
        KeyCode::Right => "Right".into(),
        KeyCode::Home => "Home".into(),
        KeyCode::End => "End".into(),
        KeyCode::PageUp => "PageUp".into(),
        KeyCode::PageDown => "PageDown".into(),
        KeyCode::Delete => "Del".into(),
        KeyCode::F(n) => format!("F{n}"),
        _ => "?".into(),
    };
    let mut s = String::from("<");
    if ctrl {
        s.push_str("C-");
    }
    if alt {
        s.push_str("M-");
    }
    s.push_str(&base);
    s.push('>');
    s
}

/// Split notation into tokens: `<leader>wc` → [leader, "w", "c"]; `<c-d>` → ["<C-d>"].
pub fn parse(notation: &str, leader: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = notation.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '<' {
            let mut name = String::new();
            let mut closed = false;
            for d in chars.by_ref() {
                if d == '>' {
                    closed = true;
                    break;
                }
                name.push(d);
            }
            if !closed {
                out.push("<".into());
                out.extend(name.chars().map(String::from));
                continue;
            }
            let lower = name.to_ascii_lowercase();
            if lower == "leader" {
                out.extend(parse(leader, " "));
                continue;
            }
            let (mods, key) = match lower.rfind('-') {
                Some(i) if i + 1 < lower.len() => (&name[..i], &name[i + 1..]),
                _ => ("", name.as_str()),
            };
            let key = match key.to_ascii_lowercase().as_str() {
                "cr" | "enter" | "return" => "CR".to_string(),
                "esc" => "Esc".into(),
                "space" => "Space".into(),
                "tab" => "Tab".into(),
                "bs" | "backspace" => "BS".into(),
                "up" => "Up".into(),
                "down" => "Down".into(),
                "left" => "Left".into(),
                "right" => "Right".into(),
                "home" => "Home".into(),
                "end" => "End".into(),
                "pageup" => "PageUp".into(),
                "pagedown" => "PageDown".into(),
                "del" => "Del".into(),
                k if k.len() == 1 => k.to_string(),
                k => k.to_string(),
            };
            let mut s = String::from("<");
            for m in mods.split('-').filter(|m| !m.is_empty()) {
                match m.to_ascii_lowercase().as_str() {
                    "c" | "ctrl" => s.push_str("C-"),
                    "m" | "a" | "alt" => s.push_str("M-"),
                    "s" | "shift" => s.push_str("S-"),
                    _ => {}
                }
            }
            s.push_str(&key);
            s.push('>');
            out.push(s);
        } else if c == ' ' {
            out.push("<Space>".into());
        } else {
            out.push(c.to_string());
        }
    }
    out
}

pub struct Keymap {
    entries: Vec<(Vec<String>, Cmd)>,
    pending: Vec<String>,
    pending_since: Instant,
}

pub enum Outcome {
    Run(Cmd),
    Pending,
    None,
}

impl Keymap {
    pub fn new(leader: &str) -> Self {
        use Cmd::*;
        let defaults: &[(&str, Cmd)] = &[
            ("j", Down(1)),
            ("<Down>", Down(1)),
            ("k", Up(1)),
            ("<Up>", Up(1)),
            ("<C-e>", Down(1)),
            ("<C-y>", Up(1)),
            ("<C-d>", HalfDown),
            ("<C-u>", HalfUp),
            ("<C-f>", PageDown),
            ("<C-b>", PageUp),
            ("<PageDown>", PageDown),
            ("<PageUp>", PageUp),
            ("<Space>", PageDown),
            ("gg", Top),
            ("<Home>", Top),
            ("G", Bottom),
            ("<End>", Bottom),
            ("]]", NextHeading),
            ("[[", PrevHeading),
            ("}", NextHeading),
            ("{", PrevHeading),
            ("<Tab>", FocusNext),
            ("<S-Tab>", FocusPrev),
            ("<CR>", Activate),
            ("x", ToggleTask),
            ("y", Yank),
            ("gO", ToggleToc),
            ("o", ToggleToc),
            ("za", FoldToggle),
            ("zM", FoldAll),
            ("zR", UnfoldAll),
            ("/", Search),
            ("n", SearchNext),
            ("N", SearchPrev),
            (":", CommandLine),
            ("e", Edit),
            ("r", Reload),
            ("<C-o>", Back),
            ("H", Back),
            ("<BS>", Back),
            ("L", Forward),
            ("?", Help),
            ("<Esc>", Escape),
            ("q", Quit),
            ("<C-c>", Quit),
            ("<C-w>h", FocusPane('h')),
            ("<C-w>l", FocusPane('l')),
            ("<C-w>w", FocusPane('w')),
            ("<C-w><C-w>", FocusPane('w')),
        ];
        let entries = defaults
            .iter()
            .map(|(k, c)| (parse(k, leader), c.clone()))
            .collect();
        Keymap {
            entries,
            pending: Vec::new(),
            pending_since: Instant::now(),
        }
    }

    /// Add or override a mapping (later mappings win).
    pub fn map(&mut self, lhs: &str, leader: &str, cmd: Cmd) {
        let keys = parse(lhs, leader);
        self.entries.retain(|(k, _)| *k != keys);
        self.entries.push((keys, cmd));
    }

    pub fn pending(&self) -> String {
        self.pending.concat()
    }

    /// If a pending sequence is an exact match and has waited long enough, run it.
    pub fn timeout(&mut self) -> Option<Cmd> {
        if self.pending.is_empty() || self.pending_since.elapsed() < Duration::from_millis(700) {
            return None;
        }
        let exact = self.entries.iter().find(|(k, _)| *k == self.pending).map(|(_, c)| c.clone());
        self.pending.clear();
        exact
    }

    pub fn feed(&mut self, key: &KeyEvent) -> Outcome {
        let tok = token(key);
        self.pending.push(tok.clone());
        self.pending_since = Instant::now();
        let mut exact = None;
        let mut longer = false;
        for (k, c) in self.entries.iter().rev() {
            if k.starts_with(&self.pending) {
                if k.len() == self.pending.len() {
                    exact.get_or_insert(c.clone());
                } else {
                    longer = true;
                }
            }
        }
        match (exact, longer) {
            (Some(c), false) => {
                self.pending.clear();
                Outcome::Run(c)
            }
            (_, true) => Outcome::Pending,
            (None, false) => {
                // dead end: retry with just this key
                let was_multi = self.pending.len() > 1;
                self.pending.clear();
                if was_multi {
                    self.feed(key)
                } else {
                    Outcome::None
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notation() {
        assert_eq!(parse("<leader>wc", "<Space>"), vec!["<Space>", "w", "c"]);
        assert_eq!(parse("<c-d>", " "), vec!["<C-d>"]);
        assert_eq!(parse("gg", " "), vec!["g", "g"]);
        assert_eq!(parse(":WordCount<CR>", " ")[0], ":");
    }

    #[test]
    fn sequences() {
        let mut km = Keymap::new("<Space>");
        let k = |c| KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE);
        assert!(matches!(km.feed(&k('g')), Outcome::Pending));
        assert!(matches!(km.feed(&k('g')), Outcome::Run(Cmd::Top)));
        assert!(matches!(km.feed(&k('j')), Outcome::Run(Cmd::Down(1))));
        km.map("<leader>t", "<Space>", Cmd::Ex("toc".into()));
        assert!(matches!(km.feed(&k(' ')), Outcome::Pending));
        assert!(matches!(km.feed(&k('t')), Outcome::Run(Cmd::Ex(_))));
        // `g` followed by unrelated key falls back to that key
        assert!(matches!(km.feed(&k('g')), Outcome::Pending));
        assert!(matches!(km.feed(&k('k')), Outcome::Run(Cmd::Up(1))));
    }
}

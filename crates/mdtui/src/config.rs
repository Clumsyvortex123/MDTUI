//! `~/.config/mdtui/config.toml`

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    /// `dark` or `light`.
    pub theme: String,
    /// Maximum width of rendered text (columns); wider panes centre the document.
    pub max_width: u16,
    pub toc: bool,
    pub toc_width: u16,
    /// Lines scrolled per mouse-wheel step.
    pub scroll_step: usize,
    /// Show line numbers in code blocks.
    pub code_line_numbers: bool,
    /// Key used for `<leader>` in keymaps.
    pub leader: String,
    pub images: ImagesConfig,
    pub mermaid: MermaidSection,
    pub editor: EditorConfig,
    pub plugins: PluginsConfig,
    /// Extra viewer keymaps: `"<leader>t" = ":toc"`.
    pub keys: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ImagesConfig {
    pub enabled: bool,
    /// `auto`, `kitty`, `sixel`, `iterm2` or `halfblocks`.
    pub protocol: String,
    pub max_height: u16,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct MermaidSection {
    pub mmdc: String,
    pub ascii: String,
    /// mermaid theme; empty = follow the mdtui theme.
    pub theme: String,
    pub background: String,
    pub puppeteer_config: Option<String>,
    pub width: u32,
    /// Retry with Chromium's --no-sandbox when its sandbox is blocked (Ubuntu AppArmor).
    pub no_sandbox_fallback: bool,
    /// `auto` (text when supported, else mmdc image), `text` or `image`.
    pub renderer: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct EditorConfig {
    pub relative_numbers: bool,
    /// LSP command; empty list disables. Started only if the binary exists.
    pub lsp: Vec<String>,
    /// Split ratio of the editor pane (0.2 – 0.8).
    pub split: f32,
    /// Auto-show completion while typing.
    pub auto_complete: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct PluginsConfig {
    pub enabled: bool,
    /// Give plugins `io`, `os`, `require` and `mdtui.exec`.
    pub trusted: bool,
    /// Plugin directory; default `~/.config/mdtui/plugins`.
    pub dir: Option<PathBuf>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            theme: "dark".into(),
            max_width: 110,
            toc: false,
            toc_width: 24,
            scroll_step: 3,
            code_line_numbers: false,
            leader: "<Space>".into(),
            images: ImagesConfig::default(),
            mermaid: MermaidSection::default(),
            editor: EditorConfig::default(),
            plugins: PluginsConfig::default(),
            keys: BTreeMap::new(),
        }
    }
}

impl Default for ImagesConfig {
    fn default() -> Self {
        ImagesConfig {
            enabled: true,
            protocol: "auto".into(),
            max_height: 24,
        }
    }
}

impl Default for MermaidSection {
    fn default() -> Self {
        MermaidSection {
            mmdc: "mmdc".into(),
            ascii: "mermaid-ascii".into(),
            theme: String::new(),
            background: "transparent".into(),
            puppeteer_config: None,
            width: 1200,
            no_sandbox_fallback: true,
            renderer: "auto".into(),
        }
    }
}

impl Default for EditorConfig {
    fn default() -> Self {
        EditorConfig {
            relative_numbers: false,
            lsp: vec!["marksman".into(), "server".into()],
            split: 0.5,
            auto_complete: true,
        }
    }
}

impl Default for PluginsConfig {
    fn default() -> Self {
        PluginsConfig {
            enabled: true,
            trusted: false,
            dir: None,
        }
    }
}

pub fn config_dir() -> PathBuf {
    if let Some(x) = std::env::var_os("MDTUI_CONFIG_DIR") {
        return PathBuf::from(x);
    }
    // prefer XDG-style ~/.config/mdtui on every unix, like most terminal tools
    if let Some(home) = std::env::var_os("XDG_CONFIG_HOME") {
        return PathBuf::from(home).join("mdtui");
    }
    if let Some(home) = directories::BaseDirs::new() {
        return home.home_dir().join(".config").join("mdtui");
    }
    PathBuf::from(".mdtui")
}

impl Config {
    pub fn load(path: Option<&PathBuf>) -> Result<Config, String> {
        let p = path.cloned().unwrap_or_else(|| config_dir().join("config.toml"));
        match std::fs::read_to_string(&p) {
            Ok(s) => toml::from_str(&s).map_err(|e| format!("{}: {e}", p.display())),
            Err(_) if path.is_none() => Ok(Config::default()),
            Err(e) => Err(format!("{}: {e}", p.display())),
        }
    }

    pub fn plugin_dir(&self) -> PathBuf {
        self.plugins.dir.clone().unwrap_or_else(|| config_dir().join("plugins"))
    }

    pub fn mermaid_theme(&self, dark: bool) -> String {
        if !self.mermaid.theme.is_empty() {
            self.mermaid.theme.clone()
        } else if dark {
            "dark".into()
        } else {
            "default".into()
        }
    }
}

pub const EXAMPLE: &str = r#"# ~/.config/mdtui/config.toml — all keys optional
theme = "dark"            # dark | light
max_width = 110           # document text width; wider panes centre it
toc = false               # show outline sidebar on start
toc_width = 24
scroll_step = 3
code_line_numbers = false
leader = "<Space>"

[images]
enabled = true
protocol = "auto"         # auto | kitty | sixel | iterm2 | halfblocks
max_height = 24           # rows

[mermaid]
renderer = "auto"         # auto: native text for flowchart/sequence/state/pie, mmdc image for others
                          # text | image to force one (per diagram: the [img]/[txt] button)
mmdc = "mmdc"             # npm i -g @mermaid-js/mermaid-cli
ascii = "mermaid-ascii"   # text fallback
theme = ""                # empty = follow mdtui theme
background = "transparent"
# puppeteer_config = "~/.config/mdtui/puppeteer.json"
width = 1200
no_sandbox_fallback = true  # retry mmdc with --no-sandbox if Chromium's sandbox is blocked

[editor]
relative_numbers = false
lsp = ["marksman", "server"]   # [] to disable
split = 0.5
auto_complete = true

[plugins]
enabled = true
trusted = false           # allow io/os/mdtui.exec in plugins
# dir = "~/.config/mdtui/plugins"

[keys]
# "<leader>t" = ":toc"
# "<leader>w" = ":WordCount"
"#;

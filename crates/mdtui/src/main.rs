//! mdtui — a VS Code-like markdown viewer/editor for the terminal.

mod app;
mod config;
mod input;
mod keys;
mod ui;
mod util;

use std::io::{IsTerminal, Read};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use clap::Parser;
use mdtui_render::mermaid::MermaidConfig;
use mdtui_render::{MediaConfig, MediaStore};
use ratatui::crossterm::event::{self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture};
use ratatui::crossterm::execute;
use ratatui_image::picker::{Picker, ProtocolType};

use crate::app::App;
use crate::config::Config;

#[derive(Parser, Debug)]
#[command(name = "mdtui", version, about = "A VS Code-like markdown viewer and editor for the terminal")]
struct Cli {
    /// Markdown file to open (`-` reads stdin). Defaults to README.md in the current directory.
    file: Option<PathBuf>,
    /// Colour theme: dark | light
    #[arg(long)]
    theme: Option<String>,
    /// Open the editor split right away
    #[arg(short, long)]
    edit: bool,
    /// Show the outline sidebar
    #[arg(long)]
    toc: bool,
    /// Don't render images (show alt text)
    #[arg(long)]
    no_images: bool,
    /// Don't load Lua plugins
    #[arg(long)]
    no_plugins: bool,
    /// Image protocol: auto | kitty | sixel | iterm2 | halfblocks
    #[arg(long)]
    protocol: Option<String>,
    /// Config file (default ~/.config/mdtui/config.toml)
    #[arg(long)]
    config: Option<PathBuf>,
    /// Print the rendered document to stdout and exit (no TUI)
    #[arg(long)]
    dump: bool,
    /// Width for --dump
    #[arg(long, default_value_t = 80)]
    width: u16,
    /// Colour for --dump: auto | always | never
    #[arg(long, default_value = "auto")]
    color: String,
    /// Print an example config.toml and exit
    #[arg(long)]
    print_config: bool,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    if cli.print_config {
        print!("{}", config::EXAMPLE);
        return Ok(());
    }
    let mut cfg = Config::load(cli.config.as_ref()).map_err(anyhow::Error::msg)?;
    if let Some(t) = &cli.theme {
        cfg.theme = t.clone();
    }
    if cli.no_images {
        cfg.images.enabled = false;
    }
    if cli.no_plugins {
        cfg.plugins.enabled = false;
    }
    if cli.toc {
        cfg.toc = true;
    }
    if let Some(p) = &cli.protocol {
        cfg.images.protocol = p.clone();
    }

    let (path, source, stdin_mode) = read_input(&cli)?;

    if cli.dump {
        return dump(cfg, path, source, &cli);
    }

    let mut terminal = ratatui::init();
    execute!(std::io::stdout(), EnableMouseCapture, EnableBracketedPaste)?;
    let result = run(&mut terminal, cfg, path, source, stdin_mode, cli.edit);
    let _ = execute!(std::io::stdout(), DisableMouseCapture, DisableBracketedPaste);
    ratatui::restore();
    result
}

fn read_input(cli: &Cli) -> Result<(Option<PathBuf>, String, bool)> {
    match &cli.file {
        Some(p) if p.as_os_str() == "-" => {
            let mut s = String::new();
            std::io::stdin().read_to_string(&mut s)?;
            Ok((None, s, true))
        }
        Some(p) => {
            let s = std::fs::read_to_string(p).with_context(|| format!("reading {}", p.display()))?;
            Ok((Some(p.clone()), s, false))
        }
        None if !std::io::stdin().is_terminal() => {
            let mut s = String::new();
            std::io::stdin().read_to_string(&mut s)?;
            Ok((None, s, true))
        }
        None => {
            for name in ["README.md", "readme.md", "Readme.md", "index.md"] {
                let p = PathBuf::from(name);
                if p.is_file() {
                    let s = std::fs::read_to_string(&p)?;
                    return Ok((Some(p), s, false));
                }
            }
            bail!("no file given and no README.md here (usage: mdtui <file.md>)")
        }
    }
}

fn media_config(cfg: &Config) -> MediaConfig {
    let dark = mdtui_core::Theme::by_name(&cfg.theme).map(|t| t.dark).unwrap_or(true);
    MediaConfig {
        renderer: mdtui_render::media::Renderer::parse(&cfg.mermaid.renderer).unwrap_or(mdtui_render::media::Renderer::Auto),
        palette: mdtui_render::diagram::Palette::default(),
        images: cfg.images.enabled,
        max_rows: cfg.images.max_height,
        mermaid: MermaidConfig {
            mmdc: cfg.mermaid.mmdc.clone(),
            ascii: cfg.mermaid.ascii.clone(),
            theme: cfg.mermaid_theme(dark),
            background: cfg.mermaid.background.clone(),
            puppeteer_config: cfg.mermaid.puppeteer_config.clone(),
            width: cfg.mermaid.width,
            no_sandbox_fallback: cfg.mermaid.no_sandbox_fallback,
        },
    }
}

fn pick(cfg: &Config) -> Picker {
    let mut picker = if cfg.images.enabled {
        Picker::from_query_stdio().unwrap_or_else(|_| Picker::halfblocks())
    } else {
        Picker::halfblocks()
    };
    let forced = match cfg.images.protocol.as_str() {
        "kitty" => Some(ProtocolType::Kitty),
        "sixel" => Some(ProtocolType::Sixel),
        "iterm2" | "iterm" => Some(ProtocolType::Iterm2),
        "halfblocks" | "blocks" => Some(ProtocolType::Halfblocks),
        _ => None,
    };
    if let Some(p) = forced {
        picker.set_protocol_type(p);
    }
    picker
}

fn base_dir(path: &Option<PathBuf>) -> PathBuf {
    path.as_ref()
        .and_then(|p| p.parent())
        .filter(|p| !p.as_os_str().is_empty())
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
}

fn run(
    terminal: &mut ratatui::DefaultTerminal,
    cfg: Config,
    path: Option<PathBuf>,
    source: String,
    stdin_mode: bool,
    edit: bool,
) -> Result<()> {
    let started = Instant::now();
    let picker = pick(&cfg);
    // late replies to the terminal capability queries must not be read as keypresses
    while event::poll(Duration::from_millis(80))? {
        let _ = event::read();
    }
    let media = MediaStore::new(picker, media_config(&cfg), base_dir(&path));
    let mut app = App::new(cfg, path, source, media, true);
    app.stdin_mode = stdin_mode;
    if edit {
        app.open_editor();
    }
    if let Some(p) = std::env::var_os("MDTUI_DEBUG_EVENTS") {
        let _ = std::fs::write(p, format!("{:?} loop start\n", started.elapsed()));
    }
    while !app.quit {
        app.tick();
        if app.dirty {
            app.dirty = false;
            terminal.draw(|f| ui::draw(f, &mut app))?;
        }
        if event::poll(Duration::from_millis(30))? {
            loop {
                let ev = event::read()?;
                if let Some(p) = std::env::var_os("MDTUI_DEBUG_EVENTS") {
                    use std::io::Write;
                    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(p) {
                        let _ = writeln!(f, "{:?} {ev:?}", started.elapsed());
                    }
                }
                input::handle(&mut app, ev);
                if app.quit || !event::poll(Duration::ZERO)? {
                    break;
                }
            }
        }
    }
    Ok(())
}

/// Non-interactive render to stdout (also handy for piping into `less -R`).
fn dump(mut cfg: Config, path: Option<PathBuf>, source: String, cli: &Cli) -> Result<()> {
    cfg.images.enabled = false;
    let media = MediaStore::new(Picker::halfblocks(), media_config(&cfg), base_dir(&path));
    let mut app = App::new(cfg, path, source, media, false);
    app.view = ratatui::layout::Rect::new(0, 0, cli.width + 3, 50);
    app.cfg.max_width = cli.width;
    app.invalidate_layout();
    app.relayout_if_needed();
    // give diagram renderers a moment
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline && app.layout_has_pending() {
        std::thread::sleep(Duration::from_millis(50));
        app.tick();
    }
    let color = match cli.color.as_str() {
        "always" => true,
        "never" => false,
        _ => std::io::stdout().is_terminal(),
    };
    for n in &app.notifications {
        eprintln!("mdtui: {}", n.msg);
    }
    print!("{}", util::lines_to_ansi(&app.layout.lines, color));
    Ok(())
}

//! Application state and behaviour (independent of drawing, see `ui.rs`).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};
use std::time::{Duration, Instant};

use mdtui_core::hitmap::Action;
use mdtui_core::ir::Block;
use mdtui_core::layout::{custom_key, layout, Layout, LayoutOptions};
use mdtui_core::search::{search, Match};
use mdtui_core::{parse, Document, ParseOptions, Theme};
use mdtui_editor::{Editor, Lsp, LspEvent, Popup};
use mdtui_plugin::{FenceCtx, FenceOutput, Level, PluginHost};
use mdtui_render::{MediaStore, Selection};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use ratatui::layout::Rect;

use crate::config::Config;
use crate::keys::{Cmd, Keymap};

pub const BUILTIN_COMMANDS: &[&str] = &[
    "q", "q!", "qa", "w", "wq", "x", "e", "e!", "edit", "vsplit", "close", "toc", "theme", "reload",
    "set", "help", "noh", "fold", "unfold", "plugins", "lsp", "open", "top", "bottom", "copy",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pane {
    Preview,
    Editor,
    Toc,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Input {
    None,
    Command { text: String, wild: Option<usize> },
    Search { text: String, origin: usize },
}

pub struct Notification {
    pub level: Level,
    pub msg: String,
    pub at: Instant,
}

pub struct Fullscreen {
    pub key: String,
    pub zoom: u32,
    /// Text diagrams: top-left cell shown. Images: source-pixel offset from the centre.
    pub pan: (i32, i32),
    /// Mouse drag in progress: (screen position at press, pan at press).
    pub drag: Option<((u16, u16), (i32, i32))>,
    /// Set on open; the first draw (once the area is known) positions the view.
    pub needs_init: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FsButton {
    Recenter,
    Close,
}

pub struct SearchState {
    pub query: String,
    pub matches: Vec<Match>,
    pub current: Option<usize>,
}

pub struct App {
    pub cfg: Config,
    pub theme: Theme,
    pub path: Option<PathBuf>,
    pub source: String,
    pub doc: Document,
    pub layout: Layout,
    layout_dirty: bool,
    pub media: MediaStore,
    pub plugins: Option<PluginHost>,
    markdown_fences: HashMap<String, Vec<Block>>,
    /// Per-diagram renderer chosen with the [img]/[txt] button.
    renderer_override: HashMap<String, mdtui_render::media::Renderer>,
    fence_errors: HashSet<String>,

    pub scroll: usize,
    pub view: Rect,
    pub x_offset: u16,
    pub toc_area: Rect,
    pub editor_area: Rect,
    pub divider_x: Option<u16>,
    pub show_toc: bool,
    pub toc_selected: usize,
    pub toc_scroll: usize,
    folded: HashSet<String>,
    show_front_matter: bool,
    show_source: HashSet<String>,
    pub focus_idx: Option<usize>,
    pub search: Option<SearchState>,
    pub input: Input,
    cmd_history: Vec<String>,
    cmd_hist_idx: Option<usize>,
    /// Text typed before the first Tab of a wildmenu cycle.
    pub wild_base: String,
    pub keymap: Keymap,
    history: Vec<(PathBuf, usize)>,
    forward: Vec<(PathBuf, usize)>,
    pub notifications: Vec<Notification>,

    pub editor: Option<Editor>,
    pub pane: Pane,
    pub split: f32,
    pub dragging_divider: bool,
    preview_dirty_at: Option<Instant>,
    last_cursor_row: usize,
    pub lsp: Option<Lsp>,
    lsp_req: Option<i64>,

    pub selection: Option<Selection>,
    sel_anchor: Option<(usize, u16)>,
    sel_moved: bool,
    pub fullscreen: Option<Fullscreen>,
    /// Area the fullscreen content is drawn in, and its title-bar buttons (set by the UI).
    pub fs_body: Rect,
    pub fs_buttons: Vec<(Rect, FsButton)>,
    pub help: bool,

    _watcher: Option<RecommendedWatcher>,
    watch_rx: Option<Receiver<()>>,
    reload_at: Option<Instant>,
    pub dirty: bool,
    pub quit: bool,
    pub stdin_mode: bool,
}

impl App {
    pub fn new(cfg: Config, path: Option<PathBuf>, source: String, media: MediaStore, live: bool) -> App {
        let theme = Theme::by_name(&cfg.theme).unwrap_or_else(Theme::dark);
        let mut keymap = Keymap::new(&cfg.leader);
        for (lhs, rhs) in &cfg.keys {
            keymap.map(lhs, &cfg.leader, Cmd::Ex(rhs.trim_start_matches(':').trim_end_matches("<CR>").to_string()));
        }
        let split = cfg.editor.split.clamp(0.2, 0.8);
        let show_toc = cfg.toc;
        let mut app = App {
            theme,
            path,
            source,
            doc: Document::default(),
            layout: Layout::default(),
            layout_dirty: true,
            media,
            plugins: None,
            markdown_fences: HashMap::new(),
            renderer_override: HashMap::new(),
            fence_errors: HashSet::new(),
            scroll: 0,
            view: Rect::default(),
            x_offset: 0,
            toc_area: Rect::default(),
            editor_area: Rect::default(),
            divider_x: None,
            show_toc,
            toc_selected: 0,
            toc_scroll: 0,
            folded: HashSet::new(),
            show_front_matter: false,
            show_source: HashSet::new(),
            focus_idx: None,
            search: None,
            input: Input::None,
            cmd_history: Vec::new(),
            cmd_hist_idx: None,
            wild_base: String::new(),
            keymap,
            history: Vec::new(),
            forward: Vec::new(),
            notifications: Vec::new(),
            editor: None,
            pane: Pane::Preview,
            split,
            dragging_divider: false,
            preview_dirty_at: None,
            last_cursor_row: usize::MAX,
            lsp: None,
            lsp_req: None,
            selection: None,
            sel_anchor: None,
            sel_moved: false,
            fullscreen: None,
            fs_body: Rect::default(),
            fs_buttons: Vec::new(),
            help: false,
            _watcher: None,
            watch_rx: None,
            reload_at: None,
            dirty: true,
            quit: false,
            stdin_mode: false,
            cfg,
        };
        app.media.set_background(app.theme.bg);
        app.media.config.palette = mdtui_render::diagram::Palette::from_theme(&app.theme);
        app.load_plugins();
        if live {
            app.watch();
        }
        app.rebuild();
        app
    }

    // ------------------------------------------------------------------ fullscreen view

    /// Inclusive pan limits ((min_x, min_y), (max_x, max_y)) for the current fullscreen view.
    pub fn fs_bounds(&self) -> Option<((i32, i32), (i32, i32))> {
        let fs = self.fullscreen.as_ref()?;
        let body = self.fs_body;
        if let Some(lines) = self.media.text_lines(&fs.key) {
            let w = mdtui_render::diagram::width(lines) as i32;
            let h = lines.len() as i32;
            let max = ((w - body.width as i32).max(0), (h - body.height as i32).max(0));
            return Some(((0, 0), max));
        }
        let g = self.media.fullscreen_geometry(&fs.key, (body.width, body.height), fs.zoom)?;
        Some(((-g.max_pan.0, -g.max_pan.1), g.max_pan))
    }

    pub fn fs_clamp(&mut self) {
        let Some((lo, hi)) = self.fs_bounds() else { return };
        if let Some(fs) = &mut self.fullscreen {
            fs.pan = (fs.pan.0.clamp(lo.0, hi.0), fs.pan.1.clamp(lo.1, hi.1));
        }
    }

    /// Centre the content.
    pub fn fs_recenter(&mut self) {
        let Some((lo, hi)) = self.fs_bounds() else { return };
        if let Some(fs) = &mut self.fullscreen {
            fs.pan = ((lo.0 + hi.0) / 2, (lo.1 + hi.1) / 2);
            fs.drag = None;
        }
        self.dirty = true;
    }

    /// First view: centred horizontally, top of the diagram (images: centred).
    pub fn fs_init_view(&mut self) {
        let is_text = self.fullscreen.as_ref().is_some_and(|f| self.media.text_lines(&f.key).is_some());
        self.fs_recenter();
        if is_text {
            if let Some(fs) = &mut self.fullscreen {
                fs.pan.1 = 0;
            }
        }
        if let Some(fs) = &mut self.fullscreen {
            fs.needs_init = false;
        }
    }

    /// Screen cells → pan units (cells for text, source pixels for images).
    fn fs_cells_to_pan(&self, dx: i32, dy: i32) -> (i32, i32) {
        let Some(fs) = &self.fullscreen else { return (0, 0) };
        if self.media.text_lines(&fs.key).is_some() {
            return (dx, dy);
        }
        match self.media.fullscreen_geometry(&fs.key, (self.fs_body.width, self.fs_body.height), fs.zoom) {
            Some(g) => ((dx as f32 * g.px_per_cell.0).round() as i32, (dy as f32 * g.px_per_cell.1).round() as i32),
            None => (0, 0),
        }
    }

    /// Move the view by screen cells (positive = towards the right / bottom of the content).
    pub fn fs_pan_by(&mut self, dx: i32, dy: i32) {
        let (px, py) = self.fs_cells_to_pan(dx, dy);
        if let Some(fs) = &mut self.fullscreen {
            fs.pan = (fs.pan.0 + px, fs.pan.1 + py);
        }
        self.fs_clamp();
        self.dirty = true;
    }

    pub fn fs_drag_start(&mut self, x: u16, y: u16) {
        if let Some(fs) = &mut self.fullscreen {
            fs.drag = Some(((x, y), fs.pan));
        }
    }

    /// Content follows the mouse.
    pub fn fs_drag_to(&mut self, x: u16, y: u16) {
        let Some(((x0, y0), start)) = self.fullscreen.as_ref().and_then(|f| f.drag) else { return };
        let (px, py) = self.fs_cells_to_pan(x as i32 - x0 as i32, y as i32 - y0 as i32);
        if let Some(fs) = &mut self.fullscreen {
            fs.pan = (start.0 - px, start.1 - py);
        }
        self.fs_clamp();
        self.dirty = true;
    }

    pub fn fs_drag_end(&mut self) {
        if let Some(fs) = &mut self.fullscreen {
            fs.drag = None;
        }
    }

    pub fn fs_zoom(&mut self, delta: i32) {
        let is_text = self.fullscreen.as_ref().is_some_and(|f| self.media.text_lines(&f.key).is_some());
        if is_text {
            return;
        }
        if let Some(fs) = &mut self.fullscreen {
            let z = (fs.zoom as i32 + delta).clamp(1, 8) as u32;
            // keep the same point centred: pan is relative to the centre, so it scales with nothing
            fs.zoom = z;
        }
        self.fs_clamp();
        self.dirty = true;
    }

    pub fn is_folded(&self, anchor: &str) -> bool {
        self.folded.contains(anchor)
    }

    // ------------------------------------------------------------------ notifications

    pub fn notify(&mut self, level: Level, msg: impl Into<String>) {
        self.notifications.push(Notification {
            level,
            msg: msg.into(),
            at: Instant::now(),
        });
        self.dirty = true;
    }

    fn info(&mut self, msg: impl Into<String>) {
        self.notify(Level::Info, msg)
    }

    fn warn(&mut self, msg: impl Into<String>) {
        self.notify(Level::Warn, msg)
    }

    fn error(&mut self, msg: impl Into<String>) {
        self.notify(Level::Error, msg)
    }

    // ------------------------------------------------------------------ plugins

    fn load_plugins(&mut self) {
        if !self.cfg.plugins.enabled {
            return;
        }
        match PluginHost::new(self.cfg.plugins.trusted) {
            Ok(mut host) => {
                let errors = host.load_dir(&self.cfg.plugin_dir());
                for e in errors {
                    self.error(e);
                }
                for km in host.keymaps() {
                    if km.mode == "n" || km.mode.is_empty() {
                        let rhs = km.rhs.trim_start_matches(':').trim_end_matches("<CR>").to_string();
                        self.keymap.map(&km.lhs, &self.cfg.leader.clone(), Cmd::Ex(rhs));
                    }
                }
                self.plugins = Some(host);
            }
            Err(e) => self.error(format!("lua: {e}")),
        }
    }

    fn drain_plugin_notes(&mut self) {
        let notes = self
            .plugins
            .as_ref()
            .map(|p| p.take_notifications())
            .unwrap_or_default();
        for (l, m) in notes {
            self.notify(l, m);
        }
    }

    // ------------------------------------------------------------------ document pipeline

    pub fn base_dir(&self) -> PathBuf {
        self.path
            .as_deref()
            .and_then(Path::parent)
            .filter(|p| !p.as_os_str().is_empty())
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."))
    }

    pub fn file_name(&self) -> String {
        self.path
            .as_ref()
            .map(|p| p.file_name().unwrap_or(p.as_os_str()).to_string_lossy().into_owned())
            .unwrap_or_else(|| "[stdin]".into())
    }

    /// Parse the source, run plugins, kick off diagram rendering, and lay out.
    pub fn rebuild(&mut self) {
        let plugin_fences = self.plugins.as_ref().map(|p| p.fence_names()).unwrap_or_default();
        let mut opts = ParseOptions::default();
        opts.fence_handlers.insert("mermaid".into());
        opts.fence_handlers.extend(plugin_fences.iter().cloned());
        let mut doc = parse(&self.source, &opts);
        if let Some(p) = &self.plugins {
            match p.transform(doc.clone()) {
                Ok(d) => doc = d,
                Err(e) => self.notify(Level::Error, e),
            }
        }

        // custom fences
        let mut customs = Vec::new();
        doc.walk_blocks(&mut |b| {
            if let Block::Custom { handler, content, .. } = b {
                customs.push((handler.clone(), content.clone()));
            }
        });
        let width = self.content_width();
        for (handler, content) in customs {
            let key = custom_key(&handler, &content);
            if self.media.contains(&key) || self.markdown_fences.contains_key(&key) {
                continue;
            }
            if plugin_fences.contains(&handler) {
                let ctx = FenceCtx {
                    width,
                    dark: self.theme.dark,
                    path: self.path.as_ref().map(|p| p.display().to_string()).unwrap_or_default(),
                };
                let r = self.plugins.as_ref().unwrap().render_fence(&handler, &content, &ctx);
                match r {
                    Ok(FenceOutput::Image(bytes)) => self.media.set_image_bytes(&key, bytes),
                    Ok(FenceOutput::Lines(lines)) => self.media.set_lines(&key, lines),
                    Ok(FenceOutput::Markdown(md)) => {
                        let sub = parse(&md, &ParseOptions::default());
                        self.markdown_fences.insert(key.clone(), sub.blocks);
                    }
                    Err(e) => {
                        if self.fence_errors.insert(key.clone()) {
                            self.notify(Level::Warn, format!("{handler}: {e}"));
                        }
                        self.media.set_failed(&key, e);
                    }
                }
            } else if handler == "mermaid" {
                let mode = self.renderer_override.get(&key).copied();
                self.media.request_mermaid(&key, &content, mode);
            }
        }
        if !self.markdown_fences.is_empty() {
            splice_markdown(&mut doc.blocks, &self.markdown_fences);
        }
        self.drain_plugin_notes();
        self.doc = doc;
        self.layout_dirty = true;
        self.dirty = true;
    }

    pub fn content_width(&self) -> u16 {
        let w = self.view.width.saturating_sub(3);
        w.min(self.cfg.max_width).max(20)
    }

    pub fn relayout_if_needed(&mut self) {
        if !self.layout_dirty {
            return;
        }
        self.layout_dirty = false;
        let width = self.content_width();
        self.x_offset = if self.view.width > width + 3 {
            (self.view.width - width) / 2
        } else {
            2
        };
        let opts = LayoutOptions {
            width,
            folded: self.folded.clone(),
            show_front_matter: self.show_front_matter,
            code_line_numbers: self.cfg.code_line_numbers,
            show_source: self.show_source.clone(),
        };
        self.layout = layout(&self.doc, &opts, &self.theme, &self.media);
        for key in self.media.take_missing() {
            if let Some(url) = key.strip_prefix("img:") {
                self.media.request_image(url);
            }
        }
        if let Some(s) = &mut self.search {
            s.matches = search(&self.layout, &s.query);
            if s.current.is_some_and(|c| c >= s.matches.len()) {
                s.current = (!s.matches.is_empty()).then_some(0);
            }
        }
        self.clamp_scroll();
        self.dirty = true;
    }

    pub fn invalidate_layout(&mut self) {
        self.layout_dirty = true;
        self.dirty = true;
    }

    fn max_scroll(&self) -> usize {
        self.layout.height().saturating_sub((self.view.height as usize).max(1) / 2)
    }

    fn clamp_scroll(&mut self) {
        self.scroll = self.scroll.min(self.max_scroll());
    }

    pub fn scroll_to(&mut self, row: usize) {
        self.scroll = row;
        self.clamp_scroll();
        self.dirty = true;
    }

    fn jump_to_row(&mut self, row: usize) {
        if let Some(p) = &self.path {
            self.history.push((p.clone(), self.scroll));
            self.forward.clear();
        }
        self.scroll_to(row.saturating_sub(1));
    }

    /// Ensure image protocols for everything on screen are being encoded.
    pub fn prepare_media(&mut self) {
        let (top, bottom) = (self.scroll, self.scroll + self.view.height as usize);
        let visible: Vec<_> = self
            .layout
            .media
            .iter()
            .filter(|p| p.row + (p.height as usize) > top && p.row < bottom)
            .cloned()
            .collect();
        self.media.ensure_protocols(visible.iter());
    }

    // ------------------------------------------------------------------ files

    fn watch(&mut self) {
        let Some(path) = self.path.clone() else { return };
        let (tx, rx) = channel();
        let target = std::fs::canonicalize(&path).unwrap_or(path.clone());
        let name = target.file_name().map(|s| s.to_os_string());
        let watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            if let Ok(ev) = res {
                if ev.kind.is_access() {
                    return;
                }
                if ev.paths.iter().any(|p| p.file_name().map(|s| s.to_os_string()) == name) {
                    let _ = tx.send(());
                }
            }
        });
        match watcher {
            Ok(mut w) => {
                let dir = target.parent().unwrap_or(Path::new(".")).to_path_buf();
                if w.watch(&dir, RecursiveMode::NonRecursive).is_ok() {
                    self._watcher = Some(w);
                    self.watch_rx = Some(rx);
                }
            }
            Err(e) => self.warn(format!("live reload unavailable: {e}")),
        }
    }

    pub fn layout_has_pending(&self) -> bool {
        self.media.pending_count() > 0
    }

    pub fn is_live(&self) -> bool {
        self.watch_rx.is_some()
    }

    /// Re-read the file from disk.
    pub fn reload(&mut self, force: bool) {
        let Some(path) = self.path.clone() else { return };
        let Ok(text) = std::fs::read_to_string(&path) else {
            self.warn(format!("{} disappeared", path.display()));
            return;
        };
        if text == self.source && !force {
            return;
        }
        if let Some(ed) = &mut self.editor {
            if ed.text() == text {
                ed.mark_saved();
                return;
            }
            if ed.is_dirty() && !force {
                self.warn("file changed on disk; you have unsaved edits (:e! to reload, :w to overwrite)");
                return;
            }
            ed.set_text(&text);
            if let Some(l) = &mut self.lsp {
                l.did_change(&text);
            }
        }
        // keep the reader's place: remember the heading at the top and the offset from it
        let anchor = self
            .layout
            .breadcrumb(self.scroll)
            .last()
            .map(|h| (h.anchor.clone(), self.layout.anchors.get(&h.anchor).copied().unwrap_or(0)));
        self.source = text;
        self.rebuild();
        self.relayout_if_needed();
        if let Some((a, old_row)) = anchor {
            if let Some(&row) = self.layout.anchors.get(&a) {
                let off = self.scroll.saturating_sub(old_row);
                self.scroll_to(row + off);
            }
        }
    }

    fn write_file(&mut self, text: &str) -> bool {
        let Some(path) = self.path.clone() else {
            self.error("no file name (stdin); use :w <file>");
            return false;
        };
        match std::fs::write(&path, text) {
            Ok(()) => true,
            Err(e) => {
                self.error(format!("write {}: {e}", path.display()));
                false
            }
        }
    }

    pub fn open_file(&mut self, path: PathBuf, push_history: bool) {
        if self.editor.as_ref().is_some_and(|e| e.is_dirty()) {
            self.warn("unsaved changes; :w first");
            return;
        }
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) => {
                self.error(format!("{}: {e}", path.display()));
                return;
            }
        };
        if push_history {
            if let Some(p) = &self.path {
                self.history.push((p.clone(), self.scroll));
                self.forward.clear();
            }
        }
        self.path = Some(path.clone());
        self.media.set_base_dir(self.base_dir());
        self.source = text.clone();
        self.folded.clear();
        self.search = None;
        self.focus_idx = None;
        self.selection = None;
        self.scroll = 0;
        if let Some(ed) = &mut self.editor {
            *ed = Editor::new(&text, Some(path.clone()));
            ed.relative_numbers = self.cfg.editor.relative_numbers;
        }
        self.lsp = None;
        if self.editor.is_some() {
            self.start_lsp();
        }
        self._watcher = None;
        self.watch_rx = None;
        self.watch();
        self.rebuild();
    }

    fn back(&mut self) {
        if let Some((p, s)) = self.history.pop() {
            if let Some(cur) = &self.path {
                self.forward.push((cur.clone(), self.scroll));
            }
            if Some(&p) != self.path.as_ref() {
                self.open_file(p, false);
                self.relayout_if_needed();
            }
            self.scroll_to(s);
        }
    }

    fn forward(&mut self) {
        if let Some((p, s)) = self.forward.pop() {
            if let Some(cur) = &self.path {
                self.history.push((cur.clone(), self.scroll));
            }
            if Some(&p) != self.path.as_ref() {
                self.open_file(p, false);
                self.relayout_if_needed();
            }
            self.scroll_to(s);
        }
    }

    // ------------------------------------------------------------------ editor

    pub fn open_editor(&mut self) {
        if self.editor.is_none() {
            let mut ed = Editor::new(&self.source, self.path.clone());
            ed.relative_numbers = self.cfg.editor.relative_numbers;
            let line = self.layout.line_for_row(self.scroll);
            ed.set_cursor_row(line.saturating_sub(1));
            self.editor = Some(ed);
            self.start_lsp();
        }
        self.pane = Pane::Editor;
        self.invalidate_layout();
    }

    fn start_lsp(&mut self) {
        let cmd = self.cfg.editor.lsp.clone();
        let (Some(prog), Some(path)) = (cmd.first(), self.path.clone()) else {
            return;
        };
        if mdtui_render::find_in_path(prog).is_none() {
            return;
        }
        self.lsp = Lsp::start(&cmd, &path, &self.source);
    }

    pub fn close_editor(&mut self, force: bool) -> bool {
        if let Some(ed) = &self.editor {
            if ed.is_dirty() && !force {
                self.warn("unsaved changes (:w to save, :close! to discard)");
                return false;
            }
        }
        self.editor = None;
        self.lsp = None;
        self.pane = Pane::Preview;
        // back to the on-disk version
        self.reload(true);
        self.invalidate_layout();
        true
    }

    pub fn save(&mut self) -> bool {
        let text = match &self.editor {
            Some(ed) => ed.text().to_string(),
            None => self.source.clone(),
        };
        if self.write_file(&text) {
            if let Some(ed) = &mut self.editor {
                ed.mark_saved();
            }
            if let Some(l) = &mut self.lsp {
                l.did_save();
            }
            self.source = text;
            self.info(format!("written {}", self.file_name()));
            true
        } else {
            false
        }
    }

    pub fn editor_changed(&mut self) {
        self.preview_dirty_at = Some(Instant::now());
        if let (Some(ed), Some(l)) = (&self.editor, &mut self.lsp) {
            l.did_change(ed.text());
        }
        self.dirty = true;
    }

    /// Compute completions at the editor cursor and open the popup.
    pub fn trigger_completion(&mut self, manual: bool) {
        let Some(ed) = &self.editor else { return };
        let before = ed.before_cursor();
        let dir = ed.dir();
        let ctx = mdtui_editor::completion::Ctx {
            before: &before,
            text: ed.text(),
            dir: &dir,
            manual,
        };
        let mut set = mdtui_editor::completion::builtin(&ctx);
        if let Some(p) = &self.plugins {
            for s in p.complete(&before) {
                let items = s.items.into_iter().map(|i| mdtui_editor::Item {
                    label: i.label,
                    insert: i.insert,
                    detail: if i.detail.is_empty() { "plugin".into() } else { i.detail },
                    kind: mdtui_editor::Kind::Plugin,
                });
                match &mut set {
                    Some(cur) if cur.replace_chars == s.replace_chars => cur.items.extend(items),
                    Some(_) => {}
                    None => {
                        set = Some(mdtui_editor::CompletionSet {
                            replace_chars: s.replace_chars,
                            items: items.collect(),
                        })
                    }
                }
            }
        }
        self.drain_plugin_notes();
        let ed = self.editor.as_mut().unwrap();
        ed.popup = set.and_then(Popup::new);
        if manual || ed.popup.is_some() {
            if let Some(l) = &mut self.lsp {
                let (row, col) = ed.cursor();
                let line = ed.current_line();
                self.lsp_req = l.completion(row, mdtui_editor::lsp::utf16_col(&line, col));
            }
        }
        self.dirty = true;
    }

    pub fn accept_completion(&mut self) {
        let Some(ed) = &mut self.editor else { return };
        let Some(p) = ed.popup.take() else { return };
        let item = p.current().clone();
        ed.insert_completion(p.set.replace_chars, &item.insert);
        self.editor_changed();
        // keep completing into directories
        if item.kind == mdtui_editor::Kind::Folder {
            self.trigger_completion(false);
        }
    }

    // ------------------------------------------------------------------ periodic work

    /// Background work: media results, file changes, debounced preview, LSP, timeouts.
    pub fn tick(&mut self) {
        let (relayout, redraw) = self.media.poll();
        if relayout {
            self.invalidate_layout();
        }
        if redraw {
            self.dirty = true;
        }

        if let Some(rx) = &self.watch_rx {
            let mut changed = false;
            while rx.try_recv().is_ok() {
                changed = true;
            }
            if changed {
                self.reload_at = Some(Instant::now() + Duration::from_millis(80));
            }
        }
        if self.reload_at.is_some_and(|t| Instant::now() >= t) {
            self.reload_at = None;
            self.reload(false);
        }

        if self.preview_dirty_at.is_some_and(|t| t.elapsed() >= Duration::from_millis(120)) {
            self.preview_dirty_at = None;
            if let Some(ed) = &self.editor {
                self.source = ed.text().to_string();
                self.rebuild();
            }
        }

        // preview follows the editor cursor
        if let Some(ed) = &self.editor {
            let row = ed.cursor().0;
            if row != self.last_cursor_row {
                self.last_cursor_row = row;
                self.relayout_if_needed();
                let target = self.layout.row_for_line(row + 1);
                let third = self.view.height as usize / 3;
                self.scroll_to(target.saturating_sub(third));
            }
        }

        if let Some(l) = &mut self.lsp {
            let events = l.poll();
            for ev in events {
                match ev {
                    LspEvent::Ready => self.dirty = true,
                    LspEvent::Diagnostics(d) => {
                        if let Some(ed) = &mut self.editor {
                            ed.diagnostics = d;
                        }
                        self.dirty = true;
                    }
                    LspEvent::Completion { id, items, replace_start } => {
                        if Some(id) != self.lsp_req || items.is_empty() {
                            continue;
                        }
                        let Some(ed) = &mut self.editor else { continue };
                        let (_, col) = ed.cursor();
                        let line = ed.current_line();
                        let cur16 = mdtui_editor::lsp::utf16_col(&line, col);
                        let replace = match replace_start {
                            Some(s) if s <= cur16 => {
                                let prefix: String = line.chars().take(col).collect();
                                // count chars from the UTF-16 start offset
                                let mut acc = 0;
                                let mut start_char = col;
                                for (i, c) in prefix.chars().enumerate() {
                                    if acc >= s {
                                        start_char = i;
                                        break;
                                    }
                                    acc += c.len_utf16();
                                }
                                col - start_char.min(col)
                            }
                            _ => 0,
                        };
                        match &mut ed.popup {
                            Some(p) if p.set.replace_chars == replace => {
                                for it in items {
                                    if !p.set.items.iter().any(|x| x.insert == it.insert) {
                                        p.set.items.push(it);
                                    }
                                }
                            }
                            Some(_) => {}
                            None => {
                                ed.popup = Popup::new(mdtui_editor::CompletionSet { replace_chars: replace, items });
                            }
                        }
                        self.dirty = true;
                    }
                    LspEvent::Exited => {
                        self.lsp = None;
                        self.warn("language server exited");
                        break;
                    }
                }
            }
        }

        if let Some(cmd) = self.keymap.timeout() {
            self.run(cmd);
        }

        let before = self.notifications.len();
        self.notifications.retain(|n| {
            let ttl = if n.level == Level::Error { 8 } else { 4 };
            n.at.elapsed() < Duration::from_secs(ttl)
        });
        if self.notifications.len() != before {
            self.dirty = true;
        }
        self.relayout_if_needed();
    }

    // ------------------------------------------------------------------ actions

    pub fn focused_action(&self) -> Option<Action> {
        let f = self.layout.hits.focusables();
        self.focus_idx.and_then(|i| f.get(i)).map(|h| h.action.clone())
    }

    fn focus_step(&mut self, forward: bool) {
        let f = self.layout.hits.focusables();
        if f.is_empty() {
            return;
        }
        let n = f.len();
        let top = self.scroll;
        let bottom = self.scroll + self.view.height as usize;
        let next = match self.focus_idx {
            Some(i) if f.get(i).is_some_and(|h| h.row >= top && h.row < bottom) => {
                if forward { (i + 1) % n } else { (i + n - 1) % n }
            }
            // start from the first/last target on screen
            _ => {
                if forward {
                    f.iter().position(|h| h.row >= top).unwrap_or(0)
                } else {
                    f.iter().rposition(|h| h.row < bottom).unwrap_or(n - 1)
                }
            }
        };
        let row = f[next].row;
        self.focus_idx = Some(next);
        if row < top || row >= bottom {
            self.scroll_to(row.saturating_sub(self.view.height as usize / 3));
        }
        self.dirty = true;
    }

    pub fn activate(&mut self, action: Action) {
        match action {
            Action::OpenLink(url) => self.open_link(&url),
            Action::ToggleTask { line, col } => self.toggle_task(line, col),
            Action::CopyCode(code) => {
                let via = crate::util::copy_to_clipboard(&code);
                self.info(format!("copied {} lines ({via})", code.lines().count()));
            }
            Action::ToggleRenderer(key) => {
                use mdtui_render::media::Renderer;
                let next = if self.media.is_text(&key) { Renderer::Image } else { Renderer::Text };
                if next == Renderer::Image && !self.media.config.images {
                    self.warn("images are off (:set images)");
                    return;
                }
                self.renderer_override.insert(key.clone(), next);
                self.media.forget(&key);
                self.rebuild();
                if next == Renderer::Image {
                    self.info("rendering with mmdc…");
                }
            }
            Action::OpenMedia(key) => {
                if self.media.image_dims(&key).is_some() || self.media.text_lines(&key).is_some() {
                    self.fullscreen = Some(Fullscreen { key, zoom: 1, pan: (0, 0), drag: None, needs_init: true });
                } else {
                    self.warn("image not loaded yet");
                }
            }
            Action::ToggleSource(key) => {
                if !self.show_source.remove(&key) {
                    self.show_source.insert(key);
                }
                self.invalidate_layout();
            }
            Action::Footnote(name) => {
                if let Some(&row) = self.layout.footnotes.get(&name) {
                    self.jump_to_row(row);
                }
            }
            Action::Fold(anchor) => {
                if !self.folded.remove(&anchor) {
                    self.folded.insert(anchor);
                }
                self.invalidate_layout();
            }
            Action::FrontMatter => {
                self.show_front_matter = !self.show_front_matter;
                self.invalidate_layout();
            }
        }
        self.dirty = true;
    }

    fn open_link(&mut self, url: &str) {
        if let Some(anchor) = url.strip_prefix('#') {
            let anchor = anchor.to_lowercase();
            match self.layout.anchors.get(&anchor).or_else(|| self.layout.anchors.get(anchor.as_str())) {
                Some(&row) => self.jump_to_row(row),
                None => self.warn(format!("no heading #{anchor}")),
            }
            return;
        }
        let is_url = url.contains("://") || url.starts_with("mailto:");
        if is_url {
            match open::that_detached(url) {
                Ok(()) => self.info(format!("opened {url}")),
                Err(e) => self.error(format!("open {url}: {e}")),
            }
            return;
        }
        let (file, anchor) = match url.split_once('#') {
            Some((f, a)) => (f, Some(a)),
            None => (url, None),
        };
        let p = Path::new(file);
        let p = if p.is_absolute() { p.to_path_buf() } else { self.base_dir().join(p) };
        // wikilinks often omit the extension
        let p = if !p.exists() && p.extension().is_none() { p.with_extension("md") } else { p };
        if !p.exists() {
            self.warn(format!("not found: {}", p.display()));
            return;
        }
        let is_md = p
            .extension()
            .is_some_and(|e| matches!(e.to_string_lossy().to_lowercase().as_str(), "md" | "markdown" | "mdown" | "mkd"));
        if is_md {
            self.open_file(p, true);
            self.relayout_if_needed();
            if let Some(a) = anchor {
                if let Some(&row) = self.layout.anchors.get(a) {
                    self.scroll_to(row.saturating_sub(1));
                }
            }
        } else if let Err(e) = open::that_detached(&p) {
            self.error(format!("open {}: {e}", p.display()));
        }
    }

    fn toggle_task(&mut self, line: usize, col: usize) {
        if let Some(ed) = &mut self.editor {
            let text = ed.text().to_string();
            if let Some(new) = mdtui_core::source::toggle_task(&text, line, col) {
                // edit through the editor so it shows up (and is saved with :w)
                ed.replace_text(&new);
                self.source = new;
                self.editor_changed();
                self.rebuild();
            }
            return;
        }
        if self.stdin_mode {
            self.warn("read-only (stdin)");
            return;
        }
        match mdtui_core::source::toggle_task(&self.source, line, col) {
            Some(new) => {
                if self.write_file(&new) {
                    self.source = new;
                    self.rebuild();
                }
            }
            None => self.warn("checkbox moved; reload with r"),
        }
    }

    // ------------------------------------------------------------------ command dispatch

    pub fn run(&mut self, cmd: Cmd) {
        let page = (self.view.height as usize).max(2);
        self.dirty = true;
        match cmd {
            Cmd::Down(n) => self.scroll_to(self.scroll + n),
            Cmd::Up(n) => self.scroll_to(self.scroll.saturating_sub(n)),
            Cmd::HalfDown => self.scroll_to(self.scroll + page / 2),
            Cmd::HalfUp => self.scroll_to(self.scroll.saturating_sub(page / 2)),
            Cmd::PageDown => self.scroll_to(self.scroll + page - 2),
            Cmd::PageUp => self.scroll_to(self.scroll.saturating_sub(page - 2)),
            Cmd::Top => self.scroll_to(0),
            Cmd::Bottom => self.scroll_to(usize::MAX),
            Cmd::NextHeading => {
                if let Some((_, r)) = self.layout.headings.iter().find(|(_, r)| *r > self.scroll + 1) {
                    self.scroll_to(r.saturating_sub(1));
                }
            }
            Cmd::PrevHeading => {
                if let Some((_, r)) = self.layout.headings.iter().rev().find(|(_, r)| r + 1 < self.scroll + 1) {
                    self.scroll_to(r.saturating_sub(1));
                }
            }
            Cmd::FocusNext => self.focus_step(true),
            Cmd::FocusPrev => self.focus_step(false),
            Cmd::Activate => {
                if self.pane == Pane::Toc {
                    self.toc_jump(self.toc_selected);
                } else if let Some(a) = self.focused_action() {
                    self.activate(a);
                }
            }
            Cmd::ToggleTask => match self.focused_action() {
                Some(a @ Action::ToggleTask { .. }) => self.activate(a),
                _ => {
                    // otherwise: the first task on screen
                    let top = self.scroll;
                    let bottom = top + self.view.height as usize;
                    let first = self
                        .layout
                        .hits
                        .all()
                        .iter()
                        .find(|h| h.row >= top && h.row < bottom && matches!(h.action, Action::ToggleTask { .. }))
                        .map(|h| h.action.clone());
                    match first {
                        Some(a) => self.activate(a),
                        None => self.info("no task on screen"),
                    }
                }
            },
            Cmd::Yank => match self.focused_action() {
                Some(Action::OpenLink(u)) => {
                    let via = crate::util::copy_to_clipboard(&u);
                    self.info(format!("copied {u} ({via})"));
                }
                Some(Action::CopyCode(c)) => self.activate(Action::CopyCode(c)),
                _ => {
                    if let Some(sel) = self.selection {
                        let t = sel.text(&self.layout);
                        let via = crate::util::copy_to_clipboard(&t);
                        self.info(format!("copied {} chars ({via})", t.chars().count()));
                    }
                }
            },
            Cmd::ToggleToc => {
                self.show_toc = !self.show_toc;
                if self.show_toc {
                    self.sync_toc_selection();
                } else if self.pane == Pane::Toc {
                    self.pane = Pane::Preview;
                }
                self.invalidate_layout();
            }
            Cmd::FoldToggle => {
                if let Some(h) = self.layout.breadcrumb(self.scroll + 1).last() {
                    let a = h.anchor.clone();
                    self.activate(Action::Fold(a));
                }
            }
            Cmd::FoldAll => {
                self.folded = self.doc.headings().into_iter().filter(|h| h.level >= 2).map(|h| h.anchor).collect();
                self.invalidate_layout();
            }
            Cmd::UnfoldAll => {
                self.folded.clear();
                self.invalidate_layout();
            }
            Cmd::Search => {
                self.input = Input::Search { text: String::new(), origin: self.scroll };
            }
            Cmd::SearchNext => self.search_step(true),
            Cmd::SearchPrev => self.search_step(false),
            Cmd::CommandLine => {
                self.input = Input::Command { text: String::new(), wild: None };
                self.cmd_hist_idx = None;
            }
            Cmd::Edit => self.open_editor(),
            Cmd::Reload => {
                self.media.clear();
                self.markdown_fences.clear();
                self.fence_errors.clear();
                self.reload(true);
                self.info("reloaded");
            }
            Cmd::Back => self.back(),
            Cmd::Forward => self.forward(),
            Cmd::Help => self.help = !self.help,
            Cmd::Escape => {
                self.focus_idx = None;
                self.selection = None;
                if let Some(s) = &mut self.search {
                    s.current = None;
                }
                self.search = None;
                if self.pane == Pane::Toc {
                    self.pane = Pane::Preview;
                }
            }
            Cmd::Quit => self.ex("q"),
            Cmd::FocusPane(c) => self.focus_pane(c),
            Cmd::Ex(s) => self.ex(&s),
        }
    }

    pub fn focus_pane(&mut self, c: char) {
        let order: Vec<Pane> = [
            self.show_toc.then_some(Pane::Toc),
            self.editor.is_some().then_some(Pane::Editor),
            Some(Pane::Preview),
        ]
        .into_iter()
        .flatten()
        .collect();
        let i = order.iter().position(|p| *p == self.pane).unwrap_or(0);
        self.pane = match c {
            'h' => order[i.saturating_sub(1)],
            'l' => order[(i + 1).min(order.len() - 1)],
            _ => order[(i + 1) % order.len()],
        };
        if self.pane == Pane::Toc {
            self.sync_toc_selection();
        }
        self.dirty = true;
    }

    pub fn sync_toc_selection(&mut self) {
        let row = self.scroll + 1;
        self.toc_selected = self
            .layout
            .headings
            .iter()
            .rposition(|(_, r)| *r <= row)
            .unwrap_or(0);
    }

    pub fn toc_jump(&mut self, i: usize) {
        if let Some((_, row)) = self.layout.headings.get(i) {
            let row = *row;
            self.toc_selected = i;
            self.jump_to_row(row);
            if let Some(ed) = &mut self.editor {
                let line = self.layout.line_for_row(row);
                ed.set_cursor_row(line.saturating_sub(1));
                self.last_cursor_row = line.saturating_sub(1);
            }
        }
    }

    // ------------------------------------------------------------------ search

    pub fn update_search(&mut self, query: &str, origin: usize) {
        let matches = search(&self.layout, query);
        let current = matches.iter().position(|m| m.row >= origin);
        let current = current.or((!matches.is_empty()).then_some(0));
        if let Some(c) = current {
            let row = matches[c].row;
            if row < self.scroll || row >= self.scroll + self.view.height as usize {
                self.scroll_to(row.saturating_sub(self.view.height as usize / 3));
            }
        } else {
            self.scroll_to(origin);
        }
        self.search = Some(SearchState { query: query.to_string(), matches, current });
        self.dirty = true;
    }

    fn search_step(&mut self, forward: bool) {
        let height = self.view.height as usize;
        let Some(s) = &mut self.search else {
            if let Some(q) = self.cmd_history.iter().rev().find(|h| h.starts_with('/')).cloned() {
                self.update_search(&q[1..], self.scroll);
            }
            return;
        };
        let n = s.matches.len();
        if n == 0 {
            let q = s.query.clone();
            self.warn(format!("pattern not found: {q}"));
            return;
        }
        let next = match s.current {
            Some(c) => if forward { (c + 1) % n } else { (c + n - 1) % n },
            None => 0,
        };
        s.current = Some(next);
        let row = s.matches[next].row;
        if row < self.scroll || row >= self.scroll + height {
            self.scroll_to(row.saturating_sub(height / 3));
        }
        self.dirty = true;
    }

    pub fn finish_search(&mut self, query: String) {
        if !query.is_empty() {
            self.cmd_history.push(format!("/{query}"));
        }
        if self.search.as_ref().is_some_and(|s| s.matches.is_empty()) {
            self.warn(format!("pattern not found: {query}"));
        }
    }

    // ------------------------------------------------------------------ command line

    pub fn command_candidates(&self, text: &str) -> Vec<String> {
        let (cmd, rest) = text.split_once(' ').unwrap_or((text, ""));
        if text.contains(' ') {
            // argument completion
            return match cmd {
                "theme" => ["dark", "light"].iter().filter(|t| t.starts_with(rest)).map(|t| format!("theme {t}")).collect(),
                "set" => ["number", "nonumber", "relativenumber", "norelativenumber", "images", "noimages", "toc", "notoc", "maxwidth=", "mermaid=auto", "mermaid=text", "mermaid=image"]
                    .iter()
                    .filter(|t| t.starts_with(rest))
                    .map(|t| format!("set {t}"))
                    .collect(),
                "e" | "edit" | "open" => {
                    let (dir_part, prefix) = match rest.rfind('/') {
                        Some(i) => (&rest[..=i], &rest[i + 1..]),
                        None => ("", rest),
                    };
                    let mut v: Vec<String> = std::fs::read_dir(self.base_dir().join(dir_part))
                        .map(|rd| {
                            rd.flatten()
                                .filter_map(|e| {
                                    let name = e.file_name().to_string_lossy().into_owned();
                                    let dir = e.file_type().ok()?.is_dir();
                                    (name.starts_with(prefix) && !name.starts_with('.'))
                                        .then(|| format!("{cmd} {dir_part}{name}{}", if dir { "/" } else { "" }))
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    v.sort();
                    v
                }
                _ => Vec::new(),
            };
        }
        let mut v: Vec<String> = BUILTIN_COMMANDS
            .iter()
            .map(|s| s.to_string())
            .chain(self.plugins.as_ref().map(|p| p.commands()).unwrap_or_default())
            .filter(|c| c.starts_with(text))
            .collect();
        v.sort_by_key(|c| (c.chars().next().is_some_and(|ch| ch.is_lowercase()), c.clone()));
        v.dedup();
        v
    }

    pub fn history_step(&mut self, up: bool) -> Option<String> {
        let cmds: Vec<&String> = self.cmd_history.iter().filter(|h| !h.starts_with('/')).collect();
        if cmds.is_empty() {
            return None;
        }
        let i = match (self.cmd_hist_idx, up) {
            (None, true) => cmds.len() - 1,
            (None, false) => return None,
            (Some(i), true) => i.saturating_sub(1),
            (Some(i), false) => {
                if i + 1 >= cmds.len() {
                    self.cmd_hist_idx = None;
                    return Some(String::new());
                }
                i + 1
            }
        };
        self.cmd_hist_idx = Some(i);
        Some(cmds[i].clone())
    }

    /// Execute an ex command line (without the leading `:`).
    pub fn ex(&mut self, line: &str) {
        let line = line.trim();
        if line.is_empty() {
            return;
        }
        if self.cmd_history.last().map(String::as_str) != Some(line) {
            self.cmd_history.push(line.to_string());
        }
        if let Ok(n) = line.parse::<usize>() {
            // :42 → source line 42
            let row = self.layout.row_for_line(n);
            self.jump_to_row(row);
            if let Some(ed) = &mut self.editor {
                ed.set_cursor_row(n.saturating_sub(1));
            }
            return;
        }
        let (cmd, arg) = match line.split_once(char::is_whitespace) {
            Some((c, a)) => (c, a.trim()),
            None => (line, ""),
        };
        let dirty = self.editor.as_ref().is_some_and(|e| e.is_dirty());
        match cmd {
            "q" | "quit" => {
                if self.editor.is_some() && self.pane == Pane::Editor {
                    self.close_editor(false);
                } else if dirty {
                    self.warn("unsaved changes (:w to save, :q! to discard)");
                } else {
                    self.quit = true;
                }
            }
            "q!" | "qa!" => self.quit = true,
            "qa" | "qall" => {
                if dirty {
                    self.warn("unsaved changes (:wq or :qa!)");
                } else {
                    self.quit = true;
                }
            }
            "w" | "write" => {
                if !arg.is_empty() {
                    self.path = Some(PathBuf::from(arg));
                    self.stdin_mode = false;
                    self.watch();
                }
                self.save();
            }
            "wq" | "x" | "wqa" | "xa" => {
                if self.save() {
                    self.quit = true;
                }
            }
            "e" | "edit" | "open" if !arg.is_empty() => {
                let p = PathBuf::from(arg);
                let p = if p.is_absolute() { p } else { self.base_dir().join(p) };
                self.open_file(p, true);
            }
            "e" | "edit" | "vsplit" | "vs" => self.open_editor(),
            "e!" => {
                if let Some(ed) = &mut self.editor {
                    ed.mark_saved();
                }
                self.reload(true);
            }
            "close" | "clo" => {
                self.close_editor(false);
            }
            "close!" => {
                self.close_editor(true);
            }
            "toc" => self.run(Cmd::ToggleToc),
            "reload" => self.run(Cmd::Reload),
            "help" | "h" => self.help = true,
            "noh" | "nohlsearch" => self.search = None,
            "fold" => self.run(Cmd::FoldAll),
            "unfold" => self.run(Cmd::UnfoldAll),
            "top" => self.run(Cmd::Top),
            "bottom" => self.run(Cmd::Bottom),
            "copy" => {
                let s = self.source.clone();
                let via = crate::util::copy_to_clipboard(&s);
                self.info(format!("copied document source ({via})"));
            }
            "theme" => match Theme::by_name(arg) {
                Some(t) => {
                    self.theme = t;
                    self.media.set_background(self.theme.bg);
                    self.media.config.palette = mdtui_render::diagram::Palette::from_theme(&self.theme);
                    self.media.clear();
                    self.media.config.mermaid.theme = self.cfg.mermaid_theme(self.theme.dark);
                    self.rebuild();
                }
                None => self.warn("themes: dark, light"),
            },
            "set" => self.set_option(arg),
            "plugins" => {
                let msg = match &self.plugins {
                    Some(p) if !p.loaded().is_empty() => format!(
                        "plugins ({}): {}{}",
                        if p.trusted() { "trusted" } else { "sandboxed" },
                        p.loaded().join(", "),
                        if p.commands().is_empty() { String::new() } else { format!(" · commands: {}", p.commands().join(", ")) }
                    ),
                    _ => format!("no plugins in {}", self.cfg.plugin_dir().display()),
                };
                self.info(msg);
            }
            "lsp" => {
                let msg = match &self.lsp {
                    Some(l) if l.is_ready() => format!("{}: running", l.name),
                    Some(l) => format!("{}: starting", l.name),
                    None => format!("no language server ({} not found or editor closed)", self.cfg.editor.lsp.first().cloned().unwrap_or_default()),
                };
                self.info(msg);
            }
            c if c.chars().next().is_some_and(|ch| ch.is_uppercase()) => self.plugin_command(c, arg),
            other => self.error(format!("not an editor command: {other}")),
        }
        self.dirty = true;
    }

    fn set_option(&mut self, arg: &str) {
        match arg {
            "number" | "nu" => self.cfg.code_line_numbers = true,
            "nonumber" | "nonu" => self.cfg.code_line_numbers = false,
            "relativenumber" | "rnu" => {
                self.cfg.editor.relative_numbers = true;
                if let Some(e) = &mut self.editor {
                    e.relative_numbers = true;
                }
            }
            "norelativenumber" | "nornu" => {
                self.cfg.editor.relative_numbers = false;
                if let Some(e) = &mut self.editor {
                    e.relative_numbers = false;
                }
            }
            "images" => {
                self.media.config.images = true;
                self.media.clear();
                self.rebuild();
            }
            "noimages" => {
                self.media.config.images = false;
                self.media.clear();
                self.rebuild();
            }
            "toc" => self.show_toc = true,
            "notoc" => self.show_toc = false,
            a if a.starts_with("mermaid=") => match mdtui_render::media::Renderer::parse(&a["mermaid=".len()..]) {
                Some(r) => {
                    self.media.config.renderer = r;
                    self.renderer_override.clear();
                    self.media.clear();
                    self.rebuild();
                }
                None => self.warn("mermaid=auto|text|image"),
            },
            a if a.starts_with("maxwidth=") => match a["maxwidth=".len()..].parse() {
                Ok(n) => self.cfg.max_width = n,
                Err(_) => self.warn("maxwidth needs a number"),
            },
            other => self.warn(format!("unknown option: {other}")),
        }
        self.invalidate_layout();
    }

    fn plugin_command(&mut self, name: &str, args: &str) {
        let Some(p) = &self.plugins else {
            self.error(format!("not an editor command: {name}"));
            return;
        };
        if !p.commands().iter().any(|c| c == name) {
            self.error(format!("not an editor command: {name}"));
            return;
        }
        let text = self.editor.as_ref().map(|e| e.text().to_string()).unwrap_or_else(|| self.source.clone());
        let path = self.path.as_ref().map(|p| p.display().to_string()).unwrap_or_default();
        let r = p.run_command(name, args, &path, &text);
        self.drain_plugin_notes();
        match r {
            Ok(Some(new)) => {
                if let Some(ed) = &mut self.editor {
                    ed.replace_text(&new);
                    self.editor_changed();
                } else if self.write_file(&new) {
                    self.source = new;
                    self.rebuild();
                }
            }
            Ok(None) => {}
            Err(e) => self.error(e),
        }
    }

    // ------------------------------------------------------------------ mouse helpers

    /// Document coordinates for a screen position inside the preview.
    pub fn doc_pos(&self, x: u16, y: u16) -> Option<(usize, u16)> {
        let v = self.view;
        if x < v.x || x >= v.right() || y < v.y || y >= v.bottom() {
            return None;
        }
        let row = self.scroll + (y - v.y) as usize;
        let col = (x - v.x).checked_sub(self.x_offset)?;
        Some((row, col))
    }

    pub fn preview_press(&mut self, x: u16, y: u16) {
        self.pane = Pane::Preview;
        self.sel_anchor = self.doc_pos(x, y);
        self.sel_moved = false;
        self.selection = None;
        self.dirty = true;
    }

    pub fn preview_drag(&mut self, x: u16, y: u16) {
        let Some(anchor) = self.sel_anchor else { return };
        // auto-scroll when dragging past the edges
        if y < self.view.y {
            self.scroll_to(self.scroll.saturating_sub(1));
        } else if y >= self.view.bottom() {
            self.scroll_to(self.scroll + 1);
        }
        let y = y.clamp(self.view.y, self.view.bottom().saturating_sub(1));
        let x = x.clamp(self.view.x, self.view.right().saturating_sub(1));
        if let Some(p) = self.doc_pos(x, y).or(Some((self.scroll + (y - self.view.y) as usize, 0))) {
            if p != anchor {
                self.sel_moved = true;
                self.selection = Some(Selection::normalized(anchor, (p.0, p.1 + 1)));
                self.dirty = true;
            }
        }
    }

    pub fn preview_release(&mut self, x: u16, y: u16) {
        let anchor = self.sel_anchor.take();
        if self.sel_moved {
            if let Some(sel) = self.selection {
                let t = sel.text(&self.layout);
                if !t.is_empty() {
                    let via = crate::util::copy_to_clipboard(&t);
                    self.info(format!("copied {} chars ({via})", t.chars().count()));
                }
            }
            return;
        }
        let Some((row, col)) = anchor.or(self.doc_pos(x, y)) else { return };
        if let Some(hit) = self.layout.hits.at(row, col).cloned() {
            // focus what was clicked so Enter / y work on it too
            self.focus_idx = self.layout.hits.focusables().iter().position(|h| h.action == hit.action);
            self.activate(hit.action);
        }
    }
}

/// Replace custom blocks whose plugin returned markdown with the parsed blocks.
fn splice_markdown(blocks: &mut Vec<Block>, map: &HashMap<String, Vec<Block>>) {
    let mut i = 0;
    while i < blocks.len() {
        let replacement = match &blocks[i] {
            Block::Custom { handler, content, .. } => map.get(&custom_key(handler, content)).cloned(),
            _ => None,
        };
        if let Some(r) = replacement {
            let n = r.len();
            blocks.splice(i..=i, r);
            i += n;
            continue;
        }
        match &mut blocks[i] {
            Block::Quote { blocks: b, .. } | Block::Alert { blocks: b, .. } | Block::FootnoteDef { blocks: b, .. } => {
                splice_markdown(b, map)
            }
            Block::List { items, .. } => items.iter_mut().for_each(|it| splice_markdown(&mut it.blocks, map)),
            _ => {}
        }
        i += 1;
    }
}

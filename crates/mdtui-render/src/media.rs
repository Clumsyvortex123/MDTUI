//! Asynchronous image / diagram store.
//!
//! Loading, decoding, mermaid rendering and protocol encoding run on worker threads; results
//! come back over a channel and are picked up by [`MediaStore::poll`]. The store implements
//! [`MediaProvider`] so layout can ask how big each image will be.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

use image::DynamicImage;
use mdtui_core::layout::{image_key, MediaInfo, MediaPlacement, MediaProvider};
use ratatui::layout::Size;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui_image::picker::Picker;
use ratatui_image::protocol::Protocol;
use ratatui_image::sliced::SlicedProtocol;
use ratatui_image::{FilterType, Resize};

use crate::diagram::{self, Palette};
use crate::fetch;
use crate::mermaid::{self, MermaidConfig, MermaidOutput};

/// How mermaid blocks are drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Renderer {
    /// Text for diagram types we can draw natively, `mmdc` images for the rest.
    Auto,
    /// Always text (falls back to mermaid-ascii, then source).
    Text,
    /// Always `mmdc` images (falls back to text).
    Image,
}

impl Renderer {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "auto" => Renderer::Auto,
            "text" | "txt" => Renderer::Text,
            "image" | "img" => Renderer::Image,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone)]
pub struct MediaConfig {
    pub images: bool,
    /// Maximum height of an inline image, in rows.
    pub max_rows: u16,
    pub mermaid: MermaidConfig,
    pub renderer: Renderer,
    pub palette: Palette,
}

impl Default for MediaConfig {
    fn default() -> Self {
        MediaConfig {
            images: true,
            max_rows: 24,
            mermaid: MermaidConfig::default(),
            renderer: Renderer::Auto,
            palette: Palette::default(),
        }
    }
}

enum Entry {
    Loading,
    Image {
        img: Arc<DynamicImage>,
        protos: HashMap<(u16, u16), ProtoState>,
    },
    Lines(Vec<Line<'static>>),
    /// Native text diagram, with an optional narrower variant.
    Text { lines: Vec<Line<'static>>, narrow: Option<Vec<Line<'static>>> },
    Failed(String),
}

enum ProtoState {
    Encoding,
    Ready(SlicedProtocol),
    Failed,
}

enum Msg {
    Loaded(String, Result<DynamicImage, String>),
    Lines(String, Vec<Line<'static>>),
    Encoded(String, (u16, u16), Result<SlicedProtocol, String>),
}

#[derive(Debug, Clone, Copy)]
pub struct FsGeometry {
    pub max_pan: (i32, i32),
    pub px_per_cell: (f32, f32),
}

/// Cached fullscreen rendering: key, area, zoom, pan → protocol.
type FullscreenCache = (String, (u16, u16), u32, (i32, i32), Protocol);

pub struct MediaStore {
    picker: Picker,
    pub config: MediaConfig,
    base_dir: PathBuf,
    entries: HashMap<String, Entry>,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    fullscreen: Option<FullscreenCache>,
    /// Keys layout asked about that we have never seen (so the app can request them).
    missing: RefCell<Vec<String>>,
}

impl MediaStore {
    pub fn new(picker: Picker, config: MediaConfig, base_dir: PathBuf) -> Self {
        let (tx, rx) = channel();
        MediaStore {
            picker,
            config,
            base_dir,
            entries: HashMap::new(),
            tx,
            rx,
            fullscreen: None,
            missing: RefCell::new(Vec::new()),
        }
    }

    /// Image keys that layout asked for but nobody requested yet.
    pub fn take_missing(&self) -> Vec<String> {
        std::mem::take(&mut *self.missing.borrow_mut())
    }

    pub fn picker(&self) -> &Picker {
        &self.picker
    }

    pub fn protocol_name(&self) -> String {
        format!("{:?}", self.picker.protocol_type()).to_lowercase()
    }

    pub fn set_base_dir(&mut self, dir: PathBuf) {
        if dir != self.base_dir {
            self.base_dir = dir;
            // relative paths may now point elsewhere
            self.entries.retain(|k, _| !k.starts_with("img:") || k.contains("://"));
        }
    }

    pub fn set_background(&mut self, c: Color) {
        if let Color::Rgb(r, g, b) = c {
            self.picker.set_background_color(Some(image::Rgba([r, g, b, 255])));
        }
    }

    /// Forget everything (e.g. after a theme change).
    pub fn clear(&mut self) {
        self.entries.clear();
        self.fullscreen = None;
    }

    /// Forget one entry (e.g. to re-render a diagram).
    pub fn forget(&mut self, key: &str) {
        self.entries.remove(key);
    }

    /// Number of loads / renders still in flight.
    pub fn pending_count(&self) -> usize {
        self.entries.values().filter(|e| matches!(e, Entry::Loading)).count()
    }

    pub fn contains(&self, key: &str) -> bool {
        self.entries.contains_key(key)
    }

    /// Start loading an image if we haven't yet.
    pub fn request_image(&mut self, url: &str) {
        let key = image_key(url);
        if self.entries.contains_key(&key) {
            return;
        }
        if !self.config.images {
            self.entries.insert(key, Entry::Failed("images disabled".into()));
            return;
        }
        self.entries.insert(key.clone(), Entry::Loading);
        let tx = self.tx.clone();
        let url = url.to_string();
        let base = self.base_dir.clone();
        std::thread::spawn(move || {
            let r = fetch::load_bytes(&url, &base).and_then(|b| fetch::decode(&b, 1600));
            let _ = tx.send(Msg::Loaded(key, r));
        });
    }

    /// Start rendering a mermaid block. `mode` overrides the configured renderer.
    pub fn request_mermaid(&mut self, key: &str, src: &str, mode: Option<Renderer>) {
        if self.entries.contains_key(key) {
            return;
        }
        let mode = mode.unwrap_or(self.config.renderer);
        let text_ok = diagram::supported(src);
        let want_text = match mode {
            Renderer::Text => true,
            Renderer::Auto => text_ok || !self.config.images,
            Renderer::Image => !self.config.images,
        };
        if want_text && text_ok {
            // native rendering is fast: do it right here
            match diagram::render(src, &self.config.palette) {
                Ok(d) => {
                    self.entries.insert(key.to_string(), Entry::Text { lines: d.lines, narrow: d.narrow });
                    return;
                }
                Err(e) if mode == Renderer::Text => {
                    self.entries.insert(key.to_string(), Entry::Failed(e));
                    return;
                }
                Err(_) => {} // try mmdc
            }
        }
        self.entries.insert(key.to_string(), Entry::Loading);
        let tx = self.tx.clone();
        let key = key.to_string();
        let src = src.to_string();
        let mut cfg = self.config.mermaid.clone();
        let palette = self.config.palette.clone();
        if mode == Renderer::Text || !self.config.images {
            // text was asked for (or images are off): only the ASCII tool may help
            cfg.mmdc = String::from("\0disabled");
        }
        std::thread::spawn(move || match mermaid::render(&src, &cfg) {
            Ok(MermaidOutput::Png(bytes)) => {
                let r = fetch::decode(&bytes, 1600);
                let _ = tx.send(Msg::Loaded(key, r));
            }
            Ok(MermaidOutput::Text(t)) => {
                let lines = t.lines().map(|l| Line::from(Span::raw(l.to_string()))).collect();
                let _ = tx.send(Msg::Lines(key, lines));
            }
            Err(e) => {
                // image route failed: native text is better than nothing
                match diagram::render(&src, &palette) {
                    Ok(d) => {
                        let _ = tx.send(Msg::Lines(key, d.lines));
                    }
                    Err(_) => {
                        let _ = tx.send(Msg::Loaded(key, Err(e)));
                    }
                }
            }
        });
    }

    /// Whether `key` is currently drawn as text (for the [img]/[txt] toggle).
    pub fn is_text(&self, key: &str) -> bool {
        matches!(self.entries.get(key), Some(Entry::Text { .. } | Entry::Lines(_)))
    }

    /// Text lines for `key` (full width), for the fullscreen pan view.
    pub fn text_lines(&self, key: &str) -> Option<&[Line<'static>]> {
        match self.entries.get(key) {
            Some(Entry::Text { lines, .. }) | Some(Entry::Lines(lines)) => Some(lines),
            _ => None,
        }
    }

    /// Set the result of a synchronous handler (Lua plugin) that produced text lines.
    pub fn set_lines(&mut self, key: &str, lines: Vec<Line<'static>>) {
        self.entries.insert(key.to_string(), Entry::Lines(lines));
    }

    /// Set the result of a handler that produced encoded image bytes.
    pub fn set_image_bytes(&mut self, key: &str, bytes: Vec<u8>) {
        match fetch::decode(&bytes, 1600) {
            Ok(img) => {
                self.entries.insert(
                    key.to_string(),
                    Entry::Image {
                        img: Arc::new(img),
                        protos: HashMap::new(),
                    },
                );
            }
            Err(e) => self.set_failed(key, e),
        }
    }

    pub fn set_failed(&mut self, key: &str, msg: String) {
        self.entries.insert(key.to_string(), Entry::Failed(msg));
    }

    /// Drain finished jobs. Returns `(relayout_needed, redraw_needed)`.
    pub fn poll(&mut self) -> (bool, bool) {
        let mut relayout = false;
        let mut redraw = false;
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Msg::Loaded(key, Ok(img)) => {
                    self.entries.insert(
                        key,
                        Entry::Image {
                            img: Arc::new(img),
                            protos: HashMap::new(),
                        },
                    );
                    relayout = true;
                }
                Msg::Loaded(key, Err(e)) => {
                    self.entries.insert(key, Entry::Failed(e));
                    relayout = true;
                }
                Msg::Lines(key, lines) => {
                    self.entries.insert(key, Entry::Lines(lines));
                    relayout = true;
                }
                Msg::Encoded(key, size, r) => {
                    if let Some(Entry::Image { protos, .. }) = self.entries.get_mut(&key) {
                        protos.insert(
                            size,
                            match r {
                                Ok(p) => ProtoState::Ready(p),
                                Err(_) => ProtoState::Failed,
                            },
                        );
                        redraw = true;
                    }
                }
            }
        }
        (relayout, redraw)
    }

    /// Cell size of an image fitted into `max_w` × `max_rows`.
    fn fit(&self, img: &DynamicImage, max_w: u16) -> (u16, u16) {
        let fs = self.picker.font_size();
        let (fw, fh) = (fs.width.max(1) as f64, fs.height.max(1) as f64);
        let cw = (img.width() as f64 / fw).ceil().max(1.0);
        let ch = (img.height() as f64 / fh).ceil().max(1.0);
        let s = 1f64
            .min(max_w.max(1) as f64 / cw)
            .min(self.config.max_rows.max(1) as f64 / ch);
        (((cw * s).floor().max(1.0)) as u16, ((ch * s).floor().max(1.0)) as u16)
    }

    /// Make sure every visible placement has (or is getting) an encoded protocol.
    pub fn ensure_protocols<'p>(&mut self, placements: impl Iterator<Item = &'p MediaPlacement>) {
        let mut seen = HashSet::new();
        for p in placements {
            if !seen.insert((p.key.clone(), p.width, p.height)) {
                continue;
            }
            let Some(Entry::Image { img, protos }) = self.entries.get_mut(&p.key) else {
                continue;
            };
            let size = (p.width, p.height);
            if protos.contains_key(&size) {
                continue;
            }
            protos.insert(size, ProtoState::Encoding);
            let tx = self.tx.clone();
            let picker = self.picker.clone();
            let img = img.clone();
            let key = p.key.clone();
            std::thread::spawn(move || {
                let r = SlicedProtocol::new_with_resize(
                    &picker,
                    (*img).clone(),
                    Size::new(size.0, size.1),
                    Resize::Fit(Some(FilterType::Triangle)),
                )
                .map_err(|e| e.to_string());
                let _ = tx.send(Msg::Encoded(key, size, r));
            });
        }
    }

    pub fn protocol(&self, key: &str, w: u16, h: u16) -> Option<&SlicedProtocol> {
        match self.entries.get(key) {
            Some(Entry::Image { protos, .. }) => match protos.get(&(w, h)) {
                Some(ProtoState::Ready(p)) => Some(p),
                _ => None,
            },
            _ => None,
        }
    }

    pub fn image_dims(&self, key: &str) -> Option<(u32, u32)> {
        match self.entries.get(key) {
            Some(Entry::Image { img, .. }) => Some((img.width(), img.height())),
            _ => None,
        }
    }

    /// Pan limits and mouse scale for a fullscreen image: pan is an offset in source pixels
    /// from the centre of the image, valid within ±`max_pan`; one screen cell is
    /// `px_per_cell` source pixels.
    pub fn fullscreen_geometry(&self, key: &str, area: (u16, u16), zoom: u32) -> Option<FsGeometry> {
        let Some(Entry::Image { img, .. }) = self.entries.get(key) else {
            return None;
        };
        let zoom = zoom.max(1);
        let (iw, ih) = (img.width(), img.height());
        let (cw, ch) = ((iw / zoom).max(1), (ih / zoom).max(1));
        let fs = self.picker.font_size();
        let (fw, fh) = (fs.width.max(1) as f32, fs.height.max(1) as f32);
        // the crop is fitted into the area keeping its aspect: source px per screen px
        let scale = (cw as f32 / (area.0.max(1) as f32 * fw)).max(ch as f32 / (area.1.max(1) as f32 * fh));
        Some(FsGeometry {
            max_pan: (((iw - cw) / 2) as i32, ((ih - ch) / 2) as i32),
            px_per_cell: (scale * fw, scale * fh),
        })
    }

    /// A protocol for showing `key` fullscreen in `area` with zoom (1 = fit) and pan (source
    /// pixels from the centre, see [`Self::fullscreen_geometry`]).
    /// Cached until any argument changes. Encoding happens synchronously (one image).
    pub fn fullscreen(&mut self, key: &str, area: (u16, u16), zoom: u32, pan: (i32, i32)) -> Option<&Protocol> {
        let cached = matches!(&self.fullscreen, Some((k, a, z, p, _)) if k == key && *a == area && *z == zoom && *p == pan);
        if !cached {
            let Some(Entry::Image { img, .. }) = self.entries.get(key) else {
                return None;
            };
            let zoom = zoom.max(1);
            let (iw, ih) = (img.width(), img.height());
            let (cw, ch) = (iw / zoom, ih / zoom);
            let fs = self.picker.font_size();
            let max_x = iw.saturating_sub(cw) as i64;
            let max_y = ih.saturating_sub(ch) as i64;
            let x = (max_x / 2 + pan.0 as i64).clamp(0, max_x) as u32;
            let y = (max_y / 2 + pan.1 as i64).clamp(0, max_y) as u32;
            let cropped = if zoom > 1 { img.crop_imm(x, y, cw.max(1), ch.max(1)) } else { (**img).clone() };
            // upscale small crops so they fill the screen
            let target_w = area.0 as u32 * fs.width as u32;
            let target_h = area.1 as u32 * fs.height as u32;
            let scaled = if cropped.width() < target_w && cropped.height() < target_h {
                cropped.resize(target_w, target_h, FilterType::Triangle)
            } else {
                cropped
            };
            let proto = self
                .picker
                .new_protocol(scaled, Size::new(area.0, area.1), Resize::Fit(Some(FilterType::Triangle)))
                .ok()?;
            self.fullscreen = Some((key.to_string(), area, zoom, pan, proto));
        }
        self.fullscreen.as_ref().map(|f| &f.4)
    }
}

impl MediaProvider for MediaStore {
    fn media(&self, key: &str, max_width: u16) -> MediaInfo {
        match self.entries.get(key) {
            None => {
                self.missing.borrow_mut().push(key.to_string());
                MediaInfo::Pending
            }
            Some(Entry::Loading) => MediaInfo::Pending,
            Some(Entry::Image { img, .. }) => {
                let (width, height) = self.fit(img, max_width);
                MediaInfo::Ready { width, height }
            }
            Some(Entry::Lines(l)) => MediaInfo::Lines(l.clone()),
            Some(Entry::Text { lines, narrow }) => {
                let fits = diagram::width(lines) <= max_width as usize;
                match narrow {
                    Some(n) if !fits => MediaInfo::Lines(n.clone()),
                    _ => MediaInfo::Lines(lines.clone()),
                }
            }
            Some(Entry::Failed(e)) => MediaInfo::Failed(e.clone()),
        }
    }
}

/// Placeholder style for images that are still encoding.
pub fn placeholder_style() -> Style {
    Style::default().fg(Color::DarkGray)
}

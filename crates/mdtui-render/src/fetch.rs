//! Loading image bytes from local paths or URLs, and decoding (including SVG).

use std::path::Path;
use std::sync::OnceLock;

use image::DynamicImage;

use crate::{cache_dir, hash_str};

/// Read the bytes behind an image reference, resolving relative paths against `base_dir`.
/// Remote images are cached in `~/.cache/mdtui/img`.
pub fn load_bytes(url: &str, base_dir: &Path) -> Result<Vec<u8>, String> {
    if url.starts_with("http://") || url.starts_with("https://") {
        let cached = cache_dir("img").join(hash_str(url));
        if let Ok(b) = std::fs::read(&cached) {
            return Ok(b);
        }
        let mut resp = ureq::get(url)
            .header("User-Agent", "mdtui (terminal markdown viewer)")
            .call()
            .map_err(|e| format!("fetch failed: {e}"))?;
        let bytes = resp
            .body_mut()
            .with_config()
            .limit(64 * 1024 * 1024)
            .read_to_vec()
            .map_err(|e| format!("fetch failed: {e}"))?;
        let _ = std::fs::write(&cached, &bytes);
        return Ok(bytes);
    }
    if let Some(data) = url.strip_prefix("data:") {
        let (meta, payload) = data.split_once(',').ok_or("bad data URI")?;
        if !meta.ends_with(";base64") {
            return Ok(percent_decode(payload).into_bytes());
        }
        return base64_decode(payload).ok_or_else(|| "bad base64 data URI".into());
    }
    let path = url.strip_prefix("file://").unwrap_or(url);
    let path = percent_decode(path);
    let p = Path::new(&path);
    let p = if p.is_absolute() { p.to_path_buf() } else { base_dir.join(p) };
    std::fs::read(&p).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => "not found".to_string(),
        _ => e.to_string(),
    })
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn base64_decode(s: &str) -> Option<Vec<u8>> {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = Vec::new();
    let mut buf = 0u32;
    let mut bits = 0;
    for c in s.bytes().filter(|c| !c.is_ascii_whitespace() && *c != b'=') {
        let v = T.iter().position(|&t| t == c)? as u32;
        buf = (buf << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
        }
    }
    Some(out)
}

fn is_svg(bytes: &[u8]) -> bool {
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(512)]).to_ascii_lowercase();
    head.contains("<svg") || (head.trim_start().starts_with("<?xml") && head.contains("svg"))
}

fn fontdb() -> std::sync::Arc<resvg::usvg::fontdb::Database> {
    static DB: OnceLock<std::sync::Arc<resvg::usvg::fontdb::Database>> = OnceLock::new();
    DB.get_or_init(|| {
        let mut db = resvg::usvg::fontdb::Database::new();
        db.load_system_fonts();
        std::sync::Arc::new(db)
    })
    .clone()
}

/// Decode raster or SVG bytes. SVGs are rasterised so their longest side is ~`svg_px` pixels.
pub fn decode(bytes: &[u8], svg_px: u32) -> Result<DynamicImage, String> {
    if is_svg(bytes) {
        let opt = resvg::usvg::Options {
            fontdb: fontdb(),
            ..Default::default()
        };
        let tree = resvg::usvg::Tree::from_data(bytes, &opt).map_err(|e| format!("svg: {e}"))?;
        let size = tree.size();
        let longest = size.width().max(size.height()).max(1.0);
        // keep small SVGs (badges) crisp but small; scale big ones down
        let scale = if longest < 200.0 { 2.0 } else { (svg_px as f32 / longest).min(2.0) };
        let w = (size.width() * scale).ceil().max(1.0) as u32;
        let h = (size.height() * scale).ceil().max(1.0) as u32;
        let mut pixmap = resvg::tiny_skia::Pixmap::new(w, h).ok_or("svg: empty size")?;
        resvg::render(
            &tree,
            resvg::tiny_skia::Transform::from_scale(scale, scale),
            &mut pixmap.as_mut(),
        );
        let img = image::RgbaImage::from_raw(w, h, pixmap.data().to_vec()).ok_or("svg: raster")?;
        // tiny-skia produces premultiplied alpha; un-premultiply for `image`
        let mut img = img;
        for p in img.pixels_mut() {
            let a = p[3] as u32;
            if a > 0 && a < 255 {
                for c in 0..3 {
                    p[c] = ((p[c] as u32 * 255) / a).min(255) as u8;
                }
            }
        }
        return Ok(DynamicImage::ImageRgba8(img));
    }
    image::load_from_memory(bytes).map_err(|e| format!("decode: {e}"))
}

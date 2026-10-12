//! Terminal rendering for mdtui: the document viewport widget, image loading/encoding and the
//! mermaid pipeline.

pub mod diagram;
pub mod fetch;
pub mod media;
pub mod mermaid;
pub mod viewport;

pub use media::{MediaConfig, MediaStore};
pub use viewport::{Selection, Viewport};

/// Look for an executable on `$PATH`.
pub fn find_in_path(name: &str) -> Option<std::path::PathBuf> {
    let p = std::path::Path::new(name);
    if p.components().count() > 1 {
        return p.is_file().then(|| p.to_path_buf());
    }
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|d| d.join(name))
            .find(|c| c.is_file())
    })
}

/// `~/.cache/mdtui/<sub>`, created on demand.
pub fn cache_dir(sub: &str) -> std::path::PathBuf {
    let base = directories::ProjectDirs::from("", "", "mdtui")
        .map(|d| d.cache_dir().to_path_buf())
        .unwrap_or_else(|| std::env::temp_dir().join("mdtui-cache"));
    let d = base.join(sub);
    let _ = std::fs::create_dir_all(&d);
    d
}

pub fn hash_str(s: &str) -> String {
    use std::hash::{DefaultHasher, Hash, Hasher};
    let mut h = DefaultHasher::new();
    s.hash(&mut h);
    format!("{:016x}", h.finish())
}

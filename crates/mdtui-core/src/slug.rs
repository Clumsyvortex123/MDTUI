//! GitHub-compatible heading anchors (`## Hello, World!` → `hello-world`).

use std::collections::HashMap;

pub fn slugify(text: &str) -> String {
    text.trim()
        .to_lowercase()
        .chars()
        .filter_map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                Some(c)
            } else if c == ' ' {
                Some('-')
            } else {
                None
            }
        })
        .collect()
}

/// Produces unique slugs the same way GitHub does: duplicates get `-1`, `-2`, … suffixes.
#[derive(Default)]
pub struct Slugger {
    seen: HashMap<String, usize>,
}

impl Slugger {
    pub fn slug(&mut self, text: &str) -> String {
        let base = slugify(text);
        let n = self.seen.entry(base.clone()).or_insert(0);
        let out = if *n == 0 { base.clone() } else { format!("{base}-{n}") };
        *n += 1;
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn github_style() {
        assert_eq!(slugify("Hello, World!"), "hello-world");
        assert_eq!(slugify("Install & Setup"), "install--setup");
        let mut s = Slugger::default();
        assert_eq!(s.slug("Usage"), "usage");
        assert_eq!(s.slug("Usage"), "usage-1");
    }
}

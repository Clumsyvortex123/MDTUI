//! Edits applied back to the markdown source (checkbox toggling).

/// Toggle the task checkbox whose symbol sits at (`line`, `col`), both 1-based, `col` in bytes.
/// Returns the new source, or `None` if the position doesn't hold a checkbox symbol.
pub fn toggle_task(src: &str, line: usize, col: usize) -> Option<String> {
    let mut offset = 0;
    for (i, l) in src.split_inclusive('\n').enumerate() {
        if i + 1 == line {
            let idx = offset + col - 1;
            let ch = src[idx..].chars().next()?;
            // sanity check: the symbol must be inside `[ ]`
            if idx == 0 || src.as_bytes()[idx - 1] != b'[' {
                return None;
            }
            let replacement = if ch == ' ' { "x" } else { " " };
            let mut out = String::with_capacity(src.len());
            out.push_str(&src[..idx]);
            out.push_str(replacement);
            out.push_str(&src[idx + ch.len_utf8()..]);
            return Some(out);
        }
        offset += l.len();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggles() {
        let src = "# t\n- [ ] a\n- [x] b\n";
        let s = toggle_task(src, 2, 4).unwrap();
        assert_eq!(s, "# t\n- [x] a\n- [x] b\n");
        let s = toggle_task(&s, 3, 4).unwrap();
        assert_eq!(s, "# t\n- [x] a\n- [ ] b\n");
        assert!(toggle_task(src, 1, 1).is_none());
    }
}

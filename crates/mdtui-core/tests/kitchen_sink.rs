//! Snapshot the kitchen-sink fixture at several widths, and check interactive hit targets.

use mdtui_core::hitmap::Action;
use mdtui_core::layout::{layout, LayoutOptions, NoMedia};
use mdtui_core::{parse, ParseOptions, Theme};

fn fixture() -> String {
    std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/kitchen-sink.md")).unwrap()
}

fn render(width: u16) -> (mdtui_core::Layout, String) {
    let mut opts = ParseOptions::default();
    opts.fence_handlers.insert("mermaid".into());
    let doc = parse(&fixture(), &opts);
    let l = layout(&doc, &LayoutOptions { width, ..Default::default() }, &Theme::dark(), &NoMedia);
    let text = (0..l.height()).map(|r| l.plain_line(r).trim_end().to_string()).collect::<Vec<_>>().join("\n");
    (l, text)
}

#[test]
fn snapshot_40() {
    insta::assert_snapshot!(render(40).1);
}

#[test]
fn snapshot_80() {
    insta::assert_snapshot!(render(80).1);
}

#[test]
fn snapshot_120() {
    insta::assert_snapshot!(render(120).1);
}

#[test]
fn every_line_fits_the_width() {
    use unicode_width::UnicodeWidthStr;
    for w in [40u16, 80, 120] {
        let (l, _) = render(w);
        for r in 0..l.height() {
            let line = l.plain_line(r);
            assert!(line.trim_end().width() <= w as usize, "width {w}, row {r}: {line:?}");
        }
    }
}

#[test]
fn clickable_targets() {
    let (l, text) = render(80);
    let src = fixture();
    // every task glyph is a ToggleTask pointing at a `[ ]` / `[x]` in the source
    for (row, line) in text.lines().enumerate() {
        if let Some(col) = line.find(['☐', '☑']).map(|b| line[..b].chars().count()) {
            let hit = l.hits.at(row, col as u16).expect("task hit");
            let Action::ToggleTask { line: sl, col: sc } = hit.action else { panic!("{:?}", hit.action) };
            let src_line = src.lines().nth(sl - 1).unwrap();
            assert_eq!(&src_line[sc - 2..sc - 1], "[", "{src_line}");
        }
    }
    // anchors from the contents list resolve
    for a in ["text", "tasks", "code", "tables", "alerts", "images", "diagrams", "math"] {
        assert!(l.anchors.contains_key(a), "missing anchor {a}");
    }
    assert!(l.hits.all().iter().any(|h| h.action == Action::OpenLink("#tasks".into())));
    assert!(l.hits.all().iter().any(|h| matches!(&h.action, Action::CopyCode(c) if c.contains("word_count"))));
    assert!(l.footnotes.contains_key("speed"));
}

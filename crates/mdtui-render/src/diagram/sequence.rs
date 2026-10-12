//! Mermaid sequence diagrams as text: participant boxes, lifelines, labelled arrows, notes and
//! loop / alt / opt frames.

use unicode_width::UnicodeWidthStr;

use super::canvas::{Canvas, Stroke};
use super::flowchart::content_lines;
use super::Palette;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Head {
    Arrow,
    Open,
    Cross,
    Async,
}

#[derive(Debug, Clone)]
enum NotePos {
    Left(usize),
    Right(usize),
    Over(usize, usize),
}

#[derive(Debug, Clone)]
enum Ev {
    Msg { from: usize, to: usize, text: String, dashed: bool, head: Head },
    Note { pos: NotePos, text: String },
    FrameStart(String),
    FrameElse(String),
    FrameEnd,
}

#[derive(Debug, Clone)]
struct Participant {
    id: String,
    label: String,
    actor: bool,
}

struct Seq {
    parts: Vec<Participant>,
    events: Vec<Ev>,
    autonumber: bool,
    title: Option<String>,
}

impl Seq {
    fn part(&mut self, id: &str) -> usize {
        let id = id.trim().trim_end_matches(['+', '-']).trim_start_matches(['+', '-']).trim();
        if let Some(i) = self.parts.iter().position(|p| p.id == id) {
            return i;
        }
        self.parts.push(Participant { id: id.into(), label: id.into(), actor: false });
        self.parts.len() - 1
    }
}

const ARROWS: &[(&str, bool, Head)] = &[
    ("-->>", true, Head::Arrow),
    ("->>", false, Head::Arrow),
    ("--x", true, Head::Cross),
    ("-x", false, Head::Cross),
    ("--)", true, Head::Async),
    ("-)", false, Head::Async),
    ("-->", true, Head::Open),
    ("->", false, Head::Open),
];

fn unbr(s: &str) -> String {
    s.replace("<br/>", " ").replace("<br>", " ").replace("<br />", " ").trim().to_string()
}

fn parse(src: &str) -> Result<Seq, String> {
    let lines = content_lines(src);
    let mut it = lines.iter();
    if it.next().map(|h| h.starts_with("sequenceDiagram")) != Some(true) {
        return Err("not a sequence diagram".into());
    }
    let mut s = Seq { parts: Vec::new(), events: Vec::new(), autonumber: false, title: None };
    for line in it {
        let lower = line.to_ascii_lowercase();
        let first = lower.split_whitespace().next().unwrap_or("");
        match first {
            "participant" | "actor" => {
                let rest = line[first.len()..].trim();
                let (id, label) = match rest.split_once(" as ") {
                    Some((i, l)) => (i.trim(), unbr(l)),
                    None => (rest, rest.to_string()),
                };
                let i = s.part(id);
                s.parts[i].label = label;
                s.parts[i].actor = first == "actor";
                continue;
            }
            "autonumber" => {
                s.autonumber = true;
                continue;
            }
            "title" => {
                s.title = Some(line[5..].trim_start_matches(':').trim().to_string());
                continue;
            }
            "loop" | "alt" | "opt" | "par" | "critical" | "break" | "rect" => {
                let label = line[first.len()..].trim();
                let label = if first == "rect" { String::new() } else { format!("{first} {label}") };
                s.events.push(Ev::FrameStart(label.trim().to_string()));
                continue;
            }
            "else" | "and" | "option" => {
                s.events.push(Ev::FrameElse(format!("{first} {}", line[first.len()..].trim()).trim().to_string()));
                continue;
            }
            "end" => {
                s.events.push(Ev::FrameEnd);
                continue;
            }
            "activate" | "deactivate" | "create" | "destroy" | "box" | "links" | "link" => continue,
            "note" => {
                let rest = line[4..].trim();
                let Some((place, text)) = rest.split_once(':') else { continue };
                let place = place.trim();
                let lp = place.to_ascii_lowercase();
                let pos = if let Some(who) = lp.strip_prefix("left of") {
                    NotePos::Left(s.part(&place[place.len() - who.len()..]))
                } else if let Some(who) = lp.strip_prefix("right of") {
                    NotePos::Right(s.part(&place[place.len() - who.len()..]))
                } else if lp.starts_with("over") {
                    let who = &place[4..];
                    let mut ids = who.split(',');
                    let a = s.part(ids.next().unwrap_or(""));
                    let b = ids.next().map(|b| s.part(b)).unwrap_or(a);
                    NotePos::Over(a.min(b), a.max(b))
                } else {
                    continue;
                };
                s.events.push(Ev::Note { pos, text: unbr(text) });
                continue;
            }
            _ => {}
        }
        // message: A->>B: text
        let (lhs, text) = match line.split_once(':') {
            Some((l, t)) => (l, unbr(t)),
            None => (line.as_str(), String::new()),
        };
        let found = ARROWS.iter().filter_map(|(tok, dashed, head)| lhs.find(tok).map(|p| (p, *tok, *dashed, *head))).min_by_key(|(p, tok, ..)| (*p, std::cmp::Reverse(tok.len())));
        if let Some((pos, tok, dashed, head)) = found {
            let from = s.part(&lhs[..pos]);
            let to = s.part(&lhs[pos + tok.len()..]);
            s.events.push(Ev::Msg { from, to, text, dashed, head });
        }
    }
    if s.parts.is_empty() {
        return Err("no participants".into());
    }
    Ok(s)
}

pub fn render(src: &str, p: &Palette) -> Result<Canvas, String> {
    let s = parse(src)?;
    let n = s.parts.len();
    let labels: Vec<String> = s
        .parts
        .iter()
        .map(|pt| if pt.actor { format!("☺ {}", pt.label) } else { pt.label.clone() })
        .collect();
    let bw: Vec<i64> = labels.iter().map(|l| l.width() as i64 + 4).collect();

    // message texts with autonumbering
    let mut k = 0;
    let texts: Vec<String> = s
        .events
        .iter()
        .map(|e| match e {
            Ev::Msg { text, .. } => {
                k += 1;
                if s.autonumber { format!("{k}. {text}") } else { text.clone() }
            }
            _ => String::new(),
        })
        .collect();

    // column spacing: gaps[i] = distance between centre i and i+1
    let mut gaps: Vec<i64> = (0..n.saturating_sub(1)).map(|i| bw[i] / 2 + bw[i + 1] / 2 + 3).collect();
    let mut left_margin = bw[0] / 2;
    let mut right_extra = bw[n - 1] / 2;
    let need = |gaps: &mut Vec<i64>, a: usize, b: usize, dist: i64| {
        if a >= b {
            return;
        }
        let have: i64 = gaps[a..b].iter().sum();
        if have < dist {
            gaps[b - 1] += dist - have;
        }
    };
    for (e, t) in s.events.iter().zip(&texts) {
        match e {
            Ev::Msg { from, to, .. } if from != to => {
                need(&mut gaps, *from.min(to), *from.max(to), t.width() as i64 + 4)
            }
            Ev::Msg { from, .. } => {
                let w = t.width() as i64 + 6;
                if *from + 1 < n {
                    need(&mut gaps, *from, from + 1, w);
                } else {
                    right_extra = right_extra.max(w);
                }
            }
            Ev::Note { pos, text } => {
                let w = text.width() as i64 + 4;
                match pos {
                    NotePos::Right(i) if i + 1 < n => need(&mut gaps, *i, i + 1, w + 2),
                    NotePos::Right(_) => right_extra = right_extra.max(w + 2),
                    NotePos::Left(0) => left_margin = left_margin.max(w + 2),
                    NotePos::Left(i) => need(&mut gaps, i - 1, *i, w + 2),
                    NotePos::Over(a, b) => {
                        if a == b {
                            left_margin = left_margin.max(w / 2 + 1);
                        } else {
                            need(&mut gaps, *a, *b, w - 2)
                        }
                    }
                }
            }
            _ => {}
        }
    }
    let mut centers = vec![left_margin + 1];
    for g in &gaps {
        centers.push(centers.last().unwrap() + g);
    }
    let width = centers[n - 1] + right_extra + 2;
    // nested frames are indented so their [end] lines can be told apart
    let mut depth = 0i64;
    let mut depths = Vec::new();
    for e in &s.events {
        match e {
            Ev::FrameStart(_) => {
                depths.push(depth);
                depth += 1;
            }
            Ev::FrameElse(_) => depths.push((depth - 1).max(0)),
            Ev::FrameEnd => {
                depth = (depth - 1).max(0);
                depths.push(depth);
            }
            _ => depths.push(0),
        }
    }

    // rows
    let mut rows = 0i64;
    let title_rows = if s.title.is_some() { 2 } else { 0 };
    rows += title_rows + 3; // header boxes
    let mut ev_rows = Vec::new();
    for (e, t) in s.events.iter().zip(&texts) {
        let h = match e {
            Ev::Msg { from, to, .. } if from == to => 3,
            Ev::Msg { .. } => if t.is_empty() { 1 } else { 2 },
            Ev::Note { .. } => 3,
            _ => 1,
        };
        ev_rows.push(rows);
        rows += h;
    }
    rows += 1;
    let footer = rows;
    rows += 3;

    let mut cv = Canvas::new(width as usize + 1, rows as usize);
    if let Some(t) = &s.title {
        cv.text(1, 0, t, p.text);
    }
    // lifelines
    for &c in &centers {
        cv.vline(c, title_rows + 3, footer - 1, Stroke::Dashed, p.edge);
    }
    // participant boxes, top and bottom
    for (i, l) in labels.iter().enumerate() {
        for y in [title_rows, footer] {
            let x = centers[i] - bw[i] / 2;
            let inner = "─".repeat((bw[i] - 2) as usize);
            cv.text(x, y, &format!("┌{inner}┐"), p.node);
            cv.text(x, y + 1, &format!("│ {l} │"), p.node);
            cv.text(x + 2, y + 1, l, p.text);
            cv.text(x, y + 2, &format!("└{inner}┘"), p.node);
        }
    }
    for (((e, t), &y), &d) in s.events.iter().zip(&texts).zip(&ev_rows).zip(&depths) {
        let frame_x0 = d * 2;
        let frame_x1 = width - 1 - d * 2;
        match e {
            Ev::Msg { from, to, dashed, head, .. } => {
                let stroke = if *dashed { Stroke::Dashed } else { Stroke::Light };
                let (a, b) = (centers[*from], centers[*to]);
                if from == to {
                    cv.text(a + 2, y, t, p.label);
                    cv.polyline(&[(a, y + 1), (a + 3, y + 1), (a + 3, y + 2), (a + 1, y + 2)], stroke, p.edge);
                    cv.glyph(a + 1, y + 2, '◀', p.edge);
                    continue;
                }
                let line_y = if t.is_empty() { y } else { y + 1 };
                if !t.is_empty() {
                    let mid = (a + b) / 2;
                    cv.text(mid - t.width() as i64 / 2, y, t, p.label);
                }
                let dir = if b > a { 1 } else { -1 };
                cv.hline(a + dir, b - dir, line_y, stroke, p.edge);
                let g = match (head, dir > 0) {
                    (Head::Arrow, true) => '▶',
                    (Head::Arrow, false) => '◀',
                    (Head::Open, true) => '>',
                    (Head::Open, false) => '<',
                    (Head::Cross, _) => '✕',
                    (Head::Async, true) => '⟩',
                    (Head::Async, false) => '⟨',
                };
                cv.glyph(b - dir, line_y, g, p.edge);
            }
            Ev::Note { pos, text } => {
                let w = text.width() as i64 + 4;
                let x = match pos {
                    NotePos::Left(i) => centers[*i] - 2 - w,
                    NotePos::Right(i) => centers[*i] + 2,
                    NotePos::Over(a, b) => (centers[*a] + centers[*b]) / 2 - w / 2,
                };
                let x = x.max(0);
                let w = if let NotePos::Over(a, b) = pos {
                    if a != b { w.max(centers[*b] - centers[*a] + 4) } else { w }
                } else {
                    w
                };
                let x = if let NotePos::Over(a, b) = pos { if a != b { centers[*a] - 2 } else { x } } else { x };
                let inner = "─".repeat((w - 2) as usize);
                cv.text(x, y, &format!("┌{inner}┐"), p.label);
                cv.text(x, y + 1, &format!("│{}│", " ".repeat((w - 2) as usize)), p.label);
                cv.text(x + 2, y + 1, text, p.text);
                cv.text(x, y + 2, &format!("└{inner}┘"), p.label);
            }
            Ev::FrameStart(label) => {
                cv.hline(frame_x0, frame_x1, y, Stroke::Dashed, p.label);
                if !label.is_empty() {
                    cv.text(frame_x0 + 1, y, &format!("[{label}]"), p.label);
                }
            }
            Ev::FrameElse(label) => {
                cv.hline(frame_x0, frame_x1, y, Stroke::Dashed, p.label);
                cv.text(frame_x0 + 1, y, &format!("[{label}]"), p.label);
            }
            Ev::FrameEnd => {
                cv.hline(frame_x0, frame_x1, y, Stroke::Dashed, p.label);
                cv.text(frame_x0 + 1, y, "[end]", p.label);
            }
        }
    }
    Ok(cv)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_messages_and_notes() {
        let src = "sequenceDiagram\n participant U as User\n participant M as mdtui\n U->>M: open README.md\n M-->>U: rendered preview\n Note over U,M: live reload\n loop every save\n M->>M: re-render\n end";
        let cv = render(src, &Palette::default()).unwrap();
        let text: Vec<String> = cv
            .to_lines()
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect();
        let all = text.join("\n");
        for w in ["User", "mdtui", "open README.md", "rendered preview", "live reload", "[loop every save]", "re-render", "▶", "◀"] {
            assert!(all.contains(w), "{w} missing:\n{all}");
        }
    }
}

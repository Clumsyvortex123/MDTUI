//! Native text rendering of mermaid diagrams: crisp in every terminal (no graphics protocol
//! needed), themed, searchable and copyable.
//!
//! Supported: flowchart / graph, sequenceDiagram, stateDiagram(-v2), pie. Anything else falls
//! back to the image pipeline (`mmdc`).

pub mod canvas;
pub mod flowchart;
pub mod sequence;

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

#[derive(Debug, Clone)]
pub struct Palette {
    pub node: Style,
    pub text: Style,
    pub edge: Style,
    pub label: Style,
    pub series: [Color; 6],
}

impl Default for Palette {
    fn default() -> Self {
        Palette {
            node: Style::default().fg(Color::Rgb(86, 156, 214)),
            text: Style::default().fg(Color::Rgb(212, 212, 212)),
            edge: Style::default().fg(Color::Rgb(128, 128, 128)),
            label: Style::default().fg(Color::Rgb(206, 145, 120)),
            series: [
                Color::Rgb(86, 156, 214),
                Color::Rgb(78, 201, 176),
                Color::Rgb(220, 220, 170),
                Color::Rgb(197, 134, 192),
                Color::Rgb(206, 145, 120),
                Color::Rgb(244, 71, 71),
            ],
        }
    }
}

impl Palette {
    pub fn from_theme(t: &mdtui_core::Theme) -> Self {
        Palette {
            node: Style::default().fg(t.headings[0]),
            text: Style::default().fg(t.fg),
            edge: Style::default().fg(t.muted),
            label: Style::default().fg(t.inline_code_fg),
            series: [t.headings[0], t.headings[1], t.headings[2], t.headings[3], t.headings[4], t.error],
        }
    }
}

pub struct TextDiagram {
    pub lines: Vec<Line<'static>>,
    /// A narrower top-down variant for left-right diagrams that don't fit.
    pub narrow: Option<Vec<Line<'static>>>,
}

fn kind(src: &str) -> String {
    flowchart::content_lines(src)
        .first()
        .and_then(|l| l.split_whitespace().next().map(str::to_string))
        .unwrap_or_default()
}

/// Whether we can draw this diagram type as text.
pub fn supported(src: &str) -> bool {
    matches!(
        kind(src).as_str(),
        "graph" | "flowchart" | "sequenceDiagram" | "stateDiagram" | "stateDiagram-v2" | "pie"
    )
}

pub fn render(src: &str, p: &Palette) -> Result<TextDiagram, String> {
    match kind(src).as_str() {
        "graph" | "flowchart" | "stateDiagram" | "stateDiagram-v2" => {
            let g = if kind(src).starts_with("state") {
                flowchart::parse_state(src)?
            } else {
                flowchart::parse(src)?
            };
            let lines = flowchart::render(&g, g.dir, p).to_lines();
            let narrow = matches!(g.dir, flowchart::Dir::LR | flowchart::Dir::RL)
                .then(|| flowchart::render(&g, flowchart::Dir::TD, p).to_lines());
            Ok(TextDiagram { lines, narrow })
        }
        "sequenceDiagram" => Ok(TextDiagram { lines: sequence::render(src, p)?.to_lines(), narrow: None }),
        "pie" => Ok(TextDiagram { lines: pie(src, p)?, narrow: None }),
        other => Err(format!("`{other}` diagrams need mmdc")),
    }
}

/// Width in cells of rendered lines.
pub fn width(lines: &[Line]) -> usize {
    lines.iter().map(|l| l.spans.iter().map(|s| s.content.width()).sum::<usize>()).max().unwrap_or(0)
}

fn pie(src: &str, p: &Palette) -> Result<Vec<Line<'static>>, String> {
    let lines = flowchart::content_lines(src);
    let mut it = lines.iter();
    let header = it.next().ok_or("empty")?;
    let show_data = header.contains("showData");
    let mut title = header.split_once("title").map(|(_, t)| t.trim().to_string());
    let mut slices: Vec<(String, f64)> = Vec::new();
    for l in it {
        if let Some(t) = l.strip_prefix("title") {
            title = Some(t.trim().to_string());
            continue;
        }
        if let Some((label, value)) = l.rsplit_once(':') {
            let label = label.trim().trim_matches('"').to_string();
            if let Ok(v) = value.trim().parse::<f64>() {
                slices.push((label, v));
            }
        }
    }
    if slices.is_empty() {
        return Err("pie chart has no slices".into());
    }
    let total: f64 = slices.iter().map(|s| s.1).sum::<f64>().max(f64::MIN_POSITIVE);
    let lw = slices.iter().map(|s| s.0.width()).max().unwrap_or(0);
    let max = slices.iter().map(|s| s.1).fold(0.0, f64::max).max(f64::MIN_POSITIVE);
    const BAR: f64 = 32.0;
    let mut out = Vec::new();
    if let Some(t) = title {
        out.push(Line::from(Span::styled(t, p.text.add_modifier(Modifier::BOLD))));
        out.push(Line::default());
    }
    for (i, (label, v)) in slices.iter().enumerate() {
        let color = p.series[i % p.series.len()];
        let n = ((v / max) * BAR).round().max(if *v > 0.0 { 1.0 } else { 0.0 }) as usize;
        let mut spans = vec![
            Span::styled(format!("{label:<lw$}  "), p.text),
            Span::styled("█".repeat(n), Style::default().fg(color)),
            Span::raw(" ".repeat(BAR as usize - n.min(BAR as usize) + 1)),
            Span::styled(format!("{:>5.1}%", v / total * 100.0), p.label),
        ];
        if show_data {
            spans.push(Span::styled(format!("  ({v})"), p.edge));
        }
        out.push(Line::from(spans));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pie_chart() {
        let l = pie("pie title Pets\n \"Dogs\" : 386\n \"Cats\" : 85.5\n", &Palette::default()).unwrap();
        let text: String = l[2].spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.starts_with("Dogs") && text.contains("81.9%"), "{text}");
    }

    #[test]
    fn support_detection() {
        assert!(supported("flowchart LR\nA-->B"));
        assert!(supported("%% c\nsequenceDiagram\nA->>B: x"));
        assert!(!supported("classDiagram\nA <|-- B"));
    }
}

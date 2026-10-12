//! Mermaid flowcharts (and state diagrams, which are translated into the same graph) rendered
//! as text: parse → layered layout (Sugiyama-style) in terminal cells → orthogonal edges.

use std::collections::HashMap;

use unicode_width::UnicodeWidthStr;

use super::canvas::{Canvas, Stroke};
use super::Palette;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    TD,
    BT,
    LR,
    RL,
}

impl Dir {
    fn parse(s: &str) -> Option<Dir> {
        Some(match s.to_ascii_uppercase().as_str() {
            "TD" | "TB" => Dir::TD,
            "BT" => Dir::BT,
            "LR" => Dir::LR,
            "RL" => Dir::RL,
            _ => return None,
        })
    }

    fn horizontal(self) -> bool {
        matches!(self, Dir::LR | Dir::RL)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    Rect,
    Rounded,
    Stadium,
    Subroutine,
    Cylinder,
    Circle,
    DoubleCircle,
    Diamond,
    Hexagon,
    Parallelogram,
    ParallelogramAlt,
    Trapezoid,
    TrapezoidAlt,
    Asymmetric,
    /// State diagram start (●) / end (◉): a single glyph, no box.
    Start,
    End,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Head {
    None,
    Arrow,
    Circle,
    Cross,
}

#[derive(Debug, Clone)]
pub struct Node {
    pub id: String,
    pub label: String,
    pub shape: Shape,
}

#[derive(Debug, Clone)]
pub struct Edge {
    pub from: usize,
    pub to: usize,
    pub label: Option<String>,
    pub stroke: Stroke,
    pub invisible: bool,
    pub head: Head,
    pub tail: Head,
}

#[derive(Debug, Clone)]
pub struct Graph {
    pub dir: Dir,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
}

impl Graph {
    fn new(dir: Dir) -> Self {
        Graph { dir, nodes: Vec::new(), edges: Vec::new() }
    }

    fn node(&mut self, id: &str, label: Option<String>, shape: Option<Shape>) -> usize {
        if let Some(i) = self.nodes.iter().position(|n| n.id == id) {
            if let Some(l) = label {
                self.nodes[i].label = l;
            }
            if let Some(s) = shape {
                self.nodes[i].shape = s;
            }
            return i;
        }
        self.nodes.push(Node {
            id: id.to_string(),
            label: label.unwrap_or_else(|| id.to_string()),
            shape: shape.unwrap_or(Shape::Rect),
        });
        self.nodes.len() - 1
    }
}

// ------------------------------------------------------------------------------------ parsing

fn clean_label(s: &str) -> String {
    let s = s.trim();
    let s = s.strip_prefix('"').and_then(|x| x.strip_suffix('"')).unwrap_or(s);
    let s = s.strip_prefix('`').and_then(|x| x.strip_suffix('`')).unwrap_or(s);
    let mut out = s
        .replace("<br/>", "\n")
        .replace("<br />", "\n")
        .replace("<br>", "\n")
        .replace("\\n", "\n")
        .replace("#quot;", "\"")
        .replace("#amp;", "&")
        .replace("#lt;", "<")
        .replace("#gt;", ">");
    // strip simple inline tags and markdown emphasis markers
    while let (Some(a), Some(b)) = (out.find('<'), out.find('>')) {
        if b > a {
            out.replace_range(a..=b, "");
        } else {
            break;
        }
    }
    out.replace("**", "").trim().to_string()
}

struct Cur<'a> {
    c: Vec<char>,
    i: usize,
    _s: &'a str,
}

impl<'a> Cur<'a> {
    fn new(s: &'a str) -> Self {
        Cur { c: s.chars().collect(), i: 0, _s: s }
    }
    fn ws(&mut self) {
        while self.i < self.c.len() && self.c[self.i].is_whitespace() {
            self.i += 1;
        }
    }
    fn peek(&self, k: usize) -> Option<char> {
        self.c.get(self.i + k).copied()
    }
    fn starts(&self, s: &str) -> bool {
        s.chars().enumerate().all(|(k, ch)| self.peek(k) == Some(ch))
    }
    fn done(&self) -> bool {
        self.i >= self.c.len()
    }
    fn rest(&self) -> String {
        self.c[self.i..].iter().collect()
    }
}

const SHAPES: &[(&str, &str, Shape)] = &[
    ("(((", ")))", Shape::DoubleCircle),
    ("((", "))", Shape::Circle),
    ("([", "])", Shape::Stadium),
    ("[[", "]]", Shape::Subroutine),
    ("[(", ")]", Shape::Cylinder),
    ("{{", "}}", Shape::Hexagon),
    ("[/", "/]", Shape::Parallelogram),
    ("[\\", "\\]", Shape::ParallelogramAlt),
    ("[/", "\\]", Shape::Trapezoid),
    ("[\\", "/]", Shape::TrapezoidAlt),
    ("[", "]", Shape::Rect),
    ("(", ")", Shape::Rounded),
    ("{", "}", Shape::Diamond),
    (">", "]", Shape::Asymmetric),
];

fn is_id_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Parse `id`, `id[label]`, `id(label)` … Returns (id, label, shape).
fn parse_node(cur: &mut Cur) -> Option<(String, Option<String>, Option<Shape>)> {
    cur.ws();
    let start = cur.i;
    while cur.peek(0).is_some_and(is_id_char) {
        cur.i += 1;
    }
    // ids may contain '-' when followed by more id chars (but not arrows like -->)
    while cur.peek(0) == Some('-') && cur.peek(1).is_some_and(is_id_char) {
        cur.i += 1;
        while cur.peek(0).is_some_and(is_id_char) {
            cur.i += 1;
        }
    }
    if cur.i == start {
        return None;
    }
    let id: String = cur.c[start..cur.i].iter().collect();
    let mut label = None;
    let mut shape = None;
    for (open, close, sh) in SHAPES {
        if !cur.starts(open) {
            continue;
        }
        let save = cur.i;
        cur.i += open.chars().count();
        // quoted label may contain closers
        let body_start = cur.i;
        if cur.peek(0) == Some('"') {
            cur.i += 1;
            while cur.peek(0).is_some_and(|c| c != '"') {
                cur.i += 1;
            }
            cur.i += 1;
        }
        let mut found = None;
        let close_c: Vec<char> = close.chars().collect();
        while cur.i + close_c.len() <= cur.c.len() {
            if cur.c[cur.i..cur.i + close_c.len()] == close_c[..] {
                found = Some(cur.i);
                break;
            }
            cur.i += 1;
        }
        match found {
            Some(end) => {
                label = Some(clean_label(&cur.c[body_start..end].iter().collect::<String>()));
                shape = Some(*sh);
                cur.i = end + close_c.len();
                break;
            }
            None => cur.i = save,
        }
    }
    // :::className
    if cur.starts(":::") {
        cur.i += 3;
        while cur.peek(0).is_some_and(is_id_char) {
            cur.i += 1;
        }
    }
    Some((id, label, shape))
}

struct EdgeTok {
    stroke: Stroke,
    invisible: bool,
    head: Head,
    tail: Head,
    label: Option<String>,
}

fn is_link_char(c: char) -> bool {
    matches!(c, '-' | '=' | '.' | '~')
}

/// Read one link token like `-->`, `<-.->`, `==x`, `---`. Does not consume labels.
fn link_token(cur: &mut Cur) -> Option<(Stroke, bool, Head, Head, String)> {
    let save = cur.i;
    let mut tail = Head::None;
    match cur.peek(0) {
        Some('<') => {
            tail = Head::Arrow;
            cur.i += 1;
        }
        Some('o') if cur.peek(1).is_some_and(|c| c == '-' || c == '=') => {
            tail = Head::Circle;
            cur.i += 1;
        }
        Some('x') if cur.peek(1).is_some_and(|c| c == '-' || c == '=') => {
            tail = Head::Cross;
            cur.i += 1;
        }
        _ => {}
    }
    let bstart = cur.i;
    while cur.peek(0).is_some_and(is_link_char) {
        cur.i += 1;
    }
    let body: String = cur.c[bstart..cur.i].iter().collect();
    if body.len() < 2 && !(body.len() == 1 && cur.peek(0) == Some('>')) {
        cur.i = save;
        return None;
    }
    let mut head = Head::None;
    match cur.peek(0) {
        Some('>') => {
            head = Head::Arrow;
            cur.i += 1;
        }
        Some('o') if cur.peek(1).is_none_or(|c| c.is_whitespace()) => {
            head = Head::Circle;
            cur.i += 1;
        }
        Some('x') if cur.peek(1).is_none_or(|c| c.is_whitespace()) => {
            head = Head::Cross;
            cur.i += 1;
        }
        _ => {}
    }
    let stroke = if body.contains('=') {
        Stroke::Heavy
    } else if body.contains('.') {
        Stroke::Dashed
    } else {
        Stroke::Light
    };
    Some((stroke, body.contains('~'), head, tail, body))
}

fn parse_edge(cur: &mut Cur) -> Option<EdgeTok> {
    cur.ws();
    let (mut stroke, invisible, mut head, tail, body) = link_token(cur)?;
    let mut label = None;
    // `A -- text --> B` / `A -. text .-> B` / `A == text ==> B`
    if head == Head::None && matches!(body.as_str(), "--" | "==" | "-.") {
        let rest_start = cur.i;
        let mut j = cur.i;
        let mut closing = None;
        while j < cur.c.len() {
            if is_link_char(cur.c[j]) && j > rest_start && cur.c[j - 1] == ' ' {
                let mut k = j;
                while k < cur.c.len() && is_link_char(cur.c[k]) {
                    k += 1;
                }
                if k - j >= 2 || cur.c.get(k) == Some(&'>') {
                    closing = Some(j);
                    break;
                }
            }
            j += 1;
        }
        if let Some(j) = closing {
            label = Some(clean_label(&cur.c[rest_start..j].iter().collect::<String>()));
            cur.i = j;
            if let Some((s2, _, h2, _, _)) = link_token(cur) {
                head = h2;
                if s2 != Stroke::Light {
                    stroke = s2;
                }
            }
        }
    }
    // `-->|label|`
    let save = cur.i;
    cur.ws();
    if cur.peek(0) == Some('|') {
        cur.i += 1;
        let s = cur.i;
        while cur.peek(0).is_some_and(|c| c != '|') {
            cur.i += 1;
        }
        label = Some(clean_label(&cur.c[s..cur.i].iter().collect::<String>()));
        cur.i += 1;
    } else {
        cur.i = save;
    }
    Some(EdgeTok {
        stroke,
        invisible,
        head,
        tail,
        label: label.filter(|l| !l.is_empty()),
    })
}

fn parse_group(cur: &mut Cur, g: &mut Graph) -> Option<Vec<usize>> {
    let mut v = Vec::new();
    loop {
        let (id, label, shape) = parse_node(cur)?;
        v.push(g.node(&id, label, shape));
        cur.ws();
        if cur.peek(0) == Some('&') {
            cur.i += 1;
            continue;
        }
        return Some(v);
    }
}

fn parse_statement(s: &str, g: &mut Graph) -> Result<(), String> {
    let mut cur = Cur::new(s);
    let mut prev = parse_group(&mut cur, g).ok_or_else(|| format!("can't parse: {s}"))?;
    loop {
        cur.ws();
        if cur.done() {
            return Ok(());
        }
        let Some(e) = parse_edge(&mut cur) else {
            return Err(format!("unexpected `{}`", cur.rest()));
        };
        let next = parse_group(&mut cur, g).ok_or_else(|| format!("missing target in: {s}"))?;
        for &a in &prev {
            for &b in &next {
                g.edges.push(Edge {
                    from: a,
                    to: b,
                    label: e.label.clone(),
                    stroke: e.stroke,
                    invisible: e.invisible,
                    head: e.head,
                    tail: e.tail,
                });
            }
        }
        prev = next;
    }
}

/// Split a line on `;` statement separators, ignoring semicolons inside quotes / brackets.
fn split_statements(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut depth = 0i32;
    for c in line.chars() {
        match c {
            '"' => quoted = !quoted,
            '[' | '(' | '{' if !quoted => depth += 1,
            ']' | ')' | '}' if !quoted => depth -= 1,
            ';' if !quoted && depth <= 0 => {
                out.push(std::mem::take(&mut cur));
                continue;
            }
            _ => {}
        }
        cur.push(c);
    }
    out.push(cur);
    out.into_iter().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
}

/// Meaningful lines: no front matter, comments or blank lines.
pub(super) fn content_lines(src: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut in_fm = false;
    for (i, l) in src.lines().enumerate() {
        let t = l.trim();
        if t == "---" && (i == 0 || in_fm) {
            in_fm = !in_fm;
            continue;
        }
        if in_fm || t.is_empty() || t.starts_with("%%") {
            continue;
        }
        out.push(t.to_string());
    }
    out
}

pub fn parse(src: &str) -> Result<Graph, String> {
    let lines = content_lines(src);
    let mut it = lines.iter();
    let header = it.next().ok_or("empty diagram")?;
    let mut hw = header.split_whitespace();
    let kw = hw.next().unwrap_or("");
    if kw != "graph" && kw != "flowchart" {
        return Err(format!("not a flowchart: {kw}"));
    }
    let dir = hw.next().and_then(Dir::parse).unwrap_or(Dir::TD);
    let mut g = Graph::new(dir);
    for line in it {
        let first = line.split_whitespace().next().unwrap_or("");
        if matches!(
            first,
            "classDef" | "class" | "style" | "linkStyle" | "click" | "direction" | "end" | "accTitle"
                | "accDescr" | "title"
        ) || first.starts_with("accTitle:")
            || first.starts_with("accDescr")
        {
            continue;
        }
        if first == "subgraph" {
            continue;
        }
        for stmt in split_statements(line) {
            parse_statement(&stmt, &mut g)?;
        }
    }
    if g.nodes.is_empty() {
        return Err("no nodes".into());
    }
    Ok(g)
}

/// stateDiagram / stateDiagram-v2 → graph.
pub fn parse_state(src: &str) -> Result<Graph, String> {
    let lines = content_lines(src);
    let mut it = lines.iter();
    let header = it.next().ok_or("empty diagram")?;
    if !header.starts_with("stateDiagram") {
        return Err("not a state diagram".into());
    }
    let mut g = Graph::new(Dir::TD);
    let mut scopes: Vec<String> = vec![String::new()];
    let mut in_note = false;
    let state_id = |g: &mut Graph, name: &str, scope: &str, is_target: bool| -> usize {
        if name == "[*]" {
            let (id, shape, label) = if is_target {
                (format!("__end{scope}"), Shape::End, "◉")
            } else {
                (format!("__start{scope}"), Shape::Start, "●")
            };
            return g.node(&id, Some(label.into()), Some(shape));
        }
        let exists = g.nodes.iter().any(|n| n.id == name);
        g.node(name, None, if exists { None } else { Some(Shape::Rounded) })
    };
    for line in it {
        if in_note {
            if line.starts_with("end note") {
                in_note = false;
            }
            continue;
        }
        if line.starts_with("note ") {
            in_note = !line.contains(':');
            continue;
        }
        if let Some(d) = line.strip_prefix("direction ") {
            if scopes.len() == 1 {
                g.dir = Dir::parse(d.trim()).unwrap_or(g.dir);
            }
            continue;
        }
        if line == "}" {
            if scopes.len() > 1 {
                scopes.pop();
            }
            continue;
        }
        if line == "--" || line.starts_with("classDef") || line.starts_with("class ") || line.starts_with("hide ") {
            continue;
        }
        let scope = scopes.last().cloned().unwrap_or_default();
        if let Some(rest) = line.strip_prefix("state ") {
            let rest = rest.trim();
            if let Some(name) = rest.strip_suffix('{') {
                scopes.push(name.trim().to_string());
                continue;
            }
            // state "Long name" as S   |   state S <<choice>>
            if let Some(q) = rest.strip_prefix('"') {
                if let Some((label, after)) = q.split_once('"') {
                    let id = after.trim().strip_prefix("as").map(str::trim).unwrap_or(label);
                    let id = id.trim_end_matches('{').trim();
                    g.node(id, Some(label.to_string()), Some(Shape::Rounded));
                    if after.trim_end().ends_with('{') {
                        scopes.push(id.to_string());
                    }
                }
                continue;
            }
            let mut parts = rest.split_whitespace();
            let id = parts.next().unwrap_or("");
            let shape = match parts.next() {
                Some("<<choice>>") => Shape::Diamond,
                Some("<<fork>>") | Some("<<join>>") => Shape::Rect,
                _ => Shape::Rounded,
            };
            let label = if shape == Shape::Diamond || shape == Shape::Rect && rest.contains("<<") { Some(String::new()) } else { None };
            g.node(id, label, Some(shape));
            continue;
        }
        // transitions: A --> B : label
        if let Some((lhs, rhs)) = line.split_once("-->") {
            let (rhs, label) = match rhs.split_once(':') {
                Some((r, l)) => (r, Some(l.trim().to_string())),
                None => (rhs, None),
            };
            let a = state_id(&mut g, lhs.trim(), &scope, false);
            let b = state_id(&mut g, rhs.trim(), &scope, true);
            g.edges.push(Edge {
                from: a,
                to: b,
                label: label.filter(|l| !l.is_empty()),
                stroke: Stroke::Light,
                invisible: false,
                head: Head::Arrow,
                tail: Head::None,
            });
            continue;
        }
        // S : description
        if let Some((id, desc)) = line.split_once(':') {
            let id = id.trim();
            let i = state_id(&mut g, id, &scope, false);
            let n = &mut g.nodes[i];
            n.label = if n.label == id { format!("{id}\n{}", desc.trim()) } else { format!("{}\n{}", n.label, desc.trim()) };
            continue;
        }
        if line.chars().all(is_id_char) {
            state_id(&mut g, line, &scope, false);
        }
    }
    if g.nodes.is_empty() {
        return Err("no states".into());
    }
    Ok(g)
}

// ------------------------------------------------------------------------------------ layout

const MAX_LABEL: usize = 24;

fn wrap_label(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    for para in s.split('\n') {
        let mut line = String::new();
        for w in para.split_whitespace() {
            if !line.is_empty() && line.width() + 1 + w.width() > MAX_LABEL {
                out.push(std::mem::take(&mut line));
            }
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(w);
        }
        out.push(line);
    }
    out
}

struct VNode {
    real: Option<usize>,
    layer: usize,
    /// size along the rank axis / cross axis
    rsize: i64,
    csize: i64,
    order: f64,
    cpos: i64,
}

impl VNode {
    fn center(&self) -> i64 {
        self.cpos + self.csize / 2
    }
}

struct Path {
    pts: Vec<(i64, i64)>,
    head_end: Head,
    head_start: Head,
    /// (x, y, text)
    label: Option<(i64, i64, String)>,
    stroke: Stroke,
}

struct Laid {
    /// for each real node: (x, y, w, h) in canvas cells
    boxes: Vec<(i64, i64, i64, i64)>,
    paths: Vec<Path>,
    width: i64,
    height: i64,
}

fn layout(g: &Graph, dir: Dir) -> Laid {
    let n = g.nodes.len();
    let horizontal = dir.horizontal();
    let labels: Vec<Vec<String>> = g.nodes.iter().map(|nd| wrap_label(&nd.label)).collect();
    let (bw, bh): (Vec<i64>, Vec<i64>) = g
        .nodes
        .iter()
        .zip(&labels)
        .map(|(nd, l)| {
            if matches!(nd.shape, Shape::Start | Shape::End) {
                return (1, 1);
            }
            let w = l.iter().map(|x| x.width()).max().unwrap_or(0) as i64;
            let pad = match nd.shape {
                Shape::Diamond | Shape::Hexagon | Shape::Circle | Shape::DoubleCircle => 6,
                _ => 4,
            };
            ((w + pad).max(5), l.len() as i64 + 2)
        })
        .unzip();

    // --- break cycles (DFS), layer by longest path
    let edges: Vec<(usize, usize, usize)> = g
        .edges
        .iter()
        .enumerate()
        .filter(|(_, e)| e.from != e.to)
        .map(|(i, e)| (i, e.from, e.to))
        .collect();
    let mut adj: Vec<Vec<(usize, usize)>> = vec![Vec::new(); n];
    for &(i, a, b) in &edges {
        adj[a].push((b, i));
    }
    let mut reversed = vec![false; g.edges.len()];
    let mut state = vec![0u8; n]; // 0 new, 1 on stack, 2 done
    for s in 0..n {
        if state[s] != 0 {
            continue;
        }
        let mut stack = vec![(s, 0usize)];
        state[s] = 1;
        while let Some(&mut (v, ref mut k)) = stack.last_mut() {
            if *k < adj[v].len() {
                let (w, ei) = adj[v][*k];
                *k += 1;
                match state[w] {
                    0 => {
                        state[w] = 1;
                        stack.push((w, 0));
                    }
                    1 => reversed[ei] = true,
                    _ => {}
                }
            } else {
                state[v] = 2;
                stack.pop();
            }
        }
    }
    let dag: Vec<(usize, usize, usize)> = edges
        .iter()
        .map(|&(i, a, b)| if reversed[i] { (i, b, a) } else { (i, a, b) })
        .collect();
    let mut layer = vec![0usize; n];
    // longest path via repeated relaxation (graphs are small)
    for _ in 0..n {
        let mut changed = false;
        for &(_, a, b) in &dag {
            if layer[b] < layer[a] + 1 {
                layer[b] = layer[a] + 1;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    // pull sinks with a single parent up next to it, and sources down next to their child
    for v in 0..n {
        let preds: Vec<usize> = dag.iter().filter(|e| e.2 == v).map(|e| e.1).collect();
        let succs: Vec<usize> = dag.iter().filter(|e| e.1 == v).map(|e| e.2).collect();
        if preds.is_empty() && !succs.is_empty() {
            let m = succs.iter().map(|&s| layer[s]).min().unwrap();
            layer[v] = m.saturating_sub(1).max(layer[v]);
        }
    }
    let nlayers = layer.iter().max().map(|m| m + 1).unwrap_or(1);

    // --- virtual nodes (real + dummies), chains per edge
    let mut vn: Vec<VNode> = (0..n)
        .map(|i| {
            let (rs, cs) = if horizontal { (bw[i], bh[i]) } else { (bh[i], bw[i]) };
            VNode { real: Some(i), layer: layer[i], rsize: rs, csize: cs, order: i as f64, cpos: 0 }
        })
        .collect();
    let mut chains: Vec<Option<Vec<usize>>> = vec![None; g.edges.len()];
    for &(i, a, b) in &dag {
        let mut chain = vec![a];
        if reversed[i] {
            chain.push(b);
            chains[i] = Some(chain);
            continue;
        }
        for l in layer[a] + 1..layer[b] {
            vn.push(VNode { real: None, layer: l, rsize: 0, csize: 1, order: a as f64 + 0.5, cpos: 0 });
            chain.push(vn.len() - 1);
        }
        chain.push(b);
        chains[i] = Some(chain);
    }
    let mut vedges: Vec<(usize, usize)> = Vec::new();
    for c in chains.iter().flatten() {
        for w in c.windows(2) {
            vedges.push((w[0], w[1]));
        }
    }

    // --- ordering: barycenter sweeps
    let mut layers: Vec<Vec<usize>> = vec![Vec::new(); nlayers];
    for (i, v) in vn.iter().enumerate() {
        layers[v.layer].push(i);
    }
    for l in &mut layers {
        l.sort_by(|a, b| vn[*a].order.partial_cmp(&vn[*b].order).unwrap());
        for (k, &v) in l.iter().enumerate() {
            vn[v].order = k as f64;
        }
    }
    for it in 0..8 {
        let down = it % 2 == 0;
        let range: Vec<usize> = if down { (1..nlayers).collect() } else { (0..nlayers.saturating_sub(1)).rev().collect() };
        for l in range {
            let mut keyed: Vec<(f64, usize)> = layers[l]
                .iter()
                .map(|&v| {
                    let nb: Vec<f64> = vedges
                        .iter()
                        .filter_map(|&(a, b)| {
                            if down && b == v {
                                Some(vn[a].order)
                            } else if !down && a == v {
                                Some(vn[b].order)
                            } else {
                                None
                            }
                        })
                        .collect();
                    let key = if nb.is_empty() { vn[v].order } else { nb.iter().sum::<f64>() / nb.len() as f64 };
                    (key, v)
                })
                .collect();
            keyed.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
            layers[l] = keyed.iter().map(|k| k.1).collect();
            for (k, &v) in layers[l].iter().enumerate() {
                vn[v].order = k as f64;
            }
        }
    }

    // --- cross-axis positions: pack, then pull towards neighbours
    let gap = if horizontal { 1 } else { 3 };
    let place = |vn: &mut Vec<VNode>, l: &[usize], desired: &dyn Fn(&VNode, usize) -> Option<i64>| {
        let mut next = i64::MIN / 4;
        for &v in l {
            let want = desired(&vn[v], v).map(|c| c - vn[v].csize / 2).unwrap_or(vn[v].cpos);
            vn[v].cpos = want.max(next);
            next = vn[v].cpos + vn[v].csize + gap;
        }
        // right-to-left pass: pull back left where there is room
        let mut limit = i64::MAX / 4;
        for &v in l.iter().rev() {
            let want = desired(&vn[v], v).map(|c| c - vn[v].csize / 2).unwrap_or(vn[v].cpos);
            let right_max = limit - vn[v].csize;
            if vn[v].cpos > want {
                vn[v].cpos = want.max(vn[v].cpos.min(right_max)).min(vn[v].cpos);
            }
            vn[v].cpos = vn[v].cpos.min(right_max);
            limit = vn[v].cpos - gap;
        }
    };
    for l in &layers {
        let mut x = 0;
        for &v in l {
            vn[v].cpos = x;
            x += vn[v].csize + gap;
        }
    }
    for it in 0..6 {
        let down = it % 2 == 0;
        let order: Vec<usize> = if down { (1..nlayers).collect() } else { (0..nlayers.saturating_sub(1)).rev().collect() };
        for l in order {
            let centers: HashMap<usize, i64> = vn.iter().enumerate().map(|(i, v)| (i, v.center())).collect();
            let ve = &vedges;
            let desired = |_: &VNode, v: usize| -> Option<i64> {
                let nb: Vec<i64> = ve
                    .iter()
                    .filter_map(|&(a, b)| {
                        if down && b == v {
                            Some(centers[&a])
                        } else if !down && a == v {
                            Some(centers[&b])
                        } else {
                            None
                        }
                    })
                    .collect();
                (!nb.is_empty()).then(|| (nb.iter().sum::<i64>() as f64 / nb.len() as f64).round() as i64)
            };
            let lv = layers[l].clone();
            place(&mut vn, &lv, &desired);
        }
    }
    let minc = vn.iter().map(|v| v.cpos).min().unwrap_or(0);
    for v in &mut vn {
        v.cpos -= minc;
    }

    // --- rank-axis positions
    //
    // Gap `G` (0..=nlayers) is the space above layer `G` (G == nlayers: below the last layer).
    // Inside a gap, along the rank axis: a stub, an optional source-side label band, one track
    // per distinct target (so unrelated edges never share a run), an optional target-side
    // label band, and the arrow row.
    #[derive(Clone, Copy, PartialEq)]
    enum Track {
        /// shared by every segment converging on this target
        To(usize),
        /// shared by every segment fanning out of this source
        From(usize),
        Out(usize),
        In(usize),
    }
    let thick: Vec<i64> = (0..nlayers)
        .map(|l| vn.iter().filter(|v| v.layer == l).map(|v| v.rsize).max().unwrap_or(0))
        .collect();
    let rightmost = |v: usize| -> bool {
        !vn.iter().any(|w| w.layer == vn[v].layer && w.real.is_some() && w.cpos > vn[v].cpos)
    };
    let mut tracks: Vec<Vec<Track>> = vec![Vec::new(); nlayers + 1];
    let add = |tracks: &mut Vec<Vec<Track>>, g: usize, t: Track| {
        if !tracks[g].contains(&t) {
            tracks[g].push(t);
        }
    };
    // labelled edges converging on one node put their labels on the source side
    let mut labelled_in: HashMap<usize, usize> = HashMap::new();
    for (i, e) in g.edges.iter().enumerate() {
        if let (Some(c), Some(_), false) = (&chains[i], &e.label, reversed[i]) {
            *labelled_in.entry(*c.last().unwrap()).or_default() += 1;
        }
    }
    // fan-out vs fan-in decides which segments share a track
    let mut outdeg: HashMap<usize, usize> = HashMap::new();
    let mut indeg: HashMap<usize, usize> = HashMap::new();
    for (i, c) in chains.iter().enumerate() {
        let (Some(c), false) = (c, reversed[i] || g.edges[i].invisible) else { continue };
        for w in c.windows(2) {
            *outdeg.entry(w[0]).or_default() += 1;
            *indeg.entry(w[1]).or_default() += 1;
        }
    }
    let seg_track = |a: usize, b: usize| -> Track {
        if outdeg.get(&a).copied().unwrap_or(0) > 1 && outdeg[&a] >= indeg.get(&b).copied().unwrap_or(0) {
            Track::From(a)
        } else {
            Track::To(b)
        }
    };
    // identical labels converging on one node are drawn once, on the target side
    let mut label_texts: HashMap<usize, Vec<String>> = HashMap::new();
    for (i, e) in g.edges.iter().enumerate() {
        if let (Some(c), Some(l), false) = (&chains[i], &e.label, reversed[i]) {
            label_texts.entry(*c.last().unwrap()).or_default().push(l.clone());
        }
    }
    let mut label_drawn: HashMap<(usize, String), bool> = HashMap::new();
    let mut src_band = vec![0i64; nlayers + 1];
    let mut tgt_band = vec![0i64; nlayers + 1];
    let mut label_side_src = vec![false; g.edges.len()];
    for (i, e) in g.edges.iter().enumerate() {
        let Some(c) = &chains[i] else { continue };
        if e.invisible {
            continue;
        }
        if reversed[i] {
            if !rightmost(e.from) {
                add(&mut tracks, vn[e.from].layer + 1, Track::Out(i));
            }
            if !rightmost(e.to) {
                add(&mut tracks, vn[e.to].layer, Track::In(i));
            }
            continue;
        }
        for w in c.windows(2) {
            add(&mut tracks, vn[w[0]].layer + 1, seg_track(w[0], w[1]));
        }
        if let Some(l) = &e.label {
            let last = *c.last().unwrap();
            let gi = vn[last].layer;
            let lw = wrap_label(l).join(" ").width() as i64;
            let band = if horizontal { lw + 2 } else { 1 };
            let texts = &label_texts[&last];
            let all_same = texts.iter().all(|t| t == l);
            if labelled_in[&last] > 1 && !all_same {
                label_side_src[i] = true;
                src_band[gi] = src_band[gi].max(band);
            } else {
                tgt_band[gi] = tgt_band[gi].max(band);
            }
        }
    }
    let (stub, entry) = if horizontal { (2i64, 2i64) } else { (1, 1) };
    let gap_size: Vec<i64> = (0..=nlayers)
        .map(|gi| {
            let n = tracks[gi].len() as i64;
            if n == 0 {
                if gi == 0 || gi == nlayers { 0 } else if horizontal { 4 } else { 2 }
            } else {
                stub + src_band[gi] + n + tgt_band[gi] + entry
            }
        })
        .collect();
    let mut rstart = vec![0i64; nlayers];
    let mut pos = gap_size[0];
    for l in 0..nlayers {
        rstart[l] = pos;
        pos += thick[l] + gap_size[l + 1];
    }
    let total_rank = pos;
    let gap_start = |gi: usize| if gi == 0 { 0 } else { rstart[gi - 1] + thick[gi - 1] };
    let track_rank = |gi: usize, t: Track| -> i64 {
        let k = tracks[gi].iter().position(|x| *x == t).unwrap_or(0) as i64;
        gap_start(gi) + stub + src_band[gi] + k
    };
    let mut total_cross = vn.iter().map(|v| v.cpos + v.csize).max().unwrap_or(1);
    let node_rstart = |v: &VNode| rstart[v.layer] + (thick[v.layer] - v.rsize) / 2;

    // (rank, cross) → (x, y) with direction flips
    let flip = matches!(dir, Dir::BT | Dir::RL);
    let to_xy = |r: i64, c: i64| -> (i64, i64) {
        let r = if flip { total_rank - 1 - r } else { r };
        if horizontal { (r, c) } else { (c, r) }
    };

    let mut boxes = vec![(0, 0, 0, 0); n];
    for v in vn.iter() {
        if let Some(i) = v.real {
            let r0 = node_rstart(v);
            let r0 = if flip { total_rank - r0 - v.rsize } else { r0 };
            boxes[i] = if horizontal { (r0, v.cpos, bw[i], bh[i]) } else { (v.cpos, r0, bw[i], bh[i]) };
        }
    }

    let tidy = |pts: Vec<(i64, i64)>| -> Vec<(i64, i64)> {
        let mut pts = pts;
        pts.dedup();
        let mut out: Vec<(i64, i64)> = Vec::new();
        for p in pts {
            if out.len() >= 2 {
                let a = out[out.len() - 2];
                let b = out[out.len() - 1];
                if (a.0 == b.0 && b.0 == p.0) || (a.1 == b.1 && b.1 == p.1) {
                    out.pop();
                }
            }
            out.push(p);
        }
        out
    };
    // label text placed along the rank axis at `r0` (band start), centred on cross `c`
    let place_label = |r0: i64, band: i64, c: i64, text: String| -> (i64, i64, String) {
        let lw = text.width() as i64;
        if horizontal {
            let r = r0 + ((band - lw) / 2).max(0);
            let (x, y) = to_xy(r, c);
            (if flip { x - lw + 1 } else { x }, y, text)
        } else {
            let (x, y) = to_xy(r0, c);
            (x - lw / 2, y, text)
        }
    };

    let mut paths = Vec::new();
    let mut back_idx = 0i64;
    for (i, e) in g.edges.iter().enumerate() {
        let Some(c) = &chains[i] else {
            // self loop: mark next to the box
            let (x, y, w, _) = boxes[e.from];
            let lbl = e.label.clone().map(|l| format!("↻ {l}")).unwrap_or_else(|| "↻".into());
            paths.push(Path {
                pts: Vec::new(),
                head_end: Head::None,
                head_start: Head::None,
                label: Some((x + w + 1, y, lbl)),
                stroke: e.stroke,
            });
            continue;
        };
        if e.invisible {
            continue;
        }
        if reversed[i] {
            // back edge: out of the source (side if nothing is to its right, else bottom), up a
            // lane beside the graph, into the target (side, else top)
            let (ui, vi) = (e.from, e.to);
            let (u, v) = (&vn[ui], &vn[vi]);
            let (lo, hi) = (v.layer.min(u.layer), v.layer.max(u.layer));
            let lane = vn
                .iter()
                .filter(|w| w.layer >= lo && w.layer <= hi)
                .map(|w| w.cpos + w.csize)
                .max()
                .unwrap_or(0)
                + 2
                + 2 * back_idx;
            back_idx += 1;
            let mut pts = Vec::new();
            let r_out;
            if rightmost(ui) {
                r_out = node_rstart(u) + u.rsize / 2;
                pts.push((r_out, u.cpos + u.csize));
            } else {
                r_out = track_rank(u.layer + 1, Track::Out(i));
                pts.push((node_rstart(u) + u.rsize, u.center()));
                pts.push((r_out, u.center()));
            }
            pts.push((r_out, lane));
            let r_in;
            if rightmost(vi) {
                r_in = node_rstart(v) + v.rsize / 2;
                pts.push((r_in, lane));
                pts.push((r_in, v.cpos + v.csize));
            } else {
                r_in = track_rank(v.layer, Track::In(i));
                pts.push((r_in, lane));
                pts.push((r_in, v.center()));
                pts.push((node_rstart(v) - 1, v.center()));
            }
            let label = e.label.as_ref().map(|l| {
                let text = wrap_label(l).join(" ");
                let lw = text.width() as i64;
                let rm = (r_out + r_in) / 2;
                if horizontal {
                    let (x, y) = to_xy(rm, lane + 1);
                    (x - lw / 2, y, text)
                } else {
                    let (x, y) = to_xy(rm, lane + 2);
                    (x, y, text)
                }
            });
            let extent = lane + 1 + if horizontal { 1 } else { label.as_ref().map(|l| l.2.width() as i64 + 3).unwrap_or(0) };
            total_cross = total_cross.max(extent);
            let xy: Vec<(i64, i64)> = tidy(pts).iter().map(|&(r, c)| to_xy(r, c)).collect();
            paths.push(Path { pts: xy, head_end: e.head, head_start: e.tail, label, stroke: e.stroke });
            continue;
        }
        let first = &vn[c[0]];
        let mut pts: Vec<(i64, i64)> = vec![(node_rstart(first) + first.rsize, first.center())];
        let mut cross = first.center();
        let mut last_src_cross = cross;
        for w in c.windows(2) {
            let (a, b) = (&vn[w[0]], &vn[w[1]]);
            let mid = track_rank(a.layer + 1, seg_track(w[0], w[1]));
            // stay straight if we can enter the target box where we already are
            let target = if b.real.is_some() && cross > b.cpos && cross < b.cpos + b.csize - 1 {
                cross
            } else {
                b.center()
            };
            last_src_cross = cross;
            pts.push((mid, cross));
            pts.push((mid, target));
            if b.real.is_some() {
                pts.push((node_rstart(b) - 1, target));
            }
            cross = target;
        }
        let pts = tidy(pts);
        let last = *c.last().unwrap();
        let dup = e.label.as_ref().is_some_and(|l| {
            !label_side_src[i] && label_drawn.insert((last, l.clone()), true).is_some()
        });
        let label = e.label.as_ref().filter(|_| !dup).map(|l| {
            let gi = vn[last].layer;
            let text = wrap_label(l).join(" ");
            if label_side_src[i] {
                place_label(gap_start(gi) + stub, src_band[gi], last_src_cross, text)
            } else {
                let r0 = gap_start(gi) + stub + src_band[gi] + tracks[gi].len() as i64;
                place_label(r0, tgt_band[gi], cross, text)
            }
        });
        let xy: Vec<(i64, i64)> = pts.iter().map(|&(r, c)| to_xy(r, c)).collect();
        paths.push(Path { pts: xy, head_end: e.head, head_start: e.tail, label, stroke: e.stroke });
    }

    let (width, height) = if horizontal { (total_rank, total_cross) } else { (total_cross, total_rank) };
    Laid { boxes, paths, width, height }
}

fn head_glyph(h: Head, from: (i64, i64), to: (i64, i64)) -> Option<char> {
    let (dx, dy) = (to.0 - from.0, to.1 - from.1);
    Some(match h {
        Head::None => return None,
        Head::Circle => '●',
        Head::Cross => '✕',
        Head::Arrow if dy > 0 => '▼',
        Head::Arrow if dy < 0 => '▲',
        Head::Arrow if dx > 0 => '▶',
        Head::Arrow => '◀',
    })
}

fn draw_box(cv: &mut Canvas, (x, y, w, h): (i64, i64, i64, i64), shape: Shape, lines: &[String], p: &Palette) {
    if matches!(shape, Shape::Start | Shape::End) {
        cv.glyph(x, y, if shape == Shape::Start { '●' } else { '◉' }, p.node);
        return;
    }
    let (tl, tr, bl, br, hz, l, r) = match shape {
        Shape::Rect => ('┌', '┐', '└', '┘', '─', '│', '│'),
        Shape::Rounded | Shape::Cylinder => ('╭', '╮', '╰', '╯', '─', '│', '│'),
        Shape::Stadium | Shape::Circle | Shape::DoubleCircle => ('╭', '╮', '╰', '╯', '─', '(', ')'),
        Shape::Subroutine => ('┌', '┐', '└', '┘', '─', '║', '║'),
        Shape::Diamond => ('╱', '╲', '╲', '╱', '─', '◇', '◇'),
        Shape::Hexagon => ('╱', '╲', '╲', '╱', '─', '⟨', '⟩'),
        Shape::Parallelogram => ('╱', '╱', '╱', '╱', '─', ' ', ' '),
        Shape::ParallelogramAlt => ('╲', '╲', '╲', '╲', '─', ' ', ' '),
        Shape::Trapezoid => ('╱', '╲', '└', '┘', '─', '│', '│'),
        Shape::TrapezoidAlt => ('┌', '┐', '╲', '╱', '─', '│', '│'),
        Shape::Asymmetric => ('▷', '┐', '▷', '┘', '─', '▷', '│'),
        Shape::Start | Shape::End => unreachable!(),
    };
    let hzs: String = std::iter::repeat_n(hz, (w - 2).max(0) as usize).collect();
    let top_hz = if shape == Shape::Cylinder { "┄".repeat((w - 2).max(0) as usize) } else { hzs.clone() };
    let top_hz = if shape == Shape::DoubleCircle { "═".repeat((w - 2).max(0) as usize) } else { top_hz };
    cv.text(x, y, &format!("{tl}{top_hz}{tr}"), p.node);
    for k in 1..h - 1 {
        cv.text(x, y + k, &l.to_string(), p.node);
        cv.text(x + 1, y + k, &" ".repeat((w - 2).max(0) as usize), p.text);
        cv.text(x + w - 1, y + k, &r.to_string(), p.node);
    }
    cv.text(x, y + h - 1, &format!("{bl}{hzs}{br}"), p.node);
    for (k, line) in lines.iter().enumerate() {
        let lw = line.width() as i64;
        let lx = x + (w - lw) / 2;
        cv.text(lx, y + 1 + k as i64, line, p.text);
    }
}

pub fn render(g: &Graph, dir: Dir, p: &Palette) -> Canvas {
    let lay = layout(g, dir);
    let extra = lay
        .paths
        .iter()
        .filter_map(|p| p.label.as_ref().map(|(x, _, t)| x + t.width() as i64 + 1))
        .max()
        .unwrap_or(0);
    let mut cv = Canvas::new((lay.width.max(extra) + 1) as usize, (lay.height + 1) as usize);
    for path in lay.paths.iter().filter(|p| p.pts.len() >= 2) {
        cv.polyline(&path.pts, path.stroke, p.edge);
    }
    for path in lay.paths.iter().filter(|p| p.pts.len() >= 2) {
        let pts = &path.pts;
        let n = pts.len();
        if let Some(gl) = head_glyph(path.head_end, pts[n - 2], pts[n - 1]) {
            cv.glyph(pts[n - 1].0, pts[n - 1].1, gl, p.edge);
        }
        if let Some(gl) = head_glyph(path.head_start, pts[1], pts[0]) {
            cv.glyph(pts[0].0, pts[0].1, gl, p.edge);
        }
    }
    let mut taken: Vec<(i64, i64, i64)> = Vec::new(); // (y, x0, x1)
    for path in &lay.paths {
        if let Some((x, y, t)) = &path.label {
            let w = t.width() as i64 + 2;
            let mut x0 = x - 1;
            while let Some(&(_, _, e)) = taken.iter().find(|(ty, a, b)| ty == y && x0 < *b && x0 + w > *a) {
                x0 = e + 1;
            }
            taken.push((*y, x0, x0 + w));
            cv.text(x0, *y, &format!(" {t} "), p.label);
        }
    }
    for (i, nd) in g.nodes.iter().enumerate() {
        draw_box(&mut cv, lay.boxes[i], nd.shape, &wrap_label(&nd.label), p);
    }
    cv
}

#[cfg(test)]
mod tests {
    use super::*;

    fn show(src: &str) -> String {
        let g = parse(src).unwrap();
        render(&g, g.dir, &Palette::default())
            .to_lines()
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn parses_shapes_edges_labels() {
        let g = parse("flowchart LR\n A[Start] -->|go| B{Ok?}\n B -- yes --> C([Done]) & D[(DB)]\n B -.-> E\n C ==> A").unwrap();
        assert_eq!(g.dir, Dir::LR);
        assert_eq!(g.nodes.len(), 5);
        assert_eq!(g.nodes[1].shape, Shape::Diamond);
        assert_eq!(g.nodes[2].shape, Shape::Stadium);
        assert_eq!(g.nodes[3].shape, Shape::Cylinder);
        assert_eq!(g.edges[0].label.as_deref(), Some("go"));
        assert_eq!(g.edges[1].label.as_deref(), Some("yes"));
        assert_eq!(g.edges.len(), 5);
        assert_eq!(g.edges[3].stroke, Stroke::Dashed);
        assert_eq!(g.edges[4].stroke, Stroke::Heavy);
    }

    #[test]
    fn semicolons_inside_labels() {
        let g = parse("flowchart TD\n A[\"a; b\"] --> B(c;d); B --> C\n").unwrap();
        assert_eq!(g.nodes[0].label, "a; b");
        assert_eq!(g.nodes[1].label, "c;d");
        assert_eq!(g.edges.len(), 2);
    }

    #[test]
    fn chains_and_ids_with_dashes() {
        let g = parse("graph TD\n a-1 --> b --> c\n").unwrap();
        assert_eq!(g.nodes.iter().map(|n| n.id.as_str()).collect::<Vec<_>>(), ["a-1", "b", "c"]);
        assert_eq!(g.edges.len(), 2);
    }

    #[test]
    fn renders_readable_td() {
        let s = show("flowchart TD\n A[Start] --> B{Is it?}\n B -->|Yes| C[OK]\n B -->|No| D[Retry]\n D --> B");
        for word in ["Start", "Is it?", "Yes", "No", "OK", "Retry"] {
            assert!(s.contains(word), "{word} missing in\n{s}");
        }
        assert!(s.contains('▼'), "{s}");
    }

    #[test]
    fn renders_lr() {
        let s = show("flowchart LR\n A[Markdown] --> B(comrak AST) --> C{IR}");
        let line = s.lines().find(|l| l.contains("Markdown")).unwrap();
        assert!(line.contains("comrak AST") && line.contains("IR") && line.contains('▶'), "{s}");
    }

    #[test]
    fn state_diagram() {
        let g = parse_state("stateDiagram-v2\n [*] --> Still\n Still --> Moving : push\n Moving --> [*]").unwrap();
        assert_eq!(g.nodes.len(), 4);
        assert!(g.edges.iter().any(|e| e.label.as_deref() == Some("push")));
    }
}

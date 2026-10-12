//! Minimal LSP client (used with `marksman`): completion and diagnostics over stdio JSON-RPC.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{channel, Receiver};

use serde_json::{json, Value};

use crate::completion::{Item, Kind};

#[derive(Debug, Clone, PartialEq)]
pub struct Diagnostic {
    /// 0-based line.
    pub line: usize,
    pub col: usize,
    pub severity: u8,
    pub message: String,
}

pub enum LspEvent {
    Ready,
    /// Completion response for request `id`.
    /// `replace_start` is the UTF-16 column where the server wants its edit to start.
    Completion { id: i64, items: Vec<Item>, replace_start: Option<usize> },
    Diagnostics(Vec<Diagnostic>),
    Exited,
}

pub struct Lsp {
    child: Child,
    stdin: ChildStdin,
    rx: Receiver<Option<Value>>,
    next_id: i64,
    init_id: i64,
    pending: HashMap<i64, &'static str>,
    uri: String,
    version: i64,
    ready: bool,
    queued_text: Option<String>,
    pub name: String,
}

fn uri_for(path: &Path) -> String {
    let abs = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let mut s = String::from("file://");
    for b in abs.to_string_lossy().bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' | b'~' => s.push(b as char),
            _ => s.push_str(&format!("%{b:02X}")),
        }
    }
    s
}

/// Convert a char column to UTF-16 code units for LSP positions.
pub fn utf16_col(line: &str, char_col: usize) -> usize {
    line.chars().take(char_col).map(|c| c.len_utf16()).sum()
}

impl Lsp {
    /// Spawn `cmd` (e.g. `["marksman", "server"]`) for `path` with initial `text`.
    pub fn start(cmd: &[String], path: &Path, text: &str) -> Option<Lsp> {
        let (prog, args) = cmd.split_first()?;
        let mut child = Command::new(prog)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        let stdin = child.stdin.take()?;
        let stdout = child.stdout.take()?;
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let mut r = BufReader::new(stdout);
            loop {
                let mut len = None;
                loop {
                    let mut header = String::new();
                    match r.read_line(&mut header) {
                        Ok(0) | Err(_) => {
                            let _ = tx.send(None);
                            return;
                        }
                        Ok(_) => {}
                    }
                    let h = header.trim();
                    if h.is_empty() {
                        break;
                    }
                    if let Some(v) = h.strip_prefix("Content-Length:") {
                        len = v.trim().parse::<usize>().ok();
                    }
                }
                let Some(len) = len else { continue };
                let mut body = vec![0; len];
                if r.read_exact(&mut body).is_err() {
                    let _ = tx.send(None);
                    return;
                }
                if let Ok(v) = serde_json::from_slice::<Value>(&body) {
                    if tx.send(Some(v)).is_err() {
                        return;
                    }
                }
            }
        });
        let root: PathBuf = path
            .parent()
            .map(|p| if p.as_os_str().is_empty() { Path::new(".") } else { p })
            .unwrap_or(Path::new("."))
            .to_path_buf();
        let mut lsp = Lsp {
            child,
            stdin,
            rx,
            next_id: 1,
            init_id: 0,
            pending: HashMap::new(),
            uri: uri_for(path),
            version: 1,
            ready: false,
            queued_text: Some(text.to_string()),
            name: prog.clone(),
        };
        let id = lsp.request(
            "initialize",
            json!({
                "processId": std::process::id(),
                "rootUri": uri_for(&root),
                "capabilities": {
                    "textDocument": {
                        "completion": { "completionItem": { "snippetSupport": false } },
                        "publishDiagnostics": {}
                    }
                },
                "clientInfo": { "name": "mdtui" }
            }),
            "initialize",
        )?;
        lsp.init_id = id;
        Some(lsp)
    }

    fn send(&mut self, msg: Value) -> Option<()> {
        let body = msg.to_string();
        write!(self.stdin, "Content-Length: {}\r\n\r\n{}", body.len(), body).ok()?;
        self.stdin.flush().ok()
    }

    fn request(&mut self, method: &str, params: Value, kind: &'static str) -> Option<i64> {
        let id = self.next_id;
        self.next_id += 1;
        self.pending.insert(id, kind);
        self.send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))?;
        Some(id)
    }

    fn notify(&mut self, method: &str, params: Value) {
        let _ = self.send(json!({"jsonrpc": "2.0", "method": method, "params": params}));
    }

    pub fn is_ready(&self) -> bool {
        self.ready
    }

    pub fn did_change(&mut self, text: &str) {
        if !self.ready {
            self.queued_text = Some(text.to_string());
            return;
        }
        self.version += 1;
        let (uri, version) = (self.uri.clone(), self.version);
        self.notify(
            "textDocument/didChange",
            json!({"textDocument": {"uri": uri, "version": version}, "contentChanges": [{"text": text}]}),
        );
    }

    pub fn did_save(&mut self) {
        if self.ready {
            let uri = self.uri.clone();
            self.notify("textDocument/didSave", json!({"textDocument": {"uri": uri}}));
        }
    }

    /// Request completion at (0-based line, UTF-16 column).
    pub fn completion(&mut self, line: usize, character: usize) -> Option<i64> {
        if !self.ready {
            return None;
        }
        let uri = self.uri.clone();
        self.request(
            "textDocument/completion",
            json!({"textDocument": {"uri": uri}, "position": {"line": line, "character": character}}),
            "completion",
        )
    }

    pub fn poll(&mut self) -> Vec<LspEvent> {
        let mut out = Vec::new();
        while let Ok(msg) = self.rx.try_recv() {
            let Some(msg) = msg else {
                out.push(LspEvent::Exited);
                break;
            };
            if let Some(id) = msg.get("id").and_then(Value::as_i64) {
                if msg.get("method").is_some() {
                    // server → client request (e.g. workspace/configuration): reply with null
                    let _ = self.send(json!({"jsonrpc": "2.0", "id": id, "result": null}));
                    continue;
                }
                let kind = self.pending.remove(&id);
                if id == self.init_id {
                    self.notify("initialized", json!({}));
                    let text = self.queued_text.take().unwrap_or_default();
                    let uri = self.uri.clone();
                    self.notify(
                        "textDocument/didOpen",
                        json!({"textDocument": {"uri": uri, "languageId": "markdown", "version": self.version, "text": text}}),
                    );
                    self.ready = true;
                    out.push(LspEvent::Ready);
                } else if kind == Some("completion") {
                    let result = msg.get("result");
                    let first = result
                        .and_then(|r| r.get("items").or(Some(r)))
                        .and_then(|a| a.get(0));
                    let replace_start = first
                        .and_then(|i| i["textEdit"]["range"]["start"]["character"].as_u64())
                        .map(|c| c as usize);
                    out.push(LspEvent::Completion {
                        id,
                        items: parse_completion(result),
                        replace_start,
                    });
                }
            } else if msg.get("method").and_then(Value::as_str) == Some("textDocument/publishDiagnostics") {
                let p = &msg["params"];
                if p["uri"].as_str() != Some(self.uri.as_str()) {
                    continue;
                }
                let diags = p["diagnostics"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .map(|d| Diagnostic {
                                line: d["range"]["start"]["line"].as_u64().unwrap_or(0) as usize,
                                col: d["range"]["start"]["character"].as_u64().unwrap_or(0) as usize,
                                severity: d["severity"].as_u64().unwrap_or(1) as u8,
                                message: d["message"].as_str().unwrap_or("").to_string(),
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                out.push(LspEvent::Diagnostics(diags));
            }
        }
        out
    }
}

impl Drop for Lsp {
    fn drop(&mut self) {
        let _ = self.request("shutdown", Value::Null, "shutdown");
        self.notify("exit", Value::Null);
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn parse_completion(result: Option<&Value>) -> Vec<Item> {
    let Some(r) = result else { return Vec::new() };
    let items = r.get("items").unwrap_or(r);
    items
        .as_array()
        .map(|a| {
            a.iter()
                .map(|i| {
                    let label = i["label"].as_str().unwrap_or("").to_string();
                    let insert = i["textEdit"]["newText"]
                        .as_str()
                        .or_else(|| i["insertText"].as_str())
                        .unwrap_or(&label)
                        .to_string();
                    Item {
                        detail: i["detail"].as_str().unwrap_or("").chars().take(40).collect(),
                        label,
                        insert,
                        kind: Kind::Lsp,
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

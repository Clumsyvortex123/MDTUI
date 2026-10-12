//! Lua plugin host for mdtui.
//!
//! Plugins are `*.lua` files (or directories with `init.lua`) in the plugin directory. Each runs
//! in its own environment table (globals don't leak between plugins) on top of a shared, limited
//! standard library. `io`, `os` and `mdtui.exec` are only available when the user marks plugins
//! as trusted in `config.toml`.
//!
//! See `prelude.lua` for the API exposed to plugins.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use mdtui_core::theme::parse_color;
use mdtui_core::Document;
use mlua::{Function, Lua, LuaOptions, LuaSerdeExt, StdLib, Table, Value};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

const PRELUDE: &str = include_str!("prelude.lua");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Info,
    Warn,
    Error,
}

/// What a fence handler produced.
#[derive(Debug, Clone)]
pub enum FenceOutput {
    Image(Vec<u8>),
    Lines(Vec<Line<'static>>),
    Markdown(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct CompletionItem {
    pub label: String,
    pub insert: String,
    pub detail: String,
}

/// Completions from one plugin source: replace `replace_chars` chars before the cursor.
#[derive(Debug, Clone)]
pub struct CompletionSet {
    pub replace_chars: usize,
    pub items: Vec<CompletionItem>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Keymap {
    pub mode: String,
    pub lhs: String,
    pub rhs: String,
}

/// Context passed to fence handlers as the second argument.
#[derive(Debug, Clone, Default)]
pub struct FenceCtx {
    pub width: u16,
    pub dark: bool,
    pub path: String,
}

pub struct PluginHost {
    lua: Lua,
    trusted: bool,
    loaded: Vec<String>,
}

impl PluginHost {
    pub fn new(trusted: bool) -> mlua::Result<Self> {
        let libs = if trusted {
            StdLib::ALL_SAFE
        } else {
            StdLib::TABLE | StdLib::STRING | StdLib::MATH | StdLib::UTF8 | StdLib::COROUTINE
        };
        let lua = Lua::new_with(libs, LuaOptions::default())?;
        let mdtui: Table = lua.load(PRELUDE).set_name("@mdtui/prelude.lua").eval()?;

        if trusted {
            let exec = lua.create_function(|lua, (argv, stdin): (Vec<String>, Option<mlua::LuaString>)| {
                let (prog, args) = argv
                    .split_first()
                    .ok_or_else(|| mlua::Error::runtime("exec: empty argv"))?;
                let mut child = Command::new(prog)
                    .args(args)
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
                    .map_err(|e| mlua::Error::runtime(format!("exec {prog}: {e}")))?;
                if let Some(input) = stdin {
                    use std::io::Write;
                    let bytes = input.as_bytes().to_vec();
                    let mut si = child.stdin.take().unwrap();
                    std::thread::spawn(move || {
                        let _ = si.write_all(&bytes);
                    });
                } else {
                    drop(child.stdin.take());
                }
                let out = child
                    .wait_with_output()
                    .map_err(|e| mlua::Error::runtime(format!("exec {prog}: {e}")))?;
                let stdout = lua.create_string(&out.stdout)?;
                if out.status.success() {
                    Ok((Value::String(stdout), Value::Nil))
                } else {
                    let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
                    Ok((Value::String(stdout), Value::String(lua.create_string(err)?)))
                }
            })?;
            mdtui.set("exec", exec)?;
        } else {
            let exec = lua.create_function(|_, _: mlua::MultiValue| -> mlua::Result<()> {
                Err(mlua::Error::runtime(
                    "mdtui.exec is disabled; set `trusted = true` under [plugins] in config.toml",
                ))
            })?;
            mdtui.set("exec", exec)?;
            // no filesystem access through the base library either
            let g = lua.globals();
            g.set("dofile", Value::Nil)?;
            g.set("loadfile", Value::Nil)?;
        }
        mdtui.set("trusted", trusted)?;
        lua.set_named_registry_value("mdtui", mdtui.clone())?;

        // require("mdtui") works with or without the package library
        let mdtui_for_require = mdtui.clone();
        let package_require: Option<Function> = lua.globals().get("require").ok();
        let require = lua.create_function(move |_, name: String| -> mlua::Result<Value> {
            if name == "mdtui" {
                return Ok(Value::Table(mdtui_for_require.clone()));
            }
            match &package_require {
                Some(f) => f.call(name),
                None => Err(mlua::Error::runtime(format!(
                    "require('{name}'): only 'mdtui' is available to untrusted plugins"
                ))),
            }
        })?;
        lua.globals().set("require", require)?;

        Ok(PluginHost {
            lua,
            trusted,
            loaded: Vec::new(),
        })
    }

    fn mdtui(&self) -> Table {
        self.lua.named_registry_value("mdtui").expect("mdtui table")
    }

    pub fn trusted(&self) -> bool {
        self.trusted
    }

    pub fn loaded(&self) -> &[String] {
        &self.loaded
    }

    /// Load every plugin in `dir`. Returns one error message per plugin that failed.
    pub fn load_dir(&mut self, dir: &Path) -> Vec<String> {
        let mut errors = Vec::new();
        let Ok(rd) = std::fs::read_dir(dir) else {
            return errors;
        };
        let mut paths: Vec<PathBuf> = rd
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter_map(|p| {
                if p.extension().is_some_and(|e| e == "lua") {
                    Some(p)
                } else if p.join("init.lua").is_file() {
                    Some(p.join("init.lua"))
                } else {
                    None
                }
            })
            .collect();
        paths.sort();
        for p in paths {
            if let Err(e) = self.load_file(&p) {
                errors.push(e);
            }
        }
        errors
    }

    pub fn load_file(&mut self, path: &Path) -> Result<(), String> {
        let src = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let name = if path.file_name().is_some_and(|f| f == "init.lua") {
            path.parent().and_then(|p| p.file_name())
        } else {
            path.file_stem()
        }
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "plugin".into());
        self.load_source(&name, &src)
    }

    /// Load a plugin from source. The plugin gets its own global environment.
    pub fn load_source(&mut self, name: &str, src: &str) -> Result<(), String> {
        let run = || -> mlua::Result<()> {
            let env = self.lua.create_table()?;
            let meta = self.lua.create_table()?;
            meta.set("__index", self.lua.globals())?;
            env.set_metatable(Some(meta))?;
            self.mdtui().set("_current", name)?;
            self.lua
                .load(src)
                .set_name(format!("@{name}.lua"))
                .set_environment(env)
                .exec()
        };
        run().map_err(|e| format!("plugin {name}: {}", short_err(&e)))?;
        self.loaded.push(name.to_string());
        Ok(())
    }

    /// Languages claimed by plugin fence handlers.
    pub fn fence_names(&self) -> Vec<String> {
        let mut v = Vec::new();
        if let Ok(t) = self.mdtui().get::<Table>("_fences") {
            for (k, _) in t.pairs::<String, Value>().flatten() {
                v.push(k);
            }
        }
        v.sort();
        v
    }

    pub fn render_fence(&self, lang: &str, src: &str, ctx: &FenceCtx) -> Result<FenceOutput, String> {
        let go = || -> mlua::Result<FenceOutput> {
            let fences: Table = self.mdtui().get("_fences")?;
            let entry: Table = fences.get(lang)?;
            let f: Function = entry.get("fn")?;
            let c = self.lua.create_table()?;
            c.set("width", ctx.width)?;
            c.set("dark", ctx.dark)?;
            c.set("path", ctx.path.clone())?;
            c.set("lang", lang)?;
            let v: Value = f.call((src, c))?;
            self.fence_output(v)
        };
        go().map_err(|e| short_err(&e))
    }

    fn fence_output(&self, v: Value) -> mlua::Result<FenceOutput> {
        Ok(match v {
            Value::String(s) => FenceOutput::Lines(text_lines(&s.to_str()?)),
            Value::Table(t) => {
                let kind: String = t.get::<Option<String>>("kind")?.unwrap_or_else(|| "text".into());
                match kind.as_str() {
                    "image" => {
                        let data: mlua::LuaString = t.get("data")?;
                        FenceOutput::Image(data.as_bytes().to_vec())
                    }
                    "markdown" => FenceOutput::Markdown(t.get("text")?),
                    "lines" => {
                        let lines: Table = t.get("lines")?;
                        let mut out = Vec::new();
                        for line in lines.sequence_values::<Value>() {
                            out.push(lua_line(line?)?);
                        }
                        FenceOutput::Lines(out)
                    }
                    _ => FenceOutput::Lines(text_lines(&t.get::<String>("text")?)),
                }
            }
            Value::Nil => return Err(mlua::Error::runtime("fence handler returned nil")),
            other => FenceOutput::Lines(text_lines(&other.to_string()?)),
        })
    }

    pub fn has_transforms(&self) -> bool {
        let m = self.mdtui();
        let n = |k: &str| m.get::<Table>(k).map(|t| t.raw_len()).unwrap_or(0);
        n("_ast") + n("_inlines") > 0
    }

    /// Run inline patterns and `on("ast")` hooks over the document.
    pub fn transform(&self, doc: Document) -> Result<Document, String> {
        if !self.has_transforms() {
            return Ok(doc);
        }
        let go = || -> mlua::Result<Document> {
            let opts = mlua::serde::SerializeOptions::new()
                .serialize_none_to_null(false)
                .serialize_unit_to_null(false)
                .set_array_metatable(false);
            let v = self.lua.to_value_with(&doc, opts)?;
            let f: Function = self.mdtui().get("_transform")?;
            let out: Value = f.call(v)?;
            self.lua.from_value(out)
        };
        go().map_err(|e| format!("plugin transform: {}", short_err(&e)))
    }

    /// Ask every completion source whose trigger appears in `before_cursor`.
    pub fn complete(&self, before_cursor: &str) -> Vec<CompletionSet> {
        let mut out = Vec::new();
        let Ok(sources) = self.mdtui().get::<Table>("_completions") else {
            return out;
        };
        for spec in sources.sequence_values::<Table>().flatten() {
            let trigger: String = spec.get::<Option<String>>("trigger").ok().flatten().unwrap_or_default();
            let Some(pos) = (if trigger.is_empty() {
                Some(before_cursor.rfind(|c: char| c.is_whitespace()).map(|i| i + 1).unwrap_or(0))
            } else {
                before_cursor.rfind(&trigger)
            }) else {
                continue;
            };
            let prefix = &before_cursor[pos + trigger.len()..];
            if prefix.contains(char::is_whitespace) {
                continue;
            }
            let Ok(f) = spec.get::<Function>("complete") else { continue };
            let Ok(items) = f.call::<Table>(prefix) else { continue };
            let mut v = Vec::new();
            for it in items.sequence_values::<Value>().flatten() {
                match it {
                    Value::String(s) => {
                        let s = s.to_string_lossy();
                        v.push(CompletionItem { label: s.clone(), insert: s, detail: String::new() })
                    }
                    Value::Table(t) => {
                        let label: String = t.get("label").unwrap_or_default();
                        let insert = t.get::<Option<String>>("insert").ok().flatten().unwrap_or_else(|| label.clone());
                        let detail = t.get::<Option<String>>("detail").ok().flatten().unwrap_or_default();
                        v.push(CompletionItem { label, insert, detail });
                    }
                    _ => {}
                }
            }
            if !v.is_empty() {
                out.push(CompletionSet {
                    replace_chars: before_cursor[pos..].chars().count(),
                    items: v,
                });
            }
        }
        out
    }

    pub fn commands(&self) -> Vec<String> {
        let mut v = Vec::new();
        if let Ok(t) = self.mdtui().get::<Table>("_commands") {
            for (k, _) in t.pairs::<String, Value>().flatten() {
                v.push(k);
            }
        }
        v.sort();
        v
    }

    /// Run `:Name args`. Returns the new buffer text if the command changed it.
    pub fn run_command(&self, name: &str, args: &str, path: &str, text: &str) -> Result<Option<String>, String> {
        let f: Function = self.mdtui().get("_run_command").map_err(|e| e.to_string())?;
        f.call::<Option<String>>((name, path, text, args))
            .map_err(|e| format!("{name}: {}", short_err(&e)))
    }

    pub fn keymaps(&self) -> Vec<Keymap> {
        let mut v = Vec::new();
        if let Ok(t) = self.mdtui().get::<Table>("_keymaps") {
            for k in t.sequence_values::<Table>().flatten() {
                v.push(Keymap {
                    mode: k.get("mode").unwrap_or_default(),
                    lhs: k.get("lhs").unwrap_or_default(),
                    rhs: k.get("rhs").unwrap_or_default(),
                });
            }
        }
        v
    }

    /// Notifications raised by plugins since the last call.
    pub fn take_notifications(&self) -> Vec<(Level, String)> {
        let m = self.mdtui();
        let Ok(t) = m.get::<Table>("_notes") else {
            return Vec::new();
        };
        let mut v = Vec::new();
        for n in t.sequence_values::<Table>().flatten() {
            let level = match n.get::<String>("level").unwrap_or_default().as_str() {
                "warn" | "warning" => Level::Warn,
                "error" => Level::Error,
                _ => Level::Info,
            };
            v.push((level, n.get("msg").unwrap_or_default()));
        }
        if let Ok(empty) = self.lua.create_table() {
            let _ = m.set("_notes", empty);
        }
        v
    }
}

/// First line of a Lua error, without the traceback.
fn short_err(e: &mlua::Error) -> String {
    let s = match e {
        mlua::Error::CallbackError { cause, .. } => return short_err(cause),
        other => other.to_string(),
    };
    s.lines().next().unwrap_or("").replace("runtime error: ", "")
}

fn text_lines(s: &str) -> Vec<Line<'static>> {
    s.lines().map(|l| Line::from(l.to_string())).collect()
}

/// A Lua line is either a string or a list of span tables / strings.
fn lua_line(v: Value) -> mlua::Result<Line<'static>> {
    match v {
        Value::String(s) => Ok(Line::from(s.to_str()?.to_string())),
        Value::Table(t) => {
            let mut spans = Vec::new();
            for sv in t.sequence_values::<Value>() {
                match sv? {
                    Value::String(s) => spans.push(Span::raw(s.to_str()?.to_string())),
                    Value::Table(st) => {
                        let text: String = st.get("text")?;
                        let mut style = Style::default();
                        if let Some(c) = st.get::<Option<String>>("fg")?.as_deref().and_then(parse_color) {
                            style = style.fg(c);
                        }
                        if let Some(c) = st.get::<Option<String>>("bg")?.as_deref().and_then(parse_color) {
                            style = style.bg(c);
                        }
                        for (k, m) in [
                            ("bold", Modifier::BOLD),
                            ("italic", Modifier::ITALIC),
                            ("underline", Modifier::UNDERLINED),
                            ("dim", Modifier::DIM),
                        ] {
                            if st.get::<Option<bool>>(k)?.unwrap_or(false) {
                                style = style.add_modifier(m);
                            }
                        }
                        spans.push(Span::styled(text, style));
                    }
                    _ => {}
                }
            }
            Ok(Line::from(spans))
        }
        _ => Ok(Line::default()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mdtui_core::ir::Block;
    use mdtui_core::{parse, ParseOptions};

    #[test]
    fn inline_patterns_and_ast_hooks() {
        let mut h = PluginHost::new(false).unwrap();
        h.load_source(
            "t",
            r#"
            local mdtui = require("mdtui")
            mdtui.register_inline("@(%w+)", function(name) return { text = name, link = "https://github.com/" .. name } end)
            mdtui.on("ast", function(doc)
              doc:walk(function(n) if n.type == "text" then n.text = n.text:gsub(":rocket:", "🚀") end end)
            end)
            "#,
        )
        .unwrap();
        let doc = parse("go :rocket: ping @bob now", &ParseOptions::default());
        let doc = h.transform(doc).unwrap();
        let Block::Paragraph { content, .. } = &doc.blocks[0] else { panic!() };
        let text = mdtui_core::ir::plain_text(content);
        assert_eq!(text, "go 🚀 ping bob now");
        assert!(content.iter().any(|i| matches!(i, mdtui_core::Inline::Styled { link: Some(l), .. } if l == "https://github.com/bob")));
    }

    #[test]
    fn fences_commands_completion_errors() {
        let mut h = PluginHost::new(false).unwrap();
        h.load_source(
            "t",
            r#"
            local mdtui = require("mdtui")
            mdtui.register_fence("shout", function(src) return src:upper() end)
            mdtui.command("Upper", function(buf) buf:set_text(buf:text():upper()) end)
            mdtui.register_completion({ trigger = "@", complete = function(p) return {{label = "@" .. p .. "x"}} end })
            mdtui.keymap("n", "<leader>u", ":Upper<CR>")
            "#,
        )
        .unwrap();
        assert_eq!(h.fence_names(), vec!["shout"]);
        match h.render_fence("shout", "hi", &FenceCtx::default()).unwrap() {
            FenceOutput::Lines(l) => assert_eq!(l[0].spans[0].content, "HI"),
            _ => panic!(),
        }
        assert_eq!(h.run_command("Upper", "", "", "abc").unwrap().as_deref(), Some("ABC"));
        let c = h.complete("hello @ab");
        assert_eq!(c[0].replace_chars, 3);
        assert_eq!(c[0].items[0].label, "@abx");
        assert_eq!(h.keymaps()[0].lhs, "<leader>u");

        // untrusted plugins cannot exec
        let err = h.load_source("bad", "require('mdtui').exec({'ls'})").unwrap_err();
        assert!(err.contains("trusted"), "{err}");
        assert!(h.load_source("syntax", "this is not lua").is_err());
    }
}

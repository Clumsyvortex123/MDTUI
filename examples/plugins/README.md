# Example plugins

Copy any of these into `~/.config/mdtui/plugins/` (or point `[plugins] dir` at this folder).

| Plugin | Shows off |
|---|---|
| `emoji.lua` | `register_inline` with Lua patterns |
| `wikilinks.lua` | `on("ast")` document rewriting |
| `csv.lua` | `register_fence` returning markdown |
| `wordcount.lua` | `command`, `keymap`, editing the buffer |
| `mentions.lua` | styled inline links and `register_completion` |
| `plantuml.lua` | `register_fence` returning an image via `mdtui.exec` (needs `trusted = true`) |

Try them on this file: `MDTUI_CONFIG_DIR=examples mdtui examples/plugins/README.md`
(`MDTUI_CONFIG_DIR=examples` makes `examples/plugins` the plugin directory).

```csv
plugin,kind,trusted
csv.lua,fence,no
plantuml.lua,fence,yes
```

Thanks @octocat :) see [[kitchen-sink]] :shipit:

## API

```lua
local mdtui = require("mdtui")

mdtui.register_fence(lang, function(src, ctx) ... end)
--   ctx = { width, dark, path, lang }
--   return "text" | {kind="text", text=} | {kind="markdown", text=}
--        | {kind="lines", lines={ {{text=, fg=, bg=, bold=, italic=, underline=}, ...}, ... }}
--        | {kind="image", data=<png/jpeg/svg bytes>}

mdtui.on("ast", function(doc) doc:walk(function(node) ... end) end)
--   nodes are plain tables: {type="heading", level=, content={...}},
--   {type="text", text=}, {type="link", url=, content=}, {type="code_block", lang=, code=}, ...
--   edit in place, or return a new doc table

mdtui.register_inline(lua_pattern, function(captures...) return "text" | {text=, link=, fg=, bg=, bold=} | nil end)
mdtui.register_completion({ trigger = "@", complete = function(prefix) return {{label=, insert=, detail=}} end })
mdtui.command("Name", function(buf, args) buf:text() buf:lines() buf:path() buf:set_text(s) end)
mdtui.keymap("n", "<leader>x", ":Name<CR>")
mdtui.notify(msg, "info" | "warn" | "error")
mdtui.exec({"prog", "arg"}, stdin) -- trusted only; returns stdout, stderr-on-failure
mdtui.text(s) mdtui.link(url, s) mdtui.styled(s, {fg=, bold=})  -- node constructors
```

Plugins run sandboxed by default: no `io`, `os`, `require` (except `mdtui`), `dofile`
or `mdtui.exec`. Errors become notifications and the block falls back to its source.

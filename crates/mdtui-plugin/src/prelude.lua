-- mdtui plugin API (loaded before any plugin). Plugins get it with `local mdtui = require("mdtui")`.
local mdtui = {
  version = "0.1.0",
  _fences = {},
  _ast = {},
  _inlines = {},
  _completions = {},
  _commands = {},
  _keymaps = {},
  _notes = {},
  _current = "?",
}

--- Render fenced code blocks of language `lang` with `fn(src, ctx)`.
--- Return a string (plain text), {kind="text", text=...}, {kind="markdown", text=...},
--- {kind="lines", lines={{ {text=..., fg=..., bold=true}, ... }, ...}} or {kind="image", data=<png bytes>}.
function mdtui.register_fence(lang, fn)
  assert(type(lang) == "string", "register_fence: lang must be a string")
  assert(type(fn) == "function", "register_fence: handler must be a function")
  mdtui._fences[lang] = { fn = fn, plugin = mdtui._current }
end

--- Subscribe to an event. Supported: "ast" (fn(doc) may edit doc in place or return a new one).
function mdtui.on(event, fn)
  assert(type(fn) == "function", "on: handler must be a function")
  if event == "ast" then
    table.insert(mdtui._ast, { fn = fn, plugin = mdtui._current })
  else
    error("mdtui.on: unknown event '" .. tostring(event) .. "'")
  end
end

--- Replace text matching a Lua pattern. `fn(captures...)` returns a string, a span table
--- {text=, link=, fg=, bg=, bold=, italic=, underline=} or nil to leave the text alone.
function mdtui.register_inline(pattern, fn)
  assert(type(pattern) == "string", "register_inline: pattern must be a string")
  table.insert(mdtui._inlines, { pattern = pattern, fn = fn, plugin = mdtui._current })
end

--- Completion source: {trigger="@", complete=function(prefix) return {{label=, insert=, detail=}} end}
function mdtui.register_completion(spec)
  assert(type(spec) == "table" and type(spec.complete) == "function", "register_completion: need {trigger=, complete=fn}")
  spec.plugin = mdtui._current
  table.insert(mdtui._completions, spec)
end

--- Define an ex command (`:Name args`). fn(buf, args) where buf has :text(), :lines(), :path(), :set_text(s).
function mdtui.command(name, fn)
  assert(type(name) == "string" and name:match("^%u"), "command: name must start with an uppercase letter")
  mdtui._commands[name] = { fn = fn, plugin = mdtui._current }
end

--- Map keys in the viewer: mdtui.keymap("n", "<leader>wc", ":WordCount<CR>")
function mdtui.keymap(mode, lhs, rhs)
  table.insert(mdtui._keymaps, { mode = mode, lhs = lhs, rhs = rhs })
end

--- Show a notification. level: "info" | "warn" | "error"
function mdtui.notify(msg, level)
  table.insert(mdtui._notes, { msg = tostring(msg), level = level or "info" })
end

-- node constructors -----------------------------------------------------------------
function mdtui.text(s) return { type = "text", text = s } end
function mdtui.link(url, s) return { type = "link", url = url, content = { mdtui.text(s or url) } } end
function mdtui.styled(s, style)
  local n = { type = "styled", content = { mdtui.text(s) } }
  for k, v in pairs(style or {}) do n[k] = v end
  return n
end

-- traversal -------------------------------------------------------------------------
local inline_lists = { "content" }
local function walk_inlines(list, fn)
  if type(list) ~= "table" then return end
  for _, node in ipairs(list) do
    fn(node)
    for _, k in ipairs(inline_lists) do walk_inlines(node[k], fn) end
  end
end

local function walk_blocks(blocks, fn)
  if type(blocks) ~= "table" then return end
  for _, b in ipairs(blocks) do
    fn(b)
    walk_inlines(b.content, fn)
    walk_blocks(b.blocks, fn)
    if b.items then
      for _, it in ipairs(b.items) do
        walk_blocks(it.blocks, fn)
        walk_blocks(it.details, fn)
        walk_inlines(it.term, fn)
      end
    end
    if b.header then for _, c in ipairs(b.header) do walk_inlines(c, fn) end end
    if b.rows then for _, r in ipairs(b.rows) do for _, c in ipairs(r) do walk_inlines(c, fn) end end end
  end
end

--- Visit every block and inline node depth-first.
function mdtui.walk(doc, fn) walk_blocks(doc.blocks, fn) end

local doc_mt = { __index = { walk = function(self, fn) mdtui.walk(self, fn) end } }

-- inline pattern application ---------------------------------------------------------
local function to_inline(res, original)
  if res == nil then return mdtui.text(original) end
  if type(res) == "string" then return mdtui.text(res) end
  if type(res) == "table" then
    if res.type then return res end
    local n = { type = "styled", content = { mdtui.text(res.text or original) } }
    n.fg, n.bg, n.link = res.fg, res.bg, res.link
    n.bold, n.italic, n.underline = res.bold or false, res.italic or false, res.underline or false
    return n
  end
  return mdtui.text(tostring(res))
end

local function process_inlines(list)
  if type(list) ~= "table" then return list end
  local out = {}
  for _, node in ipairs(list) do
    if node.type == "text" then
      local text = node.text
      while #text > 0 do
        local best
        for _, p in ipairs(mdtui._inlines) do
          local r = { string.find(text, p.pattern) }
          if r[1] and r[2] >= r[1] and (not best or r[1] < best.r[1]) then best = { r = r, p = p } end
        end
        if not best then break end
        local s, e = best.r[1], best.r[2]
        if s > 1 then table.insert(out, mdtui.text(text:sub(1, s - 1))) end
        local whole = text:sub(s, e)
        local caps = { table.unpack(best.r, 3) }
        if #caps == 0 then caps = { whole } end
        local ok, res = pcall(best.p.fn, table.unpack(caps))
        if not ok then
          mdtui.notify(best.p.plugin .. ": " .. tostring(res), "error")
          res = nil
        end
        table.insert(out, to_inline(res, whole))
        text = text:sub(e + 1)
      end
      if #text > 0 then table.insert(out, mdtui.text(text)) end
    else
      if node.content then node.content = process_inlines(node.content) end
      table.insert(out, node)
    end
  end
  return out
end

local function process_blocks(blocks)
  if type(blocks) ~= "table" then return end
  for _, b in ipairs(blocks) do
    if b.content then b.content = process_inlines(b.content) end
    process_blocks(b.blocks)
    if b.items then
      for _, it in ipairs(b.items) do
        process_blocks(it.blocks)
        process_blocks(it.details)
        if it.term then it.term = process_inlines(it.term) end
      end
    end
    if b.header then for i, c in ipairs(b.header) do b.header[i] = process_inlines(c) end end
    if b.rows then for _, r in ipairs(b.rows) do for i, c in ipairs(r) do r[i] = process_inlines(c) end end end
  end
end

-- called from Rust --------------------------------------------------------------------
function mdtui._transform(doc)
  setmetatable(doc, doc_mt)
  if #mdtui._inlines > 0 then process_blocks(doc.blocks) end
  for _, h in ipairs(mdtui._ast) do
    local ok, res = pcall(h.fn, doc)
    if not ok then
      mdtui.notify(h.plugin .. ": " .. tostring(res), "error")
    elseif type(res) == "table" then
      doc = setmetatable(res, doc_mt)
    end
  end
  setmetatable(doc, nil)
  return doc
end

function mdtui._run_command(name, path, text, args)
  local c = mdtui._commands[name]
  if not c then return nil, "unknown command " .. name end
  local buf = { _text = text, _path = path }
  function buf:text() return self._text end
  function buf:path() return self._path end
  function buf:lines()
    local t = {}
    for l in (self._text .. "\n"):gmatch("(.-)\n") do table.insert(t, l) end
    return t
  end
  function buf:set_text(s) self._text = s; self._changed = true end
  c.fn(buf, args)
  if buf._changed then return buf._text end
  return nil
end

return mdtui

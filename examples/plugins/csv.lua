-- csv.lua — render ```csv fences as markdown tables (works without trust).
local mdtui = require("mdtui")

local function split(line)
  local cells, field, quoted = {}, "", false
  for c in line:gmatch(".") do
    if c == '"' then quoted = not quoted
    elseif c == "," and not quoted then table.insert(cells, field); field = ""
    else field = field .. c end
  end
  table.insert(cells, field)
  return cells
end

mdtui.register_fence("csv", function(src)
  local rows = {}
  for line in src:gmatch("[^\n]+") do table.insert(rows, split(line)) end
  if #rows == 0 then return "(empty csv)" end
  local out = { "| " .. table.concat(rows[1], " | ") .. " |" }
  local sep = {}
  for i = 1, #rows[1] do sep[i] = "---" end
  table.insert(out, "| " .. table.concat(sep, " | ") .. " |")
  for i = 2, #rows do table.insert(out, "| " .. table.concat(rows[i], " | ") .. " |") end
  return { kind = "markdown", text = table.concat(out, "\n") }
end)

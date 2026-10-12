-- plantuml.lua — render ```plantuml fences with the plantuml CLI.
-- Needs `trusted = true` under [plugins] in config.toml (it runs a program).
local mdtui = require("mdtui")

mdtui.register_fence("plantuml", function(src, ctx)
  local args = { "plantuml", "-pipe", "-tpng" }
  if ctx.dark then table.insert(args, "-darkmode") end
  local png, err = mdtui.exec(args, src)
  if err and #err > 0 then error(err) end
  return { kind = "image", data = png }
end)

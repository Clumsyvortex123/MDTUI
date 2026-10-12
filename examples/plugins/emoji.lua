-- emoji.lua — text emoticons and a few extra shortcodes.
local mdtui = require("mdtui")

local faces = {
  [":)"] = "🙂", [":-)"] = "🙂", [":("] = "🙁", [";)"] = "😉",
  [":D"] = "😄", ["<3"] = "❤️", [":P"] = "😛",
}

-- emoticons must stand alone (start of text or after a space)
mdtui.register_inline("%f[%S][:;<][%-]?[%)%(DP3]", function(face)
  return faces[face] or face
end)

local extra = { shipit = "🐿️", lgtm = "👍", wip = "🚧", todo = "📝" }
mdtui.register_inline(":(%w+):", function(name)
  return extra[name] -- nil leaves the text untouched
end)

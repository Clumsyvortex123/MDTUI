-- mentions.lua — @user → GitHub profile link, with completion after "@".
local mdtui = require("mdtui")

local people = { "octocat", "torvalds", "gaearon", "BurntSushi" }

mdtui.register_inline("%f[%w@]@([%w%-]+)", function(user)
  return { text = "@" .. user, link = "https://github.com/" .. user, fg = "#c586c0", bold = true }
end)

mdtui.register_completion({
  trigger = "@",
  complete = function(prefix)
    local items = {}
    for _, p in ipairs(people) do
      if p:lower():find(prefix:lower(), 1, true) == 1 then
        table.insert(items, { label = "@" .. p, detail = "github user" })
      end
    end
    return items
  end,
})

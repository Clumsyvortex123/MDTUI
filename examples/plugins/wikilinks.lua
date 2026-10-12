-- wikilinks.lua — style [[Page]] links (parsed by comrak) and point them at Page.md.
local mdtui = require("mdtui")

mdtui.on("ast", function(doc)
  doc:walk(function(node)
    if node.type == "link" and not node.url:find("://") and not node.url:find("%.%w+$")
        and not node.url:find("^#") and not node.url:find("^mailto:") then
      node.url = node.url:gsub(" ", "-") .. ".md"
      table.insert(node.content, 1, mdtui.text("⟦"))
      table.insert(node.content, mdtui.text("⟧"))
    end
  end)
end)

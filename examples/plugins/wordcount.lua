-- wordcount.lua — :WordCount command, <leader>wc mapping, and :TitleCase which edits the buffer.
local mdtui = require("mdtui")

mdtui.command("WordCount", function(buf)
  local words, lines = 0, 0
  for _ in buf:text():gmatch("%S+") do words = words + 1 end
  for _ in buf:text():gmatch("[^\n]*\n?") do lines = lines + 1 end
  mdtui.notify(string.format("%d words · %d lines · ~%d min read", words, lines - 1, math.max(1, words // 220)))
end)

mdtui.command("TitleCase", function(buf)
  local out = buf:text():gsub("(\n?#+ )([^\n]+)", function(hashes, title)
    return hashes .. title:gsub("(%a)([%w']*)", function(a, b) return a:upper() .. b end)
  end)
  buf:set_text(out)
end)

mdtui.keymap("n", "<leader>wc", ":WordCount<CR>")

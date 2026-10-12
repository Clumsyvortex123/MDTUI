<p align="center"><img src="assets/logo.svg" alt="mdtui" width="512"></p>

# mdtui

A VS Code-style markdown viewer for the terminal, with a vim-style editor you can open beside
the preview. Built with [ratatui](https://ratatui.rs).

```
┌─ mdtui ─ README.md ──────────────────────────────── ● live ─┐
│ OUTLINE        │ █ Project Title                          │
│ ▾ Project      │ ══════════════════════════════           │
│   ▾ Install    │ ☑ Parse GFM with comrak                  │
│   ▾ Usage      │ ☐ Render mermaid    ← click / x toggles  │
│                │ ┌ rust ─────────────────────── [copy] ┐  │
│                │ │ fn main() { println!("hi"); }       │  │
│                │ └─────────────────────────────────────┘  │
│ VIEW  README.md        ¶ Project › Usage       42%  ln 88 │
```

## Features

- **All of GitHub-flavoured markdown** (via [comrak](https://github.com/kivikakk/comrak)):
  - headings and nested lists
  - task lists, whose checkboxes you can click (the change is written to the file)
  - tables with column alignment
  - block quotes and GitHub alerts (`> [!NOTE]` …)
  - footnotes, description lists and front matter
  - strikethrough, `==highlight==` and spoilers
  - autolinks, wikilinks and emoji shortcodes
  - inline and display math, shown as unicode
- **Code blocks** highlighted with syntect (200+ languages), each with a `[copy]` button that
  copies via OSC 52.
- **Images**:
  - local files, remote URLs, SVG, and HTML `<img>` tags
  - drawn with the Kitty, Sixel or iTerm2 graphics protocols, or unicode half-blocks as a fallback
  - click an image to view it fullscreen, with zoom and pan
- **Mermaid** diagrams are drawn natively as text: flowcharts (all node shapes, edge labels,
  dashed/thick edges, cycles), sequence diagrams, state diagrams and pie charts. Text stays
  sharp in any terminal, follows the theme, and can be searched and copied. Wide
  left-to-right charts switch to top-down when they don't fit. Other diagram types (class,
  gantt, er, …) go through `mmdc` as images.
  - `[img]`/`[txt]` switches a single diagram between text and image.
  - `[src]` shows the diagram's source.
  - `[⤢]` opens it fullscreen. Drag with the mouse (or use `h j k l` / the wheel) to pan;
    panning stops at the diagram's edges. Click `[⌖ recenter]` (or press `0`) to recenter.
    Images zoom with `+`/`-` or the wheel.
  - `[mermaid] renderer = "auto" | "text" | "image"` in the config, or `:set mermaid=…`,
    sets the default.
- **Mouse**:
  - scroll the preview
  - click links, checkboxes, buttons and outline entries
  - drag to select text (it's copied when you release)
  - drag the divider between the editor and the preview
- **Vim keys** in the viewer:
  - `j/k`, `gg/G` and `Ctrl-d/u` to move around
  - `/ n N` to search
  - `Tab` to cycle through links, `]] [[` to jump between headings
  - `za` to fold a section
  - `H`/`L` to go back and forward through followed links
  - `:` for commands, with Tab completion
- **Live reload**: the preview updates when the file changes on disk and keeps your scroll
  position.
- **Editor split** (`e`):
  - a vim-style editor ([edtui](https://github.com/preiter93/edtui)) next to the live preview
  - the preview follows the editor cursor
  - `:w` saves; mdtui won't quit or reload over unsaved changes without asking
- **Completion**, VS Code style:
  - file paths after `](`, and `#heading` anchors
  - reference labels, emoji, and code-fence languages
  - snippets (`table`, `mermaid`, `note`, …)
  - the `marksman` language server for completion and diagnostics, if installed
  - completion sources added by plugins
- **Lua plugins**: add new fenced block types, rewrite the document before it's rendered,
  style inline patterns, and add completion sources, commands and keymaps. See
  [examples/plugins](examples/plugins/README.md).

## Install

Prerequisites: Rust (`cargo`), and Node.js ≥ 22.13 for
[mermaid-cli](https://github.com/mermaid-js/mermaid-cli), which renders diagrams the text renderer
doesn't support.

```sh
./install.sh            # → ~/.local/bin/mdtui and ~/.local/bin/mmdc
PREFIX=/usr/local ./install.sh
```

`install.sh` checks the prerequisites (it loads nvm if needed), then:
- runs `npm install -g @mermaid-js/mermaid-cli`;
- writes an `mmdc` launcher that pins that Node version, so diagrams work even where nvm
  isn't loaded;
- checks that `mmdc` can render, including the Ubuntu AppArmor/Chromium sandbox case;
- builds mdtui in release mode and installs it.

Optional extras:

```sh
go install github.com/AlexanderGrooff/mermaid-ascii@latest   # text fallback for other diagram types
brew install marksman              # or your distro's package: link/heading completion + diagnostics
```

## Usage

```sh
mdtui README.md                 # view (live reload)
mdtui -e notes.md               # open with the editor split
mdtui --toc docs/guide.md       # with outline sidebar
cat file.md | mdtui             # from stdin
mdtui --dump file.md | less -R  # render to stdout, no TUI
mdtui --print-config > ~/.config/mdtui/config.toml
```

Press `?` inside mdtui for the key reference.

| Command | |
|---|---|
| `:w` `:q` `:wq` `:q!` | write / quit |
| `:e file.md` | open another file |
| `:vsplit` / `:close` | open / close the editor |
| `:theme dark\|light` | switch theme |
| `:set number` `:set noimages` `:set maxwidth=100` | options |
| `:toc` `:fold` `:unfold` | outline and folding |
| `:plugins` `:lsp` | status |
| `:42` | jump to source line 42 |
| `:WordCount` … | commands defined by plugins |

## Inside tmux

mdtui runs fine in tmux. Add this to `~/.tmux.conf` so every feature gets through:

```tmux
set -g mouse on              # pass clicks / scroll / drag to mdtui
set -g allow-passthrough on  # let Kitty/Sixel/iTerm2 image data reach the outer terminal
set -g set-clipboard on      # forward OSC 52 copies to the outer terminal
set -ga update-environment TERM_PROGRAM
```

Copying uses `tmux load-buffer -w` inside tmux (so `prefix ]` pastes it). It also uses the
desktop clipboard (`wl-copy`, `xclip`, `xsel`, `pbcopy`) and sends OSC 52, which covers SSH.
Without `allow-passthrough`, images fall back to unicode half-blocks.

## Configuration

`~/.config/mdtui/config.toml` (run `mdtui --print-config` for every option):

```toml
theme = "dark"
max_width = 110

[images]
protocol = "auto"     # kitty | sixel | iterm2 | halfblocks

[mermaid]
mmdc = "mmdc"

[editor]
lsp = ["marksman", "server"]

[plugins]
trusted = false       # allow io/os/mdtui.exec in plugins

[keys]
"<leader>t" = ":toc"
```

## Workspace layout

| Crate | Purpose |
|---|---|
| `mdtui-core` | comrak → IR, layout to styled lines, hit map, search, highlighting. No terminal I/O. |
| `mdtui-render` | Viewport widget, async image/SVG loading, protocol encoding, mermaid pipeline |
| `mdtui-plugin` | Lua 5.4 host (mlua), sandbox, plugin API |
| `mdtui-editor` | edtui wrapper, completion popup and sources, LSP client |
| `mdtui` | The binary: app state, input, drawing, config, live reload |

```sh
cargo test --workspace
cargo run -- examples/kitchen-sink.md
MDTUI_CONFIG_DIR=examples cargo run -- examples/plugins/README.md   # with the example plugins
```

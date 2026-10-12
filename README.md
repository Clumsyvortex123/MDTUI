<p align="center"><img src="assets/logo.svg" alt="mdtui" width="512"></p>

# mdtui

A VS Code-style markdown viewer for the terminal, with a vim-style editor you can open beside
the preview. Built with [ratatui](https://ratatui.rs).

![mdtui rendering examples/kitchen-sink.md](assets/demo.gif)

## Features

- **GitHub-flavoured markdown**: tables, task lists (click to toggle), alerts, footnotes,
  math, emoji and more
- **Code blocks** with syntax highlighting and a `[copy]` button
- **Images** via Kitty, Sixel or iTerm2, with a unicode fallback
- **Mermaid diagrams** drawn as text (flowcharts, sequence, state, pie); other types via `mmdc`
- **Vim keys and mouse support**, search, folding and an outline sidebar
- **Live reload** when the file changes on disk
- **Editor split** (`e`): a vim-style editor beside the live preview, with completion
- **Lua plugins**: see [examples/plugins](examples/plugins/README.md)

## Install

Needs Rust, and Node.js ≥ 22.13 for [mermaid-cli](https://github.com/mermaid-js/mermaid-cli).

```sh
./install.sh            # installs to ~/.local/bin
```

## Usage

```sh
mdtui README.md           # view
mdtui -e notes.md         # open with the editor
mdtui --toc guide.md      # with the outline sidebar
cat file.md | mdtui       # from stdin
```

Press `?` inside mdtui for the key reference.

## Configuration

Run `mdtui --print-config > ~/.config/mdtui/config.toml` to get a config file with every option.

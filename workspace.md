# mdtui workspace guide

This document describes how the mdtui repository is organised, how data flows through it, and
what each module is responsible for. It is written for agents and contributors who need to find
the right place to make a change.

mdtui is a terminal markdown viewer with an optional vim-style editor split. It is a Rust
workspace (edition 2021, stable toolchain) built on [ratatui](https://ratatui.rs) and
[comrak](https://github.com/kivikakk/comrak). It is about 10k lines of Rust across five crates.

## Repository map

```
.
├── Cargo.toml              workspace manifest: members, shared versions, all dependency versions
├── rust-toolchain.toml     channel = "stable"
├── install.sh              installs mermaid-cli (npm) + an mmdc launcher, then builds mdtui
├── README.md               user-facing docs
├── assets/                 logo.svg and demo.gif used by the README
├── examples/
│   ├── kitchen-sink.md     every supported feature; also the snapshot-test fixture
│   ├── images/             gradient.png, logo.svg (referenced by kitchen-sink.md)
│   └── plugins/            example Lua plugins + plugin API reference (README.md)
└── crates/
    ├── mdtui-core/         markdown → IR → laid-out lines. No terminal I/O.
    ├── mdtui-render/       viewport widget, async image/diagram store, mermaid renderers
    ├── mdtui-plugin/       Lua 5.4 plugin host (mlua)
    ├── mdtui-editor/       editor pane (edtui), completion, LSP client
    └── mdtui/              the binary: app state, input, drawing, config, live reload
```

All dependency versions live in `[workspace.dependencies]` in the root `Cargo.toml`. Crate
manifests use `foo.workspace = true`; add new dependencies there, not in individual crates.

## Crate dependency graph

```
                  mdtui (bin)
      ┌─────────┬─────┴──────┬──────────┐
      ▼         ▼            ▼          ▼
mdtui-render mdtui-plugin mdtui-editor  │
      │         │            │          │
      └─────────┴─────┬──────┴──────────┘
                      ▼
                  mdtui-core
```

- `mdtui-core` depends on nothing internal. It must stay free of terminal I/O (no crossterm
  calls, no threads, no filesystem access except through callers) so it can be tested headless.
- `mdtui-render`, `mdtui-plugin` and `mdtui-editor` depend only on `mdtui-core`, never on
  each other.
- Only the `mdtui` binary wires them together.

## The rendering pipeline

```
source text
   │  parse()                         mdtui-core/parse.rs   (comrak AST → owned IR)
   ▼
Document (IR)
   │  PluginHost::transform()         mdtui-plugin          (inline patterns, on("ast") hooks)
   │  custom fences dispatched        mdtui/app.rs rebuild  (mermaid → MediaStore,
   │                                                         plugin fences → PluginHost)
   ▼
Document (final)
   │  layout(doc, opts, theme, &dyn MediaProvider)          mdtui-core/layout.rs
   ▼
Layout { lines, hits, media placements, headings, anchors, src_rows }
   │  Viewport widget                 mdtui-render/viewport.rs
   ▼
terminal buffer (+ images drawn over reserved rows)
```

Key points:

1. **Parse once per source change, lay out once per width/option change.** `App::rebuild`
   re-parses and is triggered by file changes, editor edits (debounced 120 ms), plugin
   reloads and theme changes. `App::relayout_if_needed` runs `layout()` when `layout_dirty`
   is set: on resize, fold/unfold, `[src]` toggles, or when media finishes loading.
2. **Layout asks the media provider how big things are.** `layout()` takes a
   `&dyn MediaProvider` (implemented by `MediaStore`). For each image or diagram it gets back
   `Pending`, `Ready { width, height }` (in cells), `Lines(..)` (text rendering) or
   `Failed(msg)`, and reserves rows accordingly. Unknown image keys are recorded in
   `MediaStore::missing`; after layout, `App` calls `take_missing()` and requests them.
3. **Async media completes later.** Worker threads send results over an mpsc channel;
   `App::tick` calls `MediaStore::poll()`, which returns `(relayout, redraw)`. A newly
   decoded image triggers a relayout (its size is now known). A newly encoded protocol only
   triggers a redraw.
4. **Images are drawn over blank rows.** Layout emits blank or bordered rows and pushes a
   `MediaPlacement { key, row, col, width, height }`. The viewport renders text lines first,
   then draws each visible placement with `ratatui-image`'s `SlicedImage`, which handles
   partially scrolled-off images. Placement areas show `░` until encoding finishes.

### Coordinate systems

- **Layout coordinates**: `(row, col)` where `row` indexes `Layout::lines` and `col` is a
  cell offset from the left edge of the document column. The hit map, search matches,
  selections and media placements all use these coordinates, so they are independent of
  scroll and screen position.
- **Screen coordinates**: crossterm `(x, y)`. Converted with `App::doc_pos(x, y)`, which
  subtracts `view.x + x_offset` and adds `scroll`.
- **Source lines**: 1-based markdown line numbers. Every `Block` carries `line`.
  `Layout::src_rows` maps rows ↔ source lines (`row_for_line`, `line_for_row`). This keeps
  the preview in sync with the editor cursor, and is used for `:42` and checkbox toggling.

### Media keys

All images and diagrams are identified by string keys:

- `img:<url>` for images (`layout::image_key`)
- `<handler>:<16-hex hash of content>` for custom fenced blocks (`layout::custom_key`), for
  example `mermaid:1a2b…`

Keys are the link between `Block::Custom`, `MediaStore` entries, `MediaPlacement`s and the
`Action`s that open, toggle or re-render a block. Because a custom key hashes the content, an
edited diagram gets a new key and is rendered again; the old entry is simply unused.

## mdtui-core

Headless document model and layout. Public surface is re-exported from `lib.rs`.

| Module | Responsibility |
|---|---|
| `parse.rs` | `parse(src, &ParseOptions) -> Document`. Lowers comrak's arena AST into the owned IR. `comrak_options()` sets the enabled GFM extensions (tables, tasklists, footnotes, alerts, math, shortcodes, wikilinks, spoilers, highlight, front matter, description lists). `ParseOptions::fence_handlers` lists fence languages that become `Block::Custom` instead of `CodeBlock` (the app adds `mermaid` plus every plugin fence). Uses `Slugger` to assign heading anchors. |
| `ir.rs` | The IR: `Document { front_matter, blocks }`, `Block` (Heading, Paragraph, List, CodeBlock, Quote, Alert, Table, Rule, Html, Math, FootnoteDef, DescriptionList, Custom, Lines) and `Inline` (Text, Code, Emph, Strong, Strike, Link, Image, FootnoteRef, Math, Html, breaks, Styled, …). All types are `serde` with `#[serde(tag = "type", rename_all = "snake_case")]`, **because this is the shape Lua plugins see**. Renaming a variant or field breaks plugins. Helpers: `headings()`, `walk_blocks(_mut)`, `image_urls()`, `plain_text()`. |
| `layout.rs` | `layout(doc, &LayoutOptions, &Theme, &dyn MediaProvider) -> Layout`. The largest core module. The internal `L` builder keeps a prefix stack (quote bars, list bullets and indents), a style stack, and pending-gap state. Each block type has a method (`heading`, `paragraph`, `list`, `code_block`, `custom`, `table`, `images`, …). Boxes with clickable buttons (`[copy]`, `[src]`, `[img]`/`[txt]`, `[⤢]`) are built by `box_top`/`box_bottom`. Also contains `strip_tags` and `html_img_srcs` for HTML `<img>` support. |
| `wrap.rs` (private) | Word wrapping of styled `Piece`s while preserving clickable ranges (`Wrapped { lines, hits }`). Adjacent fragments with no space between them stay together. |
| `hitmap.rs` | `Action` enum (OpenLink, ToggleTask{line,col}, CopyCode, OpenMedia, ToggleSource, ToggleRenderer, Footnote, Fold, FrontMatter) and `HitMap` (sorted `Hit`s with `at(row, col)`, `on_row`, `focusables()` for Tab cycling). |
| `theme.rs` | `Theme` with the full palette (Dark+ / Light+ modelled), `Theme::by_name`, style helpers, `alert()` icon and colour per alert kind, `parse_color` (used by plugins). |
| `highlight.rs` | syntect + two-face syntax sets, loaded lazily into statics. `highlight(code, lang, theme)` returns styled spans per line. `syntect_theme(theme)` builds a syntect theme so the editor matches the preview. |
| `search.rs` | `search(&Layout, query) -> Vec<Match>`. Smart case: case-insensitive unless the query has an uppercase letter. Matches are in layout coordinates. |
| `math.rs` | `to_unicode(tex)`: small TeX → unicode converter (Greek letters, operators, sub/superscripts, `\sqrt`, `\frac`). It is not a TeX engine. |
| `slug.rs` | GitHub-compatible heading anchors, with de-duplication (`-1`, `-2`). |
| `source.rs` | `toggle_task(src, line, col)`: flips `[ ]` ↔ `[x]` in the source text for checkbox clicks. |

Invariants enforced by tests:

- Every laid-out line fits within `LayoutOptions::width` (`tests/kitchen_sink.rs::every_line_fits_the_width`).
- Task-checkbox hits point at the exact `[ ]` position in the source.
- Snapshot output of `examples/kitchen-sink.md` at widths 40, 80 and 120 (`insta`). **Editing
  `kitchen-sink.md` or layout code changes the snapshots.** Review them with
  `cargo insta review`, or accept them with `INSTA_UPDATE=always cargo test`.

## mdtui-render

Turns a `Layout` into pixels and cells, and owns everything slow (I/O, decoding, subprocesses).

| Module | Responsibility |
|---|---|
| `viewport.rs` | `Viewport` widget: draws visible `Layout` lines at `scroll`, overlays search matches, the focused link (every row of a wrapped link), and the mouse selection, then draws image placements. `Selection` (normalised range plus `text(&Layout)` for copying). |
| `media.rs` | `MediaStore`: the async store and `MediaProvider` implementation. Entries move `Loading → Image{img, protos} / Text / Lines / Failed`. Encoded protocols are cached per `(width, height)` in cells. `request_image(url)` spawns load+decode on a thread. `request_mermaid(key, src, mode)` renders native text diagrams synchronously when possible, otherwise spawns `mmdc`. `ensure_protocols(placements)` encodes protocols for visible images off-thread. `fullscreen(...)` renders the zoom/pan view for the fullscreen overlay. `Renderer` = `Auto \| Text \| Image`. Plugin results are injected with `set_lines` / `set_image_bytes` / `set_failed`. |
| `fetch.rs` | `load_bytes(url, base_dir)` handles local paths (relative to the document), `file://`, `http(s)://` via `ureq`, and `data:` URIs. `decode(bytes, svg_px)` handles raster formats via `image` and SVG via `resvg` (with a lazily loaded system font database). |
| `mermaid.rs` | External mermaid rendering. `mmdc` → PNG, cached in `~/.cache/mdtui/mermaid/<hash>.png`. A global mutex allows only one Chromium at a time. Retries with `--no-sandbox` when Chromium's sandbox is blocked (Ubuntu AppArmor). Falls back to `mermaid-ascii` text. |
| `diagram/mod.rs` | Native text renderers: `supported(src)`, `render(src, &Palette) -> TextDiagram { lines, narrow }`. Pie charts are drawn here as bar charts. `Palette::from_theme`. |
| `diagram/canvas.rs` | Character grid where line segments are direction bitmasks (UP/DOWN/LEFT/RIGHT), so crossings merge into the right box-drawing glyph. Supports solid, dashed and thick strokes. Text cells are never overwritten by lines. |
| `diagram/flowchart.rs` | Mermaid `graph`/`flowchart` parser (node shapes, chained edges, `&` groups, edge labels, link styles) and `stateDiagram` → graph translation. Layered (Sugiyama-style) layout: break cycles with DFS, assign layers by longest path, order nodes to reduce crossings, then route edges orthogonally. For LR/RL graphs, a TD variant (`narrow`) is used when the wide layout doesn't fit. |
| `diagram/sequence.rs` | Sequence diagrams: participants, lifelines, arrows with labels, notes, and loop/alt/opt frames. |
| `lib.rs` | `find_in_path`, `cache_dir(sub)`, `hash_str`. |

Try the text diagram renderers without the TUI:
`cargo run -p mdtui-render --example diagrams [file.mmd]`.

Mermaid renderer selection (`request_mermaid`):

| Mode | Natively supported type | Unsupported type |
|---|---|---|
| `Auto` | native text | `mmdc` image (then native text if mmdc fails) |
| `Text` | native text | `mermaid-ascii` only (mmdc disabled) |
| `Image` | `mmdc` image | `mmdc` image |

If images are disabled, every mode behaves like `Text`. A per-diagram override from the
`[img]`/`[txt]` button is stored in `App::renderer_override`.

## mdtui-plugin

Lua 5.4 host built on `mlua` (vendored). The plugin API is defined in `src/prelude.lua`,
which is loaded before any plugin and returns the `mdtui` table. Users get it with
`require("mdtui")`.

- **Loading**: `PluginHost::load_dir` loads every `*.lua` file and every `dir/init.lua`.
  Each plugin runs in **its own environment table**, so globals don't leak between plugins.
  Failures are returned as messages, not panics.
- **Sandbox**: untrusted (the default) gets only `table`, `string`, `math`, `utf8` and
  `coroutine`. `dofile`/`loadfile` are removed and `mdtui.exec` raises an error.
  `[plugins] trusted = true` enables the safe stdlib (`io`, `os`, `package`) and
  `mdtui.exec(argv, stdin)`, which runs a subprocess and returns stdout plus stderr on
  failure.
- **Extension points** (prelude function → Rust entry point):

| Lua API | Rust | Used by |
|---|---|---|
| `register_fence(lang, fn)` | `fence_names()`, `render_fence()` → `FenceOutput::{Image, Lines, Markdown}` | `App::rebuild` |
| `on("ast", fn)`, `register_inline(pattern, fn)` | `transform(Document) -> Document` (via serde ↔ Lua tables) | `App::rebuild` |
| `register_completion{trigger, complete}` | `complete(before_cursor) -> Vec<CompletionSet>` | editor completion |
| `command(name, fn)` | `commands()`, `run_command(name, args, path, text) -> Option<new text>` | `:Name` ex commands |
| `keymap(mode, lhs, rhs)` | `keymaps()` | merged into the viewer `Keymap` |
| `notify(msg, level)` | `take_notifications()` | app notifications |

- Fence handlers that return markdown are parsed and spliced into the document in place of
  the `Custom` block (`splice_markdown` in `app.rs`).
- Errors are shortened to their first line (`short_err`) and shown as notifications. A failed
  fence falls back to showing its source.
- Example plugins with an API summary: `examples/plugins/` (run with
  `MDTUI_CONFIG_DIR=examples cargo run -- examples/plugins/README.md`).

## mdtui-editor

The editor pane shown by `e` / `:vsplit` / `-e`.

| Module | Responsibility |
|---|---|
| `lib.rs` | `Editor`: wraps edtui's `EditorState` and `EditorEventHandler`. Tracks the `saved` text to compute `is_dirty()`, and caches the buffer string (`text()`). `key()`/`paste()` return whether the text changed. `insert_completion(replace_chars, text)` expands `$0` cursor markers and indents multi-line snippets. `render` draws a 2-column sign gutter for LSP diagnostics (E/W/I) plus edtui with markdown syntax highlighting themed from `Theme`. `set_text` (external reload, marks clean) vs `replace_text` (an edit, marks dirty). |
| `completion.rs` | Built-in sources, chosen by the text before the cursor (`builtin(&Ctx)`): file paths after `](` (relative to the document's directory), `#anchor`s from headings, `[ref]` labels, `:emoji:` shortcodes, code-fence languages after <code>```</code>, and snippets (`table`, `mermaid`, `note`, …). `Popup` is the VS Code-style list widget. |
| `lsp.rs` | Minimal JSON-RPC-over-stdio LSP client, designed for `marksman`. A reader thread parses `Content-Length` frames into a channel. `poll()` yields `LspEvent::{Ready, Completion, Diagnostics, Exited}`. Supports `didOpen/didChange/didSave` and `textDocument/completion`. Columns are converted to UTF-16 (`utf16_col`). The child process is killed on `Drop`. |

## mdtui (binary)

| Module | Responsibility |
|---|---|
| `main.rs` | CLI (`clap`): `file` (`-` or piped stdin supported; defaults to README.md or index.md in the current directory), `--theme`, `-e/--edit`, `--toc`, `--no-images`, `--no-plugins`, `--protocol`, `--config`, `--dump [--width N --color auto\|always\|never]`, `--print-config`. Detects the image protocol with `Picker::from_query_stdio()`, then drains late terminal query replies so they aren't read as keys. Runs the main loop: `tick()` → draw if `dirty` → poll events (30 ms) → `input::handle`. `--dump` lays out without a TUI, waits up to 20 s for pending diagrams, and prints ANSI. Set `MDTUI_DEBUG_EVENTS=<file>` to log every event with timestamps. |
| `app.rs` | `App`: all state, independent of drawing. Document state (`source`, `doc`, `layout`, `media`, `plugins`), view state (`scroll`, `view`, `x_offset`, folds, `show_source`, focus, search, selection), panes (`Pane::{Preview, Editor, Toc}`), input mode (`Input::{None, Command, Search}`), editor + LSP, fullscreen overlay (`Fullscreen { key, zoom, pan, drag }`), notifications, navigation history (`H`/`L`), file watcher. Important methods: `rebuild`, `relayout_if_needed`, `tick` (media poll, debounced reload, debounced editor→preview rebuild, cursor sync, LSP events, key-sequence timeout, notification expiry), `run(Cmd)`, `activate(Action)`, `ex(line)` (ex commands; built-ins listed in `BUILTIN_COMMANDS`, anything else goes to plugins), `open_link` (anchors, relative `.md` files opened in place, other URLs via `open`), `toggle_task`, `reload`, `open_file`, mouse helpers (`preview_press/drag/release`, `doc_pos`). |
| `ui.rs` | Drawing only: title bar, outline sidebar, editor │ preview split with a draggable `┃` divider, scrollbar, completion popup, status line (mode badge, breadcrumb, percentage, line), command line with wildmenu, notifications, help overlay (`HELP` table), fullscreen media view. `draw` also writes layout geometry back into `App` (`view`, `toc_area`, `editor_area`, `divider_x`, `fs_body`, `fs_buttons`), which mouse handling then uses. |
| `input.rs` | Event routing. Priority: help overlay → fullscreen → command/search line → focused pane (editor / outline / viewer). The viewer feeds keys into `Keymap`. The editor intercepts completion keys, `:`-commands in normal mode and `Esc`, and calls `App::editor_changed` on edits. Mouse: divider drag, wheel scroll, click (hit map) vs drag (text selection, copied on release), outline clicks. |
| `keys.rs` | Vim key notation (`gg`, `<C-d>`, `<leader>wc`, `<CR>`), the `Cmd` enum of viewer actions, and `Keymap` with multi-key sequence matching and a timeout for ambiguous prefixes. Default bindings, `[keys]` from config, and plugin keymaps are merged here. |
| `config.rs` | `Config` (serde, every field defaulted): theme, max_width, toc, scroll_step, code_line_numbers, leader, `[images]`, `[mermaid]`, `[editor]`, `[plugins]`, `[keys]`. `config_dir()` checks `MDTUI_CONFIG_DIR`, then `$XDG_CONFIG_HOME/mdtui`, then `~/.config/mdtui`. `EXAMPLE` is the text printed by `--print-config`; keep it in sync with the struct. |
| `util.rs` | `copy_to_clipboard`: tries the tmux buffer, then wl-copy/xclip/xsel/pbcopy/clip.exe, and **always** also emits OSC 52. `lines_to_ansi` converts styled lines to SGR escape codes for `--dump`. |

### Event and state flow

```
crossterm Event ─► input::handle ─► App::run(Cmd) / App::activate(Action) / App::ex(":…")
                                         │ mutates state, sets dirty / layout_dirty
main loop ─► App::tick ──────────────────┤ media poll, watcher, editor debounce, LSP
          └► ui::draw (if dirty) ────────┘ relayout_if_needed → prepare_media → Viewport
```

- `dirty` means "redraw". `layout_dirty` means "run layout() before the next draw". Set them
  with `invalidate_layout()` rather than calling `layout()` directly.
- Live reload: `notify` watches the file's **parent directory** (non-recursive) so that
  editors which save by renaming still trigger it. Events are debounced 80 ms. On reload, the
  scroll position is kept relative to the nearest heading anchor. Unsaved editor changes block
  reload unless you use `:e!`.
- Editor → preview: an edit sets `preview_dirty_at`, and 120 ms later the source is taken from
  the editor and rebuilt. When the cursor row changes, the preview scrolls so that row is a
  third of the way down.
- Checkbox clicks rewrite the source with `source::toggle_task`. This goes into the editor
  buffer if one is open, otherwise straight to disk.

## Where to make common changes

| Task | Touch |
|---|---|
| New markdown construct | `parse.rs` (lowering) → `ir.rs` (variant, serde-compatible) → `layout.rs` (rendering) → `kitchen-sink.md` + snapshots |
| New clickable thing | add an `Action` variant in `hitmap.rs`, emit hits in `layout.rs`, handle it in `App::activate`, describe it in `ui.rs::describe` |
| New viewer key | `Cmd` + default binding in `keys.rs`, behaviour in `App::run`, entry in `ui.rs::HELP` |
| New ex command | `App::ex` and `BUILTIN_COMMANDS` (used for Tab completion) |
| New config option | `config.rs` struct + `Default` + `EXAMPLE`; read it where it is used |
| New mermaid diagram type (text) | `diagram/` module + `supported()` / `render()` in `diagram/mod.rs` |
| New image source | `fetch.rs::load_bytes` / `decode` |
| New plugin API | `prelude.lua` (Lua side) + `PluginHost` method + call site in `app.rs`; update `examples/plugins/README.md` |
| New completion source | `completion.rs::builtin` |
| Theme colour | `Theme` fields in `theme.rs` (both `dark()` and `light()`) |

## Build, test, run

```sh
cargo build --release
cargo test --workspace                       # unit tests + kitchen-sink snapshots
cargo run -- examples/kitchen-sink.md        # the feature showcase
cargo run -- --dump --width 80 examples/kitchen-sink.md   # headless render to stdout
MDTUI_CONFIG_DIR=examples cargo run -- examples/plugins/README.md   # with example plugins
cargo run -p mdtui-render --example diagrams # text mermaid renderer samples
```

There is no CI. Run `cargo test --workspace` before pushing.

Runtime files:

- config: `~/.config/mdtui/config.toml`, plugins in `~/.config/mdtui/plugins/`
- cache: `~/.cache/mdtui/mermaid/` (mmdc PNGs and their `.mmd` inputs)
- optional external tools: `mmdc` (mermaid-cli), `mermaid-ascii`, `marksman` (LSP), and
  clipboard helpers

## Conventions

- Keep `mdtui-core` pure: it takes data and returns data, with no terminal or process I/O.
  Anything slow or effectful goes in `mdtui-render` (threads) or the binary.
- Never block the UI thread on I/O. Spawn a worker, send results over a channel, and pick them
  up in `tick()`. The exceptions are native text diagrams (fast) and Lua fences (synchronous,
  run during `rebuild`).
- Widths are terminal cells (`unicode-width`), not bytes or chars. LSP columns are UTF-16.
  Editor columns are chars.
- Source line numbers are 1-based. Editor rows and LSP lines are 0-based.
- Errors that reach the user become notifications (`App::info/warn/error`). Avoid panics on
  user input or plugin output.
- Module-level `//!` doc comments describe each file's role. Keep them accurate when changing
  responsibilities.

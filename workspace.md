# mdtui workspace guide

This document describes how the mdtui repository is organised, how data flows through it, and
what each module is responsible for. It is written for agents and contributors who need to find
the right place to make a change.

mdtui is a terminal markdown viewer with an optional vim-style editor split. It is a Rust
workspace (edition 2021, stable toolchain) built on [ratatui](https://ratatui.rs) and
[comrak](https://github.com/kivikakk/comrak). It is about 11k lines of Rust across five crates.

## Repository map

```
.
├── Cargo.toml              workspace manifest: members, shared versions, all dependency versions
├── rust-toolchain.toml     channel = "stable"
├── install.sh              installs mermaid-cli (npm, needs Node ≥ 22.13) + an mmdc launcher,
│                           smoke-tests mmdc, then builds and installs mdtui into $PREFIX/bin
│                           (default ~/.local/bin)
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
   │  splice_markdown()               mdtui/app.rs          (markdown-returning fences)
   ▼
Document (final)
   │  layout(doc, opts, theme, &dyn MediaProvider)          mdtui-core/layout.rs
   ▼
Layout { width, lines, hits, media (placements), headings, anchors, footnotes, src_rows }
   │  Viewport widget                 mdtui-render/viewport.rs
   ▼
terminal buffer (+ images drawn over reserved rows)
```

Key points:

1. **Parse once per source change, lay out once per width/option change.**
   - `App::rebuild` re-parses. It runs:
     - at startup;
     - on file reloads: the watcher (debounced 80 ms), `r`/`:reload`, `:e!`, and closing the
       editor. All of these go through `App::reload`, which returns early without rebuilding
       or relaying out in three cases:
       - there is no path (stdin mode);
       - the file can't be read;
       - an open editor's buffer already equals the file on disk.

       A watcher reload is also skipped when the text on disk is unchanged, or while the editor
       has unsaved edits. Note that `r`/`:reload` clears `MediaStore`, `markdown_fences` and
       `fence_errors` *before* calling `reload`. So in those early-return cases the old layout
       keeps pointing at cleared keys:
       - images show `░` until the next layout re-requests them;
       - mermaid and plugin-fence keys are not re-requested until the next rebuild.
     - when another file is opened: links, `:e <file>`, back/forward;
     - on editor edits (debounced 120 ms);
     - on checkbox toggles, and on plugin `:Commands` that rewrite the text;
     - on `[img]`/`[txt]` toggles, `:theme` and `:set images|noimages|mermaid=`.

     Plugins are loaded once in `App::new` and are never reloaded.
   - `App::relayout_if_needed` runs `layout()` when `layout_dirty` is set. That happens:
     - after every rebuild;
     - when the preview area changes size, which `ui::draw` detects: a terminal resize, the
       TOC toggle, opening or closing the editor, or dragging the divider;
     - on fold/unfold (including fold-all and unfold-all), `[src]` toggles, the front-matter
       toggle and any `:set` option;
     - when `MediaStore::poll` reports a relayout.
2. **Layout asks the media provider how big things are.** `layout()` takes a
   `&dyn MediaProvider` (implemented by `MediaStore`). Some blocks never query the provider,
   so their images are never loaded:
   - images in table cells, which `table()` renders as plain `🖼 alt` text;
   - images in description-list terms and plugin `Lines` blocks, which are dropped;
   - diagrams toggled to `[src]`.

   For every other image or diagram it gets back one of four answers, and only `Ready`
   reserves blank rows for a placement:
   - `Ready { width, height }`: in cells, fitted to the available width and to the
     `[images] max_height` row limit.
   - `Lines(..)`: drawn as a boxed text block, which is clipped and never wrapped.
   - `Pending` or `Failed(msg)`:
     - An image becomes a single clickable `🖼 alt (loading…)` or `🖼 alt (<error>)` line.
     - A diagram becomes a `⟳ rendering … diagram…` or `⚠ handler: msg` line, followed by its
       source as a code block. So the block changes height when the result arrives.

   Any key the store has never seen is pushed to `MediaStore::missing` and reported as
   `Pending`. After layout, `App` calls `take_missing()` and then `request_image` for the
   `img:` keys. Custom-fence keys are requested in `rebuild`.
3. **Async media completes later.** Worker threads send results over an mpsc channel.
   `App::tick` calls `MediaStore::poll()`, which returns `(relayout, redraw)`.
   - Any finished load or render triggers a relayout, because the block's size or shape has
     changed. That covers a decoded image or mmdc PNG (`Msg::Loaded(Ok)`), a failure
     (`Msg::Loaded(Err)`), and text lines from mermaid-ascii or the native fallback
     (`Msg::Lines`).
   - A newly encoded protocol (`Msg::Encoded`) only triggers a redraw.
4. **Images are drawn over blank rows.** Layout emits blank or bordered rows and pushes a
   `MediaPlacement { key, kind, row, col, width, height }`, where `kind` is
   `MediaKind::Image` or `MediaKind::Diagram`.
   - The viewport renders text lines first. It then draws each visible placement with
     `ratatui-image`'s `SlicedImage`, which handles partially scrolled-off images.
   - Placement areas show `░` until a protocol for that exact size is ready. If encoding fails
     they stay `░`, because failed encodes are not retried.

### Coordinate systems

- **Layout coordinates**: `(row, col)` where `row` indexes `Layout::lines` and `col` is a
  cell offset from the left edge of the document column. The hit map, search matches,
  selections and media placements all use these coordinates, so they are independent of
  scroll and screen position.
- **Screen coordinates**: crossterm `(x, y)`. Converted with
  `App::doc_pos(x, y) -> Option<(row, col)>`, where `row = scroll + (y - view.y)` and
  `col = (x - view.x) - x_offset`. It returns `None` outside the preview area or in the left
  margin.
- **Source lines**: 1-based markdown line numbers. Every `Block` (and `ListItem`) carries a
  `line`, with two exceptions:
  - Blocks created by plugin `on("ast")` hooks have `line` 0, and are skipped.
  - Blocks spliced in from markdown-returning plugin fences keep line numbers relative to the
    plugin's output; they are not rewritten.

  `Layout::src_rows` records `(line, row)` for each block or list-item start with
  `line > 0`, and maps rows ↔ source lines (`row_for_line`, `line_for_row`). It is used for:
  - keeping the preview in sync with the editor cursor;
  - `:42`;
  - placing the editor cursor when the editor opens or a TOC entry is chosen;
  - the `ln N/M` status indicator.

  Checkbox toggling does not go through `src_rows`. `Action::ToggleTask { line, col }`
  carries the checkbox's exact source position from `ListItem::task_pos`.

### Media keys

All images and diagrams are identified by string keys:

- `img:<url>` for images (`layout::image_key`), with the URL exactly as written in the
  markdown
- `<handler>:<16-hex hash of content>` for custom fenced blocks (`layout::custom_key`), for
  example `mermaid:1a2b…`

Keys are the link between `Block::Custom`, `MediaStore` entries, `MediaPlacement`s and the
`Action`s that open, toggle or re-render a block. Because a custom key hashes the content, an
edited diagram gets a new key and is rendered again; the old entry is simply unused.

Cache lifetime:

- Keys include neither the width, nor the theme, nor the base directory.
- `rebuild` skips any key already in `MediaStore` or in `App::markdown_fences`. So a plugin
  fence is rendered once, with the `FenceCtx.width` current at that moment, and is not
  re-rendered on resize.
- For the initial document, that width is the 20-column floor of `content_width()`. The first
  `rebuild` runs in `App::new` before `view` has been set.

Store entries are dropped only by:

- `MediaStore::clear()`, called by `:theme`, `:set images|noimages|mermaid=` and
  `r`/`:reload`;
- `forget(key)`, called by the `[img]`/`[txt]` toggle;
- `set_base_dir`, which runs when a file in another directory is opened. It drops every
  `img:` key whose URL has no `://`: relative paths, absolute paths and `data:` URIs.
  `http(s)://` and `file://` keys are kept.

Markdown-returning fences live in `App::markdown_fences`, which only `r`/`:reload` clears.

## mdtui-core

Headless document model and layout. Every module except `wrap` is `pub`. `lib.rs` re-exports
the main types:

- `Document`, `Block`, `Inline`
- `parse`, `ParseOptions`
- `layout`, `Layout`, `LayoutOptions`
- `MediaInfo`, `MediaKind`, `MediaPlacement`, `MediaProvider`
- `Action`, `Hit`, `HitMap`
- `Theme`

Everything else is reached by module path, for example
`mdtui_core::layout::{custom_key, image_key, NoMedia}`, `mdtui_core::source::toggle_task` and
`mdtui_core::highlight::languages`.

| Module | Responsibility |
|---|---|
| `parse.rs` | `parse(src, &ParseOptions) -> Document`. Lowers comrak's arena AST into the owned IR, and uses `Slugger` to assign heading anchors.<br><br>`comrak_options()` enables strikethrough, tables, autolinks (relaxed), tasklists (also inside tables), footnotes, description lists, `---` front matter, multiline (`>>>`) block quotes, alerts, math (`$…$` and code math), shortcodes, wikilinks (title after pipe), spoilers and highlight. Superscript, subscript and underline are deliberately off, so the parser never emits `Inline::Superscript`/`Subscript`/`Underline`; only plugins produce them.<br><br>`ParseOptions::fence_handlers` lists fence languages that become `Block::Custom` instead of `CodeBlock`. The app adds `mermaid` plus every plugin fence. A fence tagged `math` is checked first and always becomes `Block::Math`, so a handler cannot claim it. |
| `ir.rs` | The IR: `Document { front_matter, blocks }`, `Block` (Heading, Paragraph, List, CodeBlock, Quote, Alert, Table, Rule, Html, Math, FootnoteDef, DescriptionList, Custom, Lines) and `Inline` (Text, Code, Emph, Strong, Strike, Link, Image, FootnoteRef, Math, Html, breaks, Styled, …).<br><br>All document types derive `serde`, **because this is the shape Lua plugins see**. Renaming a variant or field breaks plugins.<br><br>- `Block` and `Inline` are internally tagged with `#[serde(tag = "type", rename_all = "snake_case")]`, e.g. `{type = "code_block", lang = …, code = …}`.<br>- `AlertKind` and `Align` are plain lowercase strings.<br>- `Document`, `ListItem` and `DescItem` are untagged tables.<br>- Many fields (`line`, `anchor`, `lang`, `title`, `alt`, `ix`, …) are `#[serde(default)]` so plugins can omit them, and `List.start` defaults to 1.<br><br>Helpers: `headings()`, `walk_blocks(_mut)`, `image_urls()`, `plain_text()`, `Inline::children()`. |
| `layout.rs` | `layout(doc, &LayoutOptions, &Theme, &dyn MediaProvider) -> Layout`. The largest core module. The internal `L` builder keeps a prefix stack (quote bars, list bullets and indents), a style stack, and pending-gap state.<br><br>`block()` dispatches on the block type. Most types have their own method: `heading`, `paragraph`, `list`, `code_block`, `custom`, `table`, `html_block`, `math_block`, `footnote_def`. Quotes, alerts, rules, description lists and `Lines` are handled inline in `block()`. `images` lays out the inline images collected from a block in a left-to-right flow, so badge rows stay on one line.<br><br>Boxes and their buttons:<br>- Code blocks are drawn by `code_block`, which builds its own border with a `[copy]` button.<br>- A custom block toggled to show its source is drawn by `code_block` with an extra `[render]` button.<br>- Rendered custom blocks (`MediaInfo::Ready`/`Lines`) use `box_top`/`box_bottom`, with `[src]` and `[⤢]`. Mermaid blocks also get `[txt]` when an image is shown, or `[img]` when text is shown.<br>- Pending or failed custom blocks print a status line and fall back to a plain `code_block`.<br><br>Also contains `strip_tags` and `html_img_srcs` for HTML `<img>` support. |
| `wrap.rs` (private) | Word wrapping of styled `Piece`s while preserving clickable ranges (`Wrapped { lines, hits }`). Adjacent fragments with no space between them stay together. |
| `hitmap.rs` | `Action` enum (OpenLink, ToggleTask{line,col}, CopyCode, OpenMedia, ToggleSource, ToggleRenderer, Footnote, Fold, FrontMatter) and `HitMap`, which holds sorted `Hit`s and provides `at(row, col)`, `on_row`, and `focusables()` for Tab cycling. |
| `theme.rs` | `Theme` with the full palette (modelled on Dark+ / Light+), `Theme::by_name`, style helpers, `alert()` (icon and colour per alert kind), and `parse_color` (used by plugins). |
| `highlight.rs` | syntect with the two-face syntax and theme sets, loaded lazily into statics.<br>- `highlight(code, lang, theme)` returns styled spans per line, or plain `fg` text for unknown languages.<br>- `syntect_theme(theme)` returns (clones) the two-face theme named by `Theme::syntax_theme` (OneHalfDark / InspiredGithub), so the editor matches the preview.<br>- `languages()` lists the fence language tokens, plus `mermaid`, `math` and `text`. The editor uses it to complete languages after <code>```</code>. |
| `search.rs` | `search(&Layout, query) -> Vec<Match>`. Smart case: case-insensitive unless the query has an uppercase letter. Matches are in layout coordinates. |
| `math.rs` | `to_unicode(tex)`: small TeX → unicode converter (Greek letters, operators, sub/superscripts, `\sqrt`, `\frac`). It is not a TeX engine. |
| `slug.rs` | GitHub-compatible heading anchors, with de-duplication (`-1`, `-2`). |
| `source.rs` | `toggle_task(src, line, col)`: flips `[ ]` ↔ `[x]` in the source text for checkbox clicks. |

What the tests check:

- **Line width.** At widths 40, 80 and 120, every line of the kitchen-sink layout fits within
  `LayoutOptions::width` (`tests/kitchen_sink.rs::every_line_fits_the_width`). The test uses
  `NoMedia` and trims trailing spaces.

  This is not a general guarantee. `avail()` never drops below 8 columns and table columns
  stop shrinking at 6, so very narrow widths, deep nesting, or tables with many columns can
  still overflow.
- **Checkbox hits.** Every ☐/☑ glyph has a `ToggleTask { line, col }` hit pointing at the
  checkbox symbol in the source: the space or `x` just after `[`, as a 1-based line and a
  1-based byte column. This is the position `source::toggle_task` expects; it rejects
  positions not preceded by `[`.

  Covered by `tests/kitchen_sink.rs::clickable_targets` and
  `layout.rs::tests::task_hits_point_at_source`. `clickable_targets` also checks that the
  contents-list anchors, the `#tasks` link, a `[copy]` hit and the `speed` footnote are
  present.
- **Snapshots.** There are two sets of `insta` snapshots:
  - `examples/kitchen-sink.md` at widths 40, 80 and 120, from `tests/kitchen_sink.rs`, stored
    in `tests/snapshots/`, with trailing spaces trimmed;
  - a small inline document at width 40, from `layout.rs::tests::snapshot_kitchen`, stored in
    `src/snapshots/`.

  Snapshots are plain text from `Layout::plain_line`, so styles and colours are dropped. They
  are rendered with `NoMedia`, so images and mermaid blocks always show their "images
  disabled" fallback.

  **Editing `kitchen-sink.md` or layout code changes the snapshots.** Review them with
  `cargo insta review`, or accept them with `INSTA_UPDATE=always cargo test`.

## mdtui-render

Turns a `Layout` into pixels and cells. It owns the slow media work on worker threads: image
fetching and decoding, mermaid subprocesses, and protocol encoding. Other subprocesses live
elsewhere: the LSP client in `mdtui-editor`, `mdtui.exec` in `mdtui-plugin`, and in the
binary, the clipboard helpers and the `open::that_detached` link/file opener. The binary's
file watcher is an in-process `notify` watcher, not a subprocess.

| Module | Responsibility |
|---|---|
| `viewport.rs` | `Viewport` widget: draws the visible `Layout` lines at `scroll`, then overlays:<br>- search matches;<br>- the focused hit (a link, checkbox, button, media block or footnote ref). Every hit with the same action within ±3 rows is highlighted too, so a wrapped link lights up on each of its rows;<br>- the mouse selection.<br><br>It then draws image placements. Also defines `Selection` (a normalised range, plus `text(&Layout)` for copying). |
| `media.rs` | `MediaStore`: the async store and `MediaProvider` implementation.<br><br>Entry states:<br>- `Loading`<br>- `Image{img, protos}`, with encoded protocols cached per `(width, height)` in cells<br>- `Text{lines, narrow}`, only for synchronous native diagrams<br>- `Lines`: plugin text, `mermaid-ascii` output, or the worker's native fallback (with no narrow variant)<br>- `Failed`<br><br>Only worker jobs pass through `Loading`; `set_*` results and `Failed("images disabled")` are inserted directly.<br><br>Methods:<br>- `request_image(url)` spawns a thread that loads and decodes the image.<br>- `request_mermaid(key, src, mode)` renders native text diagrams synchronously when possible. Otherwise it marks the key `Loading` and spawns a worker. The worker runs `mermaid::render`, which checks the PNG cache, then runs `mmdc` (unless disabled), then `mermaid-ascii`. If all of that fails, the worker falls back to the native renderer.<br>- `ensure_protocols(placements)` encodes protocols for visible images off-thread.<br>- `fullscreen(...)` renders the zoom/pan view for the fullscreen overlay.<br>- Plugin results are injected with `set_lines` / `set_image_bytes` / `set_failed`.<br><br>`Renderer` is `Auto \| Text \| Image`. |
| `fetch.rs` | `load_bytes(url, base_dir)` handles:<br>- local paths, relative to the document and percent-decoded;<br>- `file://`;<br>- `http(s)://` via `ureq`, with a 64 MiB limit. Downloads are cached permanently in `cache_dir("img")/<hash of url>` and never re-fetched;<br>- `data:` URIs.<br><br>`decode(bytes, svg_px)` handles raster formats via `image`, and SVG via `resvg` with a lazily loaded system font database. |
| `mermaid.rs` | External mermaid rendering. `mmdc` writes a PNG to `cache_dir("mermaid")/<hash>.png` (`~/.cache/mdtui/mermaid/` on Linux). The hash covers source, theme, background and width, and the cache is checked before anything else.<br><br>- A global mutex allows only one Chromium at a time.<br>- If mmdc's error mentions the sandbox (Ubuntu AppArmor), it retries with a generated `puppeteer-no-sandbox.json` (`--no-sandbox`). This needs `[mermaid] no_sandbox_fallback` on (the default) and no `puppeteer_config` set.<br>- The width flag (`-w`, or `--size` on mermaid-cli 12+) is detected once from `mmdc --help`.<br>- If mmdc is missing or fails, it falls back to `mermaid-ascii` text. |
| `diagram/mod.rs` | Native text renderers for `graph`/`flowchart`, `sequenceDiagram`, `stateDiagram(-v2)` and `pie`. The type is detected by the first keyword after front matter and `%%` comments.<br>- `supported(src)`<br>- `render(src, &Palette) -> Result<TextDiagram, String>`, which returns `TextDiagram { lines, narrow }`, or `Err` on a parse failure or an unsupported type. `narrow` is the top-down re-layout of LR/RL flowchart and state graphs<br>- `width(lines)`, which `MediaStore` uses to choose between the wide and narrow variants<br><br>Pie charts are drawn here as bar charts. Also defines `Palette::from_theme`. |
| `diagram/canvas.rs` | Character grid where line segments are direction bitmasks (UP/DOWN/LEFT/RIGHT), so crossings merge into the right box-drawing glyph. Supports solid, dashed and thick strokes. Text cells are never overwritten by lines. |
| `diagram/flowchart.rs` | Mermaid `graph`/`flowchart` parser. Supports node shapes, chained edges, `&` groups, edge labels, and link types: solid, dotted, thick or invisible, with arrow, circle or cross markers at either end. `subgraph`s are flattened, and `style`/`classDef`/`class`/`linkStyle`/`click`/`:::class` are ignored.<br><br>Also translates `stateDiagram` into the same graph.<br><br>Layout is layered (Sugiyama-style): break cycles with DFS, assign layers by longest path, order the nodes within each layer, then route edges orthogonally. For LR/RL graphs, a top-down variant (`narrow`) is used when the wide layout doesn't fit. |
| `diagram/sequence.rs` | Sequence diagrams:<br>- participants and actors, with lifelines;<br>- labelled arrows (`->>`, `->`, `-x`, `-)` and their dashed `--` forms) and self-messages;<br>- `autonumber`, `title` and notes;<br>- loop/alt/opt/par/critical/break/rect frames, with nested frames indented and else/and/option dividers.<br><br>`activate`/`deactivate`/`create`/`destroy`/`box`/`link(s)` lines are ignored. |
| `lib.rs` | `find_in_path`, `cache_dir(sub)`, `hash_str`. |

Try the text diagram renderers without the TUI:
`cargo run -p mdtui-render --example diagrams [file.mmd]`.

Mermaid renderer selection (`request_mermaid`):

| Mode | Natively supported type | Unsupported type |
|---|---|---|
| `Auto` | native text (if the native parser errors: `mmdc` image, then `mermaid-ascii`) | `mmdc` image, then `mermaid-ascii` text |
| `Text` | native text (a parse error is shown as a failure) | `mermaid-ascii` only (mmdc disabled) |
| `Image` | `mmdc` image, then `mermaid-ascii`, then native text | `mmdc` image, then `mermaid-ascii` |

If images are disabled, every mode behaves like `Text`, with two exceptions:

- A native parse error in `Auto` or `Image` mode still goes to the worker, which can only use
  `mermaid-ascii`.
- `mermaid::render` checks its PNG cache before anything else. So when a request reaches the
  worker, a previously cached mmdc image (same source, theme, background and width) is shown
  even with images off or in `Text` mode. That happens for an unsupported diagram type, or,
  with images off, for a native parse error in `Auto`/`Image` mode. Supported diagrams that
  parse natively are drawn as text synchronously and never consult the cache.

A per-diagram override from the `[img]`/`[txt]` button is stored in
`App::renderer_override`.

## mdtui-plugin

Lua 5.4 host built on `mlua` (vendored).

The Lua side of the plugin API is defined in `src/prelude.lua`, which is evaluated before any
plugin and returns the `mdtui` table. `PluginHost::new` then adds `mdtui.exec` and
`mdtui.trusted` from Rust. It also replaces the global `require` with a shim that returns the
`mdtui` table for `"mdtui"`; that is how `require("mdtui")` works even without the `package`
library.

- **Loading**: plugins are loaded once, at startup, by `App::load_plugins`. This is skipped
  if `[plugins] enabled = false` or `--no-plugins` is given.
  - It creates `PluginHost::new(trusted)` and calls `load_dir(cfg.plugin_dir())`. The
    directory is `[plugins] dir`, by default `<config_dir>/plugins`; a missing directory is
    ignored.
  - `load_dir` loads every `*.lua` file and every `dir/init.lua` in sorted path order. That is
    also the order in which `on("ast")` hooks run. The plugin name is the file stem or
    directory name.
  - Inline patterns don't run in load order. They are all applied in a single pass, before
    any ast hook runs:
    - At each point in a text node, the leftmost match of any pattern wins.
    - Load/registration order only breaks ties at the same start position.
    - Replaced text is not rescanned by any pattern.
  - Each plugin runs in **its own environment table**, so globals don't leak between plugins.
    Failures are returned as messages, not panics.
  - Lua code is never reloaded at runtime. `r`/`:reload` re-reads the document and clears the
    fence caches, but plugin changes need a restart.
- **Sandbox**: untrusted plugins (the default) get the Lua base library, which mlua always
  loads (including `load`, `pcall`, `setmetatable` and `print`). On top of that they get only
  `table`, `string`, `math`, `utf8` and `coroutine`.
  - `dofile` and `loadfile` are removed, `require` resolves only `"mdtui"`, and `mdtui.exec`
    raises an error.
  - `[plugins] trusted = true` enables the safe stdlib (`io`, `os`, `package`) and
    `mdtui.exec(argv, stdin)`. That runs a subprocess and returns stdout, plus stderr on
    failure.
- **Extension points**:

| Lua API | Rust | Used by |
|---|---|---|
| `register_fence(lang, fn)` | `fence_names()`, `render_fence(lang, src, &FenceCtx)` → `FenceOutput::{Image, Lines, Markdown}` | `App::rebuild`. See "How fence output is handled" below. |
| `on("ast", fn)`, `register_inline(pattern, fn)` | `transform(Document) -> Document` (via serde ↔ Lua tables) | `App::rebuild` |
| `register_completion{trigger, complete}` | `complete(before_cursor) -> Vec<CompletionSet>` | editor completion (see "How completion works" under mdtui-editor) |
| `command(name, fn)`, where the name must start with an uppercase letter | `commands()`, `run_command(name, args, path, text) -> Result<Option<new text>, String>` | `:Name` ex commands. See "How plugin commands run" below. |
| `keymap(mode, lhs, rhs)` | `keymaps()` | `App::load_plugins`. See "How plugin keymaps are bound" below. |
| `notify(msg, level)` | `take_notifications()` | app notifications |

How fence output is handled:

- The handler gets `(src, {width, dark, path, lang})`. It runs once per distinct
  `custom_key(lang, content)`, so neither a resize nor `--width` re-runs it.
- `width` is `App::content_width()` at the block's first render. For blocks in the
  initially loaded document this is the 20-column floor. `App::new` calls `rebuild` while the
  view is still 0×0, and `--dump` only sets its width afterwards.
- A block gets the real width only if it is first rendered later:
  - after `r`/`:reload`;
  - for non-markdown results, after `:theme` or `:set images/noimages/mermaid=`;
  - when its content is new: an edit, a reload from disk, or another opened file.
- Image, Lines and failed results are cached in `MediaStore`. That cache is cleared by
  `r`/`:reload`, `:theme` and `:set images/noimages/mermaid=`.
- Markdown results are cached in `App::markdown_fences`, which is cleared only by
  `r`/`:reload`. They are parsed with `ParseOptions::default()` and spliced into the document
  in place of the `Custom` block (`splice_markdown` in `app.rs`).
- Splicing happens after `transform()`. So plugin inline patterns and `on("ast")` hooks never
  see the spliced blocks, and fences inside the returned markdown (including mermaid and
  plugin fences) stay plain code blocks.
- `splice_markdown` recurses into quotes, alerts, footnote definitions and list items, but not
  into description-list details. A markdown-returning fence inside a description list is never
  spliced, and stays stuck at "⟳ rendering … diagram…".

How plugin commands run:

- Only ex commands whose names start with an uppercase letter are routed to plugins.
- If the editor is open, the new text goes into the buffer via `replace_text`, which marks
  the buffer dirty.
- Otherwise the new text is written straight to disk and the preview is rebuilt.

How plugin keymaps are bound:

- Only maps with mode `"n"` or an empty mode are bound into the viewer `Keymap`. Other modes
  are ignored.
- The right-hand side always becomes `Cmd::Ex(rhs)`, with a leading `:` and trailing `<CR>`
  stripped.
- Plugin maps are applied after the config's `[keys]`, so a plugin map replaces a user map
  with the same lhs.

How errors are reported:

- Load, transform, fence and command errors are shortened to their first line (`short_err`)
  and shown as notifications. Fence errors are warnings, shown once per distinct block; the
  others are errors.
- A failed fence falls back to a `⚠ lang: msg` line plus its source.
- Errors inside `on("ast")` hooks and `register_inline` callbacks are caught by `pcall` in the
  prelude and reported through `mdtui.notify` at error level, and the transform continues.
- Errors in completion sources, or a completion source that returns something other than a
  table, are silently ignored.

Example plugins, with an API summary, are in `examples/plugins/`. Run them with
`MDTUI_CONFIG_DIR=examples cargo run -- examples/plugins/README.md`.

## mdtui-editor

The editor pane shown by `e` / `:vsplit` / `-e`.

| Module | Responsibility |
|---|---|
| `lib.rs` | `Editor`: wraps edtui's `EditorState` and `EditorEventHandler`.<br>- It tracks the `saved` text to compute `is_dirty()`, and caches the buffer string (`text()`).<br>- `key()`/`paste()` return whether the text changed.<br>- `insert_completion(replace_chars, text)` deletes the replaced prefix and inserts `text`, with the cursor at the first `$0` marker (later `$0`s are inserted literally). It copies the current line's indentation onto continuation lines of multi-line snippets, and leaves the editor in Insert mode.<br>- `render` draws a 2-column sign gutter for LSP diagnostics (E/W/I), plus edtui with markdown syntax highlighting themed from `Theme`.<br>- `set_text` is for an external reload and marks the buffer clean; `replace_text` is an edit and marks it dirty. |
| `completion.rs` | Built-in sources (`builtin(&Ctx) -> Option<CompletionSet>`), tried in order. The first one that matches the text before the cursor wins, even if it has no items:<br>1. File paths after `](`, relative to the document's directory. Heading `#anchor`s are offered after `](#`, and are also appended when the target is empty.<br>2. Reference labels after `][`.<br>3. Code-fence languages after <code>```</code> or `~~~`.<br>4. `:emoji:` shortcodes: 2 or more chars after a `:` that is at the start of the line or follows a space, `(` or tab.<br>5. Snippets (`table`, `mermaid`, `note`, …). These appear only on Ctrl-Space (`Ctx::manual`), or when at least 2 chars of a snippet name are typed at the start of a line.<br><br>`Popup` is the VS Code-style list widget (up to 10 visible rows). |
| `lsp.rs` | Minimal JSON-RPC-over-stdio LSP client, designed for `marksman`.<br>- A reader thread parses `Content-Length` frames into a channel, and `poll()` yields `LspEvent::{Ready, Completion, Diagnostics, Exited}`.<br>- Supports `didOpen/didChange/didSave` (full-text sync) and `textDocument/completion`. Text changed before the `initialize` reply is queued and sent with `didOpen`.<br>- Positions are UTF-16 and the caller must convert them. `Lsp::completion` takes a UTF-16 column, which `App` computes with `utf16_col`. `App::tick` converts the server's `replace_start` back to chars. `Diagnostic.col` is left as UTF-16.<br>- `App::start_lsp` starts it when the editor opens, but only if the document has a path and `[editor] lsp[0]` is on `PATH`.<br>- `Drop` sends `shutdown`/`exit`, then kills and waits on the child process. |

How completion works. It is driven from the binary:

1. `App::trigger_completion(manual)` runs on Ctrl-Space, and also automatically after a typed
   char or Backspace in Insert mode when `[editor] auto_complete` is on.
2. It runs `completion::builtin`, then merges plugin `CompletionSet`s into the result:
   - If built-ins returned `None`, the first non-empty plugin set (in registration order)
     becomes the base. Each later plugin set is merged only if its `replace_chars` equals the
     base's.
   - Otherwise, a plugin set is merged only if its `replace_chars` equals the built-in set's.
     This also applies when a built-in source matched but has zero items, such as `](` in an
     empty directory or an unmatched `:emoji`.

   Plugin sets that don't match are dropped silently.
3. An LSP `textDocument/completion` request is sent only if the trigger was manual or a popup
   opened.
4. `App::tick` handles LSP responses. If the replace width matches, the items are appended to
   the open popup, deduplicated by `insert`. If no popup is showing, they open one.
5. Accepting a folder item triggers completion again.

## mdtui (binary)

| Module | Responsibility |
|---|---|
| `main.rs` | **CLI** (`clap`):<br>- `file`: `-` reads stdin. With no file, piped stdin is read if stdin is not a terminal; otherwise the first of `README.md`, `readme.md`, `Readme.md`, `index.md` in the current directory.<br>- `--theme`, `-e/--edit`, `--toc`, `--no-images`, `--no-plugins`, `--protocol`, `--config`, `--print-config`.<br>- `--dump [--width N --color auto\|always\|never]`.<br><br>**Startup**: the image protocol is detected with `Picker::from_query_stdio()`, but only when images are enabled (halfblocks otherwise, or if the query fails). `--protocol` / `[images] protocol` then forces a type. Late replies to the terminal query are drained so they aren't read as keys.<br><br>**Main loop**: `tick()` → draw if `dirty` → poll events (30 ms) → `input::handle`.<br><br>**`--dump`**:<br>- turns images off, so mmdc is not run and unsupported diagram types use mermaid-ascii. The exception is a PNG already in the mermaid cache (same source, theme, background and pixel width): it is still loaded, and dumps as an empty bordered box;<br>- lays out at `--width` without a TUI, and waits up to 20 s for pending diagrams;<br>- prints notifications to stderr and the lines to stdout. Output is ANSI with `--color always`, or with `auto` when stdout is a terminal; otherwise it is plain text.<br><br>Set `MDTUI_DEBUG_EVENTS=<file>` to log every event with timestamps. |
| `app.rs` | `App`: all state, independent of drawing.<br><br>**State**:<br>- document: `source`, `doc`, `layout`, `media`, `plugins`;<br>- view: `scroll`, `view`, `x_offset`, folds, `show_source`, focus, search, selection;<br>- panes (`Pane::{Preview, Editor, Toc}`) and input mode (`Input::{None, Command, Search}`);<br>- the editor and LSP client, the fullscreen overlay (`Fullscreen { key, zoom, pan, drag }`), notifications, navigation history (`H`/`L`) and the file watcher.<br><br>**Important methods**:<br>- `rebuild`, `relayout_if_needed` and `tick`. `tick` does the media poll, debounced reload, debounced editor→preview rebuild, cursor sync, LSP events, key-sequence timeout and notification expiry (4 s, 8 s for errors), and ends with `relayout_if_needed`.<br>- `run(Cmd)` and `activate(Action)`.<br>- `ex(line)`: `:N` jumps to source line N. Built-ins and their aliases are matched directly in `ex`; `BUILTIN_COMMANDS` is only the Tab-completion list. Names starting with an uppercase letter go to plugin commands, and anything else is an error. Argument completion is in `command_candidates`, and `:set` options are in `set_option`.<br>- `open_link`: `#anchor` jumps within the page. `scheme://` and `mailto:` links, and existing local files that aren't markdown, go to the system opener (`open`). Local markdown files (`.md`/`.markdown`/`.mdown`/`.mkd`, relative to the document or absolute) are opened in place. `.md` is added to extensionless wikilinks, and `file.md#anchor` scrolls to the anchor.<br>- `toggle_task`, `reload`, `open_file`, `trigger_completion`, `start_lsp`.<br>- mouse helpers: `preview_press/drag/release`, `doc_pos`. |
| `ui.rs` | Drawing only: title bar, outline sidebar, editor │ preview split with a draggable `┃` divider, scrollbar, completion popup, status line (mode badge, breadcrumb, percentage, `ln N/M`), command line with wildmenu, notifications, help overlay (the `HELP` table), and the fullscreen media view.<br><br>`draw` also writes layout geometry back into `App` (`view`, `toc_area`, `editor_area`, `divider_x`, `fs_body`, `fs_buttons`), which mouse handling then uses. It invalidates the layout when the preview size changes. |
| `input.rs` | Event routing. Priority: help overlay → fullscreen → command/search line → focused pane (editor, outline or viewer).<br><br>- **Viewer**: keys go into `Keymap`.<br>- **Editor**: intercepts the completion-popup keys (`↓`/`<C-n>`, `↑`/`<C-p>`, Tab/Enter to accept; Esc closes the popup and still reaches edtui), `<C-w>` window commands, `<C-s>` save, `<C-Space>` manual completion, and `:` in normal mode. Everything else goes to edtui. `App::editor_changed` is called when the text changes.<br>- **Mouse**: divider drag, wheel scroll, click (hit map) vs drag (text selection, copied on release), and outline clicks. |
| `keys.rs` | Vim key notation (`gg`, `<C-d>`, `<leader>wc`, `<CR>`) and the `Cmd` enum of viewer actions.<br><br>`Keymap` does multi-key sequence matching. A dead-end sequence is retried with just the last key, and a 700 ms timeout runs an exact match that was waiting on a longer mapping.<br><br>Default bindings are set up in `Keymap::new`. `App::new` then adds `[keys]` from config, and `App::load_plugins` adds plugin keymaps, both through `Keymap::map`; later mappings win. Their right-hand sides always become ex commands (`Cmd::Ex`). |
| `config.rs` | `Config` (serde, every field defaulted): theme, max_width, toc, toc_width, scroll_step, code_line_numbers, leader, `[images]`, `[mermaid]`, `[editor]`, `[plugins]`, `[keys]`.<br><br>`config_dir()` checks `MDTUI_CONFIG_DIR`, then `$XDG_CONFIG_HOME/mdtui`, then `~/.config/mdtui`. `EXAMPLE` is the text printed by `--print-config`; keep it in sync with the struct. |
| `util.rs` | `copy_to_clipboard`:<br>- loads the tmux buffer when inside tmux (`$TMUX`);<br>- independently, uses the first desktop helper that works: wl-copy if `$WAYLAND_DISPLAY` is set, xclip or xsel if `$DISPLAY` is set, pbcopy on macOS, then clip.exe;<br>- **always** also emits OSC 52;<br>- returns the name of the method used, for the "copied …" notification.<br><br>`lines_to_ansi` converts styled lines to SGR escape codes for `--dump`. |

### Event and state flow

```
crossterm Event ─► input::handle ─► App::run(Cmd) / App::activate(Action) / App::ex(":…")
                                         │ mutates state, sets dirty / layout_dirty
main loop ─► App::tick ──────────────────┤ media poll, watcher, editor debounce, cursor sync,
          │                              │ LSP, key timeout, notification expiry,
          │                              │ relayout_if_needed
          └► ui::draw (if dirty) ────────┘ size check → relayout_if_needed → prepare_media → Viewport
```

- `dirty` means "redraw". `layout_dirty` means "run layout() before the next draw". Set them
  with `invalidate_layout()` rather than calling `layout()` directly. Most relayouts happen at
  the end of `tick`. `ui::draw` runs `relayout_if_needed` again only after it notices that the
  preview size changed.
- **Live reload**:
  - `notify` watches the file's **parent directory** (non-recursive), so editors that save by
    renaming still trigger it. Events are debounced 80 ms.
  - On reload, the scroll position is kept relative to the heading whose section contains the
    top visible row.
  - A watcher-triggered reload is skipped with a warning while the editor has unsaved changes.
  - `:e!`, `r` and `:reload` force the reload and replace the editor buffer with the file on
    disk, discarding unsaved edits. `r` and `:reload` also clear the media and fence caches.
- **Editor → preview**: an edit sets `preview_dirty_at`, and 120 ms later the source is taken
  from the editor and rebuilt. When the cursor row changes, the preview scrolls so that row is
  a third of the way down.
- **Checkbox clicks** rewrite the source with `source::toggle_task`. The result goes into the
  editor buffer if one is open, otherwise straight to disk.

## Where to make common changes

| Task | Touch |
|---|---|
| New markdown construct | 1. `parse.rs` (lowering).<br>2. `ir.rs`: add the variant, keeping it serde-compatible. If it holds child blocks, extend `walk_blocks(_mut)`; if it is a container inline, extend `Inline::children()`.<br>3. `layout.rs` (rendering).<br>4. `kitchen-sink.md` and the snapshots.<br><br>Container blocks also need `splice_markdown` in `app.rs`. If they use new field names, they also need the traversals in `prelude.lua`, which follow `content`/`blocks`/`items`/`header`/`rows`. These traversals use `_ => {}` catch-alls, so the compiler won't flag a missed one. |
| New clickable thing | Add an `Action` variant in `hitmap.rs`, emit hits in `layout.rs`, handle it in `App::activate`, and describe it in `ui.rs::describe`. |
| New viewer key | Add a `Cmd` and its default binding in `keys.rs`, the behaviour in `App::run`, and an entry in `ui.rs::HELP`. |
| New ex command | `App::ex`, and `BUILTIN_COMMANDS` for Tab completion of names. Argument completion goes in `App::command_candidates`. A new `:set` option goes in `App::set_option`, plus the `set` list in `command_candidates`. Use a lowercase name, because capitalised names are routed to plugin commands. |
| New config option | Add it to the `config.rs` struct, `Default` and `EXAMPLE`, and read it where it is used. `[images]`/`[mermaid]` options must also be threaded through `media_config()` in `main.rs` into `MediaConfig`/`MermaidConfig` (mdtui-render). Optionally add a CLI override in `main()` and a `:set` option in `App::set_option` (plus its completion list). |
| New mermaid diagram type (text) | Add a `diagram/` module, and handle the type in `supported()` / `render()` in `diagram/mod.rs`. |
| New image source | `fetch.rs::load_bytes` / `decode` |
| New plugin API | `prelude.lua` (Lua side), a `PluginHost` method, and the call site in `app.rs`. Update `examples/plugins/README.md`. |
| New completion source | `completion.rs::builtin`, and its merge rules in `App::trigger_completion`. |
| Theme colour | `Theme` fields in `theme.rs`, in both `dark()` and `light()`. |

## Build, test, run

```sh
cargo build --release
cargo test --workspace                       # unit tests + snapshots
cargo run -- examples/kitchen-sink.md        # the feature showcase
cargo run -- --dump --width 80 examples/kitchen-sink.md   # headless render to stdout
MDTUI_CONFIG_DIR=examples cargo run -- examples/plugins/README.md   # with example plugins
cargo run -p mdtui-render --example diagrams # text mermaid renderer samples
```

There is no CI. Run `cargo test --workspace` before pushing.

Runtime files:

- **config**: `<config_dir>/config.toml`, with plugins in `<config_dir>/plugins/`. The config
  dir is `~/.config/mdtui` by default.
- **caches**: under the platform cache dir from `directories::ProjectDirs`. On Linux that is
  `$XDG_CACHE_HOME/mdtui/`, by default `~/.cache/mdtui/`.
  - `mermaid/`: mmdc PNGs named by a hash of source + theme + background + width, plus
    `puppeteer-no-sandbox.json` for the sandbox retry. The `<hash>.mmd` input is a temporary
    file that is normally deleted after each render.
  - `img/`: downloaded remote images, keyed by a hash of the URL and never re-fetched.
- **optional external tools**: `mmdc` (mermaid-cli), `mermaid-ascii`, `marksman` (LSP), and
  the clipboard helpers.

## Known issues

These are current behaviours in the code, found while auditing this guide. They are described
in more detail in the sections above.

- **Plugin fences render at 20 columns on first load.** In the initially loaded document,
  plugin fence handlers get `ctx.width = 20`. `App::new` rebuilds before the view size is
  known, and fence output is cached by content only.
- **Fences in description lists get stuck.** A markdown-returning plugin fence inside a
  description list is never spliced in, because `splice_markdown` doesn't descend into
  description lists. It stays at "⟳ rendering…".
- **`r`/`:reload` can blank media.** It clears the caches before `App::reload`, which may
  return early without rebuilding: in stdin mode, or when the open editor already matches
  the file. Images then show `░` and diagrams stay pending until the next rebuild.
- **`--dump` can print an empty box.** With a cached mmdc PNG for a diagram, `--dump` prints
  an empty bordered box instead of text.
- **Some images never load.** This applies to images in table cells, in description-list
  terms and in plugin `Lines` blocks.
- **Failed image encodes are not retried.** The placement stays `░`.

## Conventions

- Keep `mdtui-core` pure: it takes data and returns data, with no terminal or process I/O.
  Anything slow or effectful goes in `mdtui-render` (threads) or the binary.
- Don't block the UI thread on slow I/O. Spawn a worker, send results over a channel, and pick
  them up in `tick()`. These things currently run synchronously on the UI thread:
  - native text diagrams (they are fast);
  - all Lua callbacks: fences, the `on("ast")`/inline transforms in `rebuild`, completion
    sources and `:Commands`, including `mdtui.exec` in trusted mode;
  - decoding plugin-returned image bytes, and encoding the fullscreen image view;
  - clipboard helper subprocesses;
  - plain file reads and writes (reload, save, open, path completion).
- Widths are terminal cells (`unicode-width`), not bytes or chars. Editor columns are chars,
  and LSP columns are UTF-16.
- Source line numbers are 1-based. Editor rows and LSP lines are 0-based.
- Errors that reach the user become notifications (`App::info/warn/error`). Avoid panics on
  user input or plugin output.
- Module-level `//!` doc comments describe each file's role. Keep them accurate when changing
  responsibilities.

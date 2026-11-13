# cozy

[![crates.io](https://img.shields.io/crates/v/cozy.svg)](https://crates.io/crates/cozy)
![license](https://img.shields.io/crates/l/cozy.svg)

**English** | [Japanese](README.ja.md)

**A Comfort First TUI — type like nano, navigate like vim.**

![cozy welcome screen](https://raw.githubusercontent.com/takakix2/cozy/main/docs/assets/screenshot.png)

cozy is a small Rust terminal editor **and pager** for people who want direct, low-friction editing by default, with optional modal navigation when they need it. It is intended to feel approachable like `nano` while keeping a focused set of vim-like motions in Glide mode. The same binary set ships `czv`, which opens a file read-only in that vim-style navigation — a pager that tells you which keys it has.

## Install

**macOS — Homebrew.** This is the recommended way on a mac: Homebrew's prefix is always
on `PATH`, so `cozy` is there the moment it finishes.

```bash
brew install takakix2/tap/cozy
```

**macOS / Linux — prebuilt binary.** Installs to `~/.local/bin`. No Rust toolchain
needed.

```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/takakix2/cozy/releases/latest/download/cozy-installer.sh | sh
```

Archives for each platform are attached to every [release](https://github.com/takakix2/cozy/releases)
if you would rather place the binary yourself. Linux binaries need glibc 2.35 or newer.

**With a Rust toolchain.**

```bash
cargo install cozy
```

Or build from source:

```bash
cargo build --release
```

## Usage

```bash
# Open a new buffer
cozy

# Open a file
cozy <file>

# Open a folder — the editor, with that folder in the sidebar
cozy <folder>

# The full-screen folder tree is still there
# (Ctrl+B / F3 inside the editor, or `czv <folder>`)
```

## Highlights

- Direct text editing by default: type, save, and exit without learning a modal workflow first
- File open/save/save-as, unsaved-change prompts, and folder browsing
- Search and replace with literal, case-sensitive, word-boundary, and regex modes
- Undo/redo, line cut, clipboard paste, line numbers, line wrap, and goto-line
- Tree-sitter syntax highlighting for ~45 languages and file kinds — Rust, Python,
  JavaScript/TypeScript/TSX, Go, C, C++, C#, Java, Kotlin, Swift, Dart, Haskell, Zig, Nix,
  HCL/Terraform, Lua, Ruby, PHP, Elixir, Erlang, SQL, HTML, XML, CSS/SCSS,
  YAML, JSON, TOML, INI (`nginx.conf`, `.env`, systemd units), Make, CMake, Just,
  Dockerfile, `diff`/`patch`, `.gitignore`-style files, `~/.ssh/config`, PowerShell, Vim,
  Markdown (and Markdown preview)
- Markdown preview, including Mermaid diagram blocks (rendering code from `ratatui-markdown`)
- **PDFs open as pages.** `cozy doc.pdf` (or `czv doc.pdf`) shows page 1 and turns pages
  with `←`/`→`, `z` fit ↔ 1:1, `q` back — inside the editor, so the sidebar and the rest of
  cozy stay. The rasterizer is the one the system already has: poppler's `pdftocairo`, or
  Ghostscript. Nothing is linked in: MuPDF (the engine `tdf` uses) is AGPL-3.0, which would
  relicense cozy's MIT/Apache binary, and it is tens of megabytes of C. If neither tool is
  installed the status bar names the package to install.
- **Pictures open as pictures.** `cozy photo.png` (PNG, JPEG, GIF, WebP, BMP, ICO, AVIF)
  draws the image in the body with whatever the terminal speaks — Kitty, iTerm2, Sixel
  (4096 colours when the terminal allows it) or halfblocks — instead of filling the buffer
  with binary: `z` fit ↔ 1:1, `←`/`→` the previous/next picture in the folder, `q` back to
  the text. The sidebar keeps working while the picture is up, and the image never becomes
  the save target, so `Ctrl+S` cannot write text over it.
- Fast reading controls in Markdown preview
- Glide mode for vim-like movement, operators, yanking, changing, deleting, joining, and paste
- TOML configuration and per-action key overrides
- Reducer-based architecture with focused tests for cursor behavior, motions, editing, replace, clipboard, and browse mode

## Sidebar

The left panel is the VS Code shape: an explorer tree beside the buffer, and a Source
Control tab over the same folder.

```
≡ Files  ⑂ Git 7                 │  1  fn main() { println!("hi"); }
                                 │  2
▾ input-produksi                 │  3
  ▸ .vscode                      │  4
  ▾ src                          │  5
    rs main.rs                   │  6
    md README.md                 │  7
    tm .env                      │  8
Ctrl+S Save   Ctrl+B Browse ...  │
```

- `Alt+B` — focus the **Files** tab (again to hide the panel; on a narrow terminal it
  shows the list and never hides it)
- `Alt+G` — focus the **Git** tab: `git status` in VS Code's two groups, **Staged Changes**
  (the index) and **Changes** (the working tree), each a tree of directories with
  `M`/`A`/`D`/`U` letters, a per-folder count, and the branch name. A file that is staged
  and modified again appears in both groups.
- Every row carries **how many lines changed** — `+12 −3` per file, summed per folder and
  per group, right-aligned and coloured green/red (binary changes show no count).
- **Nested repositories are tracked too.** A folder that holds other git repositories — a
  submodule, a vendored clone, a workspace of projects — gets one header per repository,
  each with its own Staged/Changes groups, branch and counts. `git status` in the outer
  repository shows a nested one as a single opaque row (or hides it entirely when it is
  ignored), which is exactly the information a change list must not lose. Build and
  package folders (`node_modules`, `target`, `dist`, …) are not walked.
- **Click a change to read its hunks** — a single-file diff view opens beside the panel:
  `@@` headers, `+`/`-` lines with both line-number gutters, `n`/`N` to jump between hunks,
  `q`/`Esc` back to the list. Changed lines are tinted edge to edge (green added, red
  removed), so a long diff can be scanned without reading every sign. Untracked files open
  as wholly added lines.
- In the **Files** tab the tint is ranked: **red** if anything under it was deleted,
  otherwise **orange** if something new appeared, otherwise **yellow** if something was
  changed — and **neutral** when nothing changed, so yellow always means something. The
  same rule colours a file's own name, and a folder with a removal outranks one with an
  addition (red wins).
- `j`/`k` or ↑/↓ move, `Enter`/`l` open a file (or expand a folder), `h` collapse,
  `Tab` switches tabs, `r` re-reads `git status`, `Esc` hands the keyboard back

The panel is **mouse-first**:

- click a row — a folder opens or closes, a file opens, a change opens its diff
- click a tab in the header — switches views (clicking the open tab never closes it)
- horizontal wheel (or Shift+wheel / a trackpad swipe) — scrolls the text sideways when
  soft wrap is off; with wrap on it turns wrapping off first, because that is what "show me
  the rest of this line" means (`Ctrl+W` turns it back on)
- wheel over the panel — scrolls the list; the **selection stays put**, so reading never
  changes what Enter would open (the same is true of the wheel in the buffer: it scrolls the
  viewport, the cursor does not move until you press a key)
- **scrollbars** — the sidebar's track appears when the tree is taller than the panel, and
  the text gets a vertical bar when the buffer overflows and a horizontal one when soft wrap
  is off and a line runs past the window. Click or drag a bar to move the view; neither
  moves the cursor or the selection.
- hovering a row lights it up (a short fade); hover is the pointer's state, never the
  scroll's, and moving within the same row does not repaint
- drag the panel's right border — resizes it (never past half the screen)
- click in the text — the cursor goes where the character was drawn (soft wrap included)
- **drag in the text — selects it and copies on release.** The band is drawn behind the
  characters, double-click selects the word, `Ctrl+C` copies again, and any key clears it.
  Because cozy holds the mouse while the panel is up, this is what makes plain
  drag-to-copy work (the terminal's own selection needs Shift). The text goes to the
  system clipboard, or — when there is none, such as over SSH — to the terminal itself
  with `OSC 52`.

On a **narrow terminal** (below 70 columns) the panel and the text take turns instead of
sharing — the shape WA-Bot uses: the **list** fills the screen, clicking a row opens that
file as the **detail**, and `‹ Files` in the status bar (or `Esc`) walks back. The same
corner button means "fold the panel away" on a wide screen and "back to the list" on a
narrow one, so there is always something to click.

⚠️ While the panel is up the terminal hands the mouse to cozy, so the terminal's own
drag-to-select needs **Shift**. Hiding the panel (`Alt+B` twice) gives plain drag-select
back; the reading views and `czv` never take the mouse.

Type badges take the place of an icon font: two coloured letters (`rs`, `md`, `ts`, …), so
the tree reads the same in any terminal. Hidden entries are shown (`.vscode`, `.env`) but
`.git` never is. The panel steps aside on terminals narrower than 70 columns, and
`COZY_NO_SIDEBAR=1` turns it off entirely.

## Screenshots

Edit mode is the default: open a file and type directly, with line numbers and a compact shortcut footer.

![cozy edit mode](https://raw.githubusercontent.com/takakix2/cozy/main/docs/assets/edit-mode.png)

Glide mode adds vim-like movement and editing commands while keeping the visible footer focused on the active mode.

![cozy glide mode](https://raw.githubusercontent.com/takakix2/cozy/main/docs/assets/glide-mode.png)

Markdown preview renders the current document with readable wrapping, code block formatting, and Mermaid diagram blocks.

![cozy markdown preview with Mermaid diagrams](https://raw.githubusercontent.com/takakix2/cozy/main/docs/assets/markdown-preview-current.png)

## Editor Modes

- **Edit**: Default mode. Type directly, like `nano`.
- **Glide**: Vim-like modal navigation and editing (`Ctrl+G`).
- **Search**: Incremental search (`Ctrl+F`).
- **Replace**: Search and replace (`Ctrl+R`).
- **Goto**: Jump to a line number (`Ctrl+J`).
- **Save**: Save dialog (`Ctrl+S`).
- **Open**: Open-file dialog (`Ctrl+O`).
- **Browse**: Full-screen file tree (`Ctrl+B`, or `F3` inside tmux where `Ctrl+B` is the prefix).
- **Command**: Command palette (`Ctrl+P`).
- **Markdown**: Markdown reading mode (`F2` or `Ctrl+D`).
- **Help**: Help screen (`Ctrl+H` or `F1`).

## Key Bindings

The source of truth for key bindings is `src/shortcuts.rs` (`get_shortcuts()`). Defaults are shown below. Per-action overrides are available through the `[keys]` section of `config.toml`.

### File

- `Ctrl+S`: Save
- `Ctrl+Shift+S`: Save as
- `Ctrl+O`: Open file
- `Ctrl+B` / `F3`: Browse files (`F3` is a tmux-safe fallback; tmux uses `Ctrl+B` as its prefix)
- `Ctrl+X`: Exit, prompting to save when modified
- `Ctrl+Q`: Quit immediately without saving

### Navigation

- `Up` / `Down` / `Left` / `Right`: Move cursor
- `Ctrl+A`: Line start
- `Ctrl+E`: Line end
- `Alt+\` / `Ctrl+Home`: File top
- `Alt+/` / `Ctrl+End`: File bottom
- `PageUp` / `PageDown`: Scroll page
- `Ctrl+J`: Jump to line number
- `Ctrl+G`: Enter Glide mode

### Editing

- `Enter`: Insert newline
- `Backspace` / `Delete`: Delete before or at cursor
- `Ctrl+K`: Cut current line
- `Ctrl+V`: Paste from system clipboard
- `Ctrl+Z`: Undo
- `Ctrl+Y`: Redo

### Search And Replace

- `Ctrl+F`: Search
- `Ctrl+N`: Next match
- `Ctrl+P` in Search/Replace: Previous match
- `Ctrl+T`: Toggle search options
- `Ctrl+R`: Replace mode; press again to replace all
- `Tab` in replace mode: Switch between query and replacement fields
- `Enter` in replace mode: Replace current match

### Sidebar

- `Alt+B`: Sidebar — Files (focus; again hides the panel)
- `Alt+G`: Sidebar — Git changes
- `Tab`: switch Files ↔ Git while the panel has the keyboard
- `Esc`: return the keyboard to the buffer

### View And Help

- `Ctrl+H` / `F1`: Help
- `Ctrl+L`: Toggle line numbers
- `Ctrl+W`: Toggle line wrap
- `Ctrl+U`: Toggle shortcut footer visibility
- `F2` / `Ctrl+D`: Toggle Markdown preview
- `Esc` / `Ctrl+[`: Cancel current operation or leave the current mode

### Command Palette

- `Ctrl+P`: Open Command mode
- Type to filter commands
- `Up` / `Down` or `j` / `k`: Select a command
- `Tab`: Complete the common label prefix
- `Enter`: Run the selected command
- `Esc`: Return to the home mode

Built-in commands are currently grouped as:

- `Mode.*`: sustained editing and navigation modes
- `Search.Find`
- `Search.Replace`
- `File.SaveAs`
- `File.Open`
- `Browse.Files`
- `Navigate.GotoLine`
- `View.Markdown`
- `View.ToggleLineNumbers`
- `View.ToggleWrap`
- `View.ToggleFooter`
- `Config.Open`
- `Config.Reload`
- `App.Quit`
- `App.QuitWithoutSaving`

## Markdown Preview

Markdown preview is available with `F2` or `Ctrl+D`. It is a read-only view for quickly reading README files, plans, notes, and other Markdown documents. The preview renders with code taken from `ratatui-markdown` (carried inside cozy since it moved to ratatui 0.30), so headings, lists, block quotes, inline code, wrapped paragraphs, fenced code blocks, and Mermaid diagram blocks follow the renderer's output instead of the old hand-written formatter.

- Move: `j`/`k` or `Up`/`Down`
- Page: `PageUp` / `PageDown`
- Jump: `gg`/`G`, `Ngg`/`NG`
- Screen: `H`/`M`/`L` for top/middle/bottom of the visible area
- Counted move: `5j`, `5k`, `5gg`, `5G`
- `Esc`: Return to your configured home mode

## Glide Mode

Glide mode is available with `Ctrl+G`. A leading number repeats a motion or linewise operation.

- Move: `h` `j` `k` `l`, `w`/`b`/`e`, `W`/`B`/`E`, `0`/`^`/`$`, `gg`/`G`, `H`/`M`/`L`
- Find/till: `>`/`<`, `t`/`T`, then `.`/`,` to repeat the last jump forward/back
- Operators: `d`/`c`/`y` plus a motion, such as `dw`, `de`, `d$`, `dj`, `cw`, `yw`, `d3w`
- Linewise: `dd`/`cc`/`yy`, with counts such as `3dd`
- Edit: `x`, `X`, `~`, `J`
- Paste: `p`/`P`
- Insert: `i`/`I`, `a`/`A`, `o`/`O`
- `Esc`: Return to Edit mode

## Configuration

The repository ships `config.example.toml` as a template. Copy it to `config.toml` if you want a local override in the project root.

Configuration is loaded from the first path found:

1. `./config.toml`
2. `~/.config/cozy/config.toml`
3. `~/.cozy/config.toml`

Example:

```toml
page_size = 20
theme = "dark"
show_line_numbers = true
status_duration = 3
line_number_bg = "darkgray"
line_number_fg = "white"

# Footer and status bar colors accept color names or #RRGGBB true color values.
footer_bg = "#222226"
footer_key_fg = "cyan"
footer_fg = "gray"
status_bar_bg = "darkgray"
status_bar_fg = "white"

cursor_blink = true

# Which mode you rest in: "edit" (default, type like nano) or "glide"
# (navigate like vim). This is your home — every action returns here, not
# just startup. Newcomers stay in "edit" (zero hidden state); vim users can
# opt into "glide" and enter Edit with i/a/o.
default_mode = "edit"
```

Key bindings can be overridden by action name:

```toml
[keys]
enter_browse = "ctrl+b"
enter_glide = "ctrl+g"
enter_help = "f1"
toggle_markdown = "f2"
toggle_footer = "ctrl+u"
```

## Architecture

cozy uses a Redux-inspired core with thin host adapters:

```text
Host (CLI / embedded)
  -> EventSource + input mapping
  -> Keymap
  -> Action
  -> Reducer
  -> EditorState
  -> UI render

File / config / clipboard / runtime IO stay behind small adapter modules.
```

The main editor state lives in `EditorState`, text is stored in a line-based `TextBuffer`, and editor behavior is implemented through reducers. CLI terminal setup, event sources, file/config loading, clipboard access, and startup runtime are kept at the host/IO boundary so the core editor behavior can be tested directly and embedded by hosts such as hsh-ios.

## Development

```bash
cargo test
cargo fmt
```

The current test suite covers editor reducers, cursor movement, word and screen motions, replace behavior, clipboard/register handling, and browse mode behavior.

## License

Licensed under either of:

- MIT License ([LICENSE-MIT](LICENSE-MIT))
- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))

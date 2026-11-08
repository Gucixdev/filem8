# filem8

A small, fast GUI file manager in Rust with its **own widget toolkit** (no GTK/Qt/egui).
Goal: Thunar's minimalism with Dolphin-class features and better UX: tabs, split panes, bookmarks,
previews for images/audio/3D/game assets, keyboard first.

    ./run        # builds release (nice -n 19) and opens the window; no arguments

Knobs (env): `FM8_BACKEND=vulkan|gl|soft` (default: vulkan then GL, then automatic CPU surface), `FM8_SCALE=1.5`.
Theme: `~/.config/filem8/theme` (colorado `.theme` format, reloaded live when the file changes; see docs/colorado-generator.md).
Headless render of the real UI: `filem8 --screenshot out.png [DIR] [WxH] [SCALE]`.

## Crates

| crate | what |
|---|---|
| `fm8-ui` | immediate-mode widgets, layout, theme, display list, CPU rasteriser (software fallback + golden tests) |
| `fm8-core` | model: async listing, sort/filter/selection, file ops + undo, trash, bookmarks, mounts, recents, watch, fuzzy |
| `fm8-preview` | loaders for the preview pane (image/gif/webp/avif/svg, text, archives, audio, 3D, atlases, pdf) |
| `filem8` | the app: window (winit), presenter (wgpu, CPU fallback), UI composition |

## Feature table (honest)

Legend: **done** = implemented and covered by `cargo test`; **built** = implemented, not covered by an automated test or not seen on real hardware; **planned**.

| feature | status |
|---|---|
| own widget system: layout, text, buttons, input, virtual list/grid (100k rows), tabs, split, menu, tooltip, DPI | done |
| theme from plain file (colorado format), live reload | built (parse tested; reload not tested) |
| headless render to PNG + golden/pixel tests | done |
| window on Wayland via winit; wgpu present; CPU fallback | built (never run on the real dwl screen by the author's agent) |
| GPU-native draw (quads/glyph atlas in wgpu) | planned (now: CPU rasterise, wgpu/softbuffer only presents) |
| async listing, natural sort, filters, selection (click/ctrl/shift), type-to-find | done |
| rename / copy / move / new folder with undo, trash (freedesktop) with undo | done in core; UI for rename/copy/move/paste **planned** (UI has trash+undo, new folder) |
| fs watching, auto refresh | done (core) / built (UI) |
| bookmarks (GTK file compatible), XDG dirs, mounts, recents | done (core); sidebar shows bookmarks, places, devices |
| breadcrumbs + editable path bar with Tab completion | done |
| tabs (Ctrl+T/W, middle click) | done |
| per-folder view memory (sort, hidden) | done |
| list view | done; grid/compact views **planned** (virtualised grid widget exists) |
| split dual-pane (F3) | planned (split widget exists) |
| drag and drop | planned (DragStart event exists) |
| preview loaders: raster images, animated gif/webp, avif, svg, text with syntax-light, zip/tar/tar.gz, audio waveform, glTF/OBJ/STL software render, Tiled/atlas JSON, PDF page 1 (poppler) | see git log; preview pane UI **planned** |
| audio playback (cpal) | built |

## Keys

Arrows/PgUp/PgDn/Home/End move; Enter open; Backspace back (or up); Alt+Left/Right/Up history/parent;
Ctrl+L path; Ctrl+T/W tabs; Ctrl+Tab next tab; Ctrl+H hidden; Ctrl+A all; Del trash; Ctrl+Z undo; F5 reload;
typing jumps to a matching name; middle click opens folder in new tab; right click menu; Ctrl+Q quit.

## Design notes

* Immediate mode: the UI is a pure function of state, virtual lists are "loop the visible range", persistent bits (scroll, split ratio, caret, focus) live in `Ui` keyed by hashed ids.
* Everything the widgets draw is a backend-neutral display list; `soft` rasterises it on the CPU, so tests need no GPU and `FM8_BACKEND=soft` works everywhere.
* Tests never touch real files: temp dirs, trash tests set `XDG_DATA_HOME` to a temp dir.

# fm8-lz

filem8's folder view on lazyUI-rs, in a native Wayland window: `cargo run -p fm8-lz -- [DIR]`. It follows finedesktop's
theme when that runs. fm8-ui stays until this has the same features.

| Key | Does |
|---|---|
| space, ctrl-a, ctrl-click | mark rows in the list (`*`); delete, ctrl-c and ctrl-x then act on all of them |
| ctrl-p | preview pane: text, archive listing, picture, folder size, metadata of the entry under the cursor (loaded off the UI thread by `fm8-preview`) |
| enter | open the folder, or go to the path typed in the bar |
| alt-up | parent folder, cursor on the one just left |
| ctrl-l | focus the path bar |
| ctrl-g | switch between the list and the thumbnail grid |
| ctrl-i | properties window (floats, drag the title, esc closes) |
| delete | ask, then move the entry to the trash |
| f2 | rename the entry (a name box) |
| ctrl-n | new folder (a name box) |
| ctrl-c, ctrl-x, ctrl-v | copy or cut the entry, paste it into the folder shown (on its own thread, a taken name gets a copy name) |
| ctrl-z | undo the last change (trash, rename, new folder, paste) |

Thumbnails and theme icons are made on a worker thread (`src/thumbs.rs`): pictures are decoded with fm8-codec and shrunk to
128 px, other entries get an icon from the first usable PNG icon theme (AdwaitaLegacy and Adwaita are tried by name when the
index hides them). SVG-only themes are not read yet.

`cargo run -p fm8-lz --example shot -- DIR OUT` writes `OUT-list.png`, `OUT-grid.png` and `OUT-props.png` without a
compositor, drawn by the same rasteriser as the window. Tests: `cargo test -p fm8-lz`.

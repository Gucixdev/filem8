# colorado generator spec: `filem8`

filem8 reads colorado's own theme format directly (`key = value`, `#rrggbb`),
so a colorado theme file can be copied unchanged to `~/.config/filem8/theme`
(or passed with `--theme FILE`). A colorado generator therefore only has to
write the subset below (unknown keys are ignored, missing keys fall back to
the built-in dark/light defaults chosen by `variant`).

| key | role in filem8 |
|---|---|
| `variant` | `dark` / `light`, picks defaults for missing keys |
| `bg` | window / content background, text-input fill |
| `bg_alt` | toolbar, sidebar, tab strip, buttons, menus |
| `fg`, `fg_dim` | primary text, secondary text and icons |
| `accent`, `accent_fg` | primary button, active-tab bar, folder icons; text on accent |
| `border`, `border_focus` | 1px outlines, dividers; focus ring and focused input |
| `selection_bg`, `selection_fg` | selected rows, hovered menu item |
| `urgent`, `warn`, `ok` | error / warning / success toasts and progress |
| `link` | breadcrumb hover, links in previews |
| `font_size` | UI text size in logical px (clamped 8..40) |
| `radius`, `gap` | corner radius and base spacing (logical px) |

`colorN` (ANSI) are used only by the text-preview syntax highlighter
(`color1` red, `color2` green, `color3` yellow, `color4` blue, `color5` magenta, `color6` cyan).
Suggested generator: `colorado export NAME filem8 --out DIR` writes `DIR/filem8/theme`
containing exactly the keys above plus `colorN`, plus a reload hint:
filem8 re-reads the file when its mtime changes (planned: currently read at start).

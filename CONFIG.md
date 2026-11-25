# Jade configuration (`config.toml`)

Jade reads `config.toml` from its config folder: `~/.config/jade/` on Linux,
the OS config folder plus `jade/` elsewhere. `config.sample.toml` in the repo
lists every key at its built-in value; with that file the editor looks the same
as with none.

## Rules

- Every key is optional. Delete a line or a whole table to get the built-in
  value back.
- A bad value (wrong type, out of range, unknown name) is reported on stderr
  and only that key falls back; a number out of range is clamped. An unknown
  key is reported and ignored. A file that is not valid TOML is reported and
  the defaults are used.
- Edits are picked up while the app runs. A layout key (anything that changes
  how tall or wide text is) rebuilds the rows once; a paint-only key just
  repaints.
- Settings (the dialog) writes the file in place: the key it changes keeps its
  place and comment, other lines are untouched, and a file that does not parse
  is never overwritten.
- Sizes are multiples of the body size (em, or rows); only radii, border and
  bar widths are px. Per-level values are arrays of 1 to 6 entries, H1 first;
  a short array repeats its last value.
- Colours belong to the theme (see `crates/app/assets/themes/FORMAT.md`); this
  file only recolors single roles (see Colours below).

## Application keys

| Key | Meaning |
| --- | --- |
| `justify` | Stretch wrapped prose rows to the full width (default `false`). |
| `theme` | Theme id; written by Settings. |
| `vault` | Vault folder; written on first start. |

## Tables

"Layout" keys change row heights or widths; the others only repaint.

### `[font]`
| Key | Range / values | Default | Layout |
| --- | --- | --- | --- |
| `family`, `code_family` | `"IBM Plex Mono"` (the only bundled family) | that | yes |
| `size` | px, 8..40 | 16 | yes |
| `line_height` | x size, 1..3 | 1.625 | yes |
| `math_scale` | x text, 0.5..3 | 1.0 | yes |

### `[shape]`
`radius` (px 0..64 or `"full"`, default 0) is the corner of every box that sets
none of its own; `border_width` (px 0..8, default 1).

### `[heading]` (arrays, H1..H6)
| Key | Range | Default | Layout |
| --- | --- | --- | --- |
| `scale` | x size, 0.5..4; a heading row is `round(scale)` body rows | 1.0 | yes |
| `weight` | 100..900 (only Regular and Bold are bundled: 600 and up look bold) | 700 | no |
| `space_above`, `space_below` | empty body rows, 0..4, rounded | 0 | yes |
| `rule` | `true` / `false`: a line under the heading | `false` | no |

### `[inline_code]`, `[code_block]`
| Key | Meaning | Default | Layout |
| --- | --- | --- | --- |
| `inline_code.background` | painted chip behind `code` | `false` | no |
| `inline_code.padding` | em, 0..4 | 0 | no |
| `code_block.background` | box behind fenced code (also insets the lines) | `false` | yes |
| `code_block.padding` | em above and below, 0..4 | 0 | no |
| `code_block.border` | outline | `false` | no |
| `code_block.language_label` | the fence language in the corner (needs `background`) | `false` | no |
| `code_block.font_scale` | 0.5..1 (code can shrink, not grow) | 1.0 | yes |
| `radius` (in either) | px or `"full"`; omitted follows `[shape]` | | no |

### `[callout]`, `[table]`, `[image]`
`callout.bar_width` (px 0..16, default 3); `table.cell_padding` (em 0..4, default
0, layout), `table.zebra` (default `false`), `table.header_bold` (default
`true`); `radius` in each (px or `"full"`; the table's default is 6, the others
follow `[shape]`).

### `[link]`
`underline` and `wikilink_underline`: `"always"`, `"hover"` or `"never"`
(default `"never"`). A hover underline needs the pointer over the link.

### `[blockquote]`
`italic` (default `true`), `bar_width` (px 0..16, default 2), `indent` (em per
level 0..8, default 1.25, layout).

### `[list]`
`indent` (em per level 0..8, default 1.25, layout); `item_gap` (empty body rows
below each item 0..8, rounded, default 0, layout); `bullet_shapes` (`"disc"`,
`"ring"` or `"square"` by depth, deeper levels repeat the last, default all
discs); `number_style` (`"decimal"`, the only style for now).

### `[task]`, `[strikethrough]`
`task.done_style`: `"dim"`, `"strike"`, `"both"` or `"none"` (default) for a
checked item; `task.checkbox_radius` (px or `"full"`, default 3);
`strikethrough.muted` (struck text and its line in the muted colour, default
`false`).

### `[spacing]`
`block_gap` (0..8) is read and checked but nothing uses it yet.

## Colours

`[colors]` recolors single theme roles on top of the active theme and stays
when the theme changes: `heading_1 = "#c0392b"`, `code_background = "#10203040"`,
`quote_bar = "accent"`. A value is `#rrggbb`, `#rrggbbaa` or another role's name.
The roles (including `heading`, `heading_1`..`heading_6`, `code_background`,
`quote_bar`, `link_underline`, and the ten `syntax_*` roles for highlighted code once that lands, #99) are listed in `crates/app/assets/themes/FORMAT.md`.

## Settings dialog

`ctrl+p`, then Open Settings: theme, text size (`font.size`), heading size (four
`heading.scale` presets), corner radius (`shape.radius`) and `justify`.

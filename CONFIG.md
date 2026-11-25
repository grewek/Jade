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
| `code_block.highlight` | colour the tokens of fenced code (`syntax_*` roles); needs `.sublime-syntax` files, see Syntax highlighting below | `false` | no |
| `code_block.syntax_folder` | where the `.sublime-syntax` files are: relative to the config folder, or absolute | `syntaxes` | |
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
`quote_bar`, `link_underline`, and the ten `syntax_*` roles for highlighted code) are listed in `crates/app/assets/themes/FORMAT.md`.

## Syntax highlighting

`code_block.highlight = true` colours the tokens of fenced code (` ```rust `).
It is off by default, and Jade ships **no language definitions**: a language is
a file you put in a folder.

**The folder.** `syntaxes/` inside the config folder, next to `config.toml`
(`~/.config/jade/syntaxes/` on Linux; `target/jade-dev/syntaxes/` in a debug
build; under `JADE_CONFIG_DIR` when that is set). `code_block.syntax_folder`
changes it: a path relative to the config folder, or an absolute one. A vault is
never searched, since vaults get shared.

```
~/.config/jade/
├── config.toml
└── syntaxes/
    ├── Rust.sublime-syntax
    ├── Python.sublime-syntax
    └── Bash.sublime-syntax
```

**Where to get definitions.** Jade reads Sublime Text syntax definitions
(`*.sublime-syntax`). The Sublime Text packages at
<https://github.com/sublimehq/Packages> have one per language, in a folder per
language (`Python/Python.sublime-syntax`). Copy the files into the one folder:
only files directly in it are read, not subfolders. That repository's licence
lets you copy, use, modify and distribute its files, except those that carry
their own licence text or have a `-license` file beside them, so check the
languages you take; Jade does not bundle any.

**Which fence gets which language.** The first word after the backticks is
matched, in any case, against the `name` and the `file_extensions` a definition
lists, so `rust`, `Rust` and `rs` all find the Rust one. A word no definition
knows leaves the block plain, without a message.

**Definitions that include others.** Some refer to other files (Python to its
regular-expression and SQL definitions, Rust to TOML for embedded blocks). Copy
those too; without them a block stops being coloured at the first line that
needs the missing one, and Jade says so once for that language. Not every
Sublime definition loads in Jade's reader either: one it cannot read is skipped
with a message and the others still load.

**Messages** go to stderr, start with `syntax:`, and appear once: one per file
that cannot be read, one when the folder cannot be read, one per language whose
definition needs a file the folder lacks.

**When it happens.** Definitions are read once, in the background, when the first
highlighted block is on screen, and not again until Jade restarts (adding or
changing a file needs a restart; changing `syntax_folder` re-reads). Blocks are
coloured in the background too: a block is plain for a moment (about half a
second the first time, a few milliseconds after), then coloured, and nothing
moves. Only the blocks near the screen are coloured.

**Limits.** A block over 2000 lines or 128 KiB stays plain, and so does a line
over 2000 bytes inside a coloured block. Code inside columns is coloured like
any other, inline code never is.

**Colours** are the ten `syntax_*` theme roles (comment, string, number,
keyword, function, type, constant, operator, punctuation, variable); every
built-in theme sets them. Recolour one in `[colors]`:

```toml
[code_block]
highlight = true

[colors]
syntax_keyword = "#c0392b"
```

## Settings dialog

`ctrl+p`, then Open Settings: theme, text size (`font.size`), heading size (four
`heading.scale` presets), corner radius (`shape.radius`) and `justify`.

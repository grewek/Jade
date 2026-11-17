# Jade theme format

A theme is one TOML file. `jade-light.toml` is the look Jade has today; the rest are Catppuccin. Every theme defines the **same roles**; only the palette behind them differs, so an element has the same role in every theme (a button is `surface`, a link is `link`). The shipped themes are the four Catppuccin flavors, with roles assigned after Catppuccin's [style guide](https://github.com/catppuccin/catppuccin/blob/main/docs/style-guide.md).

```toml
name = "Catppuccin Latte"
kind = "light"            # "light" | "dark": which gpui-component base theme to start from

[palette]                 # raw colors, "#rrggbb", any names
base = "#eff1f5"

[roles]                   # every role below is required; a value is a palette name or "#rrggbb"
background = "base"

[alpha]                   # every key below is required; 0.0..=1.0
selection = 0.25
```

Validation (to be enforced by the loader): all roles and alphas present, no unknown keys, every role value resolves, hex is `#rrggbb`.

## Roles

| Role | Used by | Replaces (hardcoded today) |
|---|---|---|
| `background` | window, editor, active tab, dialogs | theme `background`, `tab_active` |
| `background_secondary` | sidebars, tab bar, status bar | theme `tab_bar` |
| `background_tertiary` | toggle rails, window frame | (new) |
| `surface` | buttons, inputs, hovered rows | `#E8E8F8` (row hover) |
| `surface_hover` | hovered buttons, selected row, active sidebar tab | `#D0D0F0` (selected row, active tab) |
| `surface_active` | pressed buttons | (new) |
| `border` | dividers between panes, tabs, status bar | theme `border` |
| `border_strong` | quote bar, table grid, horizontal rule | `MUTED_COLOR` as a line |
| `text` | body, headings, active tab | theme `foreground`, `tab_active_foreground` |
| `text_muted` | labels, inactive tabs, status bar, placeholders, empty states | `#888888` (4 places), theme `muted_foreground` |
| `text_subtle` | blockquote text, list markers | `MUTED_COLOR` as text |
| `text_on_accent` | text on an accent fill | (new) |
| `accent` | focus, primary buttons, resize handle | theme `primary` |
| `cursor` | text cursor | the ambient text color |
| `selection` | text selection fill (with `alpha.selection`) | `SELECTION_HIGHLIGHT` |
| `link` | `[text](url)`, autolinks | `LINK_COLOR` |
| `wikilink` | `[[wikilink]]` | `WIKILINK_COLOR` |
| `code` | inline code | `CODE_COLOR` |
| `info` | callouts Note, Info | `callout_color` |
| `summary` | callout Summary | `callout_color` |
| `tip` | callout Tip | `callout_color` |
| `success` | callouts Todo, Success | `callout_color` |
| `warning` | callouts Question, Warning | `callout_color` |
| `error` | callouts Failure, Danger, Bug | `callout_color` |
| `neutral` | callouts Example, Quote | `callout_color` |

| Alpha | Used by |
|---|---|
| `callout_background` | the tint behind a callout (its bar is the full role color) |
| `selection` | selection fill |
| `accent_hover`, `accent_active` | resize handle hovered, pressed |

## What collapsed

- `#888888` x4, theme `muted_foreground`: one `text_muted`.
- `#D0D0F0` x2 (selected row, active sidebar tab): one `surface_hover`.
- `#E8E8F8` x2 (two hover rows): one `surface`.
- `MUTED_COLOR` served text, bars, grid lines and rules: split into `text_subtle` and `border_strong`, since text needs contrast a line does not.
- Callout accents: seven hues became seven semantic roles, so diagnostics (spell/grammar) can reuse `warning` and `error`.

## Contrast (WCAG ratio, text on `background`)

Catppuccin's guide gives no ratios. Measured: Mocha, Macchiato and Frappé are 4.4 or better for every text role except `text_subtle` (3.6 to 4.4, used for de-emphasized text). **Latte** is weaker: `text_muted` 4.4, `link` 4.3, `wikilink` 2.8, `code` 3.0, `text_subtle` 2.8, and `text_on_accent` on `accent` 2.8. A loader should allow per-theme role overrides before shipping Latte as is.

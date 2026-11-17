# Jade theme format

Themes are TOML files. `roles.toml` assigns every role a palette color once, so an element has the same role in every theme (a button is `surface`, a link is `link`). A theme file supplies the **palette** and may **override** individual roles. `jade-light.toml` is the look Jade has today; the rest are Catppuccin flavors, with roles assigned after Catppuccin's [style guide](https://github.com/catppuccin/catppuccin/blob/main/docs/style-guide.md).

```toml
# roles.toml: shared by every theme
[roles]                   # every role below is required; a value is a palette name or "#rrggbb"
background = "base"
[alpha]                   # every key below is required; 0.0..=1.0
selection = 0.25

# a theme file
name = "Catppuccin Latte"
kind = "light"            # "light" | "dark": which gpui-component base theme to start from
[palette]                 # raw colors, "#rrggbb", any names (roles.toml uses Catppuccin's)
base = "#eff1f5"
[overrides]               # optional: roles this theme assigns differently, palette name or "#rrggbb"
link = "#1761f5"
[alpha]                   # optional: alphas this theme changes
selection = 0.35
```

A role resolves to its `[overrides]` value if the theme has one, else to `roles.toml`'s. Validation (to be enforced by the loader): no unknown keys or roles, hex is `#rrggbb`, alpha is 0..=1, and every name a resolved role uses exists in the theme's `[palette]` (a theme that overrides a role does not need the palette name `roles.toml` would have used).

Use an override for a real difference (contrast, a palette that lacks the default color), not to restyle one element: that breaks "same role, same look" across themes.

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

## Contrast (WCAG ratio, 4.5 for text)

Catppuccin's guide gives no ratios, so these were measured on the resolved themes (`text_muted` on `background_secondary`, `text_on_accent` on `accent`, the rest on `background`).

- **Mocha, Macchiato, Frappe**: every text role is at least 4.5 except `text_subtle` (4.4, 4.1, 3.6).
- **Latte**: its pastels fell to 2.3 to 4.4, so it overrides 11 roles (`text_muted`, `text_subtle`, `link`, `wikilink`, `code`, `info`, `tip`, `success`, `warning`, `neutral`, `accent`) with darkened versions of the same hues; all measured roles now reach 4.5.
- **Jade Light** reproduces today's colors as they are; several are low (`link` 1.8, `tip` 1.9, `success` 2.2, `warning` 2.5).

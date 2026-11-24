# Jade theme format

Themes are TOML files. `roles.toml` assigns every role a palette color once, so an element has the same role in every theme (a button is `surface`, a link is `link`). A theme file supplies the **palette** and may **override** individual roles. `jade-light.toml` is the look Jade has today, `jade-neon.toml` a dark jade theme with neon accents and `jade-neon-light.toml` its light counterpart; the rest are Catppuccin flavors, with roles assigned after Catppuccin's [style guide](https://github.com/catppuccin/catppuccin/blob/main/docs/style-guide.md).

```toml
# roles.toml: shared by every theme
format_version = 1        # required in every file; bumped when the format changes
[roles]                   # every role below is required; a value is a palette name or "#rrggbb"
background = "base"
[alpha]                   # every key below is required; 0.0..=1.0
selection = 0.25

# a theme file
format_version = 1
name = "Catppuccin Latte"
kind = "light"            # "light" | "dark": which gpui-component base theme to start from
[palette]                 # raw colors, "#rrggbb", any names (roles.toml uses Catppuccin's)
base = "#eff1f5"
[overrides]               # optional: roles this theme assigns differently, palette name or "#rrggbb"
link = "#1761f5"
[alpha]                   # optional: alphas this theme changes
selection = 0.35
```

A role resolves to its `[overrides]` value if the theme has one, else to `roles.toml`'s. A role with a *fallback* (`heading`, `heading_1`..`heading_6`, `quote_bar`, `link_underline`, `code_background`) is not required in `roles.toml`: without a value anywhere it takes the color of the role it follows, so a theme that sets `heading` colors every level, and one that also sets `heading_5` changes just that level. Validation (enforced by `theme::ResolvedTheme::parse`, which reports every problem at once): `format_version` is 1, no unknown keys or roles, name 1-64 bytes, at most 256 palette entries, file at most 64 KiB, hex is `#rrggbb`, alpha is 0..=1, and every name a resolved role uses exists in the theme's `[palette]` (a theme that overrides a role does not need the palette name `roles.toml` would have used).

Use an override for a real difference (contrast, a palette that lacks the default color), not to restyle one element: that breaks "same role, same look" across themes.

## Roles

| Role | Used by | Replaces (hardcoded today) |
|---|---|---|
| `background` | window, editor, active tab, dialogs | theme `background`, `tab_active` |
| `background_secondary` | sidebars, tab bar, status bar | theme `tab_bar` |
| `background_tertiary` | toggle rails, window frame | (new) |
| `scrim` | dim layer behind dialogs (with `alpha.overlay`) | theme `overlay` |
| `surface` | buttons, inputs, hovered rows | `#E8E8F8` (row hover) |
| `surface_hover` | hovered buttons, selected row, active sidebar tab | `#D0D0F0` (selected row, active tab) |
| `surface_active` | pressed buttons | (new) |
| `border` | dividers between panes, tabs, status bar | theme `border` |
| `border_strong` | quote bar, table grid, horizontal rule | `MUTED_COLOR` as a line |
| `table_header` | table header row background | (new) |
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
| `heading` | all headings; follows `text` | (new) |
| `heading_1` | heading level 1; follows `heading` | (new) |
| `heading_2` | heading level 2; follows `heading` | (new) |
| `heading_3` | heading level 3; follows `heading` | (new) |
| `heading_4` | heading level 4; follows `heading` | (new) |
| `heading_5` | heading level 5; follows `heading` | (new) |
| `heading_6` | heading level 6; follows `heading` | (new) |
| `quote_bar` | the bar beside a quote; follows `border_strong` | (new) |
| `link_underline` | the underline of a link; follows `link` | (new) |
| `code_background` | the box behind code; follows `surface` | (new) |
| `highlight` | `==highlight==` (not drawn yet) | (new) |
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
| `overlay` | scrim opacity |
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
- **Jade Neon, Jade Neon Light**: no overrides for contrast and no warnings; every measured role reaches 4.5 (the light one gets there by deepening each hue in its palette).
- **Jade Light** reproduces today's colors as they are; several are low (`link` 1.8, `tip` 1.9, `success` 2.2, `warning` 2.5).

## Mapping to gpui-component (`Theme::apply_config`)

The engine builds one `ThemeConfig` per theme. gpui-component derives most of its 135 colors from a small root set when a key is absent (hover and active from the base, `input` from `border`, `caret` from `primary`, the light base colors from the base colors, button variants from `primary`/`secondary`/`danger`/...), but a key it cannot derive keeps its **stock light/dark default**. So the engine sets the roots below and lets everything else derive. Colors with alpha are written `#rrggbbaa`. Fonts, radii and shadows are not colors and are left alone.

| gpui-component key(s) | Role |
|---|---|
| `background`, `popover.background`, `accordion.background` | `background` |
| `foreground`, `popover.foreground`, `accent.foreground`, `secondary.foreground`, `group_box.foreground`, `tab.active.foreground`, `sidebar.foreground` | `text` |
| `border`, `input.border`, `sidebar.border`, `title_bar.border`, `status_bar.border`, `window.border`, `table.row.border` | `border` |
| `muted.background`, `secondary.background`, `skeleton.background`, `tab_bar.segmented.background`, `list.hover.background`, `table.hover.background` | `surface` |
| `secondary.hover.background`, `accent.background`, `list.active.background`, `table.active.background`, `sidebar.accent.background`, `slider.background`, `switch.background` | `surface_hover` |
| `secondary.active.background` | `surface_active` |
| `muted.foreground`, `tab.foreground`, `group_box.title.foreground`, `description_list.label.foreground`, `table.head.foreground` | `text_muted` |
| `sidebar.background`, `tab_bar.background`, `title_bar.background`, `status_bar.background`, `list.head.background`, `table.head.background`, `group_box.background`, `description_list.label.background` | `background_secondary` |
| `list.background`, `list.even.background`, `table.background`, `table.even.background`, `tab.active.background`, `switch.thumb.background` | `background` |
| `primary.background`, `ring`, `drag.border`, `list.active.border`, `table.active.border`, `progress.bar.background`, `slider.thumb.background`, `sidebar.primary.background` | `accent` |
| `primary.foreground`, `sidebar.primary.foreground`, `danger/success/info/warning.foreground` | `text_on_accent` |
| `caret` | `cursor` |
| `selection.background` | `selection` at `alpha.selection` |
| `drop_target.background` | `accent` at `alpha.accent_hover` |
| `link` | `link` |
| `scrollbar.thumb.background` | `border_strong` |
| `scrollbar.thumb.hover.background` | `text_subtle` |
| `red`, `danger.background` | `error` |
| `green`, `success.background` | `success` |
| `blue`, `info.background` | `info` |
| `yellow`, `warning.background` | `warning` |
| `cyan` | `tip` |
| `magenta` | `summary` |
| `overlay` | `scrim` at `alpha.overlay` |

Left derived (not set): all `button.*` (they follow `secondary`, `primary`, `danger`, ...), every hover/active of the above, `link.hover`/`link.active`, the `*.light` base colors, `scrollbar.background`, `tab.background`, `chart.*`, `title_bar` extras. `background_tertiary`, `neutral` and `code` have no gpui-component counterpart; only Jade's own UI reads them.

### Open items

1. **Stock leaks.** Any key neither set nor derivable stays stock. Done: a test applies Mocha and Latte and fails if any resulting widget color still equals the stock one (verified to catch a dropped mapping), and another checks every mapped key is a real gpui-component key. `link` and `ring` derive correctly without being mapped.
2. **Dialog scrim.** Done: role `scrim` (black in Jade Light, `crust` elsewhere) with `alpha.overlay` 0.50.
3. **Jade Light buttons change** (decided: try the lavender, judge it when seen). Buttons follow `secondary` = `surface` = `#E8E8F8`, where today they are gray `#E4E4E4` (`#D3D3D3` pressed). That follows from "one color for the same element", so Jade Light will look slightly lavender on buttons; if that is unwanted, Jade Light must override `surface` and give up the lavender row hover.
4. **One source of truth.** Jade's own UI stops reading gpui-component's `Theme` colors and reads the engine's palette, so a role cannot be answered by two places.

use std::ops::Range;

use gpui_kit::*;

use super::state::{LineState, TableLineKind, indent_px};
use crate::text_editor::block_preview::LineRole;
use crate::text_editor::callout::Callout;
use crate::text_editor::colors::InlineColors;
use crate::text_editor::image;
use crate::text_editor::live_preview::{
    DisplayLine, InlineStyle, LineIndent, build_callout_header_line, build_display_line,
    build_display_line_in_context, hidden_line, is_thematic_break,
};
use crate::text_editor::table_preview;

/// The `Font` to paint a [`DisplaySegment`](super::live_preview::DisplaySegment)'s
/// visible text in, derived from `base` (the editor's ambient mono font) —
/// only weight/style vary per [`InlineStyle`], not family or size, so every
/// run in a shaped line still lines up on the same baseline/metrics.
fn font_for_style(style: InlineStyle, base: &Font) -> Font {
    let mut font = base.clone();
    match style {
        InlineStyle::Bold | InlineStyle::Heading(_) | InlineStyle::CalloutTitle => {
            font.weight = FontWeight::BOLD
        }
        InlineStyle::Italic | InlineStyle::Blockquote(_) => font.style = FontStyle::Italic,
        InlineStyle::Plain
        | InlineStyle::Code
        | InlineStyle::Strikethrough
        | InlineStyle::WikiLink
        | InlineStyle::Link
        | InlineStyle::ListMarker
        | InlineStyle::Widget(_) => {}
    }
    font
}

/// The color to paint a visible segment's text in — distinct colors for
/// inline code, wikilinks, and blockquote/list-marker text; `base` (the
/// editor's ambient text color) otherwise. `callout` is the line's own
/// callout, if it belongs to one: a header's title is colored with that
/// callout's own accent (`callout_color`) rather than the generic
/// `CalloutTitle` default of `base`, and a callout's *body* text — still
/// styled plain `Blockquote`/`ListMarker` by `live_preview.rs`, which has
/// no idea callouts exist — uses `base` instead of the usual muted
/// blockquote gray, since the background tint already carries the visual
/// distinction a plain quote's dimmer text otherwise provides.
fn color_for_style(style: InlineStyle, colors: &InlineColors, callout: Option<&Callout>) -> Hsla {
    let base = colors.text;
    match style {
        InlineStyle::Code => colors.code,
        InlineStyle::WikiLink => colors.wikilink,
        InlineStyle::Link => colors.link,
        InlineStyle::CalloutTitle => callout.map_or(base, |c| colors.callout(c.kind)),
        InlineStyle::Blockquote(_) | InlineStyle::ListMarker => {
            if callout.is_some() {
                base
            } else {
                colors.subtle
            }
        }
        InlineStyle::Plain
        | InlineStyle::Bold
        | InlineStyle::Italic
        | InlineStyle::Heading(_)
        | InlineStyle::Strikethrough
        | InlineStyle::Widget(_) => base,
    }
}

/// The strikethrough line to paint over a visible segment's text, or `None`
/// for every style but [`InlineStyle::Strikethrough`] itself.
fn strikethrough_for_style(style: InlineStyle, color: Hsla) -> Option<StrikethroughStyle> {
    match style {
        InlineStyle::Strikethrough => Some(StrikethroughStyle {
            thickness: px(1.),
            color: Some(color),
        }),
        _ => None,
    }
}

/// The wrap width to shape a line at, given how much horizontal space
/// `available_width` actually leaves it once its own [`indent_px`] shift is
/// accounted for — `None` (no wrapping at all) for a table row/rule
/// (`table_kind`, unless `table_active`, since wrapping would break its
/// column alignment; an *active* table renders as plain raw text instead,
/// which wraps like any other line), `Some(width)` otherwise. `width` is
/// floored at `1px` so a pathologically narrow container can't make
/// `shape_text` divide-by-zero or loop forever trying to fit even a single
/// character.
fn wrap_width_for(
    table_kind: &TableLineKind,
    table_active: bool,
    available_width: Pixels,
    indent_level: u8,
) -> Option<Pixels> {
    if table_active {
        return Some((available_width - indent_px(indent_level)).max(px(1.)));
    }
    match table_kind {
        TableLineKind::None => Some((available_width - indent_px(indent_level)).max(px(1.))),
        TableLineKind::Delimiter | TableLineKind::Row(_) => None,
    }
}

/// Shapes `line`'s Live Preview display text into one multi-styled,
/// soft-wrapped `WrappedLine` (so a line too wide for the editor's own
/// width wraps onto additional visual rows instead of overflowing
/// horizontally — `wrap_width_for` picks the width to wrap at, or `None`
/// for a table row/rule, which must stay unwrapped or its column alignment
/// breaks), alongside the [`DisplayLine`] mapping needed to convert a raw
/// buffer offset into a position in it (or back). `role` (from
/// `block_preview::classify_lines`, computed once per document) picks
/// between three cases:
/// - [`LineRole::FenceContent`]: always literal (`build_display_line(line,
///   true)`, i.e. the same "show it verbatim" path an active line takes) —
///   code fence content is never Markdown, so `active` doesn't apply to it.
/// - [`LineRole::FenceDelimiter`]: raw when `active`, else [`hidden_line`]
///   — the fence marker itself has no "remainder" to show once hidden,
///   unlike every other hidden-prefix case in `live_preview.rs`.
/// - [`LineRole::Plain`]: a callout header line (`callout_is_header`, from
///   `callout::classify_callouts`) outside any table renders via
///   `build_callout_header_line` instead — see that function's own doc
///   comment; a callout's *body* lines need no special dispatch here at
///   all, rendering as ordinary blockquote content exactly like any other
///   blockquote (only their *color* differs, resolved by `color_for_style`
///   from the `callout` parameter at paint time). A thematic break
///   (`is_thematic_break`, `---`/`***`/`___`) outside any table and not the
///   active line is [`hidden_line`] too — `paint_lines` paints it as a real
///   vector rule directly over the gap, same reasoning as a table's own
///   grid lines. Otherwise, a line that isn't part of a table renders via
///   `build_display_line_in_context(line, active, block_ctx)` — `block_ctx`
///   (from `live_preview::classify_block_context`, also computed once per
///   document) resolves nested-list depth and lazy continuation, neither
///   of which `line` alone carries enough information for. A line that
///   *is* part of a table (`table_kind`, from `table_line_kind`) instead
///   renders as an aligned grid row/rule via `build_table_row`/
///   `build_table_rule` — unless `table_active` (the cursor is anywhere in
///   that whole table, from `is_table_active`), in which case it falls
///   back to raw text, same as every other construct's active line. A
///   table's delimiter row has no cell content of its own to show at all,
///   so it's just [`hidden_line`] — `element.rs`'s `paint_lines` paints its
///   header/body divider as a real vector line directly over it (see
///   `paint_table_grid_lines`), not as dashed-rule text.
///
/// This is the *pure* half of that dispatch — no real text-shaping happens
/// here (see [`mono_char_width`]'s own "cheap, cached single-character
/// shape" exception) — split out from [`shape_display_line`] so
/// [`analyze_line_layout`] can get a line's real `display_text` (and so its
/// real wrapped-row count, computed arithmetically via [`wrapped_row_count`]
/// — this editor's font is always monospace, so a row count is just a
/// character-count/width division) without paying for the actual expensive
/// `shape_text` call `shape_display_line` still does, for every line it's
/// actually asked to shape. See this module's own notes on viewport-bounded
/// rendering for why that split matters.
pub(in crate::text_editor) fn build_display_lines(
    line: &str,
    state: &LineState,
    font: &Font,
    font_size: Pixels,
    text_color: Hsla,
    available_width: Pixels,
    window: &mut Window,
) -> (Vec<DisplayLine>, Option<Pixels>) {
    let LineState {
        role,
        block_ctx,
        ref table_kind,
        table_active,
        callout_is_header,
        active,
        ..
    } = *state;
    if state.hidden {
        return (Vec::new(), None);
    }
    let block_rows = image::embed(line, state).map(|embed| {
        let sizes = state.images;
        image::rows(line, &embed, sizes, available_width, window.line_height())
    });
    let mut display_lines: Vec<DisplayLine> = match role {
        // A block is `rows` stacked empty rows; `paint_lines` draws the box over them.
        LineRole::Plain if let Some(rows) = block_rows => vec![hidden_line(line); rows],
        LineRole::FenceContent => vec![build_display_line(line, true)],
        LineRole::FenceDelimiter if !active => vec![hidden_line(line)],
        LineRole::FenceDelimiter => vec![build_display_line(line, active)],
        LineRole::Plain if table_active && !matches!(table_kind, TableLineKind::None) => {
            vec![build_display_line(line, true)]
        }
        LineRole::Plain if state.column_frame && !active => vec![hidden_line(line)],
        LineRole::Plain if callout_is_header => vec![build_callout_header_line(line, active)],
        LineRole::Plain => match table_kind {
            TableLineKind::None if !active && is_thematic_break(line) => vec![hidden_line(line)],
            TableLineKind::None => vec![build_display_line_in_context(line, active, block_ctx)],
            TableLineKind::Delimiter => vec![hidden_line(line)],
            // `paint_table_row` draws the cells over these empty rows.
            TableLineKind::Row(table) => {
                let max_chars =
                    table_row_char_budget(available_width, font, font_size, text_color, window);
                let widths = table_preview::resolve_widths(&table.columns, max_chars);
                vec![
                    hidden_line(line);
                    table_preview::row_line_count_editing(line, &widths, state.editing_cell)
                ]
            }
        },
    };

    // Fenced code content is never Markdown (see `shape_display_line`'s own
    // doc comment) — so even if it happens to start with `>` or `- `,
    // that's not a real blockquote/list line and shouldn't indent as one.
    if matches!(role, LineRole::FenceContent) {
        for display_line in &mut display_lines {
            display_line.indent = LineIndent::None;
        }
    }

    // A table's own indent is always `None` across every one of its
    // (possibly several, once cells wrap) returned lines, same as every
    // other case here only ever returning one — safe to read just the
    // first for the whole line's wrap width.
    let wrap_width = wrap_width_for(
        table_kind,
        table_active,
        available_width,
        display_lines[0].indent.level(),
    );
    (display_lines, wrap_width)
}

/// How many visual rows `display_text` wraps onto at `wrap_width`, given a
/// monospace advance of `char_width` — pure arithmetic (a character-count/
/// width division), not a real `shape_text` call. This editor's font is
/// always monospace (`render_editor`'s own font override), so this is exact
/// for *wrap existing or not* (`wrap_width` is `None` for a table row/rule,
/// which never wraps — always `1`) and a close, cheap stand-in for the real
/// row count otherwise — real `shape_text` wraps at word boundaries, not raw
/// character counts, so this can be off by a row for text with unusually
/// long/short words, a deliberate v1 approximation (see this module's own
/// notes on viewport-bounded rendering) rather than requiring the real
/// shaping call this function exists specifically to avoid.
pub(in crate::text_editor) fn wrapped_row_count(
    display_text: &str,
    wrap_width: Option<Pixels>,
    char_width: Pixels,
) -> usize {
    let Some(wrap_width) = wrap_width else {
        return 1;
    };
    if char_width <= px(0.) {
        return 1;
    }
    let chars_per_row = ((wrap_width / char_width) as usize).max(1);
    // Bytes are never fewer than chars, so a text whose byte length fits
    // needs no counting: the common case, and counting dominated opening a
    // note with hundreds of thousands of lines.
    if display_text.len() <= chars_per_row {
        return 1;
    }
    let char_count = display_text.chars().count();
    char_count.max(1).div_ceil(chars_per_row).max(1)
}

/// [`build_display_lines`] (see its own doc comment for the actual
/// per-role dispatch logic) plus the real, expensive `shape_text` call —
/// the full, painted-output version of that pure function. Only called for
/// lines actually being rendered/hit-tested (the viewport's visible
/// range), not every line in the document — see this module's own notes
/// on viewport-bounded rendering.
pub(in crate::text_editor) fn shape_display_line(
    line: &str,
    state: &LineState,
    font: &Font,
    font_size: Pixels,
    colors: impl Into<InlineColors>,
    available_width: Pixels,
    window: &mut Window,
) -> Vec<(WrappedLine, DisplayLine)> {
    let colors: InlineColors = colors.into();
    let text_color = colors.text;
    let (display_lines, wrap_width) = build_display_lines(
        line,
        state,
        font,
        font_size,
        text_color,
        available_width,
        window,
    );
    let (broken, callout) = (state.broken, state.callout);

    display_lines
        .into_iter()
        .map(|display_line| {
            let wrapped = shape_entry(
                &display_line,
                wrap_width,
                (font, font_size),
                &colors,
                (broken, callout),
                false,
                window,
            );
            (wrapped, display_line)
        })
        .collect()
}

/// The styled runs of `display_line`'s text, one per visible segment (they tile
/// `display_text` in order); `bold` makes every run bold (a table header).
pub(in crate::text_editor) fn line_runs(
    display_line: &DisplayLine,
    (font, colors): (&Font, &InlineColors),
    (broken, callout): (&[Range<usize>], Option<&Callout>),
    bold: bool,
) -> Vec<TextRun> {
    let text_color = colors.text;
    let mut runs: Vec<TextRun> = display_line
        .segments
        .iter()
        .filter(|seg| !seg.is_hidden())
        .map(|seg| {
            let is_broken = matches!(seg.style, InlineStyle::Link | InlineStyle::WikiLink)
                && broken
                    .iter()
                    .any(|r| r.start <= seg.raw.start && seg.raw.end <= r.end);
            let color = if is_broken {
                colors.broken
            } else {
                color_for_style(seg.style, colors, callout)
            };
            let mut font = font_for_style(seg.style, font);
            if bold {
                font.weight = FontWeight::BOLD;
            }
            TextRun {
                len: seg.display.len(),
                font,
                color,
                background_color: None,
                underline: None,
                strikethrough: strikethrough_for_style(seg.style, color),
            }
        })
        .collect();
    if runs.is_empty() {
        runs.push(TextRun {
            len: 0,
            font: font.clone(),
            color: text_color,
            background_color: None,
            underline: None,
            strikethrough: None,
        });
    }
    runs
}

/// Shapes one display line. `bold` makes every run bold (a table header).
pub(in crate::text_editor) fn shape_entry(
    display_line: &DisplayLine,
    wrap_width: Option<Pixels>,
    (font, font_size): (&Font, Pixels),
    colors: &InlineColors,
    (broken, callout): (&[Range<usize>], Option<&Callout>),
    bold: bool,
    window: &mut Window,
) -> WrappedLine {
    let runs = line_runs(display_line, (font, colors), (broken, callout), bold);
    window
        .text_system()
        .shape_text(
            display_line.display_text.clone().into(),
            font_size,
            &runs,
            wrap_width,
            None,
        )
        .expect("shaping a single line of text should not fail")
        .pop()
        .expect("shape_text on a \\n-free string returns exactly one WrappedLine")
}

/// This editor's monospace advance width — one character's pixel width,
/// identical for every character since the font is always monospace (see
/// `render_editor`'s own font override). `0.` if it can't be measured for
/// some reason. Measured by shaping a single reference character rather
/// than hard-coding an advance width, so it stays correct across font/size
/// changes without either caller needing to know either value's meaning —
/// cheap even called once per table row/rule/grid-line, since GPUI caches
/// shaped text internally and this exact one-character shape repeats every
/// call. Shared by `table_row_char_budget` (converts it to a *character*
/// budget) and `paint_table_grid_lines` (converts a *character offset* to
/// an actual pixel x) — both need the same measurement to agree.
pub(in crate::text_editor) fn mono_char_width(
    font: &Font,
    font_size: Pixels,
    text_color: Hsla,
    window: &mut Window,
) -> Pixels {
    let run = TextRun {
        len: 1,
        font: font.clone(),
        color: text_color,
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    window
        .text_system()
        .shape_line("0".into(), font_size, &[run], None)
        .width
}

/// The character budget a table row/rule should fit within, given
/// `available_width` — `None` if that's non-positive (shouldn't happen in
/// practice, but a column-width budget of `0` would make
/// `table_preview::cap_column_widths` collapse every column uselessly) or
/// [`mono_char_width`] can't be measured for some reason.
fn table_row_char_budget(
    available_width: Pixels,
    font: &Font,
    font_size: Pixels,
    text_color: Hsla,
    window: &mut Window,
) -> Option<usize> {
    if available_width <= px(0.) {
        return None;
    }
    char_budget(
        available_width,
        mono_char_width(font, font_size, text_color, window),
    )
}

/// How many monospace characters fit on a row indented `level` levels
/// (zero if `char_width` is not positive); the wrap width is floored at 1px
/// like [`wrap_width_for`].
pub(in crate::text_editor) fn fit_with_indent(
    available_width: Pixels,
    char_width: Pixels,
    level: u8,
) -> usize {
    if char_width <= px(0.) {
        return 0;
    }
    ((available_width - indent_px(level)).max(px(1.)) / char_width) as usize
}

/// How many monospace characters of `char_width` fit in `available_width`;
/// `None` if either is non-positive. Pure, so a loop over many lines can
/// measure `char_width` once ([`mono_char_width`] shapes text).
pub(in crate::text_editor) fn char_budget(
    available_width: Pixels,
    char_width: Pixels,
) -> Option<usize> {
    if available_width <= px(0.) || char_width <= px(0.) {
        return None;
    }
    Some((available_width / char_width) as usize)
}

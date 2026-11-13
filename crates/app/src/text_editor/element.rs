use gpui_kit::*;

use super::block_preview::LineRole;
use super::live_preview::{
    build_display_line, build_display_line_in_context, build_table_row, classify_block_context, hidden_line, is_thematic_break,
    DisplayLine, InlineStyle, LineContext, LineIndent,
};
use super::table_preview::{self, classify_tables, Table};
use super::Editor;

/// Which table-related rendering a `LineRole::Plain` line needs, resolved
/// by the caller (which already knows the line's absolute index, needed to
/// tell a table's delimiter row from its header/body rows) — see
/// `table_line_kind`.
pub(super) enum TableLineKind<'a> {
    None,
    Delimiter(&'a Table),
    Row(&'a Table),
}

/// Resolves `line_index`'s [`TableLineKind`] from `classify_tables`'
/// per-line table-index lookup and the resolved `tables` themselves.
pub(super) fn table_line_kind<'a>(tables: &'a [Table], table_for_line: &[Option<usize>], line_index: usize) -> TableLineKind<'a> {
    match table_for_line[line_index] {
        None => TableLineKind::None,
        Some(idx) => {
            let table = &tables[idx];
            if table.is_delimiter(line_index) {
                TableLineKind::Delimiter(table)
            } else {
                TableLineKind::Row(table)
            }
        }
    }
}

/// Whether `line_index`'s table (if any) should render raw rather than as
/// an aligned grid — true whenever the cursor sits anywhere inside that
/// table's own byte range, not just on this specific line. Unlike every
/// other construct's single "active line," a table's row/column alignment
/// can't be resolved from one line considered in isolation, so there's no
/// sensible way to keep the rest of the grid rendered while one row inside
/// it goes raw — the whole table becomes the unit of "active," same
/// `focused`-gating as `is_active_line`.
pub(super) fn is_table_active(tables: &[Table], table_for_line: &[Option<usize>], line_index: usize, cursor: usize, focused: bool) -> bool {
    focused
        && table_for_line[line_index].is_some_and(|idx| {
            let range = &tables[idx].byte_range;
            cursor >= range.start && cursor <= range.end
        })
}

/// A pale, half-transparent highlight — a reasonable Phase 1 default, not yet
/// pulled from the app's theme (`ActiveTheme`/`cx.theme()`, as `gpui-component`
/// itself does for its own selection color).
const SELECTION_HIGHLIGHT: Hsla = Hsla { h: 0.6, s: 0.9, l: 0.6, a: 0.35 };

/// Live Preview colors for inline code, wikilink, and blockquote/list-marker
/// text — same "flat constant, not yet pulled from the theme" Phase 1
/// compromise as `SELECTION_HIGHLIGHT` above.
const CODE_COLOR: Hsla = Hsla { h: 0.75, s: 0.2, l: 0.55, a: 1.0 };
const WIKILINK_COLOR: Hsla = Hsla { h: 0.58, s: 0.65, l: 0.6, a: 1.0 };
/// Distinct hue from `WIKILINK_COLOR` so an internal `[[wikilink]]` and an
/// external/inline `[text](url)` link or autolink read as visually
/// different kinds of thing.
const LINK_COLOR: Hsla = Hsla { h: 0.5, s: 0.65, l: 0.55, a: 1.0 };
const MUTED_COLOR: Hsla = Hsla { h: 0.0, s: 0.0, l: 0.6, a: 1.0 };

/// Pixel width of one structural indent level (see
/// `live_preview::LineIndent`) — applied identically by `paint_lines`,
/// `offset_for_position`, and (via [`indent_px`]) `input_handler.rs`'s
/// `bounds_for_range`, so a line's painted position and its hit-testing/IME
/// math always agree.
const INDENT_UNIT: Pixels = px(20.);

/// The left inset, within an indent level's own lane, that a blockquote's
/// quote bar is painted at — see `paint_lines`.
const QUOTE_BAR_INSET: Pixels = px(4.);
const QUOTE_BAR_WIDTH: Pixels = px(2.);

/// The thickness of a table's grid lines — see `paint_table_grid_lines`.
const TABLE_LINE_WIDTH: Pixels = px(1.);

/// The pixel x-offset `level` structural indent levels shifts a line's
/// content by — `level` is [`LineIndent::level`], `0` for a line with no
/// indent.
pub(super) fn indent_px(level: u8) -> Pixels {
    INDENT_UNIT * level as f32
}

/// Whether the line spanning `line_range` should render raw/unstyled rather
/// than with Live Preview markup hidden: only while `focused` (the editor
/// actually has focus — a freshly opened note has an unfocused editor whose
/// `EditorBuffer::cursor()` still defaults to byte `0`, which would
/// otherwise permanently pin line 0 as "active" even though no editing is
/// happening) *and* `cursor` sits inside `line_range` (inclusive of both
/// ends, so a cursor resting right at a line's last byte, before its `\n`,
/// still counts).
pub(super) fn is_active_line(line_range: &std::ops::Range<usize>, cursor: usize, focused: bool) -> bool {
    focused && cursor >= line_range.start && cursor <= line_range.end
}

/// Scrolls `scroll_handle`'s tracked container (`main.rs`'s note-editor-pane
/// div — see `Editor::scroll_handle`'s doc comment) just enough to bring the
/// cursor's line fully into view, if it currently isn't. `content_bounds` is
/// this element's own (already scroll-shifted) on-screen bounds, the same
/// `bounds` `paint_lines`/mouse hit-testing use. Adding `cursor_top` (the
/// cursor's line's accumulated Y offset *within* the document — see
/// `measure_document`, since a wrapped line ahead of it can make this more
/// than `line_height * cursor_line_index`) to that origin gives the cursor
/// line's actual current screen position, directly comparable against
/// `scroll_handle.bounds()` (the viewport's fixed on-screen bounds, which
/// don't move as you scroll). `cursor_height` is that same line's own
/// height (a multiple of `line_height` if it's wrapped across several
/// rows), so a tall wrapped line scrolls its *bottom* row into view rather
/// than stopping as soon as its top row is visible.
///
/// A no-op if the cursor's line is already fully visible, or before the
/// container has been laid out at least once (`scroll_handle.bounds()` is
/// zero-sized until then).
fn autoscroll_to_cursor(scroll_handle: &ScrollHandle, content_bounds: Bounds<Pixels>, cursor_top: Pixels, cursor_height: Pixels) {
    let viewport = scroll_handle.bounds();
    if viewport.size.height <= px(0.) {
        return;
    }
    let cursor_top = content_bounds.origin.y + cursor_top;
    let cursor_bottom = cursor_top + cursor_height;
    let viewport_top = viewport.origin.y;
    let viewport_bottom = viewport_top + viewport.size.height;

    let mut offset = scroll_handle.offset();
    if cursor_top < viewport_top {
        offset.y += viewport_top - cursor_top;
    } else if cursor_bottom > viewport_bottom {
        offset.y += viewport_bottom - cursor_bottom;
    } else {
        return;
    }
    // `ScrollHandle::max_offset()` is a positive magnitude (how much
    // scrollable overflow there is), not the offset's own lower bound — the
    // valid offset range is `[-max_offset, 0]`, same convention `gpui-pre`'s
    // own scroll-clamping code uses internally.
    let max_offset = scroll_handle.max_offset();
    offset.y = offset.y.clamp(-max_offset.y, px(0.));
    scroll_handle.set_offset(offset);
}

/// Computes the whole document's total rendered height and the cursor's
/// own line's accumulated top-Y offset and height — needed by
/// `render_editor` *before* it can construct its element tree (the outer
/// `div`'s own `.h(...)` sizing, and `autoscroll_to_cursor`'s inputs),
/// which is before the canvas paint closure where `paint_lines`/
/// `offset_for_position` do their own per-line shaping as they go. Trivial
/// when every line was exactly one `line_height` tall; not anymore now
/// that a line can soft-wrap into several rows (see `shape_display_line`'s
/// own `wrap_width_for`), so this shapes every line once just to measure
/// it — the same per-line dispatch (`is_active_line`/`table_line_kind`/
/// `is_table_active`) `paint_lines` uses, since a line's *active* wrapping
/// shows different (unhidden) text than its previewed form and so can
/// genuinely wrap differently.
#[allow(clippy::too_many_arguments)]
fn measure_document(
    text: &str,
    line_roles: &[LineRole],
    block_contexts: &[LineContext],
    table_for_line: &[Option<usize>],
    tables: &[Table],
    cursor: usize,
    focused: bool,
    available_width: Pixels,
    line_height: Pixels,
    font: &Font,
    font_size: Pixels,
    text_color: Hsla,
    window: &mut Window,
) -> (Pixels, Pixels, Pixels) {
    let mut total = px(0.);
    let mut cursor_top = px(0.);
    let mut cursor_height = line_height;
    let mut line_start = 0usize;
    for (i, line_text) in text.split('\n').enumerate() {
        let line_range = line_start..line_start + line_text.len();
        let active = is_active_line(&line_range, cursor, focused);
        let table_kind = table_line_kind(tables, table_for_line, i);
        let table_active = is_table_active(tables, table_for_line, i, cursor, focused);
        let results = shape_display_line(
            line_text, line_roles[i], block_contexts[i], table_kind, table_active, active, font, font_size, text_color,
            available_width, window,
        );
        // Usually one entry; more than one only for a table row whose
        // cells had to wrap (see `shape_display_line`'s own doc comment) —
        // the raw line's total height is their combined height either way.
        let height: Pixels = results.iter().map(|(wrapped, _)| wrapped.size(line_height).height).sum();
        if cursor >= line_range.start && cursor <= line_range.end {
            cursor_top = total;
            cursor_height = height;
        }
        total += height;
        line_start = line_range.end + 1;
    }
    (total, cursor_top, cursor_height)
}

/// The `Font` to paint a [`DisplaySegment`](super::live_preview::DisplaySegment)'s
/// visible text in, derived from `base` (the editor's ambient mono font) —
/// only weight/style vary per [`InlineStyle`], not family or size, so every
/// run in a shaped line still lines up on the same baseline/metrics.
fn font_for_style(style: InlineStyle, base: &Font) -> Font {
    let mut font = base.clone();
    match style {
        InlineStyle::Bold | InlineStyle::Heading(_) => font.weight = FontWeight::BOLD,
        InlineStyle::Italic | InlineStyle::Blockquote(_) => font.style = FontStyle::Italic,
        InlineStyle::Plain | InlineStyle::Code | InlineStyle::Strikethrough | InlineStyle::WikiLink | InlineStyle::Link | InlineStyle::ListMarker => {}
    }
    font
}

/// The color to paint a visible segment's text in — distinct colors for
/// inline code, wikilinks, and blockquote/list-marker text; `base` (the
/// editor's ambient text color) otherwise.
fn color_for_style(style: InlineStyle, base: Hsla) -> Hsla {
    match style {
        InlineStyle::Code => CODE_COLOR,
        InlineStyle::WikiLink => WIKILINK_COLOR,
        InlineStyle::Link => LINK_COLOR,
        InlineStyle::Blockquote(_) | InlineStyle::ListMarker => MUTED_COLOR,
        InlineStyle::Plain | InlineStyle::Bold | InlineStyle::Italic | InlineStyle::Heading(_) | InlineStyle::Strikethrough => base,
    }
}

/// The strikethrough line to paint over a visible segment's text, or `None`
/// for every style but [`InlineStyle::Strikethrough`] itself.
fn strikethrough_for_style(style: InlineStyle, color: Hsla) -> Option<StrikethroughStyle> {
    match style {
        InlineStyle::Strikethrough => Some(StrikethroughStyle { thickness: px(1.), color: Some(color) }),
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
fn wrap_width_for(table_kind: &TableLineKind, table_active: bool, available_width: Pixels, indent_level: u8) -> Option<Pixels> {
    if table_active {
        return Some((available_width - indent_px(indent_level)).max(px(1.)));
    }
    match table_kind {
        TableLineKind::None => Some((available_width - indent_px(indent_level)).max(px(1.))),
        TableLineKind::Delimiter(_) | TableLineKind::Row(_) => None,
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
/// - [`LineRole::Plain`]: a thematic break (`is_thematic_break`, `---`/
///   `***`/`___`) outside any table and not the active line is [`hidden_line`]
///   too — `paint_lines` paints it as a real vector rule directly over the
///   gap, same reasoning as a table's own grid lines. Otherwise, a line that
///   isn't part of a table renders via
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
#[allow(clippy::too_many_arguments)]
pub(super) fn shape_display_line(
    line: &str,
    role: LineRole,
    block_ctx: LineContext,
    table_kind: TableLineKind,
    table_active: bool,
    active: bool,
    font: &Font,
    font_size: Pixels,
    text_color: Hsla,
    available_width: Pixels,
    window: &mut Window,
) -> Vec<(WrappedLine, DisplayLine)> {
    let display_lines: Vec<DisplayLine> = match role {
        LineRole::FenceContent => vec![build_display_line(line, true)],
        LineRole::FenceDelimiter if !active => vec![hidden_line(line)],
        LineRole::FenceDelimiter => vec![build_display_line(line, active)],
        LineRole::Plain if table_active && !matches!(table_kind, TableLineKind::None) => {
            vec![build_display_line(line, true)]
        }
        LineRole::Plain => match table_kind {
            TableLineKind::None if !active && is_thematic_break(line) => vec![hidden_line(line)],
            TableLineKind::None => vec![build_display_line_in_context(line, active, block_ctx)],
            TableLineKind::Delimiter(_) => vec![hidden_line(line)],
            TableLineKind::Row(table) => {
                let max_chars = table_row_char_budget(available_width, font, font_size, text_color, window);
                build_table_row(line, &table.columns, max_chars)
            }
        },
    };

    // A table's own indent is always `None` across every one of its
    // (possibly several, once cells wrap) returned lines, same as every
    // other case here only ever returning one — safe to read just the
    // first for the whole line's wrap width.
    let wrap_width = wrap_width_for(&table_kind, table_active, available_width, display_lines[0].indent.level());

    display_lines
        .into_iter()
        .map(|mut display_line| {
            // Fenced code content is never Markdown (see this function's
            // own doc comment above) — so even if it happens to start with
            // `>` or `- `, that's not a real blockquote/list line and
            // shouldn't indent as one.
            if matches!(role, LineRole::FenceContent) {
                display_line.indent = LineIndent::None;
            }

            let mut runs: Vec<TextRun> = display_line
                .segments
                .iter()
                .filter(|seg| !seg.is_hidden())
                .map(|seg| {
                    let color = color_for_style(seg.style, text_color);
                    TextRun {
                        len: seg.display.len(),
                        font: font_for_style(seg.style, font),
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

            let wrapped = window
                .text_system()
                .shape_text(display_line.display_text.clone().into(), font_size, &runs, wrap_width, None)
                .expect("shaping a single line of text should not fail")
                .pop()
                .expect("shape_text on a \\n-free string returns exactly one WrappedLine");
            (wrapped, display_line)
        })
        .collect()
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
fn mono_char_width(font: &Font, font_size: Pixels, text_color: Hsla, window: &mut Window) -> Pixels {
    let run = TextRun { len: 1, font: font.clone(), color: text_color, background_color: None, underline: None, strikethrough: None };
    window.text_system().shape_line("0".into(), font_size, &[run], None).width
}

/// The character budget a table row/rule should fit within, given
/// `available_width` — `None` if that's non-positive (shouldn't happen in
/// practice, but a column-width budget of `0` would make
/// `table_preview::cap_column_widths` collapse every column uselessly) or
/// [`mono_char_width`] can't be measured for some reason.
fn table_row_char_budget(available_width: Pixels, font: &Font, font_size: Pixels, text_color: Hsla, window: &mut Window) -> Option<usize> {
    if available_width <= px(0.) {
        return None;
    }
    let char_width = mono_char_width(font, font_size, text_color, window);
    if char_width <= px(0.) {
        return None;
    }
    Some((available_width / char_width) as usize)
}

/// Renders `editor`'s buffer: each `\n`-delimited line shaped and painted, a
/// selection highlight rect behind any selected text, and (while focused) a
/// thin cursor caret. Also registers mouse handling for the current frame
/// (click to place the cursor, drag to select) — keyboard input is the next
/// Phase 1 step.
///
/// Composed from plain `div()` + `canvas()` rather than a hand-implemented
/// `gpui::Element` — the outer `div` gets sizing and focus for free from the
/// exact same builder methods already used throughout `main.rs`; `canvas`
/// only needs to own the actual text/cursor painting and (since it has
/// `bounds` in scope precisely when painting, unlike an event registered
/// outside of paint) the mouse hit-testing math.
pub(super) fn render_editor(
    editor: &Editor,
    editor_entity: Entity<Editor>,
    window: &mut Window,
    cx: &mut Context<Editor>,
) -> impl IntoElement {
    // Read straight from `editor` (the `&mut self` `Editor::render` already
    // has), not `editor_entity.read(cx)` — this function runs *as* that
    // entity's own render, so reading through the entity handle here panics
    // ("cannot read while already being updated"). `editor_entity` is only
    // for the mouse closures below, which run later, once rendering has
    // finished and reading/updating the entity is safe again.
    let buffer = editor.buffer();
    let text = buffer.text().to_string();
    let cursor = buffer.cursor();
    let selection = buffer.selection();
    let focus_handle = editor.focus_handle.clone();
    let scroll_handle = editor.scroll_handle.clone();
    let last_autoscrolled_cursor = editor.last_autoscrolled_cursor.clone();
    // Computed once per paint (not per line) since it needs the whole
    // document to tell whether a given line sits inside an open code
    // fence — see `block_preview`'s module doc.
    let line_roles = super::block_preview::classify_lines(&text);
    // Same "whole document, once per paint" reasoning as `line_roles` above
    // — nested-list depth and lazy continuation both depend on prior lines,
    // see `live_preview::classify_block_context`'s own doc comment.
    let block_contexts = classify_block_context(&text);
    // Same reasoning again — a table's column widths/alignment depend on
    // every row together, see `table_preview`'s own module doc.
    let (table_for_line, tables) = classify_tables(&text);

    let line_height = window.line_height();
    let focused = focus_handle.is_focused(window);

    let text_style = window.text_style();
    // The note editor's own font, not the ambient `text_style`'s — only the
    // bundled mono family has real Bold/Italic/BoldItalic faces (the Sans UI
    // family only has Regular/Italic), which Live Preview's bold/italic runs
    // need. Fixes the font-family gap `text_editor`'s module doc previously
    // called out as a deferred cosmetic cut, now that this code path
    // actually needs the distinction.
    let mut font = text_style.font();
    font.family = gpui_kit::component::Theme::global(cx).mono_font_family.clone();
    let font_size = text_style.font_size.to_pixels(window.rem_size());
    let text_color = text_style.color;

    // The editor's own on-screen width isn't known yet at this point (this
    // function only *builds* the element tree — actual layout, and the
    // `bounds` the canvas paint closure below receives, happens afterward)
    // but is needed now regardless, to size this `div`'s height up front
    // once wrapping can make that depend on width. `scroll_handle`'s
    // tracked container bounds (the same "last completed layout" value
    // `autoscroll_to_cursor` already relies on) stands in for it — zero
    // (and so falls back to effectively "don't wrap" via `Pixels::MAX`,
    // same as this editor's pre-wrapping behavior) only before the very
    // first layout, self-correcting the moment that completes.
    let available_width = {
        let width = scroll_handle.bounds().size.width;
        if width > px(0.) { width } else { Pixels::MAX }
    };
    let (total_height, cursor_top, cursor_height) = measure_document(
        &text, &line_roles, &block_contexts, &table_for_line, &tables, cursor, focused, available_width,
        line_height, &font, font_size, text_color, window,
    );

    div()
        .id("editor")
        .test_support()
        .w_full()
        .h(total_height)
        .track_focus(&focus_handle)
        // Must out-rank `gpui_kit::init`'s global `Root` component, which
        // binds plain "tab" to its own focus-next action in a "Root"
        // context wrapping the whole app — see `Editor::handle_insert_tab`'s
        // doc comment for why an `on_key_down` match arm alone can't do this.
        .key_context("TextEditor")
        .on_action(cx.listener(Editor::handle_insert_tab))
        .on_key_down(cx.listener(Editor::handle_key_down))
        .child(
            canvas(
                move |_, _, _| {},
                move |bounds, _, window, cx| {
                    let focused = focus_handle.is_focused(window);
                    // Only snap the view back to the cursor the first time
                    // we see this particular cursor position, not on every
                    // repaint — a plain mouse-wheel scroll alone triggers a
                    // repaint too (with the cursor unchanged), and without
                    // this check it would get fought right back to the
                    // cursor's line the moment that line scrolled out of
                    // view. See `last_autoscrolled_cursor`'s own doc comment.
                    if focused && last_autoscrolled_cursor.get() != Some(cursor) {
                        autoscroll_to_cursor(&scroll_handle, bounds, cursor_top, cursor_height);
                        last_autoscrolled_cursor.set(Some(cursor));
                    }
                    paint_lines(
                        &text,
                        &line_roles,
                        &block_contexts,
                        &table_for_line,
                        &tables,
                        cursor,
                        selection.clone(),
                        focused,
                        bounds,
                        line_height,
                        &font,
                        font_size,
                        text_color,
                        window,
                        cx,
                    );

                    register_mouse_handling(
                        editor_entity.clone(),
                        text.clone(),
                        line_roles.clone(),
                        block_contexts.clone(),
                        table_for_line.clone(),
                        tables.clone(),
                        cursor,
                        focused,
                        bounds,
                        line_height,
                        font.clone(),
                        font_size,
                        text_color,
                        focus_handle.clone(),
                        window,
                    );

                    // Only registers while `focus_handle` is actually focused
                    // (checked internally by `handle_input`) — installs this
                    // editor as the platform's text input target so typing
                    // and IME composition reach `input_handler.rs` instead of
                    // (or before falling through to) `.on_key_down`.
                    window.handle_input(&focus_handle, ElementInputHandler::new(bounds, editor_entity.clone()), cx);
                },
            )
            .size_full(),
        )
}

/// Registers this frame's mouse listeners: press to focus and place the
/// cursor (collapsing any selection), drag (move while the button that
/// pressed inside these bounds is still down) to extend it, release to stop
/// dragging. `window.on_mouse_event` listeners last one frame each — this
/// runs on every paint, same as the painting itself.
#[allow(clippy::too_many_arguments)]
fn register_mouse_handling(
    editor_entity: Entity<Editor>,
    text: String,
    line_roles: Vec<LineRole>,
    block_contexts: Vec<LineContext>,
    table_for_line: Vec<Option<usize>>,
    tables: Vec<Table>,
    cursor: usize,
    focused: bool,
    bounds: Bounds<Pixels>,
    line_height: Pixels,
    font: Font,
    font_size: Pixels,
    text_color: Hsla,
    focus_handle: FocusHandle,
    window: &mut Window,
) {
    window.on_mouse_event({
        let editor_entity = editor_entity.clone();
        let text = text.clone();
        let line_roles = line_roles.clone();
        let block_contexts = block_contexts.clone();
        let table_for_line = table_for_line.clone();
        let tables = tables.clone();
        let font = font.clone();
        move |event: &MouseDownEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble
                || event.button != MouseButton::Left
                || !bounds.contains(&event.position)
            {
                return;
            }
            let offset = offset_for_position(
                &text, &line_roles, &block_contexts, &table_for_line, &tables, cursor, focused, event.position,
                bounds, line_height, &font, font_size, text_color, window,
            );
            editor_entity.update(cx, |editor, cx| {
                editor.buffer_mut().set_cursor(offset, false);
                editor.is_dragging = true;
                cx.notify();
            });
            focus_handle.focus(window, cx);
        }
    });

    window.on_mouse_event({
        let editor_entity = editor_entity.clone();
        move |event: &MouseMoveEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble || !editor_entity.read(cx).is_dragging {
                return;
            }
            let offset = offset_for_position(
                &text, &line_roles, &block_contexts, &table_for_line, &tables, cursor, focused, event.position,
                bounds, line_height, &font, font_size, text_color, window,
            );
            editor_entity.update(cx, |editor, cx| {
                editor.buffer_mut().set_cursor(offset, true);
                cx.notify();
            });
        }
    });

    window.on_mouse_event(move |event: &MouseUpEvent, phase, _window, cx| {
        if phase != DispatchPhase::Bubble || event.button != MouseButton::Left {
            return;
        }
        editor_entity.update(cx, |editor, cx| {
            if editor.is_dragging {
                editor.is_dragging = false;
                cx.notify();
            }
        });
    });
}

/// The byte offset closest to `position` (window coordinates): the line
/// whose accumulated vertical extent `position`'s `y` falls into (no
/// longer a plain division by `line_height`, now that a wrapped line's own
/// height can be a multiple of it — see `shape_display_line`'s
/// `wrap_width_for`), then the closest char boundary within it — in its
/// *display* text, i.e. accounting for any Live Preview markup hidden on a
/// non-active line — via `WrappedLineLayout::closest_index_for_position`
/// (through `WrappedLine`'s `Deref`, now taking a full `Point` rather than
/// a bare `x`, since which *row* of a wrapped line matters too), mapped
/// back to a raw buffer offset via `DisplayLine::raw_offset_for_display`.
/// Still only reshapes each line once while searching for the target (same
/// "cheap enough, no cross-frame shape cache needed" reasoning as before)
/// rather than needing some separately persisted per-line height table.
#[allow(clippy::too_many_arguments)]
fn offset_for_position(
    text: &str,
    line_roles: &[LineRole],
    block_contexts: &[LineContext],
    table_for_line: &[Option<usize>],
    tables: &[Table],
    cursor: usize,
    focused: bool,
    position: Point<Pixels>,
    bounds: Bounds<Pixels>,
    line_height: Pixels,
    font: &Font,
    font_size: Pixels,
    text_color: Hsla,
    window: &mut Window,
) -> usize {
    let relative_y = (position.y - bounds.origin.y).max(px(0.));

    let mut line_start = 0usize;
    let mut y_offset = px(0.);
    for (i, line_text) in text.split('\n').enumerate() {
        let line_range = line_start..line_start + line_text.len();
        let active = is_active_line(&line_range, cursor, focused);
        let table_kind = table_line_kind(tables, table_for_line, i);
        let table_active = is_table_active(tables, table_for_line, i, cursor, focused);
        let results = shape_display_line(
            line_text, line_roles[i], block_contexts[i], table_kind, table_active, active, font, font_size, text_color,
            bounds.size.width, window,
        );
        // Usually one entry; more than one only for a table row whose
        // cells had to wrap, each entry then its own (unwrapped, since
        // already sized to fit) sub-row of that row — same click-then-find
        // the right sub-row logic, just one level deeper.
        let multi_entry = results.len() > 1;
        for (wrapped, display_line) in &results {
            let height = wrapped.size(line_height).height;
            if relative_y < y_offset + height {
                let relative_x = (position.x - bounds.origin.x - indent_px(display_line.indent.level())).max(px(0.));
                let local_y = (relative_y - y_offset).max(px(0.));
                let display_offset = match wrapped.closest_index_for_position(point(relative_x, local_y), line_height) {
                    Ok(ix) | Err(ix) => ix,
                };
                // A wrapped table sub-row has no genuinely visible segment
                // to land on (every cell's text is `is_replaced()` there,
                // see `build_table_row`'s own doc comment) — only
                // `nearest_segment_raw_start` handles that case correctly,
                // `raw_offset_for_display` would always fall through to
                // this line's very end regardless of where it was clicked.
                let raw_offset = if multi_entry {
                    display_line.nearest_segment_raw_start(display_offset)
                } else {
                    display_line.raw_offset_for_display(display_offset)
                };
                return line_start + raw_offset;
            }
            y_offset += height;
        }
        line_start = line_range.end + 1;
    }
    // Past the last line (clicked below all the text): land at the very end.
    text.len()
}

/// Accumulated while `paint_lines` walks one table's lines, top to bottom —
/// everything `paint_table_grid_lines` needs, gathered once `top`/`column_x`
/// are known (the table's header row) and finalized once `bottom` is known
/// (the table's own last line), at which point it's painted and dropped.
struct TablePaint {
    top: Pixels,
    /// The y to paint the header/body divider line at — `None` until the
    /// delimiter row is reached (always set by the time painting actually
    /// happens, since every table `classify_tables` produces has one).
    divider_y: Option<Pixels>,
    /// Pixel x-position of every vertical grid line this table needs: the
    /// left outer edge, one between each pair of adjacent columns, and the
    /// right outer edge — converted once, from `table_preview::column_
    /// divider_char_offsets`, when the header row is reached, and reused
    /// unchanged for every other line (a table's columns don't change
    /// width partway through it).
    column_x: Vec<Pixels>,
}

/// Paints one table's grid as real vector lines — the outer box, a vertical
/// divider between every pair of columns, and the header/body divider —
/// rather than relying on box-drawing glyphs (`┌`/`│`/`┼`/etc.) shaped and
/// painted as ordinary text. A monospace font's glyphs don't reliably abut
/// pixel-for-pixel across separately-shaped/painted lines or at junctions,
/// which reads as visible seams/gaps between them; a quad painted directly
/// at the right pixel position has no such seam regardless of font/hinting
/// — the same reasoning `paint_lines`'s own blockquote quote-bar already
/// uses a painted quad for, rather than a vertical-bar character.
fn paint_table_grid_lines(paint: &TablePaint, bottom: Pixels, window: &mut Window) {
    for &x in &paint.column_x {
        window.paint_quad(fill(Bounds { origin: point(x, paint.top), size: size(TABLE_LINE_WIDTH, bottom - paint.top) }, MUTED_COLOR));
    }
    let left = paint.column_x.first().copied().unwrap_or(px(0.));
    let right = paint.column_x.last().copied().unwrap_or(px(0.));
    let width = right - left;
    window.paint_quad(fill(Bounds { origin: point(left, paint.top), size: size(width, TABLE_LINE_WIDTH) }, MUTED_COLOR));
    window.paint_quad(fill(
        Bounds { origin: point(left, bottom - TABLE_LINE_WIDTH), size: size(width, TABLE_LINE_WIDTH) },
        MUTED_COLOR,
    ));
    if let Some(divider_y) = paint.divider_y {
        window.paint_quad(fill(Bounds { origin: point(left, divider_y), size: size(width, TABLE_LINE_WIDTH) }, MUTED_COLOR));
    }
}

#[allow(clippy::too_many_arguments)]
fn paint_lines(
    text: &str,
    line_roles: &[LineRole],
    block_contexts: &[LineContext],
    table_for_line: &[Option<usize>],
    tables: &[Table],
    cursor: usize,
    selection: Option<std::ops::Range<usize>>,
    show_cursor: bool,
    bounds: Bounds<Pixels>,
    line_height: Pixels,
    font: &Font,
    font_size: Pixels,
    text_color: Hsla,
    window: &mut Window,
    cx: &mut App,
) {
    let mut line_start = 0usize;
    let mut y_offset = px(0.);
    // Accumulated across a table's own lines, then painted and cleared
    // once its last line is reached — see `TablePaint`/`paint_table_grid_
    // lines`. Never started at all for a table that's rendering raw
    // (`table_active`), matching every other construct's own "border
    // disappears while active" behavior.
    let mut table_paint: Option<TablePaint> = None;
    for (i, line_text) in text.split('\n').enumerate() {
        let line_range = line_start..line_start + line_text.len();
        let active = is_active_line(&line_range, cursor, show_cursor);
        let table_kind = table_line_kind(tables, table_for_line, i);
        let table_active = is_table_active(tables, table_for_line, i, cursor, show_cursor);

        let results = shape_display_line(
            line_text, line_roles[i], block_contexts[i], table_kind, table_active, active, font, font_size, text_color,
            bounds.size.width, window,
        );
        // Usually one entry; more than one only for a table row whose
        // cells had to wrap (see `shape_display_line`'s own doc comment).
        let total_height: Pixels = results.iter().map(|(wrapped, _)| wrapped.size(line_height).height).sum();
        // Every entry of one raw line shares the same indent (a table's is
        // always `None` across all of them, same as the common one-entry
        // case) — fine to read it off the first.
        let indent_level = results[0].1.indent.level();
        let line_origin = bounds.origin + point(indent_px(indent_level), y_offset);
        // The width a wrapped row should be highlighted to when a
        // selection continues past it onto the next row — the same budget
        // `wrap_width_for` gave this line to wrap within.
        let full_row_width = bounds.size.width - indent_px(indent_level);

        // A thematic break (`---`/`***`/`___`) renders as a real vector
        // line, same "quad over a blank `hidden_line` gap" technique a
        // table's own grid lines use — a monospace rule glyph repeated
        // across the line wouldn't reliably abut pixel-for-pixel either.
        // Only for a `LineRole::Plain` line outside any table (matching
        // `shape_display_line`'s own gating) and not the active line (which
        // renders the raw `---` text instead, same as every other construct).
        // `table_kind` was already moved into `shape_display_line` above —
        // cheap to resolve again rather than restructuring this loop just
        // to keep the first value alive (same reasoning as
        // `table_kind_for_geometry` below).
        if !active
            && matches!(line_roles[i], LineRole::Plain)
            && matches!(table_line_kind(tables, table_for_line, i), TableLineKind::None)
            && is_thematic_break(line_text)
        {
            let mid_y = bounds.origin.y + y_offset + total_height / 2.;
            window.paint_quad(fill(
                Bounds { origin: point(bounds.origin.x, mid_y), size: size(bounds.size.width, TABLE_LINE_WIDTH) },
                MUTED_COLOR,
            ));
        }

        if !table_active {
            // `table_kind` was already moved into `shape_display_line`
            // above — cheap to resolve again (a plain array lookup) rather
            // than restructuring this loop just to keep the first value
            // alive.
            let table_kind_for_geometry = table_line_kind(tables, table_for_line, i);
            if let TableLineKind::Row(table) | TableLineKind::Delimiter(table) = &table_kind_for_geometry {
                if table.is_first_row(i) {
                    let max_chars = table_row_char_budget(bounds.size.width, font, font_size, text_color, window);
                    let widths = table_preview::resolve_widths(&table.columns, max_chars);
                    let char_width = mono_char_width(font, font_size, text_color, window);
                    let column_x = table_preview::column_divider_char_offsets(&widths)
                        .into_iter()
                        .map(|offset| line_origin.x + char_width * offset as f32)
                        .collect();
                    table_paint = Some(TablePaint { top: bounds.origin.y + y_offset, divider_y: None, column_x });
                }
                if table.is_delimiter(i)
                    && let Some(paint) = table_paint.as_mut()
                {
                    paint.divider_y = Some(bounds.origin.y + y_offset + total_height / 2.);
                }
                if table.is_last_line(i)
                    && let Some(paint) = table_paint.take()
                {
                    paint_table_grid_lines(&paint, bounds.origin.y + y_offset + total_height, window);
                }
            }
        }

        if let LineIndent::Blockquote(depth) = results[0].1.indent {
            for level in 0..depth {
                let bar_x = bounds.origin.x + indent_px(level) + QUOTE_BAR_INSET;
                window.paint_quad(fill(
                    Bounds { origin: point(bar_x, bounds.origin.y + y_offset), size: size(QUOTE_BAR_WIDTH, total_height) },
                    MUTED_COLOR,
                ));
            }
        }

        if results.len() > 1 {
            // A wrapped table row — see `shape_display_line`'s own doc
            // comment on why this sub-row split can't carry a byte-exact
            // raw↔display mapping the way a regular wrapped paragraph
            // does, which rules out the per-character selection/cursor
            // placement below. Pragmatic fallback: highlight the row's
            // *whole* combined block if the selection touches it at all,
            // and always place the caret on its first sub-row.
            if let Some(selection) = &selection
                && selection.start.max(line_range.start) < selection.end.min(line_range.end)
            {
                window.paint_quad(fill(
                    Bounds { origin: line_origin, size: size(full_row_width, total_height) },
                    SELECTION_HIGHLIGHT,
                ));
            }
            let mut row_y = y_offset;
            for (wrapped, _) in &results {
                let _ = wrapped.paint(bounds.origin + point(indent_px(indent_level), row_y), line_height, TextAlign::Left, None, window, cx);
                row_y += wrapped.size(line_height).height;
            }
            if show_cursor && cursor >= line_range.start && cursor <= line_range.end {
                window.paint_quad(fill(
                    Bounds { origin: line_origin, size: size(px(1.), line_height) },
                    text_color,
                ));
            }
        } else {
            let (wrapped, display_line) = &results[0];

            if let Some(selection) = &selection {
                let start = selection.start.max(line_range.start);
                let end = selection.end.min(line_range.end);
                if start < end {
                    let start_pos = wrapped
                        .position_for_index(display_line.display_offset_for_raw(start - line_start), line_height)
                        .unwrap_or(point(px(0.), px(0.)));
                    let end_pos = wrapped
                        .position_for_index(display_line.display_offset_for_raw(end - line_start), line_height)
                        .unwrap_or(point(full_row_width, total_height - line_height));
                    let start_row = (start_pos.y / line_height) as i32;
                    let end_row = (end_pos.y / line_height) as i32;
                    if start_row == end_row {
                        window.paint_quad(fill(
                            Bounds { origin: line_origin + start_pos, size: size(end_pos.x - start_pos.x, line_height) },
                            SELECTION_HIGHLIGHT,
                        ));
                    } else {
                        // Spans more than one wrapped row of this same
                        // line: the first row highlights from the
                        // selection's start to the row's own right edge,
                        // the last row from the row's left edge to the
                        // selection's end, and any rows in between are
                        // highlighted in full.
                        for row in start_row..=end_row {
                            let (x0, x1) = if row == start_row {
                                (start_pos.x, full_row_width)
                            } else if row == end_row {
                                (px(0.), end_pos.x)
                            } else {
                                (px(0.), full_row_width)
                            };
                            if x1 > x0 {
                                window.paint_quad(fill(
                                    Bounds {
                                        origin: line_origin + point(x0, line_height * row as f32),
                                        size: size(x1 - x0, line_height),
                                    },
                                    SELECTION_HIGHLIGHT,
                                ));
                            }
                        }
                    }
                }
            }

            let _ = wrapped.paint(line_origin, line_height, TextAlign::Left, None, window, cx);

            if show_cursor && cursor >= line_range.start && cursor <= line_range.end {
                let pos = wrapped
                    .position_for_index(display_line.display_offset_for_raw(cursor - line_start), line_height)
                    .unwrap_or(point(px(0.), px(0.)));
                window.paint_quad(fill(
                    Bounds { origin: line_origin + pos, size: size(px(1.), line_height) },
                    text_color,
                ));
            }
        }

        y_offset += total_height;
        line_start = line_range.end + 1; // +1 for the '\n' this line was split on
    }
}

// Deliberately not `use super::*`/`use gpui_kit::*` here: with `test-support`
// enabled, that glob also re-exports GPUI's `test` attribute macro, which then
// shadows plain `#[test]` and makes `#[gpui_kit::test]`'s own expansion
// recurse infinitely (see the project-wide note on this in `main.rs`).
#[cfg(test)]
mod tests {
    use super::super::block_preview::LineRole;
    use super::super::live_preview::{LineContext, LineIndent};
    use super::super::table_preview::classify_tables;
    use super::super::{Editor, InsertTab};
    use super::{is_active_line, shape_display_line, TableLineKind};
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{
        AnyWindowHandle, AppContext, Entity, EntityInputHandler, KeyBinding, Pixels, Point,
        TestAppContext, WindowOptions, point, px,
    };

    fn open_editor(cx: &mut TestAppContext, text: &'static str) -> (AnyWindowHandle, Entity<Editor>) {
        cx.update(gpui_kit::init);
        cx.update(|cx| {
            // Must out-rank `gpui_kit::init`'s own global "tab" binding —
            // see `Editor::handle_insert_tab`'s doc comment.
            cx.bind_keys([KeyBinding::new("tab", InsertTab, Some("TextEditor"))]);
            gpui_kit::open_window(WindowOptions::default(), cx, move |_window, cx| {
                cx.new(|cx| Editor::new(text, cx))
            })
            .expect("open test window")
        })
    }

    #[gpui_kit::test]
    fn renders_without_panicking_for_various_buffer_states(cx: &mut TestAppContext) {
        for text in ["", "hello", "line1\nline2\nline3", "trailing newline\n"] {
            let (window, _editor) = open_editor(cx, text);
            cx.update_window(window, |_, window, cx| {
                window.render_frame(cx);
                assert!(window.try_find("editor").is_some(), "text={text:?}");
            })
            .unwrap();
        }
    }

    #[gpui_kit::test]
    fn height_scales_with_line_count(cx: &mut TestAppContext) {
        let (one_line_window, _editor) = open_editor(cx, "just one line");
        let one_line_height = cx
            .update_window(one_line_window, |_, window, cx| {
                window.render_frame(cx);
                window.find("editor").bounds().size.height
            })
            .unwrap();

        let (three_line_window, _editor) = open_editor(cx, "line1\nline2\nline3");
        let three_line_height = cx
            .update_window(three_line_window, |_, window, cx| {
                window.render_frame(cx);
                window.find("editor").bounds().size.height
            })
            .unwrap();

        assert_eq!(three_line_height, one_line_height * 3., "three lines should be exactly 3x as tall as one");
    }

    #[gpui_kit::test]
    fn shape_display_line_wraps_at_the_given_width(cx: &mut TestAppContext) {
        // A direct, low-level check of the wrapping mechanism itself
        // (`shape_display_line`'s `wrap_width_for` dispatch into
        // `WindowTextSystem::shape_text`), independent of the *rest* of
        // `main.rs`'s `scrolling_away_from_the_cursor_does_not_get_snapped_back`-
        // style tests, which check the surrounding sizing/autoscroll
        // plumbing instead. A bare `Editor` has no tracked ancestor to
        // measure wrapping against for its *own* div sizing (see
        // `main.rs`'s `a_line_too_wide_for_the_pane_wraps_instead_of_overflowing`,
        // which needs the real app for that reason) — but `shape_display_line`
        // itself just takes whatever `available_width` it's given, so
        // calling it directly sidesteps that entirely.
        let (window, _editor) = open_editor(cx, "");
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            let text_style = window.text_style();
            let font = text_style.font();
            let font_size = text_style.font_size.to_pixels(window.rem_size());
            let line = "word ".repeat(50);
            let line_height = window.line_height();

            let (unwrapped, _) = {
            let mut results = shape_display_line(
                &line, LineRole::Plain, LineContext::Plain, TableLineKind::None, false, false, &font, font_size,
                text_style.color, Pixels::MAX, window,
            );
            assert_eq!(results.len(), 1, "expected exactly one entry");
            results.remove(0)
        };
            assert_eq!(
                unwrapped.size(line_height).height, line_height,
                "an effectively unbounded width should never wrap"
            );

            let (wrapped, _) = {
            let mut results = shape_display_line(
                &line, LineRole::Plain, LineContext::Plain, TableLineKind::None, false, false, &font, font_size,
                text_style.color, px(100.), window,
            );
            assert_eq!(results.len(), 1, "expected exactly one entry");
            results.remove(0)
        };
            assert!(
                wrapped.size(line_height).height > line_height,
                "a line far wider than a 100px budget should wrap onto more than one row"
            );
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn shape_display_line_never_gpui_wraps_a_table_row_even_at_a_narrow_width(cx: &mut TestAppContext) {
        // GPUI's own character-level line-wrapping is never applied to a
        // table row's *entries* (`wrap_width_for`'s table exception) —
        // doing that would break column alignment. A row too wide for a
        // narrow budget instead splits into multiple *entries* (one per
        // sub-row of wrapped cell content, see
        // `shape_display_line_wraps_a_table_rows_cells_within_their_own_column`),
        // each of which still individually measures exactly one
        // `line_height` tall.
        let text = "| Name | Age |\n| --- | --- |\n| Alice | 30 |";
        let (window, _editor) = open_editor(cx, "");
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            let text_style = window.text_style();
            let font = text_style.font();
            let font_size = text_style.font_size.to_pixels(window.rem_size());
            let line_height = window.line_height();
            let (_, tables) = classify_tables(text);
            let table = &tables[0];

            let results = shape_display_line(
                "| Alice | 30 |", LineRole::Plain, LineContext::Plain, TableLineKind::Row(table), false, false, &font,
                font_size, text_style.color, px(10.), window,
            );
            for (wrapped, _) in &results {
                assert_eq!(
                    wrapped.size(line_height).height, line_height,
                    "no individual entry should itself further (GPUI-level) wrap"
                );
            }
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn shape_display_line_wraps_a_table_rows_cells_within_their_own_column(cx: &mut TestAppContext) {
        // The actual fix: a cell too wide for the editor's own width wraps
        // *within its column* (multiple `shape_display_line` entries for
        // the one raw row) rather than letting the row overflow past the
        // editor's right edge.
        let text = "| Name | Bio |\n| --- | --- |\n| Alice | A very long biography that does not fit |";
        let (window, _editor) = open_editor(cx, "");
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            let text_style = window.text_style();
            let font = text_style.font();
            let font_size = text_style.font_size.to_pixels(window.rem_size());
            let line_height = window.line_height();
            let (_, tables) = classify_tables(text);
            let table = &tables[0];
            let row = "| Alice | A very long biography that does not fit |";

            let narrow = shape_display_line(
                row, LineRole::Plain, LineContext::Plain, TableLineKind::Row(table), false, false, &font, font_size,
                text_style.color, px(150.), window,
            );
            assert!(narrow.len() > 1, "a cell too wide for a 150px budget should split into multiple sub-rows");

            // None of those sub-rows' own rendered width should exceed the
            // budget they were wrapped to fit.
            for (wrapped, _) in &narrow {
                assert!(
                    wrapped.size(line_height).width <= px(150.),
                    "a wrapped sub-row must not itself overflow the width budget, got {:?}",
                    wrapped.size(line_height).width
                );
            }

            let wide = shape_display_line(
                row, LineRole::Plain, LineContext::Plain, TableLineKind::Row(table), false, false, &font, font_size,
                text_style.color, Pixels::MAX, window,
            );
            assert_eq!(wide.len(), 1, "an effectively unbounded width should never need to wrap a cell");
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn selecting_across_a_wrap_boundary_does_not_panic(cx: &mut TestAppContext) {
        let long_text: &'static str = Box::leak("word ".repeat(300).into_boxed_str());
        let (window, editor) = open_editor(cx, long_text);
        editor.update(cx, |editor, _| {
            editor.buffer_mut().set_cursor(0, false);
            editor.buffer_mut().set_cursor(long_text.len(), true);
        });
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("editor").is_some());
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn renders_with_a_selection_without_panicking(cx: &mut TestAppContext) {
        let (window, editor) = open_editor(cx, "hello world");
        editor.update(cx, |editor, _| {
            editor.buffer_mut().set_cursor(0, false);
            editor.buffer_mut().set_cursor(5, true);
        });
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("editor").is_some());
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn clicking_focuses_the_editor_and_places_the_cursor_near_the_click(cx: &mut TestAppContext) {
        let (window, editor) = open_editor(cx, "hello world");

        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            // Near the left edge, not exactly at x=0 — target_position's own
            // click-at-center default would otherwise land mid-string.
            window.click_at("editor", point(px(1.), px(1.)), cx);
        })
        .unwrap();

        let focused = cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.find("editor").focused()
        }).unwrap();
        assert_eq!(focused, Some(true), "clicking the editor should focus it");

        let cursor = editor.read_with(cx, |editor, _| editor.buffer().cursor());
        assert!(cursor <= 2, "clicking near the start should place the cursor near byte 0, got {cursor}");
    }

    #[gpui_kit::test]
    fn dragging_selects_a_range_of_text(cx: &mut TestAppContext) {
        let (window, editor) = open_editor(cx, "hello world");

        let bounds = cx
            .update_window(window, |_, window, cx| {
                window.render_frame(cx);
                window.find("editor").bounds()
            })
            .unwrap();
        let from: Point<Pixels> = bounds.origin + point(px(1.), px(1.));
        let to: Point<Pixels> = bounds.origin + point(bounds.size.width - px(1.), px(1.));

        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.drag(from, to, cx);
        })
        .unwrap();

        let selection = editor.read_with(cx, |editor, _| editor.buffer().selection());
        let selection = selection.expect("dragging across the text should select some of it");
        assert!(selection.start < selection.end);
        assert!(
            selection.end - selection.start >= "hello world".len() / 2,
            "dragging nearly the whole width should select most of the text, got {selection:?}"
        );
    }

    /// Clicks the editor to focus it, placing the cursor wherever the click
    /// lands — a specific position isn't needed since every test below moves
    /// it explicitly afterward (Home/End/select-all) before asserting.
    fn focus_editor(window: &mut gpui_kit::Window, cx: &mut gpui_kit::App) {
        window.render_frame(cx);
        window.click("editor", cx);
    }

    #[gpui_kit::test]
    fn typing_inserts_text_at_the_cursor(cx: &mut TestAppContext) {
        let (window, editor) = open_editor(cx, "");
        cx.update_window(window, |_, window, cx| {
            focus_editor(window, cx);
            window.input("hello", cx);
        })
        .unwrap();

        let text = editor.read_with(cx, |editor, _| editor.buffer().text().to_string());
        assert_eq!(text, "hello");
    }

    #[gpui_kit::test]
    fn backspace_deletes_the_previous_character(cx: &mut TestAppContext) {
        let (window, editor) = open_editor(cx, "hello");
        cx.update_window(window, |_, window, cx| {
            focus_editor(window, cx);
            window.press("end", cx);
            window.press("backspace", cx);
        })
        .unwrap();

        let text = editor.read_with(cx, |editor, _| editor.buffer().text().to_string());
        assert_eq!(text, "hell");
    }

    #[gpui_kit::test]
    fn home_and_shift_right_select_from_the_line_start(cx: &mut TestAppContext) {
        let (window, editor) = open_editor(cx, "hello");
        cx.update_window(window, |_, window, cx| {
            focus_editor(window, cx);
            window.press("end", cx);
            window.press("home", cx);
            window.press("shift-right", cx);
            window.press("shift-right", cx);
        })
        .unwrap();

        let selection = editor.read_with(cx, |editor, _| editor.buffer().selection());
        assert_eq!(selection, Some(0..2));
    }

    #[gpui_kit::test]
    fn enter_inserts_a_newline(cx: &mut TestAppContext) {
        let (window, editor) = open_editor(cx, "hello");
        cx.update_window(window, |_, window, cx| {
            focus_editor(window, cx);
            window.press("end", cx);
            window.press("enter", cx);
            window.input("world", cx);
        })
        .unwrap();

        let text = editor.read_with(cx, |editor, _| editor.buffer().text().to_string());
        assert_eq!(text, "hello\nworld");
    }

    #[gpui_kit::test]
    fn tab_inserts_a_tab_character_instead_of_moving_focus_away(cx: &mut TestAppContext) {
        let (window, editor) = open_editor(cx, "hello");
        cx.update_window(window, |_, window, cx| {
            focus_editor(window, cx);
            window.press("end", cx);
            window.press("tab", cx);
        })
        .unwrap();

        let text = editor.read_with(cx, |editor, _| editor.buffer().text().to_string());
        assert_eq!(text, "hello\t", "tab should insert a tab character, not fall through to a no-op");

        let focused = cx
            .update_window(window, |_, window, cx| {
                window.render_frame(cx);
                window.find("editor").focused()
            })
            .unwrap();
        assert_eq!(focused, Some(true), "tab must not move focus out of the editor");
    }

    #[gpui_kit::test]
    fn ctrl_a_selects_everything(cx: &mut TestAppContext) {
        let (window, editor) = open_editor(cx, "hello world");
        cx.update_window(window, |_, window, cx| {
            focus_editor(window, cx);
            window.press("ctrl-a", cx);
        })
        .unwrap();

        let selection = editor.read_with(cx, |editor, _| editor.buffer().selection());
        assert_eq!(selection, Some(0.."hello world".len()));
    }

    #[gpui_kit::test]
    fn typing_replaces_an_active_selection(cx: &mut TestAppContext) {
        let (window, editor) = open_editor(cx, "hello world");
        cx.update_window(window, |_, window, cx| {
            focus_editor(window, cx);
            window.press("ctrl-a", cx);
            window.input("x", cx);
        })
        .unwrap();

        let text = editor.read_with(cx, |editor, _| editor.buffer().text().to_string());
        assert_eq!(text, "x");
    }

    #[gpui_kit::test]
    fn copy_then_paste_duplicates_the_selection(cx: &mut TestAppContext) {
        let (window, editor) = open_editor(cx, "hello world");
        cx.update_window(window, |_, window, cx| {
            focus_editor(window, cx);
            window.press("ctrl-a", cx);
            window.press("ctrl-c", cx);
            window.press("end", cx);
            window.input(" ", cx);
            window.press("ctrl-v", cx);
        })
        .unwrap();

        let text = editor.read_with(cx, |editor, _| editor.buffer().text().to_string());
        assert_eq!(text, "hello world hello world");
    }

    #[gpui_kit::test]
    fn cut_removes_the_selection_and_copies_it(cx: &mut TestAppContext) {
        let (window, editor) = open_editor(cx, "hello world");
        cx.update_window(window, |_, window, cx| {
            focus_editor(window, cx);
            window.press("ctrl-a", cx);
            window.press("ctrl-x", cx);
        })
        .unwrap();

        let text = editor.read_with(cx, |editor, _| editor.buffer().text().to_string());
        assert_eq!(text, "", "cut should remove the selected text");

        cx.update_window(window, |_, window, cx| {
            window.press("ctrl-v", cx);
        })
        .unwrap();
        let text = editor.read_with(cx, |editor, _| editor.buffer().text().to_string());
        assert_eq!(text, "hello world", "pasting should bring the cut text back");
    }

    #[gpui_kit::test]
    fn ctrl_z_undoes_the_last_keystrokes_edit(cx: &mut TestAppContext) {
        // `window.input` dispatches one character per keystroke, and each
        // character arrives through `EntityInputHandler::replace_text_in_range`
        // as its own `buffer.insert` call — so, per this editor's documented
        // "basic" (non-coalescing) undo, one ctrl-z only undoes the single
        // last-typed character, not the whole burst of typing.
        let (window, editor) = open_editor(cx, "hello");
        cx.update_window(window, |_, window, cx| {
            focus_editor(window, cx);
            window.press("end", cx);
            window.input(" world", cx);
            window.press("ctrl-z", cx);
        })
        .unwrap();

        let text = editor.read_with(cx, |editor, _| editor.buffer().text().to_string());
        assert_eq!(text, "hello worl", "ctrl-z should undo only the last-typed character");
    }

    #[gpui_kit::test]
    fn ctrl_z_then_ctrl_shift_z_round_trips_a_pasted_insert(cx: &mut TestAppContext) {
        // Pasted text arrives in one `buffer.insert` call, so (unlike typing,
        // see the test above) it's a single undo/redo step.
        let (window, editor) = open_editor(cx, "hello");
        cx.update_window(window, |_, window, cx| {
            focus_editor(window, cx);
            window.press("end", cx);
            window.press("ctrl-a", cx);
            window.press("ctrl-c", cx);
            window.press("end", cx);
            window.press("ctrl-v", cx);
            window.press("ctrl-z", cx);
        })
        .unwrap();
        let text = editor.read_with(cx, |editor, _| editor.buffer().text().to_string());
        assert_eq!(text, "hello", "ctrl-z should undo the whole pasted insert in one step");

        cx.update_window(window, |_, window, cx| {
            window.press("ctrl-shift-z", cx);
        })
        .unwrap();
        let text = editor.read_with(cx, |editor, _| editor.buffer().text().to_string());
        assert_eq!(text, "hellohello", "ctrl-shift-z should redo the paste");
    }

    #[gpui_kit::test]
    fn committing_a_non_combining_ime_composition_does_not_duplicate_the_marked_character(
        cx: &mut TestAppContext,
    ) {
        // Reproduces a real dead-key sequence (e.g. "^" or "`" on layouts where
        // they're dead keys): the platform IME first shows a provisional marked
        // preview via `replace_and_mark_text_in_range`, then — once the next
        // keystroke doesn't combine with it (e.g. pressing space) — commits via
        // `replace_text_in_range` with `range_utf16: None`. That commit must
        // replace the marked preview, not insert next to it.
        let (window, editor) = open_editor(cx, "");
        cx.update_window(window, |_, window, cx| {
            focus_editor(window, cx);
            editor.update(cx, |editor, cx| {
                editor.replace_and_mark_text_in_range(None, "^", None, window, cx);
            });
        })
        .unwrap();

        let marked_preview = editor.read_with(cx, |editor, _| editor.buffer().text().to_string());
        assert_eq!(marked_preview, "^", "the dead-key preview should appear immediately");

        cx.update_window(window, |_, window, cx| {
            editor.update(cx, |editor, cx| {
                editor.replace_text_in_range(None, "^", window, cx);
            });
        })
        .unwrap();

        let text = editor.read_with(cx, |editor, _| editor.buffer().text().to_string());
        assert_eq!(text, "^", "committing the composition must replace the preview, not duplicate it");
    }

    #[gpui_kit::test]
    fn ctrl_right_jumps_to_the_end_of_the_current_word(cx: &mut TestAppContext) {
        let (window, editor) = open_editor(cx, "hello world");
        cx.update_window(window, |_, window, cx| {
            focus_editor(window, cx);
            editor.update(cx, |editor, _| editor.buffer_mut().set_cursor(0, false));
            window.press("ctrl-right", cx);
        })
        .unwrap();

        let cursor = editor.read_with(cx, |editor, _| editor.buffer().cursor());
        assert_eq!(cursor, 5, "ctrl-right should stop at the end of \"hello\"");
    }

    #[gpui_kit::test]
    fn ctrl_left_jumps_to_the_start_of_the_current_word(cx: &mut TestAppContext) {
        let (window, editor) = open_editor(cx, "hello world");
        cx.update_window(window, |_, window, cx| {
            focus_editor(window, cx);
            editor.update(cx, |editor, _| editor.buffer_mut().set_cursor(9, false));
            window.press("ctrl-left", cx);
        })
        .unwrap();

        let cursor = editor.read_with(cx, |editor, _| editor.buffer().cursor());
        assert_eq!(cursor, 6, "ctrl-left should stop at the start of \"world\"");
    }

    #[gpui_kit::test]
    fn ctrl_shift_right_extends_the_selection_by_a_word(cx: &mut TestAppContext) {
        let (window, editor) = open_editor(cx, "hello world");
        cx.update_window(window, |_, window, cx| {
            focus_editor(window, cx);
            editor.update(cx, |editor, _| editor.buffer_mut().set_cursor(0, false));
            window.press("ctrl-shift-right", cx);
        })
        .unwrap();

        let selection = editor.read_with(cx, |editor, _| editor.buffer().selection());
        assert_eq!(selection, Some(0..5), "ctrl-shift-right should select through the end of \"hello\"");
    }

    #[gpui_kit::test]
    fn an_unfocused_freshly_opened_editor_still_previews_markup(cx: &mut TestAppContext) {
        let text = "# Heading";
        let (window, editor) = open_editor(cx, text);
        // Never focused — `EditorBuffer::new`'s cursor defaults to byte 0,
        // which sits inside this (only) line's range. Before gating
        // activation on focus too, that alone made the line "active" and
        // rendered it raw even with nothing being edited: opening a note
        // (without clicking into its editor) left its markup unrendered.
        let heading_text_start = text.find("Heading").unwrap();

        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            // Near the left edge; the "# " marker should be hidden, so this
            // should land right after it, not at raw byte 0.
            window.click_at("editor", point(px(1.), px(1.)), cx);
        })
        .unwrap();

        let cursor = editor.read_with(cx, |editor, _| editor.buffer().cursor());
        assert_eq!(
            cursor, heading_text_start,
            "a freshly opened, unfocused editor should still preview markup, hiding the heading's \"# \""
        );
    }

    #[gpui_kit::test]
    fn renders_live_preview_constructs_without_panicking(cx: &mut TestAppContext) {
        let text = "# Heading\nsome **bold** and *italic* and `code`\nsee [[Other Note|here]]\nactive line";
        let (window, editor) = open_editor(cx, text);
        // Cursor on the last line, so every other line renders previewed
        // (hidden markup) rather than raw.
        editor.update(cx, |editor, _| editor.buffer_mut().set_cursor(text.len(), false));
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("editor").is_some());
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn renders_the_new_spec_gap_fill_constructs_without_panicking(cx: &mut TestAppContext) {
        let text = concat!(
            "escaped \\*star\\* and entity &amp; &#39;\n",
            "---\n",
            "a [link](https://example.com) and <https://example.com> and bare https://example.com\n",
            "- [ ] todo\n",
            "- [x] done\n",
            "active line"
        );
        let (window, editor) = open_editor(cx, text);
        editor.update(cx, |editor, _| editor.buffer_mut().set_cursor(text.len(), false));
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("editor").is_some());
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn clicking_a_previewed_heading_lands_past_its_hidden_marker_and_activates_it(cx: &mut TestAppContext) {
        let text = "first line\n# Heading";
        let (window, editor) = open_editor(cx, text);
        // Cursor starts on line 0, so line 1 ("# Heading") renders previewed
        // (its "# " hidden, showing just "Heading").
        editor.update(cx, |editor, _| editor.buffer_mut().set_cursor(0, false));

        let line_height = cx
            .update_window(window, |_, window, cx| {
                window.render_frame(cx);
                window.line_height()
            })
            .unwrap();

        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            // Near the left edge of the second rendered row.
            window.click_at("editor", point(px(1.), line_height + px(1.)), cx);
        })
        .unwrap();

        let cursor = editor.read_with(cx, |editor, _| editor.buffer().cursor());
        let heading_text_start = text.find("Heading").unwrap();
        assert_eq!(
            cursor, heading_text_start,
            "clicking near the start of the previewed heading should land right after its hidden \"# \", not before it"
        );

        // Having moved the cursor onto that line, a re-render should now
        // show it raw rather than previewed.
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
        })
        .unwrap();
        // The click also focused the editor (covered separately by
        // `clicking_focuses_the_editor_and_places_the_cursor_near_the_click`).
        let active = editor.read_with(cx, |editor, _| is_active_line(&(11..text.len()), editor.buffer().cursor(), true));
        assert!(active, "the line the cursor landed on should now be active");
    }

    #[gpui_kit::test]
    fn clicking_a_previewed_list_item_lands_past_its_hidden_marker(cx: &mut TestAppContext) {
        let text = "first line\n- list item";
        let (window, editor) = open_editor(cx, text);
        // Cursor starts on line 0, so line 1 ("- list item") renders
        // previewed: its "- " hidden behind a bullet glyph.
        editor.update(cx, |editor, _| editor.buffer_mut().set_cursor(0, false));

        let line_height = cx
            .update_window(window, |_, window, cx| {
                window.render_frame(cx);
                window.line_height()
            })
            .unwrap();

        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            // Near the left edge of the second rendered row, i.e. on the
            // bullet glyph itself.
            window.click_at("editor", point(px(1.), line_height + px(1.)), cx);
        })
        .unwrap();

        let cursor = editor.read_with(cx, |editor, _| editor.buffer().cursor());
        let item_text_start = text.find("list item").unwrap();
        assert_eq!(
            cursor, item_text_start,
            "clicking the bullet glyph should land right after the hidden \"- \" marker, not before it"
        );
    }

    #[gpui_kit::test]
    fn blockquote_indentation_shifts_the_whole_line_right(cx: &mut TestAppContext) {
        let text = "first line\n> quoted content";
        let (window, editor) = open_editor(cx, text);
        // Cursor starts on line 0, so line 1 renders previewed: its "> " is
        // hidden entirely (zero display width), unlike a list marker's
        // replacement bullet — so without the indent fix, the shaped text
        // would start right at the row's left edge.
        editor.update(cx, |editor, _| editor.buffer_mut().set_cursor(0, false));

        let line_height = cx
            .update_window(window, |_, window, cx| {
                window.render_frame(cx);
                window.line_height()
            })
            .unwrap();

        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            // x=15 sits inside the blockquote's indent gutter (< INDENT_UNIT,
            // 20px) but, with no hidden-prefix width to absorb it the way a
            // list's bullet does, would already land partway into "quoted
            // content"'s shaped text without the indent shift.
            window.click_at("editor", point(px(15.), line_height + px(1.)), cx);
        })
        .unwrap();

        let cursor = editor.read_with(cx, |editor, _| editor.buffer().cursor());
        let content_start = text.find("quoted").unwrap();
        assert_eq!(
            cursor, content_start,
            "clicking inside the indent gutter should land at the very start of the quoted content"
        );
    }

    #[gpui_kit::test]
    fn renders_nested_lists_and_lazy_continuation_without_panicking(cx: &mut TestAppContext) {
        // A wiring smoke test for `classify_block_context`'s `Vec<LineContext>`
        // being indexed in step with `text.split('\n')` and `line_roles` —
        // an off-by-one there would panic (index out of bounds) rather than
        // just render wrong, so this alone is a meaningful regression net
        // on top of `live_preview`'s own exhaustive pure-logic tests.
        let text = "1. outer\n   - inner\n     still inner, lazy continuation\n> quoted\nlazy quote continuation";
        let (window, editor) = open_editor(cx, text);
        editor.update(cx, |editor, _| editor.buffer_mut().set_cursor(text.len(), false));
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("editor").is_some());
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn a_nested_list_marker_indents_deeper_than_its_top_level_parent(cx: &mut TestAppContext) {
        let text = "first line\n1. outer\n   - inner";
        let (window, editor) = open_editor(cx, text);
        // Cursor starts on line 0, so both list lines render previewed.
        editor.update(cx, |editor, _| editor.buffer_mut().set_cursor(0, false));

        let line_height = cx
            .update_window(window, |_, window, cx| {
                window.render_frame(cx);
                window.line_height()
            })
            .unwrap();

        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            // x=35 sits inside the nested (depth-2) item's indent gutter
            // (< 2 * INDENT_UNIT, 40px) but *past* a top-level item's own
            // gutter (>= INDENT_UNIT, 20px) — if `classify_block_context`'s
            // resolved depth of 2 weren't actually reaching this line (e.g.
            // an indexing bug silently left it at `build_display_line`'s
            // own depth-1 default), this would land inside "inner"'s
            // shaped text instead of at its very start.
            window.click_at("editor", point(px(35.), line_height * 2. + px(1.)), cx);
        })
        .unwrap();

        let cursor = editor.read_with(cx, |editor, _| editor.buffer().cursor());
        let inner_start = text.find("inner").unwrap();
        assert_eq!(
            cursor, inner_start,
            "clicking inside the nested item's indent gutter should land at the start of its content"
        );
    }

    #[gpui_kit::test]
    fn renders_a_table_without_panicking(cx: &mut TestAppContext) {
        // A wiring smoke test, same spirit as `renders_nested_lists_and_
        // lazy_continuation_without_panicking` — `classify_tables`'
        // `table_for_line` must stay index-aligned with `text.split('\n')`
        // across `paint_lines`/`offset_for_position`, or this panics rather
        // than just rendering wrong.
        let text = "before\n\n| Name | Age |\n| --- | --- |\n| Alice | 30 |\n| Bob | 25 |\n\nafter";
        let (window, _editor) = open_editor(cx, text);
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("editor").is_some());
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn renders_a_table_with_no_body_rows_without_panicking(cx: &mut TestAppContext) {
        // The delimiter row is itself the table's last line here (no body
        // rows at all) — the one case `paint_lines`'s table-geometry
        // tracking finalizes (and paints the grid for) on a
        // `TableLineKind::Delimiter` line rather than a `Row` one.
        let text = "| Name | Age |\n| --- | --- |";
        let (window, _editor) = open_editor(cx, text);
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("editor").is_some());
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn renders_a_table_whose_last_row_wraps_without_panicking(cx: &mut TestAppContext) {
        // A very long cell forces wrapping regardless of the test window's
        // actual width — exercising `paint_lines`'s table-geometry tracking
        // finalizing the bottom border/divider from a *multi-entry* last
        // line's combined height, not just a plain single-entry one.
        let long_bio: &'static str =
            Box::leak(format!("| Name | Bio |\n| --- | --- |\n| Alice | {} |", "word ".repeat(100)).into_boxed_str());
        let (window, _editor) = open_editor(cx, long_bio);
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("editor").is_some());
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn a_table_renders_as_a_grid_until_the_cursor_enters_it_then_renders_raw(cx: &mut TestAppContext) {
        let text = "| Name | Age |\n| --- | --- |\n| Alice | 30 |";
        let (window, _editor) = open_editor(cx, text);
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            let text_style = window.text_style();
            let font = text_style.font();
            let font_size = text_style.font_size.to_pixels(window.rem_size());
            let (_, tables) = classify_tables(text);
            let table = &tables[0];
            let header = "| Name | Age |";

            let (_, grid) = {
            let mut results = shape_display_line(
                header, LineRole::Plain, LineContext::Plain, TableLineKind::Row(table), false, false, &font, font_size, text_style.color,
                Pixels::MAX, window,
            );
            assert_eq!(results.len(), 1, "expected exactly one entry");
            results.remove(0)
        };
            // Columns resolve to width 5 ("Alice" is the widest cell in
            // column 0) and width 3 ("Age"/"Age" is widest in column 1).
            // The blank gaps at the border positions aren't drawn as text —
            // `element.rs`'s `paint_table_grid_lines` paints real vector
            // lines over them instead, see that function's own doc comment.
            assert_eq!(grid.display_text, "  Name    Age  ");

            let (_, raw) = {
            let mut results = shape_display_line(
                header, LineRole::Plain, LineContext::Plain, TableLineKind::Row(table), true, false, &font, font_size, text_style.color,
                Pixels::MAX, window,
            );
            assert_eq!(results.len(), 1, "expected exactly one entry");
            results.remove(0)
        };
            assert_eq!(raw.display_text, header, "once the cursor is anywhere in the table, it must render raw");
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn clicking_near_the_left_edge_of_a_table_row_lands_at_its_first_cell(cx: &mut TestAppContext) {
        let text = "first line\n| Name | Age |\n| --- | --- |\n| Alice | 30 |";
        let (window, editor) = open_editor(cx, text);
        // Cursor starts on line 0, outside the table, so it renders as the
        // aligned grid rather than raw.
        editor.update(cx, |editor, _| editor.buffer_mut().set_cursor(0, false));

        let line_height = cx
            .update_window(window, |_, window, cx| {
                window.render_frame(cx);
                window.line_height()
            })
            .unwrap();

        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            // Near the left edge of the header row (line index 1) — within
            // the blank gap the real left-border line is painted over (see
            // `paint_table_grid_lines`), which should land at the start of
            // the first cell's own content, same "click near the start of
            // replaced markup lands past it" behavior every other construct
            // in this editor already has.
            window.click_at("editor", point(px(1.), line_height + px(1.)), cx);
        })
        .unwrap();

        let cursor = editor.read_with(cx, |editor, _| editor.buffer().cursor());
        let name_start = text.find("Name").unwrap();
        assert_eq!(cursor, name_start, "clicking near the table's left border should land at the first cell's content");
    }

    #[gpui_kit::test]
    fn fenced_code_content_is_never_inline_scanned_even_on_the_active_line(cx: &mut TestAppContext) {
        let (window, _editor) = open_editor(cx, "");
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            let line = "**not bold** [[not a wikilink]]";
            let text_style = window.text_style();
            let font = text_style.font();
            let font_size = text_style.font_size.to_pixels(window.rem_size());
            // `active: true` on its own would normally be enough to show a
            // line verbatim anyway (see `live_preview`'s own tests) — the
            // real thing this test guards is that `FenceContent` forces the
            // same literal treatment regardless of `active`, since a fenced
            // code line must never have its content inline-scanned at all,
            // active or not.
            let (_, display_line) = {
            let mut results = shape_display_line(
                line, LineRole::FenceContent, LineContext::Plain, TableLineKind::None, false, true, &font, font_size, text_style.color,
                Pixels::MAX, window,
            );
            assert_eq!(results.len(), 1, "expected exactly one entry");
            results.remove(0)
        };
            assert_eq!(display_line.display_text, line, "fenced code content must never be treated as Markdown");
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn fenced_code_content_starting_with_quote_or_list_syntax_is_never_indented(cx: &mut TestAppContext) {
        let (window, _editor) = open_editor(cx, "");
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            let text_style = window.text_style();
            let font = text_style.font();
            let font_size = text_style.font_size.to_pixels(window.rem_size());
            for line in ["> looks like a quote", "- looks like a list item"] {
                let (_, display_line) = {
            let mut results = shape_display_line(
                line, LineRole::FenceContent, LineContext::Plain, TableLineKind::None, false, false, &font, font_size, text_style.color,
                Pixels::MAX, window,
            );
            assert_eq!(results.len(), 1, "expected exactly one entry");
            results.remove(0)
        };
                assert_eq!(
                    display_line.indent,
                    LineIndent::None,
                    "fenced code content must never indent as a quote/list, even if it looks like one: {line:?}"
                );
            }
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn a_fence_delimiter_line_is_hidden_when_not_active(cx: &mut TestAppContext) {
        let (window, _editor) = open_editor(cx, "");
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            let line = "```rust";
            let text_style = window.text_style();
            let font = text_style.font();
            let font_size = text_style.font_size.to_pixels(window.rem_size());
            let (_, display_line) = {
            let mut results = shape_display_line(
                line, LineRole::FenceDelimiter, LineContext::Plain, TableLineKind::None, false, false, &font, font_size, text_style.color,
                Pixels::MAX, window,
            );
            assert_eq!(results.len(), 1, "expected exactly one entry");
            results.remove(0)
        };
            assert_eq!(display_line.display_text, "", "an inactive fence delimiter line should show nothing");

            let (_, active_display_line) = {
            let mut results = shape_display_line(
                line, LineRole::FenceDelimiter, LineContext::Plain, TableLineKind::None, false, true, &font, font_size, text_style.color,
                Pixels::MAX, window,
            );
            assert_eq!(results.len(), 1, "expected exactly one entry");
            results.remove(0)
        };
            assert_eq!(active_display_line.display_text, line, "the active fence delimiter line should show its raw text");
        })
        .unwrap();
    }
}
